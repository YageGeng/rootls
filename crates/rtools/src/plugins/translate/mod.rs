//! Built-in Chinese-English translation through DeepSeek-compatible Chat Completions.
pub mod client;
pub mod core;
pub mod credentials;
pub mod session;
mod view;

use crate::plugin::{Launch, Plugin, PluginManifest, PluginView};
use gpui_kit::assets::IconName;
use gpui_kit::{App, AppContext as _, SharedString, Window};
use rust_i18n::t;
use view::TranslateView;

static MANIFEST: PluginManifest = PluginManifest {
    id: "translate",
    title: "translate.title",
    description: "translate.description",
    icon: IconName::Languages,
    keywords: &["translate", "translation", "fanyi", "翻译"],
};

/// Chinese-English translation is a built-in plugin, never a network search provider.
pub struct TranslatePlugin;

impl Plugin for TranslatePlugin {
    /// Returns static launcher metadata.
    fn manifest(&self) -> &PluginManifest {
        &MANIFEST
    }

    /// Recognizes nonempty text locally without reading credentials or calling the model.
    fn accepts(&self, input: &str) -> Option<SharedString> {
        (!input.trim().is_empty()).then(|| t!("translate.recognized").into())
    }

    /// Opens a fresh in-memory session seeded from pasted launcher content.
    fn open(&self, launch: Launch, window: &mut Window, cx: &mut App) -> PluginView {
        PluginView::new(cx.new(|cx| TranslateView::new(launch, window, cx)), cx)
    }

    /// Registers translation-only keyboard actions.
    fn bind_keys(&self, cx: &mut App) {
        TranslateView::bind_keys(cx);
    }
}
