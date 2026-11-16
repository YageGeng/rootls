//! Connection drafts and platform credentials have separate explicit save actions.
use futures::{StreamExt as _, channel::mpsc};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rust_i18n::t;

use super::SettingsStore;
use crate::plugins::translate::{
    client::DeepSeekClient,
    core::{
        Completion, DEFAULT_ENDPOINT, DEFAULT_MODEL, Direction, Endpoint, Scenario, TranslateError,
        TranslateSettings, TranslationRequest,
    },
    credentials::{Credential, KeySource},
};

/// Owns drafts and tasks only for the lifetime of this settings window.
pub(super) struct TranslateSettingsView {
    endpoint: Entity<InputState>,
    model: Entity<InputState>,
    key: Entity<InputState>,
    active: TranslateSettings,
    source: Option<KeySource>,
    key_error: Option<TranslateError>,
    notice: Option<String>,
    key_busy: bool,
    testing: bool,
    clear_key: bool,
    key_read: Option<Task<()>>,
    key_write: Option<Task<()>>,
    test: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl TranslateSettingsView {
    /// Builds editable drafts without sending a model request or persisting a key.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let active = cx.global::<SettingsStore>().settings.translate.clone();
        let endpoint =
            cx.new(|cx| InputState::new(window, cx).default_value(active.endpoint.clone()));
        let model = cx.new(|cx| InputState::new(window, cx).default_value(active.model.clone()));
        let key = cx.new(|cx| InputState::new(window, cx).masked(true));
        let subscriptions = [&endpoint, &model, &key]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.test.take();
                        this.testing = false;
                        this.notice = None;
                        cx.notify();
                    }
                })
            })
            .collect();
        let mut this = Self {
            endpoint,
            model,
            key,
            active,
            source: None,
            key_error: None,
            notice: None,
            key_busy: false,
            testing: false,
            clear_key: false,
            key_read: None,
            key_write: None,
            test: None,
            _subscriptions: subscriptions,
        };
        this.reload_key(cx);
        this
    }

    /// Reads only the effective credential source, retaining no secret in this view.
    fn reload_key(&mut self, cx: &mut Context<Self>) {
        self.key_read.take();
        self.source = None;
        self.key_error = None;
        let endpoint = match self.active.endpoint.parse::<Endpoint>() {
            Ok(endpoint) => endpoint,
            Err(error) => {
                self.key_error = Some(error);
                return;
            }
        };
        self.key_busy = true;
        let read = endpoint.load_key(cx);
        self.key_read = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                this.key_busy = false;
                match result {
                    Ok(key) => {
                        this.source = Some(key.source);
                        this.key_error = key.storage_failed.then_some(TranslateError::Credentials);
                    }
                    Err(error) => this.key_error = Some(error),
                }
                cx.notify();
            });
        }));
    }

    /// Saves a valid connection before replacing active settings or its credential identity.
    fn save_connection(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let mut draft = cx.global::<SettingsStore>().settings.translate.clone();
        draft.endpoint = self.endpoint.read(cx).value().to_string();
        draft.model = self.model.read(cx).value().to_string();
        let draft = match draft.validated() {
            Ok(draft) => draft,
            Err(error) => {
                tracing::warn!("translation connection draft rejected: {}", error);
                self.notice = Some(t!(error.label()).into());
                cx.notify();
                return;
            }
        };
        if SettingsStore::save_translation(cx, draft.clone()).is_err() {
            self.notice = Some(t!("translate.error_save").into());
        } else {
            self.active = draft;
            self.endpoint.update(cx, |state, cx| {
                state.set_value(self.active.endpoint.clone(), window, cx)
            });
            self.model.update(cx, |state, cx| {
                state.set_value(self.active.model.clone(), window, cx)
            });
            self.key
                .update(cx, |state, cx| state.set_value("", window, cx));
            self.test.take();
            self.testing = false;
            self.notice = Some(t!("translate.connection_saved").into());
            self.reload_key(cx);
        }
        cx.notify();
    }

    /// Restores connection draft defaults while leaving credentials and user text intact.
    fn restore_defaults(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.endpoint.update(cx, |state, cx| {
            state.set_value(DEFAULT_ENDPOINT, window, cx)
        });
        self.model
            .update(cx, |state, cx| state.set_value(DEFAULT_MODEL, window, cx));
        self.test.take();
        self.testing = false;
        self.notice = None;
        cx.notify();
    }

    /// Saves or removes a key only for the active normalized endpoint.
    fn change_key(&mut self, remove: bool, cx: &mut Context<Self>) {
        let Ok(endpoint) = self.active.endpoint.parse::<Endpoint>() else {
            return;
        };
        let task = if remove {
            cx.delete_credentials(&endpoint.credential_id())
        } else {
            let secret = self.key.read(cx).unmask_value().to_string();
            let key = match Credential::resolve(Some(secret.into_bytes()), None) {
                Ok(key) => key,
                Err(error) => {
                    self.notice = Some(t!(error.label()).into());
                    cx.notify();
                    return;
                }
            };
            cx.write_credentials(&endpoint.credential_id(), "api-key", key.secret.as_bytes())
        };
        self.test.take();
        self.testing = false;
        self.key_busy = true;
        self.notice = None;
        tracing::info!(
            "translation credential {} started for {}",
            if remove { "removal" } else { "save" },
            endpoint.host()
        );
        self.key_write = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.key_busy = false;
                match result {
                    Ok(()) => {
                        tracing::info!("translation credential operation completed");
                        this.clear_key = true;
                        cx.global_mut::<SettingsStore>().credential_revision += 1;
                        this.notice = Some(
                            t!(if remove {
                                "translate.key_removed"
                            } else {
                                "translate.key_saved"
                            })
                            .into(),
                        );
                        this.reload_key(cx);
                    }
                    Err(_) => {
                        tracing::warn!("translation credential operation failed");
                        this.notice = Some(t!("translate.error_credentials").into());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Sends one fixed short translation to verify authorization, endpoint and model together.
    fn test_connection(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let request = TranslationRequest::new(
            &self.active,
            "Hello.".into(),
            Direction::EnZh,
            Scenario::General,
        );
        let (request, client) = match (request, cx.try_global::<DeepSeekClient>()) {
            (Ok(request), Some(client)) => (request, client.clone()),
            (Err(error), _) => {
                self.notice = Some(t!(error.label()).into());
                cx.notify();
                return;
            }
            (_, None) => {
                self.notice = Some(t!("translate.error_network").into());
                cx.notify();
                return;
            }
        };
        let key = request.endpoint.load_key(cx);
        self.testing = true;
        self.notice = None;
        let work = cx.background_spawn(async move {
            let key = key.await?;
            let (sender, mut receiver) = mpsc::channel(8);
            let (result, ()) = futures::join!(client.translate(request, key, sender), async move {
                while receiver.next().await.is_some() {}
            });
            result
        });
        self.test = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.testing = false;
                this.notice = Some(match result {
                    Ok(Completion::Complete) => t!("translate.test_success").into(),
                    Ok(Completion::Truncated) => t!("translate.truncated").into(),
                    Err(error) => t!(error.label()).into(),
                });
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for TranslateSettingsView {
    /// Keeps secrets out of ordinary preferences and explains every disabled save/test action.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.clear_key) {
            self.key
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        let dirty = self.endpoint.read(cx).value().trim() != self.active.endpoint
            || self.model.read(cx).value().trim() != self.active.model;
        let has_key_draft = !self.key.read(cx).unmask_value().trim().is_empty();
        let source = if self.key_busy {
            t!("translate.key_loading")
        } else {
            t!(self
                .source
                .map(KeySource::label)
                .unwrap_or("translate.key_missing_source"))
        };
        let muted = cx.theme().muted_foreground;
        let host = self
            .endpoint
            .read(cx)
            .value()
            .parse::<Endpoint>()
            .ok()
            .filter(|url| !url.is_official())
            .map(|url| url.host().to_string());
        v_flex()
            .gap_3()
            .pt_5()
            .min_w_0()
            .child(
                div()
                    .font_medium()
                    .child(t!("translate.connection").to_string()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(t!("translate.endpoint").to_string())
                    .child(Input::new(&self.endpoint))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(t!("translate.endpoint_hint").to_string()),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(t!("translate.model").to_string())
                    .child(Input::new(&self.model)),
            )
            .children(host.map(|host| {
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("translate.custom_host", host = host).to_string())
            }))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("save-translation-connection")
                            .primary()
                            .label(t!("translate.save_connection"))
                            .disabled(!dirty || self.key_busy || self.testing)
                            .on_click(cx.listener(Self::save_connection)),
                    )
                    .child(
                        Button::new("restore-translation-defaults")
                            .ghost()
                            .label(t!("translate.restore_defaults"))
                            .disabled(self.key_busy)
                            .on_click(cx.listener(Self::restore_defaults)),
                    ),
            )
            .when(dirty, |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("translate.connection_dirty").to_string()),
                )
            })
            .child(
                div()
                    .mt_3()
                    .pt_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .font_medium()
                    .child(t!("translate.key_title").to_string()),
            )
            .child(div().text_sm().child(source.to_string()))
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("translate.key_hint").to_string()),
            )
            .child(
                Input::new(&self.key)
                    .mask_toggle()
                    .disabled(dirty || self.key_busy)
                    .aria_label(t!("translate.key_title")),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("save-translation-key")
                            .outline()
                            .label(t!("translate.save_key"))
                            .disabled(dirty || self.key_busy || !has_key_draft)
                            .on_click(cx.listener(|this, _, _, cx| this.change_key(false, cx))),
                    )
                    .child(
                        Button::new("remove-translation-key")
                            .ghost()
                            .label(t!("translate.remove_key"))
                            .disabled(
                                dirty || self.key_busy || self.source != Some(KeySource::Saved),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.change_key(true, cx))),
                    ),
            )
            .children(
                self.key_error
                    .as_ref()
                    .filter(|error| **error != TranslateError::MissingKey)
                    .map(|error| {
                        div()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(t!(error.label()).to_string())
                    }),
            )
            .child(
                h_flex()
                    .mt_3()
                    .gap_3()
                    .flex_wrap()
                    .child(
                        Button::new("test-translation-connection")
                            .outline()
                            .label(t!(if self.testing {
                                "translate.testing"
                            } else {
                                "translate.test_connection"
                            }))
                            .disabled(
                                dirty
                                    || has_key_draft
                                    || self.key_busy
                                    || self.testing
                                    || self.source.is_none(),
                            )
                            .on_click(cx.listener(Self::test_connection)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(t!("translate.test_hint").to_string()),
                    ),
            )
            .children(
                self.notice
                    .as_ref()
                    .map(|notice| div().text_sm().child(notice.clone())),
            )
    }
}
