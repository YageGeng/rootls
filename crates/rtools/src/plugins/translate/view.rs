//! An in-memory translation session whose tasks follow the view, not its current window.
use std::{rc::Rc, time::Duration};

use futures::{StreamExt as _, channel::mpsc};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::{Enter, InputEvent, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::resizable::{ResizableState, resizable_panel, v_resizable};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_i18n::t;

use super::{
    client::DeepSeekClient,
    core::{Direction, Endpoint, INPUT_LIMIT, Scenario, TranslateError, TranslationRequest},
    credentials::KeySource,
    session::{Session, Status},
};
use crate::{
    plugin::Launch,
    settings::{SettingsStore, window::Page},
    shell::Shell,
    ui::{ButtonExt as _, Toolbar},
};

actions!(ai_translate, [Translate, StopTranslation, CopyTranslation]);
const CONTEXT: &str = "AITranslate";

/// All request state belongs to this entity so hiding and detaching preserve it.
pub(super) struct TranslateView {
    input: Entity<TextareaState>,
    output: Entity<TextareaState>,
    direction: Direction,
    scenario: Scenario,
    split: Entity<ResizableState>,
    session: Session,
    count: usize,
    detected: Option<Direction>,
    pending: String,
    reset_output: bool,
    copied: bool,
    source_copied: bool,
    locale: String,
    connection: (String, String, u64),
    key_source: Option<KeySource>,
    key_error: Option<TranslateError>,
    key_task: Option<Task<()>>,
    task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl TranslateView {
    /// Keeps the modified submit shortcut and ordinary selection Copy available.
    pub fn bind_keys(cx: &mut App) {
        let submit = if cfg!(target_os = "macos") {
            "cmd-enter"
        } else {
            "ctrl-enter"
        };
        let copy = if cfg!(target_os = "macos") {
            "cmd-shift-c"
        } else {
            "ctrl-shift-c"
        };
        cx.bind_keys([
            KeyBinding::new(submit, Translate, Some(CONTEXT)),
            KeyBinding::new(copy, CopyTranslation, Some(CONTEXT)),
            KeyBinding::new("escape", StopTranslation, Some(CONTEXT)),
        ]);
    }

    /// Seeds a new session from launcher content without sending anything to the model.
    pub fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.global::<SettingsStore>();
        let settings = store.settings.translate.clone();
        let connection = (
            settings.endpoint.clone(),
            settings.model.clone(),
            store.credential_revision,
        );
        let source = launch.input.unwrap_or_default();
        let count = source.chars().count();
        let detected = Direction::Auto.resolve(&source).ok();
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(source)
                .submit_on_enter(true)
        });
        let output = cx.new(|cx| TextareaState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value();
                    this.count = value.chars().count();
                    this.detected = Direction::Auto.resolve(&value).ok();
                    this.source_copied = false;
                    this.invalidate(cx);
                }
            }),
            cx.observe_global::<SettingsStore>(|this, cx| {
                let store = cx.global::<SettingsStore>();
                let connection = (
                    &store.settings.translate.endpoint,
                    &store.settings.translate.model,
                    store.credential_revision,
                );
                if connection != (&this.connection.0, &this.connection.1, this.connection.2) {
                    this.connection = (connection.0.clone(), connection.1.clone(), connection.2);
                    this.invalidate(cx);
                    this.reload_key(cx);
                }
                cx.notify();
            }),
        ];
        let mut this = Self {
            input,
            output,
            direction: settings.direction,
            scenario: settings.scenario,
            split: cx.new(|_| ResizableState::default()),
            session: Session::default(),
            count,
            detected,
            pending: String::new(),
            reset_output: false,
            copied: false,
            source_copied: false,
            locale: String::new(),
            connection,
            key_source: None,
            key_error: None,
            key_task: None,
            task: None,
            _subscriptions: subscriptions,
        };
        this.reload_key(cx);
        this.relabel(window, cx);
        this
    }

    /// Changes only the language pair, retaining source text and marking old results stale.
    fn set_direction(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if self.direction == direction {
            return;
        }
        self.direction = direction;
        self.invalidate(cx);
        SettingsStore::update(cx, |settings| settings.translate.direction = direction);
    }

    /// Swaps both text areas, cancelling the old request and retaining source undo history.
    fn exchange_text(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.running() {
            tracing::info!("translation cancelled because source and result were exchanged");
        }
        self.task.take();
        let source = self.input.read(cx).value().to_string();
        let input = self.session.exchange(source);
        self.count = input.chars().count();
        self.detected = Direction::Auto.resolve(&input).ok();
        self.pending.clone_from(&self.session.output);
        self.reset_output = true;
        self.copied = false;
        self.source_copied = false;
        self.input
            .update(cx, |state, cx| state.replace_all(input, window, cx));
        cx.notify();
    }

    /// Handles Enter only from the source field after the input component handles IME and Shift.
    fn submit_input(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if !action.shift && self.input.focus_handle(cx).is_focused(window) {
            self.translate(&Translate, window, cx);
        } else {
            cx.propagate();
        }
    }

    /// Refreshes source status after settings change without retaining a credential in the view.
    fn reload_key(&mut self, cx: &mut Context<Self>) {
        self.key_task.take();
        self.key_source = None;
        self.key_error = None;
        let endpoint = match self.connection.0.parse::<Endpoint>() {
            Ok(endpoint) => endpoint,
            Err(error) => {
                self.key_error = Some(error);
                return;
            }
        };
        let key = endpoint.load_key(cx);
        self.key_task = Some(cx.spawn(async move |this, cx| {
            let result = key.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(key) => {
                        this.key_source = Some(key.source);
                        this.key_error = key.storage_failed.then_some(TranslateError::Credentials);
                    }
                    Err(error) => this.key_error = Some(error),
                }
                cx.notify();
            });
        }));
    }

    /// Cancels the owned task and marks the retained translation as previous input.
    fn invalidate(&mut self, cx: &mut Context<Self>) {
        if self.session.running() {
            tracing::info!("translation cancelled because input or configuration changed");
        }
        self.task.take();
        self.session.invalidate();
        self.copied = false;
        cx.notify();
    }

    /// Starts one immutable request, with IME composition taking priority over the shortcut.
    fn translate(&mut self, _: &Translate, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.running() {
            return;
        }
        if self.input.update(cx, |state, cx| {
            state.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        let request = TranslationRequest::new(
            &cx.global::<SettingsStore>().settings.translate,
            self.input.read(cx).value().to_string(),
            self.direction,
            self.scenario,
        );
        let revision = self.session.begin();
        let (request, client) = match (request, cx.try_global::<DeepSeekClient>()) {
            (Ok(request), Some(client)) => (request, client.clone()),
            (Err(error), _) => {
                tracing::warn!("translation request rejected: {}", error);
                self.session.finish(revision, Err(error));
                cx.notify();
                return;
            }
            (_, None) => {
                self.session.finish(revision, Err(TranslateError::Network));
                cx.notify();
                return;
            }
        };
        self.copied = false;
        let key = request.endpoint.load_key(cx);
        let (sender, mut receiver) = mpsc::channel(8);
        let work = cx.background_spawn(async move {
            let key = key.await?;
            client.translate(request, key, sender).await
        });
        // Owning the worker here makes Task drop cancel both the stream and the UI consumer.
        self.task = Some(cx.spawn(async move |this, cx| {
            while let Some(mut batch) = receiver.next().await {
                async_io::Timer::after(Duration::from_millis(40)).await;
                while let Ok(chunk) = receiver.try_recv() {
                    batch.push_str(&chunk);
                }
                if this
                    .update(cx, |this, cx| {
                        let first = !this.session.output_is_current();
                        if this.session.append(revision, &batch) {
                            this.copied = false;
                            if first {
                                this.pending.clear();
                                this.reset_output = true;
                            }
                            this.pending.push_str(&batch);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.session.finish(revision, result);
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Stops only an active request; otherwise Escape continues to the launcher.
    fn stop(&mut self, _: &StopTranslation, _: &mut Window, cx: &mut Context<Self>) {
        if self.session.running() {
            self.task.take();
            self.session.stop();
            tracing::info!("translation stopped by user");
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    /// Copies the entire visible result, leaving ordinary selection copying to Textarea.
    fn copy(&mut self, _: &CopyTranslation, _: &mut Window, cx: &mut Context<Self>) {
        if self.session.output.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(self.session.output.clone()));
        self.copied = true;
        cx.notify();
    }

    /// Copies the complete source without disturbing its selection or undo history.
    fn copy_source(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let source = self.input.read(cx).value();
        if source.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(source.to_string()));
        self.source_copied = true;
        cx.notify();
    }

    /// Clears the session while preserving the source field's normal undo history.
    fn clear(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate(cx);
        self.session.output.clear();
        self.session.status = Status::Idle;
        self.pending.clear();
        self.reset_output = true;
        self.input
            .update(cx, |state, cx| state.replace_all("", window, cx));
        self.count = 0;
        self.detected = None;
        self.source_copied = false;
        self.input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Applies accumulated deltas to the current window without cloning the whole document.
    fn sync_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = std::mem::take(&mut self.pending);
        let reset = std::mem::take(&mut self.reset_output);
        if !reset && text.is_empty() {
            return;
        }
        self.output.update(cx, |state, cx| {
            if reset {
                state.set_value(text, window, cx);
            } else {
                let selected = state.selected_range();
                let reversed = state.is_selection_reversed();
                let scroll = state.scroll_offset();
                let end = state.text().len();
                state.set_selected_range(end..end, cx);
                state.insert(text, window, cx);
                // Programmatic edits reveal the caret; preserve the reader's viewport and selection.
                state.set_selected_range_with_direction(selected, reversed, cx);
                state.set_scroll_offset(scroll, cx);
            }
        });
    }

    /// Uses the shared menu for intrinsic sizing, keyboard navigation and focus restoration.
    fn language_menu(&self, target: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = if target && self.direction == Direction::Auto {
            self.detected.unwrap_or(Direction::EnZh)
        } else {
            self.direction
        };
        let (id, options): (&str, &[(Direction, &str)]) = if target {
            (
                "translate.target_language",
                &[
                    (Direction::EnZh, "translate.chinese"),
                    (Direction::ZhEn, "translate.english"),
                ],
            )
        } else {
            (
                "translate.source_language",
                &[
                    (Direction::Auto, "translate.auto_detect"),
                    (Direction::EnZh, "translate.english"),
                    (Direction::ZhEn, "translate.chinese"),
                ],
            )
        };
        let label = options
            .iter()
            .find(|(value, _)| *value == selected)
            .map_or(options[0].1, |(_, label)| *label);
        let view = cx.entity().downgrade();
        Button::new(id)
            .outline()
            .small()
            .label(t!(label))
            .dropdown_caret(true)
            .accessibility_label(t!(
                "translate.language_choice",
                kind = t!(id),
                language = t!(label)
            ))
            .dropdown_menu(move |mut menu, window, _| {
                menu = menu.min_w(window.rem_size() * 12.);
                for &(value, label) in options {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(t!(label))
                            .checked(value == selected)
                            .on_click(move |_, _, cx| {
                                let _ = view.update(cx, |this, cx| this.set_direction(value, cx));
                            }),
                    );
                }
                menu
            })
    }

    /// Refreshes editor placeholders when the interface locale changes.
    fn relabel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let locale = rust_i18n::locale().to_string();
        if locale == self.locale {
            return;
        }
        self.locale = locale;
        self.input.update(cx, |state, cx| {
            state.set_placeholder(t!("translate.input_hint"), window, cx)
        });
        self.output.update(cx, |state, cx| {
            state.set_placeholder(t!("translate.output_hint"), window, cx)
        });
    }

    /// Shows actionable failures without repeating normal or stale-result status.
    fn feedback(&self) -> Option<&'static str> {
        match &self.session.status {
            Status::Failed(error) => Some(error.label()),
            Status::Truncated => Some("translate.truncated"),
            _ if self.session.running() => None,
            _ if self.count > INPUT_LIMIT => Some("translate.error_input_limit"),
            _ if self.count > 0 && self.direction == Direction::Auto && self.detected.is_none() => {
                Some("translate.error_direction")
            }
            _ => self.key_error.as_ref().map(TranslateError::label),
        }
    }
}

impl Focusable for TranslateView {
    /// Places initial keyboard focus in the source field.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for TranslateView {
    /// Renders the same adjustable top/bottom layout in the launcher and detached windows.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.relabel(window, cx);
        if self.reset_output || !self.pending.is_empty() {
            cx.defer_in(window, |this, window, cx| this.sync_output(window, cx));
        }
        let running = self.session.running();
        let valid = self.count > 0
            && self.count <= INPUT_LIMIT
            && (self.direction != Direction::Auto || self.detected.is_some());
        let muted = cx.theme().muted_foreground;
        let previous = !self.session.output.is_empty() && !self.session.output_is_current();
        let partial = !self.session.output.is_empty() && self.session.status != Status::Complete;
        let copy_label = if self.copied {
            "translate.copied"
        } else if previous {
            "translate.copy_previous"
        } else if partial {
            "translate.copy_partial"
        } else {
            "translate.copy"
        };
        let status = match &self.session.status {
            Status::Connecting => Some("translate.connecting"),
            Status::Streaming => Some("translate.streaming"),
            Status::Stale => Some("translate.stale"),
            Status::Stopped if !previous => Some("translate.incomplete"),
            _ if previous => Some("translate.stale"),
            _ if partial => Some("translate.incomplete"),
            _ => None,
        };
        let feedback = self.feedback();
        // At narrow widths or large text sizes, retain the menu's name and value without crowding it.
        let compact = window.viewport_size().width < window.rem_size() * 48.;
        let input = v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .flex_none()
                    .min_h_10()
                    .px_4()
                    .py_1()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .rounded_t(cx.theme().radius_lg)
                    .when(!compact, |view| {
                        view.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(t!("translate.source_language")),
                        )
                    })
                    .child(self.language_menu(false, cx))
                    .when(self.direction == Direction::Auto && !compact, |view| {
                        view.when_some(self.detected, |view, detected| {
                            view.child(div().text_xs().text_color(muted).child(t!(
                                match detected {
                                    Direction::EnZh => "translate.detected_english",
                                    _ => "translate.detected_chinese",
                                }
                            )))
                        })
                    })
                    .child(div().flex_1())
                    .child(
                        Toolbar::button(
                            "copy-source",
                            if self.source_copied {
                                IconName::Check
                            } else {
                                IconName::Copy
                            },
                        )
                        .disabled(self.count == 0)
                        .on_click(cx.listener(Self::copy_source))
                        .with_tooltip(t!(if self.source_copied {
                            "translate.source_copied"
                        } else {
                            "translate.copy_source"
                        })),
                    )
                    .child(
                        Toolbar::button("clear-translation", IconName::Close)
                            .on_click(cx.listener(Self::clear))
                            .with_tooltip(t!("translate.clear")),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Textarea::new(&self.input)
                        .aria_label(t!("translate.input"))
                        .h(relative(1.))
                        .appearance(false)
                        .bordered(false),
                ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .px_4()
                    .pb_1()
                    .gap_2()
                    .text_xs()
                    .text_color(muted)
                    .child(t!("translate.enter_hint").to_string())
                    .child(div().flex_1())
                    .child(format!("{} / {}", self.count, INPUT_LIMIT)),
            );
        let output = v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().muted)
            .rounded_b(cx.theme().radius_lg)
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .min_h_10()
                    .px_4()
                    .py_1()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    // Equal flexible side groups keep the exchange button at the actual center.
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .when(!compact, |view| {
                                view.child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(t!("translate.target_language")),
                                )
                            })
                            .child(self.language_menu(true, cx))
                            .when_some(status, |view, status| {
                                view.child(
                                    div()
                                        .id("translation-status")
                                        .text_xs()
                                        .text_color(muted)
                                        .truncate()
                                        .child(t!(status))
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(t!(status).to_string()).build(window, cx)
                                        }),
                                )
                            }),
                    )
                    .child(
                        Button::new("exchange-translation-text")
                            .outline()
                            .small()
                            .icon(IconName::ArrowUpDown)
                            .rounded(cx.theme().radius_full())
                            .disabled(self.count == 0 && self.session.output.is_empty())
                            .on_click(cx.listener(Self::exchange_text))
                            .with_tooltip(t!("translate.exchange_text")),
                    )
                    .child(
                        h_flex().flex_1().min_w_0().justify_end().child(
                            Toolbar::button(
                                "copy-translation",
                                if self.copied {
                                    IconName::Check
                                } else {
                                    IconName::Copy
                                },
                            )
                            .disabled(self.session.output.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.copy(&CopyTranslation, window, cx)
                            }))
                            .with_key_tooltip(t!(copy_label), Some((&CopyTranslation, CONTEXT))),
                        ),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Textarea::new(&self.output)
                        .aria_label(t!("translate.output"))
                        .h(relative(1.))
                        .appearance(false)
                        .bordered(false)
                        .readonly(true),
                ),
            )
            .when_some(feedback, |view, feedback| {
                view.child(
                    h_flex()
                        .flex_none()
                        .px_4()
                        .py_2()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(t!(feedback)),
                        )
                        .when(self.key_error.is_some(), |view| {
                            view.child(
                                Button::new("configure-deepseek")
                                    .ghost()
                                    .small()
                                    .label(t!("translate.configure"))
                                    .on_click(|_, _, cx| {
                                        Shell::open_settings_page(Some(Page::Translate), cx)
                                    }),
                            )
                        }),
                )
            });
        v_flex()
            .size_full()
            .min_w_0()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::translate))
            .on_action(cx.listener(Self::submit_input))
            .on_action(cx.listener(Self::stop))
            .on_action(cx.listener(Self::copy))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .m_3()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius_lg)
                    .child(
                        v_resizable("translation-panes")
                            .with_state(&self.split)
                            .with_handle_appearance(Rc::new(|_, _, cx| {
                                Some(
                                    div()
                                        .flex_none()
                                        .h(px(1.))
                                        .w_full()
                                        .bg(cx.theme().border)
                                        .into_any_element(),
                                )
                            }))
                            .child(
                                resizable_panel()
                                    .size(window.rem_size() * 11.)
                                    .size_range(window.rem_size() * 7. ..px(f32::MAX))
                                    .child(input),
                            )
                            .child(
                                resizable_panel()
                                    .size(window.rem_size() * 16.5)
                                    .size_range(window.rem_size() * 7. ..px(f32::MAX))
                                    .child(output),
                            ),
                    ),
            )
            .child(
                Toolbar::default()
                    .child(
                        Toolbar::button("translation-settings", IconName::Settings)
                            .on_click(|_, _, cx| {
                                Shell::open_settings_page(Some(Page::Translate), cx)
                            })
                            .with_tooltip(t!("app.settings_title")),
                    )
                    .child(
                        ButtonGroup::new("translation-style")
                            .small()
                            .outline()
                            // Keep handlers on the buttons so keyboard activation follows the same path.
                            .children([Scenario::General, Scenario::Technical].into_iter().map(
                                |scenario| {
                                    Button::new(scenario.label())
                                        .label(t!(scenario.label()))
                                        .selected(self.scenario == scenario)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if this.scenario != scenario {
                                                this.scenario = scenario;
                                                this.invalidate(cx);
                                                SettingsStore::update(cx, |settings| {
                                                    settings.translate.scenario = scenario
                                                });
                                            }
                                        }))
                                },
                            )),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("submit-translation")
                            .primary()
                            .small()
                            .label(t!(if running {
                                "translate.stop"
                            } else if matches!(self.session.status, Status::Failed(_)) {
                                "translate.retry"
                            } else {
                                "translate.submit"
                            }))
                            .disabled(!running && (!valid || self.key_source.is_none()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.session.running() {
                                    this.stop(&StopTranslation, window, cx);
                                } else {
                                    this.translate(&Translate, window, cx);
                                }
                            })),
                    ),
            )
    }
}
