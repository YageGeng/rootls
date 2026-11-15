//! The diff plugin's root view: two editors, then a read-only diff of them
//! (diff spec §3, §4.3). Both bottom bars live in `toolbar`.

mod toolbar;

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::diff::{
    Diff, DiffFile, DiffHunkSeparator, DiffInlineUnit, DiffMode, DiffState,
};
use gpui_kit::component::input::{Editor, EditorState, Enter, InputEvent, Rope, RopeExt as _};
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;
use rust_i18n::t;

use crate::plugin::{Autosave, Draft, Launch, SearchPanels};
use crate::plugins::diff::core::{CompareOptions, Comparison, FormatJson, Language, Texts};
use crate::settings::SettingsStore;
use crate::ui::{Choice, StatusPage};

actions!(diff_plugin, [BackToEdit, PreviousChange, NextChange]);

/// Language dropdown options: auto (`None`) first, then every language.
type LanguageChoices = SearchableVec<Choice<Option<Language>>>;

/// Key context of the compare view, nested inside the launcher's plugin context.
pub(super) const COMPARE_CONTEXT: &str = "DiffCompare";
/// How long entering the compare view may take before it says it is comparing;
/// shorter waits show nothing, so a quick result never flickers.
const SLOW_COMPARISON: Duration = Duration::from_millis(250);

/// Which of the plugin's two views is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Edit,
    Compare,
}

/// What the compare view shows for the latest comparison.
enum Outcome {
    /// The parsed patch, already handed to the `DiffState`.
    Diff(Vec<DiffFile>),
    /// No differences; `switched` when a comparison switch, not the texts
    /// themselves, made them equal.
    Identical {
        switched: bool,
    },
    OriginalEmpty,
    ModifiedEmpty,
    BothEmpty,
    /// The generated patch did not parse (a bug); carries the parser's message.
    Failed(SharedString),
}

/// The latest comparison as the compare view needs it.
struct Summary {
    additions: usize,
    deletions: usize,
    format_json: FormatJson,
    outcome: Outcome,
}

impl Summary {
    /// Decides what to show for `comparison`, computed with `options`: an
    /// empty side first, then identical texts, then the parsed diff colored
    /// as `language`. Runs off the UI thread, so a large patch parses there.
    fn new(comparison: Comparison, options: CompareOptions, language: Language) -> Self {
        let outcome = match (comparison.original_empty, comparison.modified_empty) {
            (true, true) => Outcome::BothEmpty,
            (true, false) => Outcome::OriginalEmpty,
            (false, true) => Outcome::ModifiedEmpty,
            _ if comparison.additions + comparison.deletions == 0 => Outcome::Identical {
                switched: options.ignore_whitespace
                    || options.ignore_case
                    || comparison.format_json == FormatJson::Applied,
            },
            _ => match DiffFile::parse(&comparison.patch) {
                Ok(files) => Outcome::Diff(
                    files
                        .into_iter()
                        .map(|file| file.with_language(language.name()))
                        .collect(),
                ),
                Err(err) => Outcome::Failed(err.to_string().into()),
            },
        };
        Self {
            additions: comparison.additions,
            deletions: comparison.deletions,
            format_json: comparison.format_json,
            outcome,
        }
    }
}

/// The diff plugin's root view.
pub struct DiffPluginView {
    /// Stable handle the launcher focuses on every summon; `render` forwards
    /// it to the editor or diff body that should take keys.
    focus: FocusHandle,
    original: Entity<EditorState>,
    modified: Entity<EditorState>,
    /// The editor last typed in (one of the two above); focus returns to it.
    editing: Entity<EditorState>,
    /// The language picked in the dropdown; `None` follows `detected`.
    language: Option<Language>,
    /// The language guessed from the start of the texts.
    detected: Language,
    language_select: Entity<SelectState<LanguageChoices>>,
    diff: Entity<DiffState>,
    mode: Mode,
    /// Keeps both sides as `original.txt` and `modified.txt` across runs.
    autosave: Autosave,
    /// Options of the latest comparison request.
    options: CompareOptions,
    /// Latest result; `None` from entering the compare view until it lands.
    summary: Option<Summary>,
    /// The first result after entering the compare view is taking a while.
    slow: bool,
    /// The diff shows every unchanged line: `fold_unchanged` off, as last
    /// applied to it, and applied again to each new result.
    unchanged_expanded: bool,
    /// Replacing it cancels the running comparison before its patch is parsed.
    compare_task: Task<()>,
    /// Marks the comparison slow when it has not landed in time.
    slow_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl DiffPluginView {
    /// Builds the view in its edit state, with both sides from their drafts.
    pub(super) fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let autosave: Autosave = ["original.txt", "modified.txt"]
            .into_iter()
            .map(|name| Draft::new(&launch.data_dir, name))
            .collect();
        let [original_text, modified_text]: [String; 2] =
            autosave.load().try_into().unwrap_or_default();
        // Plain text with line numbers; both open the built-in search panel on
        // Ctrl+F (Cmd+F). The editor wraps by default, which these must not.
        let editor = |text: String, window: &mut Window, cx: &mut Context<EditorState>| {
            EditorState::new(window, cx)
                .line_number(true)
                .soft_wrap(false)
                .searchable(true)
                .default_value(text)
        };
        let original = cx.new(|cx| editor(original_text, window, cx));
        let modified = cx.new(|cx| editor(modified_text, window, cx));
        let diff = cx.new(|cx| DiffState::new(Vec::new(), cx));
        // Filled with the auto entry and every language by `sync_language`.
        let language_select = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
        });

        let subscriptions = vec![
            cx.subscribe_in(&original, window, Self::on_editor_event),
            cx.subscribe_in(&modified, window, Self::on_editor_event),
            cx.subscribe_in(
                &language_select,
                window,
                |this, _, event: &SelectEvent<LanguageChoices>, window, cx| {
                    if let SelectEvent::Confirm(Some(language)) = event {
                        this.language = *language;
                        this.sync_language(window, cx);
                    }
                },
            ),
            // Toolbar clicks, the reload on every summon and hand edits of
            // `settings.toml` all arrive here.
            cx.observe_global::<SettingsStore>(Self::apply_settings),
            // Pending edits must reach the drafts even when the app quits or
            // the view goes away before the save debounce fires.
            cx.on_app_quit(|this, _| {
                this.autosave.flush();
                async {}
            }),
            cx.on_release(|this, _| this.autosave.flush()),
        ];

        let mut view = Self {
            focus: cx.focus_handle(),
            editing: original.clone(),
            original,
            modified,
            language: None,
            // `sync_language` below guesses and fills the still empty dropdown.
            detected: Language::Plain,
            language_select,
            diff,
            mode: Mode::Edit,
            autosave,
            options: CompareOptions::default(),
            summary: None,
            slow: false,
            unchanged_expanded: false,
            compare_task: Task::ready(()),
            slow_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        view.apply_settings(cx);
        view.sync_language(window, cx);
        view
    }

    /// Both sides' text; cloning a rope is cheap.
    fn ropes(&self, cx: &App) -> [Rope; 2] {
        [&self.original, &self.modified].map(|editor| editor.read(cx).text().clone())
    }

    /// Saves after an edit and refreshes the line counts.
    fn on_editor_event(
        &mut self,
        _: &Entity<EditorState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.sync_language(window, cx);
            self.autosave.schedule(self.ropes(cx).into(), cx);
            cx.notify();
        }
    }

    /// Guesses the language again from the start of the texts (the
    /// original, or the modified when the original looks plain), colors both
    /// editors with the picked language or that guess, and relabels the
    /// dropdown's auto entry when the guess changed. Reads only the first
    /// chunks of each text, so it is cheap on every edit.
    fn sync_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let detected = self
            .ropes(cx)
            .iter()
            .map(|rope| {
                let mut sample = String::new();
                for chunk in rope.chunks() {
                    sample.push_str(chunk);
                    if sample.len() >= Language::SAMPLE_BYTES {
                        break;
                    }
                }
                Language::detect(&sample)
            })
            .find(|language| *language != Language::Plain)
            .unwrap_or(Language::Plain);
        if detected != self.detected || self.language_select.read(cx).selected_value().is_none() {
            self.detected = detected;
            let picked = self.language;
            let choices: Vec<Choice<Option<Language>>> = std::iter::once(Choice {
                label: t!("diff.auto_language", language = detected).into(),
                value: None,
            })
            .chain(Language::ALL.map(|language| Choice {
                label: language.to_string().into(),
                value: Some(language),
            }))
            .collect();
            self.language_select.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(choices), window, cx);
                state.set_selected_value(&picked, window, cx);
            });
        }
        let name = self.language.unwrap_or(detected).name();
        for editor in [&self.original, &self.modified] {
            if editor.read(cx).language_name() != name {
                editor.update(cx, |state, cx| state.set_highlighter(name, cx));
            }
        }
    }

    /// Puts the persisted layout, emphasis and folding into effect on the
    /// diff, and recomputes a showing comparison whose switches changed; the
    /// previous result stays up until the new one lands.
    fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let settings = &cx.global::<SettingsStore>().settings.diff;
        let options = CompareOptions::from(settings);
        let expand = !settings.fold_unchanged;
        let refold = expand != self.unchanged_expanded;
        let mode = DiffMode::from(settings.layout);
        let unit = Some(if settings.char_level {
            DiffInlineUnit::Character
        } else {
            DiffInlineUnit::Word
        });
        self.diff.update(cx, |state, cx| {
            if state.mode() != mode {
                state.set_mode(mode, cx);
            }
            if state.inline_unit() != unit {
                state.set_inline_unit(unit, cx);
            }
            if refold && expand {
                state.expand_unchanged(cx);
            } else if refold {
                state.collapse_unchanged(cx);
            }
        });
        self.unchanged_expanded = expand;
        if self.mode == Mode::Compare && options != self.options {
            self.compare(cx);
        }
        cx.notify();
    }

    /// Compares both sides with the current switches on the background
    /// executor, then parses the patch there too. The two run as separate
    /// jobs, so a comparison superseded meanwhile is never parsed.
    fn compare(&mut self, cx: &mut Context<Self>) {
        self.options = CompareOptions::from(&cx.global::<SettingsStore>().settings.diff);
        let options = self.options;
        let (picked, detected) = (self.language, self.detected);
        let [original, modified] = self.ropes(cx);
        self.compare_task = cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let comparison = executor
                .spawn(async move {
                    let (original, modified) = (original.to_string(), modified.to_string());
                    let texts = Texts {
                        original: &original,
                        modified: &modified,
                    };
                    texts.compare(options)
                })
                .await;
            // The picked language wins; on auto, JSON on both sides (also
            // after formatting) beats the guess from the raw start.
            let language = picked.unwrap_or(if comparison.json {
                Language::Json
            } else {
                detected
            });
            let summary = executor
                .spawn(async move { Summary::new(comparison, options, language) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match &summary.outcome {
                    Outcome::Diff(files) => {
                        let expand = this.unchanged_expanded;
                        this.diff.update(cx, |state, cx| {
                            state.set_files(files.iter().cloned(), cx);
                            // Replacing the files folds unchanged lines again.
                            if expand {
                                state.expand_unchanged(cx);
                            }
                        });
                    }
                    Outcome::Failed(err) => {
                        tracing::warn!("parsing the generated diff failed: {}", err)
                    }
                    _ => {}
                }
                this.summary = Some(summary);
                this.slow = false;
                this.slow_task = Task::ready(());
                cx.notify();
            });
        });
    }

    /// Switches to the compare view and computes a fresh result; the old one
    /// is dropped so a stale diff never shows.
    fn show_compare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = Mode::Compare;
        self.summary = None;
        self.slow = false;
        window.focus(&self.focus, cx);
        self.compare(cx);
        self.slow_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SLOW_COMPARISON).await;
            let _ = this.update(cx, |this, cx| {
                this.slow = this.summary.is_none();
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Returns to the editor last typed in, dropping a comparison still running.
    fn back_to_edit(&mut self, _: &BackToEdit, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = Mode::Edit;
        self.compare_task = Task::ready(());
        self.slow_task = Task::ready(());
        self.editing.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Ctrl+Enter (Cmd+Enter) compares. The editor binds that key to its own
    /// `Enter { secondary: true }` and consumes it, so it is caught here in the
    /// capture phase; any other Enter goes on to the editor.
    fn on_enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if action.secondary && self.mode == Mode::Edit {
            self.show_compare(window, cx);
            // Capture listeners do not stop an action: without this the
            // editor would still insert a line break into the text.
            cx.stop_propagation();
        }
    }

    /// Swaps the two sides; each editor can undo its half.
    fn swap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let [original, modified] = self.ropes(cx).map(|rope| rope.to_string());
        self.original
            .update(cx, |state, cx| state.replace_all(modified, window, cx));
        self.modified
            .update(cx, |state, cx| state.replace_all(original, window, cx));
    }

    /// Empties both sides; each editor can undo it.
    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for editor in [&self.original, &self.modified] {
            editor.update(cx, |state, cx| state.replace_all("", window, cx));
        }
    }

    /// Whether the compare view shows a diff rather than a status page.
    fn showing_diff(&self) -> bool {
        matches!(
            self.summary,
            Some(Summary {
                outcome: Outcome::Diff(_),
                ..
            })
        )
    }

    /// Scrolls to the change above the current one.
    fn previous_change(&mut self, _: &PreviousChange, _: &mut Window, cx: &mut Context<Self>) {
        if self.showing_diff() {
            self.diff.update(cx, |state, cx| state.previous_change(cx));
        }
    }

    /// Scrolls to the change below the current one.
    fn next_change(&mut self, _: &NextChange, _: &mut Window, cx: &mut Context<Self>) {
        if self.showing_diff() {
            self.diff.update(cx, |state, cx| state.next_change(cx));
        }
    }

    /// Remembers the editor being typed in, then moves focus off a handle that
    /// has nothing to take keys: the view's own handle (focused by the
    /// launcher on every summon, and by `Shell::detach` in a new window)
    /// forwards to that editor or to the diff body, and a diff body replaced
    /// by a status page hands focus back so `Esc` keeps working. Runs from
    /// `render`, which always has the view's current window.
    fn forward_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = [&self.original, &self.modified]
            .into_iter()
            .find(|editor| editor.read(cx).focus_handle(cx).is_focused(window))
        {
            self.editing = editor.clone();
        }
        let diff_focus = self.diff.read(cx).focus_handle(cx);
        let showing_diff = self.showing_diff();
        let target = match self.mode {
            Mode::Edit if self.focus.is_focused(window) => self.editing.read(cx).focus_handle(cx),
            Mode::Compare if showing_diff && self.focus.is_focused(window) => diff_focus,
            Mode::Compare if !showing_diff && diff_focus.is_focused(window) => self.focus.clone(),
            _ => return,
        };
        cx.defer_in(window, move |_, window, cx| window.focus(&target, cx));
    }

    /// The two editors side by side, each under a strip with its name and size.
    fn render_editors(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let pane = |editor: &Entity<EditorState>, name: &'static str| {
            let text = editor.read(cx).text();
            // An empty editor shows no count rather than "1 line".
            let size = (text.len() > 0).then(|| -> SharedString {
                match text.lines_len() {
                    1 => t!("diff.one_line").into(),
                    count => t!("diff.lines", count = count).into(),
                }
            });
            v_flex()
                .w_1_2()
                .min_w_0()
                .child(
                    h_flex()
                        .h_8()
                        .px_3()
                        .justify_between()
                        .border_b_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t!(name).to_string())
                        .children(size),
                )
                .child(
                    div().flex_1().min_h_0().child(
                        Editor::new(editor)
                            .bordered(false)
                            .appearance(false)
                            .h_full(),
                    ),
                )
        };
        h_flex()
            .items_stretch()
            .flex_1()
            .min_h_0()
            .child(pane(&self.original, "diff.original"))
            .child(
                pane(&self.modified, "diff.modified")
                    .border_l_1()
                    .border_color(theme.border),
            )
    }

    /// The diff, a status page, or nothing while the first result is computed
    /// (a "comparing" page once that takes a while).
    fn render_compare(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(summary) = &self.summary else {
            return if self.slow {
                StatusPage::new(
                    IconName::LoaderCircle,
                    theme.muted_foreground,
                    t!("diff.comparing"),
                )
                .into_any_element()
            } else {
                div().flex_1().into_any_element()
            };
        };
        let page = match &summary.outcome {
            Outcome::Diff(_) => {
                return div()
                    .flex_1()
                    .min_h_0()
                    .child(
                        Diff::new(&self.diff)
                            // Side by side, each column gets half the width and
                            // long lines wrap in it; otherwise both columns size
                            // to the longest line and the modified one sits
                            // off screen behind a horizontal scroll.
                            .soft_wrap(self.diff.read(cx).mode() == DiffMode::Split)
                            .header_visible(false)
                            .hunk_separator(DiffHunkSeparator::Simple)
                            .line_number(true)
                            // The panel header and the toolbar already draw
                            // the lines around it.
                            .border_0()
                            .size_full(),
                    )
                    .into_any_element();
            }
            Outcome::Identical { switched } => {
                let page =
                    StatusPage::new(IconName::CircleCheck, theme.success, t!("diff.identical"));
                // Say so when a switch, not the texts themselves, made them equal.
                if *switched {
                    page.detail(t!("diff.identical_options"))
                } else {
                    page
                }
            }
            Outcome::OriginalEmpty => StatusPage::new(
                IconName::FileText,
                theme.muted_foreground,
                t!("diff.empty_original"),
            ),
            Outcome::ModifiedEmpty => StatusPage::new(
                IconName::FileText,
                theme.muted_foreground,
                t!("diff.empty_modified"),
            ),
            Outcome::BothEmpty => StatusPage::new(
                IconName::FileText,
                theme.muted_foreground,
                t!("diff.empty_both"),
            ),
            Outcome::Failed(err) => {
                StatusPage::new(IconName::CircleAlert, theme.danger, t!("diff.cannot_show"))
                    .detail(err.clone())
            }
        };
        page.into_any_element()
    }
}

impl SearchPanels for DiffPluginView {
    /// Both editors of the edit view; in the compare view no editor has
    /// focus, so Esc never reaches `close_search` there.
    fn search_editors(&self) -> impl IntoIterator<Item = &Entity<EditorState>> {
        [&self.original, &self.modified]
    }
}

impl Focusable for DiffPluginView {
    /// The view's own stable handle; see `forward_focus`.
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DiffPluginView {
    /// Lays out the edit or the compare view above its toolbar.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.forward_focus(window, cx);
        let root = v_flex()
            .size_full()
            .track_focus(&self.focus)
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::close_search));
        match self.mode {
            Mode::Edit => root
                .child(self.render_editors(cx))
                .child(self.render_edit_toolbar(window, cx)),
            Mode::Compare => root
                .key_context(COMPARE_CONTEXT)
                .on_action(cx.listener(Self::back_to_edit))
                .on_action(cx.listener(Self::previous_change))
                .on_action(cx.listener(Self::next_change))
                .child(self.render_compare(cx))
                .child(self.render_compare_toolbar(window, cx)),
        }
    }
}
