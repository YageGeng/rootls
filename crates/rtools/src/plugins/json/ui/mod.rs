//! GPUI presentation of the JSON plugin (spec §5.1, §5.7).

mod view;

use gpui_kit::assets::IconName;
use gpui_kit::component::highlighter::{LanguageConfig, LanguageRegistry};
use gpui_kit::{App, AppContext as _, SharedString, Window};
use rust_i18n::t;
use serde_json::Value;

use crate::plugin::{Launch, Plugin, PluginManifest, PluginView};
use crate::plugins::json::core::InputKind;
use crate::plugins::json::core::convert::yaml::Yaml;
use view::JsonPluginView;

/// Static metadata shown in the launcher.
static MANIFEST: PluginManifest = PluginManifest {
    id: "json",
    title: "json.title",
    description: "json.description",
    icon: IconName::Braces,
    keywords: &["json", "jsoneditor"],
};

/// The JSON editor plugin.
pub struct JsonPlugin;

impl Plugin for JsonPlugin {
    /// Returns the JSON editor's metadata.
    fn manifest(&self) -> &PluginManifest {
        &MANIFEST
    }

    /// Recognizes JSON, XML and URL parameters by their prefix; multi-line text
    /// (a pasted chip, never a keystroke) is additionally tried as YAML.
    fn accepts(&self, input: &str) -> Option<SharedString> {
        let kind = InputKind::sniff(input).or_else(|| {
            // Parsing is enough to recognize YAML; formatting waits for the editor.
            (input.contains('\n') && Value::try_from(Yaml(input.trim())).is_ok())
                .then_some(InputKind::Yaml)
        })?;
        Some(t!("json.recognized", kind = kind.label()).into())
    }

    /// Builds a new editor view.
    fn open(&self, launch: Launch, window: &mut Window, cx: &mut App) -> PluginView {
        // GPUI Kit ships no XML grammar; the result pane needs one for XML
        // output (the HTML grammar breaks on `_` in tag names).
        let languages = LanguageRegistry::singleton();
        if languages.language("xml").is_none() {
            let xml = LanguageConfig::new(
                "xml",
                tree_sitter_xml::LANGUAGE_XML.into(),
                Vec::new(),
                tree_sitter_xml::XML_HIGHLIGHT_QUERY,
                "",
                "",
            );
            languages.register("xml", &xml);
        }
        let view = cx.new(|cx| JsonPluginView::new(launch, window, cx));
        PluginView::new(view, cx)
    }
}
