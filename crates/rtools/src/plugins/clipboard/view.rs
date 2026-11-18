//! A chronological history with actions attached to each entry.

mod preview;

use super::{
    Error,
    history::{Selection, Summary},
    service::ClipboardState,
};
use crate::{
    settings::window::Page,
    shell::Shell,
    ui::{ButtonExt as _, Toolbar},
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use preview::Preview;
use rust_i18n::t;
use std::{sync::Arc, time::Duration};

actions!(
    clipboard_history,
    [
        PreviousClip,
        NextClip,
        ClosePreview,
        PreviousImage,
        NextImage
    ]
);
const CONTEXT: &str = "ClipboardHistory";
const PREVIEW_CONTEXT: &str = "ClipboardImagePreview";

pub(super) struct HistoryView {
    search: Entity<InputState>,
    rows: Vec<Summary>,
    selected: Option<i64>,
    preview: Option<Preview>,
    images_only: bool,
    revision: u64,
    scroll: UniformListScrollHandle,
    query_task: Task<()>,
    query_pending: bool,
    copy_after_query: bool,
    busy: bool,
    confirm_clear: bool,
    partial: Option<(i64, Vec<i64>)>,
    message: Option<String>,
    locale: String,
    _subscriptions: Vec<Subscription>,
}

impl HistoryView {
    /// Keeps history and gallery shortcuts local to their focused surface.
    pub fn bind_keys(cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new("up", PreviousClip, Some(CONTEXT)),
            KeyBinding::new("down", NextClip, Some(CONTEXT)),
            KeyBinding::new("escape", ClosePreview, Some(PREVIEW_CONTEXT)),
            KeyBinding::new("left", PreviousImage, Some(PREVIEW_CONTEXT)),
            KeyBinding::new("right", NextImage, Some(PREVIEW_CONTEXT)),
        ]);
    }

    /// Queries existing snapshots independently of the recording preference.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t!("clipboard.search")));
        let subscriptions = vec![
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.copy_after_query = false;
                    this.refresh(true, cx);
                }
                InputEvent::PressEnter { .. } => {
                    if this.query_pending {
                        // Resolve Enter against the text currently in the search field.
                        this.copy_after_query = !this.busy && !this.confirm_clear;
                    } else if let Some(id) = this.selected {
                        this.copy(id, Selection::All, cx);
                    }
                }
                _ => {}
            }),
            cx.on_release(|this, cx| {
                if let Some(preview) = &mut this.preview {
                    preview.unload(None, cx);
                }
            }),
            cx.observe_global::<ClipboardState>(|this, cx| {
                let revision = cx.global::<ClipboardState>().revision;
                if this.revision != revision {
                    this.revision = revision;
                    this.refresh(false, cx);
                }
                cx.notify();
            }),
        ];
        let mut view = Self {
            search,
            rows: Vec::new(),
            selected: None,
            preview: None,
            images_only: false,
            revision: cx.global::<ClipboardState>().revision,
            scroll: UniformListScrollHandle::new(),
            query_task: Task::ready(()),
            query_pending: true,
            copy_after_query: false,
            busy: false,
            confirm_clear: false,
            partial: None,
            message: None,
            locale: rust_i18n::locale().to_string(),
            _subscriptions: subscriptions,
        };
        view.refresh(false, cx);
        view
    }

    /// Replaces outdated queries and preserves selection by database ID across record changes.
    fn refresh(&mut self, debounce: bool, cx: &mut Context<Self>) {
        self.query_pending = true;
        let history = cx.global::<ClipboardState>().core.history.clone();
        let query = self.search.read(cx).value().to_string();
        let images = self.images_only;
        self.query_task = cx.spawn(async move |this, cx| {
            let bg = cx.background_executor().clone();
            if debounce {
                bg.timer(Duration::from_millis(65)).await;
            }
            let rows = bg
                .spawn(async move { history.search(&query, images) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.query_pending = false;
                let copy = std::mem::take(&mut this.copy_after_query);
                match rows {
                    Ok(rows) => {
                        let selected = this
                            .selected
                            .filter(|id| rows.iter().any(|r| r.id == *id))
                            .or_else(|| rows.first().map(|r| r.id));
                        if let Some(preview) = &mut this.preview
                            && !rows.iter().any(|r| r.id == preview.id)
                        {
                            // Another window or retention may remove the group while it is open.
                            preview.unload(None, cx);
                            preview.picture = Some(Err("clipboard.missing"));
                        }
                        this.rows = rows;
                        this.select(selected);
                        if copy && let Some(id) = selected {
                            this.copy(id, Selection::All, cx);
                        }
                    }
                    Err(err) => {
                        this.rows.clear();
                        this.select(None);
                        this.report(&err);
                    }
                }
                cx.notify();
            });
        });
    }

    /// Clears feedback belonging to the previous row without reading any image data.
    fn select(&mut self, id: Option<i64>) {
        if self.selected != id {
            self.selected = id;
            self.partial = None;
            self.message = None;
        }
    }

    /// Moves the selection and scrolls its row into view.
    fn navigate(&mut self, next: bool, cx: &mut Context<Self>) {
        if self.rows.is_empty() || self.query_pending || self.confirm_clear {
            return;
        }
        let index = self
            .rows
            .iter()
            .position(|r| Some(r.id) == self.selected)
            .unwrap_or(0);
        let index = if next {
            (index + 1).min(self.rows.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        self.select(Some(self.rows[index].id));
        cx.notify();
    }

    /// Shows a localized failure without logging clipboard data or filenames.
    fn report(&mut self, err: &Error) {
        tracing::warn!("clipboard history operation failed: {}", err);
        self.message = Some(t!(err.key()).to_string());
    }

    /// Publishes the clicked entry or member, reconfirming if a partial group's members change.
    fn copy(&mut self, id: i64, selection: Selection, cx: &mut Context<Self>) {
        if self.busy || self.confirm_clear {
            return;
        }
        self.busy = true;
        self.copy_after_query = false;
        self.select(Some(id));
        let confirmed = self
            .partial
            .take()
            .filter(|(entry, _)| *entry == id && selection == Selection::All)
            .map(|(_, members)| members);
        let core = Arc::clone(&cx.global::<ClipboardState>().core);
        let task =
            cx.background_spawn(async move { core.copy(id, selection, confirmed.as_deref()) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if this.selected == Some(id) {
                    match result {
                        Ok(()) => this.message = Some(t!("clipboard.copied").to_string()),
                        Err(Error::Partial(members)) => {
                            this.partial = Some((id, members));
                            this.message = Some(t!("clipboard.partial").to_string());
                        }
                        Err(err) => this.report(&err),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Removes the clicked entry, or every entry after explicit clear confirmation.
    fn remove(&mut self, id: Option<i64>, cx: &mut Context<Self>) {
        if self.busy || (self.confirm_clear && id.is_some()) {
            return;
        }
        self.busy = true;
        self.confirm_clear = false;
        self.copy_after_query = false;
        self.partial = None;
        self.message = None;
        let core = Arc::clone(&cx.global::<ClipboardState>().core);
        let task = cx.background_spawn(async move { core.remove(id) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Err(err) = result {
                    this.report(&err);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Keeps row actions visible when a long path or a multiline text entry is truncated.
    fn row(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let row = &self.rows[index];
        let id = row.id;
        let disabled = self.busy || self.confirm_clear;
        let title = row
            .title
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .to_string();
        let confirmed = self.partial.as_ref().filter(|(entry, _)| *entry == id);
        h_flex()
            .id(("clip", id as usize))
            .w_full()
            .h_12()
            .px_2()
            .gap_2()
            .when(self.selected == Some(id), |d| d.bg(cx.theme().secondary))
            .hover(|d| d.bg(cx.theme().muted))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(Some(id));
                this.copy_after_query = false;
                cx.notify();
            }))
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .child(div().truncate().child(title))
                    .when(row.total > 1 || row.missing > 0, |d| {
                        let count = if row.missing > 0 {
                            t!("clipboard.failed_count", count = row.missing).to_string()
                        } else if row.images == row.total {
                            t!("clipboard.image_count", count = row.images).to_string()
                        } else if row.images > 0 {
                            t!(
                                "clipboard.group",
                                images = row.images,
                                others = row.total - row.images
                            )
                            .to_string()
                        } else {
                            t!("clipboard.item_count", count = row.total).to_string()
                        };
                        d.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(count),
                        )
                    }),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .when(row.images > 0, |actions| {
                        let count = row.images;
                        let paths = row.has_paths;
                        actions.child(
                            Toolbar::button(("clip-preview", id as usize), IconName::Eye)
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.open_preview(id, count, paths, window, cx);
                                }))
                                .with_tooltip(t!("clipboard.preview")),
                        )
                    })
                    .when_some(confirmed, |actions, (_, members)| {
                        actions.child(
                            Button::new(("confirm-clip-copy", id as usize))
                                .small()
                                .label(t!("clipboard.copy_available", count = members.len()))
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.copy(id, Selection::All, cx);
                                })),
                        )
                    })
                    .when(confirmed.is_none(), |actions| {
                        actions.child(
                            Toolbar::button(("clip-copy", id as usize), IconName::Copy)
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.copy(id, Selection::All, cx);
                                }))
                                .with_tooltip(t!("clipboard.copy")),
                        )
                    })
                    .child(
                        Toolbar::button(("clip-delete", id as usize), IconName::Trash)
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.remove(Some(id), cx);
                            }))
                            .with_tooltip(t!("clipboard.delete")),
                    ),
            )
    }
}

impl Focusable for HistoryView {
    /// Restores focus to the active surface when the launcher reattaches this view.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.preview
            .as_ref()
            .map(|p| p.focus.clone())
            .unwrap_or_else(|| self.search.focus_handle(cx))
    }
}

impl Render for HistoryView {
    /// Renders the single-column history or the full-width image preview.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.locale != *rust_i18n::locale() {
            self.locale = rust_i18n::locale().to_string();
            self.search.update(cx, |s, cx| {
                s.set_placeholder(t!("clipboard.search"), window, cx)
            });
        }
        if let Some(preview) = &self.preview {
            return self.render_preview(preview, cx).into_any_element();
        }
        let muted = cx.theme().muted_foreground;
        let status = cx.global::<ClipboardState>().status();
        let rows = uniform_list(
            "clipboard-results",
            self.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|i| this.row(i, cx).into_any_element()).collect()
            }),
        )
        .track_scroll(&self.scroll)
        .w_full()
        .flex_1()
        .min_h_0();
        v_flex()
            .size_full()
            .min_h_0()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &PreviousClip, _, cx| this.navigate(false, cx)))
            .on_action(cx.listener(|this, _: &NextClip, _, cx| this.navigate(true, cx)))
            .child(
                h_flex()
                    .p_2()
                    .gap_2()
                    .child(Input::new(&self.search).flex_1())
                    .child(
                        Button::new("clip-images")
                            .small()
                            .label(t!("clipboard.images"))
                            .selected(self.images_only)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.images_only = !this.images_only;
                                this.copy_after_query = false;
                                this.refresh(false, cx);
                                cx.notify();
                            })),
                    ),
            )
            .when(self.rows.is_empty() && !self.query_pending, |d| {
                d.child(
                    div()
                        .p_2()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("clipboard.empty").to_string()),
                )
            })
            .child(rows)
            .when(self.confirm_clear, |d| {
                d.child(
                    h_flex()
                        .px_2()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .child(t!("clipboard.clear_detail").to_string()),
                        )
                        .child(
                            Button::new("cancel-clear-clips")
                                .small()
                                .label(t!("clipboard.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_clear = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("confirm-clear-clips")
                                .small()
                                .label(t!("clipboard.clear"))
                                .on_click(cx.listener(|this, _, _, cx| this.remove(None, cx))),
                        ),
                )
            })
            .child(
                Toolbar::default()
                    .child(
                        Toolbar::button("clip-settings", IconName::Settings)
                            .on_click(|_, _, cx| {
                                Shell::open_settings_page(Some(Page::Clipboard), cx)
                            })
                            .with_tooltip(t!("app.settings_title")),
                    )
                    .child(
                        Toolbar::button("clip-clear", IconName::Eraser)
                            .disabled(self.busy || self.confirm_clear)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_clear = true;
                                this.copy_after_query = false;
                                cx.notify();
                            }))
                            .with_tooltip(t!("clipboard.clear")),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(
                                self.message
                                    .clone()
                                    .unwrap_or_else(|| t!(status).to_string()),
                            ),
                    )
                    .when_some(
                        self.selected.and_then(|id| {
                            self.rows.iter().find(|r| {
                                r.id == id
                                    && r.has_paths
                                    && (r.missing > 0 || self.partial.is_some())
                            })
                        }),
                        |bar, row| {
                            let id = row.id;
                            bar.child(
                                Button::new("copy-clip-paths")
                                    .small()
                                    .label(t!("clipboard.copy_original_paths"))
                                    .disabled(self.busy || self.confirm_clear)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.copy(id, Selection::Paths, cx)
                                    })),
                            )
                        },
                    ),
            )
            .into_any_element()
    }
}
