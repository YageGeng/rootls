//! Presentation pieces shared by the launcher and the built-in plugins, so
//! every surface draws them the same way. Plugins still never depend on each
//! other: they share only these pieces and the `crate::plugin` contract.

mod button;
mod choice;
mod status;
mod toolbar;

use gpui_kit::component::highlighter::{LanguageConfig, LanguageRegistry};

pub use button::ButtonExt;
pub use choice::Choice;
pub use status::StatusPage;
pub use toolbar::Toolbar;

/// GPUI Kit's key context of its inputs and editors, where it binds Enter and
/// Ctrl+Enter (Cmd+Enter); key hints for those look their binding up here.
pub const INPUT_CONTEXT: &str = "Input";

/// Registers the grammars GPUI Kit does not ship, once at startup: XML, for
/// the JSON editor's XML output and the text diff (GPUI Kit's HTML grammar
/// breaks on `_` in tag names).
pub fn register_languages() {
    let xml = LanguageConfig::new(
        "xml",
        tree_sitter_xml::LANGUAGE_XML.into(),
        Vec::new(),
        tree_sitter_xml::XML_HIGHLIGHT_QUERY,
        "",
        "",
    );
    LanguageRegistry::singleton().register("xml", &xml);
}
