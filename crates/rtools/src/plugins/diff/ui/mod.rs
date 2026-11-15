//! GPUI presentation of the text diff plugin (diff spec §2, §3).

mod view;

use gpui_kit::assets::IconName;
use gpui_kit::component::diff::DiffMode;
use gpui_kit::{App, AppContext as _, KeyBinding, Window};

use crate::plugin::{Launch, Plugin, PluginManifest, PluginView};
use crate::plugins::diff::core::CompareOptions;
use crate::settings::{DiffLayout, DiffSettings};
use view::{BackToEdit, COMPARE_CONTEXT, DiffPluginView, NextChange, PreviousChange};

/// Static metadata shown in the launcher.
static MANIFEST: PluginManifest = PluginManifest {
    id: "diff",
    title: "diff.title",
    description: "diff.description",
    icon: IconName::GitCompare,
    keywords: &["diff", "compare"],
};

/// The text diff plugin.
pub struct DiffPlugin;

impl Plugin for DiffPlugin {
    /// Returns the text diff's metadata.
    fn manifest(&self) -> &PluginManifest {
        &MANIFEST
    }

    /// Builds a new view in its edit state.
    fn open(&self, launch: Launch, window: &mut Window, cx: &mut App) -> PluginView {
        let view = cx.new(|cx| DiffPluginView::new(launch, window, cx));
        PluginView::new(view, cx)
    }

    /// The compare view's shortcuts: `Esc` back to editing, `Alt+↑` /
    /// `Alt+↓` to the previous / next change. Its key context sits inside the
    /// launcher's plugin context, so `Esc` returns to editing before it
    /// closes the plugin.
    fn bind_keys(&self, cx: &mut App) {
        cx.bind_keys([
            KeyBinding::new("escape", BackToEdit, Some(COMPARE_CONTEXT)),
            KeyBinding::new("alt-up", PreviousChange, Some(COMPARE_CONTEXT)),
            KeyBinding::new("alt-down", NextChange, Some(COMPARE_CONTEXT)),
        ]);
    }
}

impl From<&DiffSettings> for CompareOptions {
    /// Takes the three switches that decide which lines count as equal.
    fn from(settings: &DiffSettings) -> Self {
        Self {
            ignore_whitespace: settings.ignore_whitespace,
            ignore_case: settings.ignore_case,
            format_json: settings.format_json,
        }
    }
}

impl From<DiffLayout> for DiffMode {
    /// Maps the persisted layout to the component's display mode.
    fn from(layout: DiffLayout) -> Self {
        match layout {
            DiffLayout::Split => Self::Split,
            DiffLayout::Unified => Self::Unified,
        }
    }
}
