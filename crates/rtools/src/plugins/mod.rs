//! Built-in plugins. Each lives in its own module and talks to the shell only
//! through the `crate::plugin` contract.

pub mod clipboard;
pub mod diff;
pub mod json;
pub mod translate;

/// Returns the application plugin catalog in its default launcher order.
pub fn builtins() -> [Box<dyn crate::plugin::Plugin>; 4] {
    [
        Box::new(json::ui::JsonPlugin),
        Box::new(diff::ui::DiffPlugin),
        Box::new(translate::TranslatePlugin),
        Box::new(clipboard::ClipboardPlugin),
    ]
}
