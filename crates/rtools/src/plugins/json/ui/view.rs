//! The JSON editor view: source editor, `this` filter bar, toolbar and result pane.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, Escape, Input, InputEvent, InputState};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_i18n::t;
use serde_json::Value;

use crate::plugin::Launch;
use crate::plugins::json::core::convert::xml::AsXml;
use crate::plugins::json::core::{
    Draft, FILTER_TIMEOUT, FilterEngine, Json, JsonError, Normalized, Rewrite,
};
use crate::settings::SettingsStore;

/// Pause after the last filter keystroke before evaluating.
const FILTER_DEBOUNCE: Duration = Duration::from_millis(200);
/// Pause after the last source edit before validating and re-filtering.
const SYNC_DEBOUNCE: Duration = Duration::from_millis(300);
/// Pause after the last source edit before writing the draft.
const SAVE_DEBOUNCE: Duration = Duration::from_secs(1);

/// What the result pane shows.
enum ResultPane {
    Hidden,
    /// The latest filter outcome, `JsonPluginView::filtered`.
    Filter,
    Xml(Result<SharedString, JsonError>),
}

/// The JSON plugin's root view.
pub struct JsonPluginView {
    left: Entity<EditorState>,
    right: Entity<EditorState>,
    filter: Entity<InputState>,
    engine: Option<FilterEngine>,
    draft: Arc<Draft>,
    pane: ResultPane,
    /// Outcome of the active filter; `None` while the filter is empty.
    filtered: Option<Result<SharedString, JsonError>>,
    doc_error: Option<JsonError>,
    /// Counts source edits; background results computed from an older
    /// revision are dropped instead of overwriting newer edits.
    revision: u64,
    /// Background output waiting to replace the source, with the revision it
    /// was computed from; applied by `sync_editors`.
    pending_source: Option<(u64, String)>,
    /// The result pane changed and the read-only editor still shows old text.
    right_dirty: bool,
    /// Soft wrap as last applied to both editors.
    soft_wrap: bool,
    /// The source changed since the draft was last written.
    unsaved: bool,
    filter_task: Task<()>,
    /// Replacing it cancels the previous evaluation, so only the newest result lands.
    eval_task: Task<()>,
    sync_task: Task<()>,
    save_task: Task<()>,
    /// Background paste normalization or toolbar rewrite.
    rewrite_task: Task<()>,
    xml_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl JsonPluginView {
    /// Builds the view; content comes from the launch input, else the draft.
    pub(super) fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = Arc::new(Draft::new(&launch.data_dir));
        // Launch input takes the whole-document paste path below, so the editor
        // starts empty rather than with the draft it replaces.
        let content = match launch.input {
            Some(_) => String::new(),
            None => draft
                .load()
                .unwrap_or_else(|err| {
                    tracing::warn!("reading the JSON draft failed: {}", err);
                    None
                })
                .unwrap_or_default(),
        };
        let engine = FilterEngine::spawn()
            .map_err(|err| tracing::error!("starting the filter engine failed: {}", err))
            .ok();

        let soft_wrap = cx.global::<SettingsStore>().settings.json.soft_wrap;
        // Both editors open the built-in search panel on Ctrl+F (Cmd+F); the
        // panel offers replace (also Ctrl+H) only where the text is editable.
        let editor = |window: &mut Window, cx: &mut Context<EditorState>| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .soft_wrap(soft_wrap)
                .searchable(true)
        };
        let left = cx.new(|cx| editor(window, cx).default_value(content));
        let right = cx.new(|cx| editor(window, cx));
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("json.filter_placeholder")));
        left.update(cx, |state, cx| state.focus(window, cx));

        let subscriptions = vec![
            cx.subscribe_in(&left, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.on_source_changed(cx);
                }
            }),
            cx.subscribe_in(&filter, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.on_filter_changed(cx);
                }
            }),
            // Redraws so `render` picks up a changed soft-wrap preference.
            cx.observe_global::<SettingsStore>(|_, cx| cx.notify()),
            // Pending edits must reach the draft even when the app quits or the
            // view goes away before the save debounce fires.
            cx.on_app_quit(|this, cx| {
                this.flush_draft(cx);
                async {}
            }),
            cx.on_release(|this, cx| this.flush_draft(cx)),
        ];

        let mut view = Self {
            left,
            right,
            filter,
            engine,
            draft,
            pane: ResultPane::Hidden,
            filtered: None,
            doc_error: None,
            revision: 0,
            pending_source: None,
            right_dirty: false,
            soft_wrap,
            unsaved: false,
            filter_task: Task::ready(()),
            eval_task: Task::ready(()),
            sync_task: Task::ready(()),
            save_task: Task::ready(()),
            rewrite_task: Task::ready(()),
            xml_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        match launch.input {
            Some(input) => view.load_pasted(input, cx),
            None => view.schedule_sync(Duration::ZERO, cx),
        }
        view
    }

    /// Current filter expression.
    fn filter_text(&self, cx: &App) -> SharedString {
        self.filter.read(cx).value()
    }

    /// Reacts to a source edit: save later, re-validate and re-filter after a pause.
    fn on_source_changed(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        self.schedule_save(cx);
        if matches!(self.pane, ResultPane::Xml(_)) {
            self.show(ResultPane::Hidden, cx);
        }
        self.schedule_sync(SYNC_DEBOUNCE, cx);
        cx.notify();
    }

    /// Writes the draft after a pause; failures are logged and never touch the editor.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.unsaved = true;
        let draft = Arc::clone(&self.draft);
        self.save_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            // Cloning the rope is cheap; the text is built on the background thread.
            let Ok(rope) = this.update(cx, |this, cx| {
                this.unsaved = false;
                this.left.read(cx).text().clone()
            }) else {
                return;
            };
            let saved = cx
                .background_executor()
                .spawn(async move { draft.save(&rope.to_string()) })
                .await;
            if let Err(err) = saved {
                tracing::warn!("saving the JSON draft failed: {}", err);
            }
        });
    }

    /// Writes a pending draft right away (app quit, view released).
    fn flush_draft(&mut self, cx: &mut App) {
        if !self.unsaved {
            return;
        }
        self.unsaved = false;
        self.save_task = Task::ready(());
        if let Err(err) = self.draft.save(&self.left.read(cx).text().to_string()) {
            tracing::warn!("saving the JSON draft failed: {}", err);
        }
    }

    /// After `delay`, validates the source and hands it to the filter engine off
    /// the UI thread, then refreshes an active filter; a newer call supersedes it.
    fn schedule_sync(&mut self, delay: Duration, cx: &mut Context<Self>) {
        self.sync_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(rope) = this.update(cx, |this, cx| this.left.read(cx).text().clone()) else {
                return;
            };
            let (text, error) = cx
                .background_executor()
                .spawn(async move {
                    let text: Arc<str> = rope.to_string().into();
                    let error = Json(&text).validate().err();
                    (text, error)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.doc_error = error;
                if let Some(engine) = &this.engine {
                    engine.set_document(text);
                }
                if !this.filter_text(cx).trim().is_empty() {
                    this.run_filter(cx);
                }
                cx.notify();
            });
        });
    }

    /// Debounces filter keystrokes; an empty filter hides its result.
    fn on_filter_changed(&mut self, cx: &mut Context<Self>) {
        if self.filter_text(cx).trim().is_empty() {
            self.filter_task = Task::ready(());
            self.eval_task = Task::ready(());
            self.filtered = None;
            if matches!(self.pane, ResultPane::Filter) {
                self.show(ResultPane::Hidden, cx);
            }
            return;
        }
        self.filter_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FILTER_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| this.run_filter(cx));
        });
    }

    /// Evaluates the filter and shows the outcome.
    fn run_filter(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = &self.engine else {
            self.filtered = Some(Err(JsonError::EngineStopped));
            self.show(ResultPane::Filter, cx);
            return;
        };
        let pending = engine.evaluate(&self.filter_text(cx));
        self.eval_task = cx.spawn(async move |this, cx| {
            let result = pending.await.map(SharedString::from);
            let _ = this.update(cx, |this, cx| {
                this.filtered = Some(result);
                this.show(ResultPane::Filter, cx);
            });
        });
    }

    /// The result the pane shows, if any.
    fn shown_result(&self) -> Option<&Result<SharedString, JsonError>> {
        match &self.pane {
            ResultPane::Hidden => None,
            ResultPane::Filter => self.filtered.as_ref(),
            ResultPane::Xml(result) => Some(result),
        }
    }

    /// Replaces the result pane; the read-only editor follows in `sync_editors`.
    fn show(&mut self, pane: ResultPane, cx: &mut Context<Self>) {
        self.pane = pane;
        self.right_dirty = true;
        cx.notify();
    }

    /// Brings both editors up to date with the view: the soft-wrap preference,
    /// background output for the source and the result pane. Runs deferred
    /// from `render`, the one place that knows the view's current window: the
    /// view outlives popup windows, so a task cannot hold one.
    fn sync_editors(&mut self, soft_wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        if soft_wrap != self.soft_wrap {
            self.soft_wrap = soft_wrap;
            for editor in [&self.left, &self.right] {
                editor.update(cx, |state, cx| state.set_soft_wrap(soft_wrap, window, cx));
            }
        }
        // Output computed from an older revision would undo the edits made since.
        if let Some((revision, text)) = self.pending_source.take()
            && revision == self.revision
        {
            self.left
                .update(cx, |state, cx| state.replace_all(text, window, cx));
        }
        if std::mem::take(&mut self.right_dirty) {
            let text = match self.shown_result() {
                Some(Ok(text)) => text.clone(),
                _ => SharedString::default(),
            };
            let language = match self.pane {
                ResultPane::Xml(_) => "xml",
                ResultPane::Hidden | ResultPane::Filter => "json",
            };
            self.right.update(cx, |state, cx| {
                // Switched before the text arrives, so it is parsed only once.
                if state.language_name() != language {
                    state.set_highlighter(language, cx);
                }
                state.set_value(text, window, cx);
            });
        }
    }

    /// Computes a replacement for the whole source off the UI thread. It lands
    /// as one undoable edit unless the source is edited meanwhile; a failure is
    /// shown in the status strip.
    fn replace_source(
        &mut self,
        job: impl FnOnce() -> Result<String, JsonError> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let revision = self.revision;
        self.rewrite_task = cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { job() }).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(text) => this.pending_source = Some((revision, text)),
                    Err(err) if this.revision == revision => this.doc_error = Some(err),
                    Err(_) => {}
                }
                cx.notify();
            });
        });
    }

    /// Runs a toolbar rewrite on the current source (see `replace_source`).
    fn rewrite(&mut self, rewrite: Rewrite, cx: &mut Context<Self>) {
        // Cloning the rope is cheap; the text is built on the background thread.
        let rope = self.left.read(cx).text().clone();
        self.replace_source(move || rewrite.apply(&rope.to_string()), cx);
    }

    /// Puts pasted text in place of the whole source: recognized formats become
    /// formatted JSON, anything else stays verbatim (see `replace_source`).
    fn load_pasted(&mut self, text: String, cx: &mut Context<Self>) {
        self.replace_source(
            move || {
                Ok(match text.parse::<Normalized>() {
                    Ok(Normalized(json)) => json,
                    Err(_) => text,
                })
            },
            cx,
        );
    }

    /// Converts the filter result (while a filter is active) or the source to
    /// XML off the UI thread; a failed filter is reported rather than silently
    /// replaced by the source, and a result for an edited source is dropped.
    fn convert_to_xml(&mut self, cx: &mut Context<Self>) {
        let filtered = self.filtered.clone();
        let rope = self.left.read(cx).text().clone();
        let revision = self.revision;
        self.xml_task = cx.spawn(async move |this, cx| {
            let xml = cx
                .background_executor()
                .spawn(async move {
                    let value = match filtered {
                        Some(Ok(text)) => Value::try_from(Json(&text)),
                        Some(Err(err)) => Err(err),
                        None => Value::try_from(Json(&rope.to_string())),
                    };
                    value.map(|value| AsXml(&value).to_string().into())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.revision == revision {
                    this.show(ResultPane::Xml(xml), cx);
                }
            });
        });
    }

    /// Esc closes the open search panels first; GPUI Kit's inputs would
    /// otherwise take Esc to move focus and leave a panel open. The panel sits
    /// beside the editor's text element, so focus in it does not count as
    /// inside the editor, hence no per-editor focus test.
    fn close_search(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        let mut closed = false;
        for editor in [&self.left, &self.right] {
            if editor.read(cx).search_session().open {
                editor.update(cx, |state, cx| state.close_search(cx));
                closed = true;
            }
        }
        if !closed {
            cx.propagate();
        }
    }

    /// Copies the result pane while it shows a result, otherwise the source.
    fn copy(&self, cx: &mut Context<Self>) {
        let text = match self.shown_result() {
            Some(Ok(text)) => text.to_string(),
            _ => self.left.read(cx).text().to_string(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Takes over a paste that replaces the whole document (empty or fully
    /// selected) and hands it to `load_pasted`; returns `false` for other
    /// pastes, which the editor inserts itself.
    fn paste_whole_document(&mut self, text: String, cx: &mut Context<Self>) -> bool {
        let state = self.left.read(cx);
        let len = state.text().len();
        if len != 0 && state.selected_range() != (0..len) {
            return false;
        }
        self.load_pasted(text, cx);
        true
    }

    /// Source editor with the parse-error strip below it.
    fn render_source(&self, cx: &Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .min_w_0()
            .child(
                div().flex_1().min_h_0().child(
                    Editor::new(&self.left)
                        .bordered(false)
                        .appearance(false)
                        .h_full()
                        .on_paste(move |item, _, cx| {
                            let Some(text) = item.text() else {
                                return false;
                            };
                            weak.update(cx, |this, cx| this.paste_whole_document(text, cx))
                                .unwrap_or(false)
                        }),
                ),
            )
            .when_some(self.doc_error.as_ref(), |this, err| {
                this.child(
                    h_flex()
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .border_t_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.danger)
                        .child(Icon::new(IconName::CircleAlert).size_3p5())
                        .child(err.brief()),
                )
            })
    }

    /// Result pane: the read-only editor, or an error state.
    fn render_result(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(result) = self.shown_result() else {
            return div().into_any_element();
        };
        let is_xml = matches!(self.pane, ResultPane::Xml(_));
        let theme = cx.theme();
        let body = match result {
            Ok(_) => div()
                .flex_1()
                .min_h_0()
                .child(
                    Editor::new(&self.right)
                        .bordered(false)
                        .appearance(false)
                        .readonly(true)
                        .h_full(),
                )
                .into_any_element(),
            Err(err) => {
                let (headline, detail) = match err {
                    _ if is_xml => (t!("json.xml_failed"), err.brief()),
                    JsonError::Timeout => (
                        t!("json.error.timeout"),
                        t!("json.timeout_detail", secs = FILTER_TIMEOUT.as_secs()).into_owned(),
                    ),
                    JsonError::EngineStopped => (t!("json.engine_unavailable"), err.brief()),
                    JsonError::Script(message) => (t!("json.expression_error"), message.clone()),
                    _ => (t!("json.expression_error"), err.brief()),
                };
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .px_6()
                    .child(
                        Icon::new(IconName::CircleAlert)
                            .size_5()
                            .text_color(theme.danger),
                    )
                    .child(div().text_sm().child(headline.to_string()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    )
                    .into_any_element()
            }
        };
        v_flex()
            .w_1_2()
            .min_w_0()
            .border_l_1()
            .border_color(theme.border)
            .child(body)
            .into_any_element()
    }

    /// Filter bar with the `this` prefix and the grouped icon toolbar.
    fn render_footer(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let prefix = div()
            .font_family(theme.mono_font_family.clone())
            .text_sm()
            .text_color(theme.muted_foreground)
            .child("this");
        let tool = |id: &'static str, icon: IconName, key: &'static str| {
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .tooltip(t!(key).to_string())
        };
        let rewrite = |id, icon, key, rewrite: Rewrite| {
            tool(id, icon, key)
                .on_click(cx.listener(move |this, _, _, cx| this.rewrite(rewrite, cx)))
        };
        h_flex()
            .px_2()
            .py_1p5()
            .gap_2()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.filter)
                        .small()
                        .appearance(false)
                        .bg(theme.secondary)
                        .rounded(theme.radius)
                        .prefix(prefix)
                        .font_family(theme.mono_font_family.clone()),
                ),
            )
            .child(
                h_flex()
                    .gap_0p5()
                    .child(rewrite(
                        "format",
                        IconName::ListIndentIncrease,
                        "json.format",
                        Rewrite::Format,
                    ))
                    .child(rewrite(
                        "minify",
                        IconName::FoldVertical,
                        "json.minify",
                        Rewrite::Minify,
                    ))
                    .child(Separator::vertical().h_4().mx_1())
                    .child(rewrite(
                        "escape",
                        IconName::Quote,
                        "json.escape",
                        Rewrite::Escape,
                    ))
                    .child(rewrite(
                        "unescape",
                        IconName::RemoveFormatting,
                        "json.unescape",
                        Rewrite::Unescape,
                    ))
                    .child(Separator::vertical().h_4().mx_1())
                    .child(
                        tool("xml", IconName::CodeXml, "json.to_xml")
                            .on_click(cx.listener(|this, _, _, cx| this.convert_to_xml(cx))),
                    )
                    .child(
                        tool("copy", IconName::Copy, "json.copy")
                            .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                    ),
            )
    }
}

impl Focusable for JsonPluginView {
    /// Keyboard input goes to the source editor.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.left.read(cx).focus_handle(cx)
    }
}

impl Render for JsonPluginView {
    /// Lays out source and result side by side above the filter bar.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let soft_wrap = cx.global::<SettingsStore>().settings.json.soft_wrap;
        if self.right_dirty || self.pending_source.is_some() || soft_wrap != self.soft_wrap {
            cx.defer_in(window, move |this, window, cx| {
                this.sync_editors(soft_wrap, window, cx);
            });
        }
        let show_result = self.shown_result().is_some();
        v_flex()
            .size_full()
            .capture_action(cx.listener(Self::close_search))
            .child(
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_source(cx))
                    .when(show_result, |this| this.child(self.render_result(cx))),
            )
            .child(self.render_footer(cx))
    }
}
