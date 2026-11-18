//! GPUI presentation of the JSON plugin (spec §5.1, §5.7).

mod view;

use gpui_kit::assets::IconName;
use gpui_kit::{App, AppContext as _, Window};

use crate::plugin::{
    ContentKind, ContentMatch, ContentRecognizer, Launch, Plugin, PluginManifest, PluginView,
};
use crate::plugins::json::core::InputKind;
use view::JsonPluginView;

/// Static metadata shown in the launcher.
static MANIFEST: PluginManifest = PluginManifest {
    id: "json",
    title: "json.title",
    description: "json.description",
    icon: IconName::Braces,
    triggers: &["json"],
    keywords: &["jsoneditor"],
};

/// The JSON editor plugin.
pub struct JsonPlugin;

impl JsonPlugin {
    /// Maps the core format assessment to a launcher rank and a translatable row description.
    fn recognize(input: &str) -> Option<ContentMatch> {
        let (format, verified) = InputKind::recognize(input)?;
        let message_key = match format {
            InputKind::Json if verified => "json.matches.json",
            InputKind::Json => "json.matches.json_hint",
            InputKind::Yaml => "json.matches.yaml",
            InputKind::Xml => "json.matches.xml",
            InputKind::UrlParams => "json.matches.url_params",
        };
        Some(ContentMatch {
            kind: if verified {
                ContentKind::Recognized
            } else {
                ContentKind::Hint
            },
            message_key,
        })
    }
}

impl Plugin for JsonPlugin {
    /// Returns the JSON editor's metadata.
    fn manifest(&self) -> &PluginManifest {
        &MANIFEST
    }

    /// Shares the same recognizer between inline edits and background paste processing.
    fn recognizer(&self) -> Option<ContentRecognizer> {
        Some(Self::recognize)
    }

    /// Builds a new editor view; the XML grammar of its result pane is
    /// registered at startup (`crate::ui::register_languages`).
    fn open(&self, launch: Launch, window: &mut Window, cx: &mut App) -> PluginView {
        let view = cx.new(|cx| JsonPluginView::new(launch, window, cx));
        PluginView::new(view, cx)
    }
}
