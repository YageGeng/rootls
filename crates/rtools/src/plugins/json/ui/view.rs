//! The JSON editor view: source editor, `this` filter bar, toolbar and result pane.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
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

/// Pause after the last filter keystroke before evaluating.
const FILTER_DEBOUNCE: Duration = Duration::from_millis(200);
/// Pause after the last source edit before validating and re-filtering.
const SYNC_DEBOUNCE: Duration = Duration::from_millis(300);
/// Pause after the last source edit before writing the draft.
const SAVE_DEBOUNCE: Duration = Duration::from_secs(1);

/// A successful result: cheap-to-clone text and its top-level item count.
struct Shown {
    text: SharedString,
    /// Set for filter results that are arrays.
    count: Option<usize>,
}

impl From<String> for Shown {
    /// Wraps filter output, counting top-level items when it is an array; run
    /// it off the UI thread, since counting parses the whole text.
    fn from(text: String) -> Self {
        let count = Json(&text).array_len();
        Self {
            text: text.into(),
            count,
        }
    }
}

/// What the result pane shows.
enum ResultPane {
    Hidden,
    /// The latest filter outcome, `JsonPluginView::filtered`.
    Filter,
    Xml(Result<Shown, JsonError>),
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
    filtered: Option<Result<Shown, JsonError>>,
    doc_error: Option<JsonError>,
    /// The result pane changed and the read-only editor still shows old text.
    /// `render` refreshes it because only render knows the view's current
    /// window: the view outlives popup windows, so a task cannot hold one.
    right_dirty: bool,
    /// The source changed since the draft was last written.
    unsaved: bool,
    filter_task: Task<()>,
    /// Replacing it cancels the previous evaluation, so only the newest result lands.
    eval_task: Task<()>,
    sync_task: Task<()>,
    save_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl JsonPluginView {
    /// Builds the view; content comes from the launch input, else the draft.
    pub(super) fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = Arc::new(Draft::new(&launch.data_dir));
        let has_input = launch.input.is_some();
        let (content, doc_error) = match launch.input {
            Some(input) => match input.parse::<Normalized>() {
                // Normalized output is pretty-printed JSON, so it is valid already.
                Ok(Normalized(text)) => (text, None),
                // Kept verbatim; the editor reports where it stops being JSON.
                Err(_) => {
                    let error = Json(&input).validate().err();
                    (input, error)
                }
            },
            None => {
                let draft_text = draft
                    .load()
                    .unwrap_or_else(|err| {
                        tracing::warn!("reading the JSON draft failed: {}", err);
                        None
                    })
                    .unwrap_or_default();
                let error = Json(&draft_text).validate().err();
                (draft_text, error)
            }
        };
        let engine = FilterEngine::spawn()
            .map_err(|err| tracing::error!("starting the filter engine failed: {}", err))
            .ok();
        if let Some(engine) = &engine {
            engine.set_document(Arc::from(content.as_str()));
        }

        let left = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
                .default_value(content)
        });
        let right = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .line_number(true)
        });
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
            doc_error,
            right_dirty: false,
            unsaved: false,
            filter_task: Task::ready(()),
            eval_task: Task::ready(()),
            sync_task: Task::ready(()),
            save_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        // Launch input replaces the previous draft, so persist it like an edit.
        if has_input {
            view.schedule_save(cx);
        }
        view
    }

    /// Current filter expression.
    fn filter_text(&self, cx: &App) -> SharedString {
        self.filter.read(cx).value()
    }

    /// Reacts to a source edit: save later, re-validate and re-filter after a pause.
    fn on_source_changed(&mut self, cx: &mut Context<Self>) {
        self.schedule_save(cx);
        if matches!(self.pane, ResultPane::Xml(_)) {
            self.show(ResultPane::Hidden, cx);
        }
        self.sync_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SYNC_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| this.sync_document(cx));
        });
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

    /// Validates the source, hands it to the engine and refreshes an active filter.
    fn sync_document(&mut self, cx: &mut Context<Self>) {
        // ponytail: validation runs on the UI thread (~20 ms for 4.5 MB in release);
        // move it to the background executor if larger documents become common.
        let text = self.left.read(cx).value();
        self.doc_error = Json(&text).validate().err();
        if let Some(engine) = &self.engine {
            engine.set_document(text.into());
        }
        if !self.filter_text(cx).trim().is_empty() {
            self.run_filter(cx);
        }
        cx.notify();
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
            let result = pending.await;
            let shown = cx
                .background_executor()
                .spawn(async move { result.map(Shown::from) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.filtered = Some(shown);
                this.show(ResultPane::Filter, cx);
            });
        });
    }

    /// The result the pane shows, if any.
    fn shown_result(&self) -> Option<&Result<Shown, JsonError>> {
        match &self.pane {
            ResultPane::Hidden => None,
            ResultPane::Filter => self.filtered.as_ref(),
            ResultPane::Xml(result) => Some(result),
        }
    }

    /// Replaces the result pane; the read-only editor is refreshed on the next frame.
    fn show(&mut self, pane: ResultPane, cx: &mut Context<Self>) {
        self.pane = pane;
        self.right_dirty = true;
        cx.notify();
    }

    /// Copies the pane's text into the read-only editor.
    fn push_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.right_dirty = false;
        let text = match self.shown_result() {
            Some(Ok(shown)) => shown.text.clone(),
            _ => SharedString::default(),
        };
        self.right
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    /// Runs a toolbar rewrite; success replaces the source as one undoable edit.
    fn rewrite(&mut self, rewrite: Rewrite, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.left.read(cx).value();
        match rewrite.apply(&text) {
            Ok(rewritten) => self
                .left
                .update(cx, |state, cx| state.replace_all(rewritten, window, cx)),
            Err(err) => {
                self.doc_error = Some(err);
                cx.notify();
            }
        }
    }

    /// Converts the filter result (while a filter is active) or the source to
    /// XML; a failed filter is reported rather than silently replaced by the source.
    fn convert_to_xml(&mut self, cx: &mut Context<Self>) {
        let value = match &self.filtered {
            Some(Ok(shown)) => Value::try_from(Json(&shown.text)),
            Some(Err(err)) => Err(err.clone()),
            None => Value::try_from(Json(&self.left.read(cx).value())),
        };
        let xml = value.map(|value| Shown {
            text: AsXml(&value).to_string().into(),
            count: None,
        });
        self.show(ResultPane::Xml(xml), cx);
    }

    /// Copies the shown result to the clipboard.
    fn copy_result(&mut self, cx: &mut Context<Self>) {
        if let Some(Ok(shown)) = self.shown_result() {
            cx.write_to_clipboard(ClipboardItem::new_string(shown.text.to_string()));
        }
    }

    /// Handles a paste that replaces the whole document (empty or fully selected):
    /// recognized formats become formatted JSON in one undoable edit.
    /// Returns `false` to let the editor insert the text normally.
    fn paste_whole_document(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let state = self.left.read(cx);
        let len = state.text().len();
        if len != 0 && state.selected_range() != (0..len) {
            return false;
        }
        match text.parse::<Normalized>() {
            Ok(Normalized(text)) => {
                self.left
                    .update(cx, |state, cx| state.replace_all(text, window, cx));
                true
            }
            Err(_) => false,
        }
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
                        .on_paste(move |item, window, cx| {
                            let Some(text) = item.text() else {
                                return false;
                            };
                            weak.update(cx, |this, cx| this.paste_whole_document(text, window, cx))
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

    /// Result pane: title row plus either the read-only editor or an error state.
    fn render_result(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(result) = self.shown_result() else {
            return div().into_any_element();
        };
        let is_xml = matches!(self.pane, ResultPane::Xml(_));
        let title = if is_xml {
            SharedString::from("XML")
        } else {
            t!("json.filter_result").into()
        };
        let theme = cx.theme();
        let header = h_flex()
            .h_8()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(title),
            )
            .when_some(result.as_ref().ok(), |this, shown| {
                this.when_some(shown.count, |this, count| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("json.items", count = count).to_string()),
                    )
                })
                .child(div().flex_1())
                .child(
                    Button::new("copy")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Copy)
                        .label(t!("json.copy").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.copy_result(cx))),
                )
            });
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
            .child(header)
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
                .on_click(cx.listener(move |this, _, window, cx| this.rewrite(rewrite, window, cx)))
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
        if self.right_dirty {
            cx.defer_in(window, |this, window, cx| this.push_result(window, cx));
        }
        let show_result = self.shown_result().is_some();
        v_flex()
            .size_full()
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
