//! The settings window (spec §4.6, §5.7): a Zed-style sidebar with search and
//! page navigation, and one row per setting with a fixed-width control lane.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_i18n::t;

use super::{FONT_SIZES, Language, Settings, SettingsStore};

actions!(settings_window, [CloseSettings]);

/// Key context of the settings window.
const CONTEXT: &str = "SettingsWindow";

/// Installs the settings window key bindings.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseSettings, Some(CONTEXT))]);
}

/// One option of a settings dropdown: its label and the value it stands for.
#[derive(Clone)]
struct Choice<V> {
    label: SharedString,
    value: V,
}

impl<V: Clone + PartialEq + 'static> SearchableListItem for Choice<V> {
    type Value = V;

    /// The label shown in the menu and the trigger.
    fn title(&self) -> SharedString {
        self.label.clone()
    }

    /// The setting value this option selects.
    fn value(&self) -> &V {
        &self.value
    }
}

/// Font dropdown options; `None` follows the theme.
type FontChoices = SearchableVec<Choice<Option<SharedString>>>;
/// Language dropdown options.
type LanguageChoices = Vec<Choice<Language>>;

/// A page of the sidebar navigation.
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Appearance,
    Language,
}

impl Page {
    /// Every page, in sidebar order.
    const ALL: [Page; 2] = [Page::Appearance, Page::Language];

    /// Translation keys of the page title and subtitle.
    fn keys(self) -> (&'static str, &'static str) {
        match self {
            Page::Appearance => ("settings.appearance", "settings.appearance_subtitle"),
            Page::Language => ("settings.language", "settings.language_subtitle"),
        }
    }
}

/// One setting row; rows are listed in display order.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    UiFont,
    MonoFont,
    FontSize,
    Language,
}

impl Row {
    /// Every row, in display order.
    const ALL: [Row; 4] = [Row::UiFont, Row::MonoFont, Row::FontSize, Row::Language];

    /// The page the row belongs to.
    fn page(self) -> Page {
        match self {
            Row::UiFont | Row::MonoFont | Row::FontSize => Page::Appearance,
            Row::Language => Page::Language,
        }
    }

    /// Translation keys of the row's section, title and description.
    fn keys(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Row::UiFont => (
                "settings.fonts",
                "settings.ui_font",
                "settings.ui_font_detail",
            ),
            Row::MonoFont => (
                "settings.fonts",
                "settings.mono_font",
                "settings.mono_font_detail",
            ),
            Row::FontSize => (
                "settings.scale",
                "settings.font_size",
                "settings.font_size_detail",
            ),
            Row::Language => (
                "settings.display",
                "settings.interface_language",
                "settings.interface_language_detail",
            ),
        }
    }

    /// Whether the localized title or description contains the lowercase `query`.
    fn matches(self, query: &str) -> bool {
        let (_, title, detail) = self.keys();
        t!(title).to_lowercase().contains(query) || t!(detail).to_lowercase().contains(query)
    }
}

/// Points a font dropdown at the setting it edits.
type FontField = fn(&mut Settings) -> &mut Option<String>;

/// The settings page: sidebar plus the rows of the current page or search.
pub struct SettingsWindow {
    focus: FocusHandle,
    search: Entity<InputState>,
    page: Page,
    ui_font: Entity<SelectState<FontChoices>>,
    mono_font: Entity<SelectState<FontChoices>>,
    font_size: Entity<InputState>,
    language: Entity<SelectState<LanguageChoices>>,
    /// Installed font families, sorted; enumerated once per window.
    font_names: Vec<SharedString>,
    /// The settings file as shown above the page, e.g. `~/.config/rtools/settings.toml`.
    path_label: SharedString,
    /// Locale the labels were built in; a change rebuilds the dropdown options.
    locale: SharedString,
    _subscriptions: Vec<Subscription>,
}

impl SettingsWindow {
    /// Builds the page from the settings in effect.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.global::<SettingsStore>();
        let font_size_value = store.settings.font_size.to_string();
        let path_label = match dirs::home_dir().and_then(|home| store.path.strip_prefix(home).ok())
        {
            Some(relative) => format!("~/{}", relative.display()),
            None => store.path.display().to_string(),
        }
        .into();
        let mut font_names = cx.text_system().all_font_names();
        font_names.sort_unstable();
        font_names.dedup();

        // Options, selections and placeholders are filled by `relabel` below.
        let search = cx.new(|cx| InputState::new(window, cx));
        let ui_font = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
        });
        let mono_font = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
        });
        let font_size = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(font_size_value)
                .step(1.)
                .min(f64::from(*FONT_SIZES.start()))
                .max(f64::from(*FONT_SIZES.end()))
        });
        let language = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));

        let font_fields: [(&Entity<SelectState<FontChoices>>, FontField); 2] = [
            (&ui_font, |settings| &mut settings.ui_font),
            (&mono_font, |settings| &mut settings.mono_font),
        ];
        let mut subscriptions: Vec<Subscription> = font_fields
            .into_iter()
            .map(|(state, field)| {
                cx.subscribe(state, move |_, _, event: &SelectEvent<FontChoices>, cx| {
                    if let SelectEvent::Confirm(Some(family)) = event {
                        let family = family.as_ref().map(ToString::to_string);
                        SettingsStore::update(cx, |settings| *field(settings) = family);
                    }
                })
            })
            .collect();
        subscriptions.extend([
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&font_size, |_, state, event: &InputEvent, cx| {
                // Partial input such as "1" while typing "18" is ignored until it is in range.
                if matches!(event, InputEvent::Change)
                    && let Ok(size) = state.read(cx).value().trim().parse::<f32>()
                    && FONT_SIZES.contains(&size)
                {
                    SettingsStore::update(cx, |settings| settings.font_size = size);
                }
            }),
            cx.subscribe(
                &language,
                |_, _, event: &SelectEvent<LanguageChoices>, cx| {
                    if let SelectEvent::Confirm(Some(language)) = event {
                        let language = *language;
                        SettingsStore::update(cx, |settings| settings.language = language);
                    }
                },
            ),
            cx.observe_global_in::<SettingsStore>(window, |this, window, cx| {
                this.relabel(window, cx);
            }),
        ]);

        let mut this = Self {
            focus: cx.focus_handle(),
            search,
            page: Page::Appearance,
            ui_font,
            mono_font,
            font_size,
            language,
            font_names: font_names.into_iter().map(Into::into).collect(),
            path_label,
            // No locale yet, so `relabel` fills everything in.
            locale: SharedString::default(),
            _subscriptions: subscriptions,
        };
        this.relabel(window, cx);
        this
    }

    /// Fills the localized dropdown options, their selections and the search
    /// placeholder whenever the locale differs from the one they were built in.
    fn relabel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let locale: SharedString = rust_i18n::locale().to_string().into();
        if locale == self.locale {
            return;
        }
        self.locale = locale;
        let settings = cx.global::<SettingsStore>().settings.clone();
        // "Theme default" first, then every installed family.
        let fonts: Vec<Choice<Option<SharedString>>> = std::iter::once(Choice {
            label: t!("settings.theme_font").into(),
            value: None,
        })
        .chain(self.font_names.iter().map(|name| Choice {
            label: name.clone(),
            value: Some(name.clone()),
        }))
        .collect();
        for (state, family) in [
            (&self.ui_font, settings.ui_font),
            (&self.mono_font, settings.mono_font),
        ] {
            let items = SearchableVec::new(fonts.clone());
            let family: Option<SharedString> = family.map(Into::into);
            state.update(cx, |state, cx| {
                state.set_items(items, window, cx);
                state.set_selected_value(&family, window, cx);
            });
        }
        let languages = [
            (Language::System, "settings.language_system"),
            (Language::ZhCn, "settings.language_zh_cn"),
            (Language::En, "settings.language_en"),
        ]
        .into_iter()
        .map(|(value, key)| Choice {
            label: t!(key).into(),
            value,
        })
        .collect();
        self.language.update(cx, |state, cx| {
            state.set_items(languages, window, cx);
            state.set_selected_value(&settings.language, window, cx);
        });
        self.search.update(cx, |state, cx| {
            state.set_placeholder(t!("settings.search"), window, cx);
        });
    }

    /// Shows a page, leaving any search.
    fn open_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.search
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    /// Opens the settings file in the system editor, writing it first if missing.
    fn edit_file(&mut self, cx: &mut Context<Self>) {
        let store = cx.global::<SettingsStore>();
        let path = store.path.clone();
        if !path.exists()
            && let Err(err) = store.settings.save(&path)
        {
            tracing::warn!("creating settings file {} failed: {}", path.display(), err);
            return;
        }
        cx.open_with_system(&path);
    }

    /// Closes the window (Esc).
    fn close(&mut self, _: &CloseSettings, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    /// The control lane element of a row.
    fn control(&self, row: Row, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let select_font = |state: &Entity<SelectState<FontChoices>>| {
            Select::new(state)
                .appearance(false)
                .icon(IconName::ChevronsUpDown)
                .menu_width(px(280.))
                .bg(theme.secondary)
                .rounded(theme.radius)
                .into_any_element()
        };
        match row {
            Row::UiFont => select_font(&self.ui_font),
            Row::MonoFont => select_font(&self.mono_font),
            Row::FontSize => NumberInput::new(&self.font_size)
                .appearance(false)
                .bg(theme.secondary)
                .rounded(theme.radius)
                .into_any_element(),
            Row::Language => Select::new(&self.language)
                .appearance(false)
                .icon(IconName::ChevronsUpDown)
                .bg(theme.secondary)
                .rounded(theme.radius)
                .into_any_element(),
        }
    }

    /// Sidebar: search box, page navigation and the Esc hint.
    fn render_sidebar(&self, searching: bool, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let nav = Page::ALL.map(|page| {
            let active = !searching && page == self.page;
            let (title, _) = page.keys();
            h_flex()
                .id(title)
                .h_8()
                .px_2()
                .gap_1p5()
                .rounded(theme.radius)
                .text_base()
                .cursor_pointer()
                .when(active, |this| this.bg(theme.secondary))
                .when(!active, |this| {
                    this.text_color(muted)
                        .hover(|style| style.bg(theme.secondary.opacity(0.5)))
                })
                .on_click(cx.listener(move |this, _, window, cx| this.open_page(page, window, cx)))
                .child(
                    Icon::new(IconName::ChevronRight)
                        .size_3p5()
                        .text_color(muted),
                )
                .child(t!(title).to_string())
        });
        v_flex()
            .w_64()
            .flex_none()
            .p_2p5()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(Input::new(&self.search).prefix(Icon::new(IconName::Search).text_color(muted)))
            .child(div().h_2())
            .children(nav)
            .child(div().flex_1())
            .child(
                h_flex()
                    .px_1()
                    .gap_2()
                    .child(Kbd::new(
                        Keystroke::parse("escape").expect("valid keystroke"),
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(t!("settings.close").to_string()),
                    ),
            )
    }

    /// File path row, page title and the visible setting rows.
    fn render_content(&self, query: &str, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let rows: Vec<Row> = Row::ALL
            .into_iter()
            .filter(|row| {
                if query.is_empty() {
                    row.page() == self.page
                } else {
                    row.matches(query)
                }
            })
            .collect();
        let (title, subtitle) = if query.is_empty() {
            let (title, subtitle) = self.page.keys();
            (t!(title), Some(t!(subtitle)))
        } else {
            (t!("settings.search_results"), None)
        };

        let mut body = v_flex();
        let mut last_section = None;
        for row in &rows {
            let (section, row_title, detail) = row.keys();
            if last_section != Some(section) {
                last_section = Some(section);
                body = body.child(
                    div()
                        .pt_6()
                        .pb_2()
                        .border_b_1()
                        .border_color(border)
                        .text_sm()
                        .text_color(muted)
                        .child(t!(section).to_string()),
                );
            }
            body = body.child(
                h_flex()
                    .py_5()
                    .gap_6()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().text_base().child(t!(row_title).to_string()))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(t!(detail).to_string()),
                            ),
                    )
                    .child(div().w_56().flex_none().child(self.control(*row, cx))),
            );
        }
        if rows.is_empty() {
            body = body.child(
                div()
                    .pt_10()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("settings.no_match").to_string()),
            );
        }

        v_flex()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .px_8()
            .pt_5()
            .pb_8()
            .overflow_y_scroll()
            .child(
                h_flex()
                    .h_7()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme.mono_font_family.clone())
                            .text_sm()
                            .text_color(muted)
                            .child(self.path_label.clone()),
                    )
                    .child(
                        Button::new("edit-file")
                            .outline()
                            .small()
                            .label(t!("settings.edit_file").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.edit_file(cx))),
                    ),
            )
            .child(
                v_flex()
                    .pt_5()
                    .gap_0p5()
                    .child(div().text_lg().font_medium().child(title.to_string()))
                    .when_some(subtitle, |this, subtitle| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child(subtitle.to_string()),
                        )
                    }),
            )
            .child(body)
    }
}

impl Focusable for SettingsWindow {
    /// The page itself takes focus so Esc reaches it.
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsWindow {
    /// Client-side title bar above the sidebar and the content.
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().trim().to_lowercase();
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::close))
            .child(TitleBar::new())
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.render_sidebar(!query.is_empty(), cx))
                    .child(self.render_content(&query, cx)),
            )
    }
}
