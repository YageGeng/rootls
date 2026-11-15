//! The bottom bars of the diff view: swap / clear / compare under the
//! editors, and counts, navigation, switches, layout and unfolding under the
//! diff (diff spec §3.1, §3.2).

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Enter;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::select::Select;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_i18n::t;

use super::{BackToEdit, COMPARE_CONTEXT, DiffPluginView, NextChange, Outcome, PreviousChange};
use crate::plugins::diff::core::{FormatJson, Side};
use crate::settings::{DiffLayout, DiffSettings, SettingsStore};
use crate::ui::{ButtonExt as _, INPUT_CONTEXT, Toolbar};

impl DiffPluginView {
    /// Swap / clear on the left, the compare button on the right; its key
    /// hint is the editor's own Ctrl+Enter (Cmd+Enter) binding.
    pub(super) fn render_edit_toolbar(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let compare = Enter {
            secondary: true,
            shift: false,
        };
        Toolbar::default()
            .child(
                h_flex()
                    .gap_0p5()
                    .child(
                        Toolbar::button("swap", IconName::ArrowLeftRight)
                            .on_click(cx.listener(|this, _, window, cx| this.swap(window, cx)))
                            .with_tooltip(t!("diff.swap")),
                    )
                    .child(
                        Toolbar::button("clear", IconName::Eraser)
                            .on_click(cx.listener(|this, _, window, cx| this.clear(window, cx)))
                            .with_tooltip(t!("diff.clear")),
                    )
                    .child(Toolbar::separator())
                    // Syntax colors of both editors and the diff: the guess, or a pick.
                    .child(
                        Select::new(&self.language_select)
                            .small()
                            .appearance(false)
                            .icon(IconName::ChevronsUpDown)
                            .menu_width(px(220.))
                            .w(rems(11.)),
                    ),
            )
            .child(div().flex_1())
            .child(
                Button::new("compare")
                    .primary()
                    .small()
                    .label(t!("diff.compare"))
                    .children(Kbd::binding_for_action(
                        &compare,
                        Some(INPUT_CONTEXT),
                        window,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.show_compare(window, cx))),
            )
    }

    /// Change counts and navigation, the comparison switches, then layout,
    /// unfolding and the way back to editing. Every control stays in place on
    /// every state, so a switch never moves under the pointer when its click
    /// turns the diff into a status page; navigation and unfolding are only
    /// disabled there.
    pub(super) fn render_compare_toolbar(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let settings = &cx.global::<SettingsStore>().settings.diff;
        let showing = self.showing_diff();
        let (additions, deletions) = self
            .summary
            .as_ref()
            .map_or((0, 0), |summary| (summary.additions, summary.deletions));
        // A switch flips one `[diff]` field; `apply_settings` does the rest.
        let switch = |id: &'static str,
                      icon: IconName,
                      key: &'static str,
                      on: bool,
                      field: fn(&mut DiffSettings) -> &mut bool| {
            Toolbar::button(id, icon)
                .selected(on)
                .toggled(on)
                .on_click(move |_, _, cx| {
                    SettingsStore::update(cx, |settings| {
                        let value = field(&mut settings.diff);
                        *value = !*value;
                    })
                })
                .with_tooltip(t!(key))
        };
        let unavailable = match self.summary.as_ref() {
            // The empty-side pages already say what is wrong.
            Some(summary)
                if !matches!(
                    summary.outcome,
                    Outcome::OriginalEmpty | Outcome::ModifiedEmpty | Outcome::BothEmpty
                ) =>
            {
                match summary.format_json {
                    FormatJson::Unavailable(side) => Some(side),
                    _ => None,
                }
            }
            _ => None,
        };
        let format_tip = match unavailable {
            Some(Side::Original) => "diff.format_json_unavailable_original",
            Some(Side::Modified) => "diff.format_json_unavailable_modified",
            None => "diff.format_json",
        };
        let (unfold_icon, unfold_tip) = if self.unchanged_expanded {
            (IconName::FoldVertical, "diff.collapse_unchanged")
        } else {
            (IconName::UnfoldVertical, "diff.expand_unchanged")
        };
        Toolbar::default()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_1p5()
                    .text_xs()
                    .child(
                        div()
                            .text_color(theme.success)
                            .child(format!("+{additions}")),
                    )
                    .child(
                        div()
                            .text_color(theme.danger)
                            .child(format!("\u{2212}{deletions}")),
                    ),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_0p5()
                    .child(Toolbar::separator())
                    .child(
                        Toolbar::button("previous", IconName::ChevronUp)
                            .disabled(!showing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.previous_change(&PreviousChange, window, cx)
                            }))
                            .with_key_tooltip(
                                t!("diff.previous"),
                                Some((&PreviousChange, COMPARE_CONTEXT)),
                            ),
                    )
                    .child(
                        Toolbar::button("next", IconName::ChevronDown)
                            .disabled(!showing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.next_change(&NextChange, window, cx)
                            }))
                            .with_key_tooltip(
                                t!("diff.next"),
                                Some((&NextChange, COMPARE_CONTEXT)),
                            ),
                    )
                    .child(Toolbar::separator()),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_0p5()
                    .child(switch(
                        "ignore-whitespace",
                        IconName::Space,
                        "diff.ignore_whitespace",
                        settings.ignore_whitespace,
                        |diff| &mut diff.ignore_whitespace,
                    ))
                    .child(switch(
                        "ignore-case",
                        IconName::CaseSensitive,
                        "diff.ignore_case",
                        settings.ignore_case,
                        |diff| &mut diff.ignore_case,
                    ))
                    .child(
                        switch(
                            "format-json",
                            IconName::Braces,
                            format_tip,
                            settings.format_json,
                            |diff| &mut diff.format_json,
                        )
                        // Still clickable so it can be switched off, but
                        // dimmed while one side is not JSON.
                        .when(unavailable.is_some(), |this| this.opacity(0.5)),
                    )
                    .child(switch(
                        "char-level",
                        IconName::Type,
                        "diff.char_level",
                        settings.char_level,
                        |diff| &mut diff.char_level,
                    )),
            )
            .child(div().flex_1())
            .child(
                TabBar::new("layout")
                    .segmented()
                    .small()
                    .flex_shrink_0()
                    .selected_index(match settings.layout {
                        DiffLayout::Split => 0,
                        DiffLayout::Unified => 1,
                    })
                    .child(Tab::new().label(t!("diff.split")))
                    .child(Tab::new().label(t!("diff.unified")))
                    .on_click(|index, _, cx| {
                        let layout = if *index == 0 {
                            DiffLayout::Split
                        } else {
                            DiffLayout::Unified
                        };
                        SettingsStore::update(cx, |settings| settings.diff.layout = layout);
                    }),
            )
            .child(
                Toolbar::button("unfold", unfold_icon)
                    .disabled(!showing)
                    .on_click(|_, _, cx| {
                        SettingsStore::update(cx, |settings| {
                            settings.diff.fold_unchanged = !settings.diff.fold_unchanged;
                        })
                    })
                    .with_tooltip(t!(unfold_tip))
                    .flex_shrink_0(),
            )
            .child(
                Button::new("edit")
                    .outline()
                    .small()
                    .flex_shrink_0()
                    .label(t!("diff.edit"))
                    .children(Kbd::binding_for_action(
                        &BackToEdit,
                        Some(COMPARE_CONTEXT),
                        window,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.back_to_edit(&BackToEdit, window, cx)
                    })),
            )
    }
}
