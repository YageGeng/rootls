//! The contract between the rtools shell and its built-in plugins (spec §3).
//!
//! A plugin describes itself with a [`PluginManifest`], optionally recognizes
//! pasted or typed content through [`Plugin::recognizer`], and builds its root view
//! in [`Plugin::open`]. The shell finds plugins through a [`PluginRegistry`].
//! Plugins read their user preferences (their table in `settings.toml`) from the
//! [`crate::settings::SettingsStore`] global while rendering, and may keep
//! their text across runs with a [`Draft`].

mod draft;
mod registry;
mod search;

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::{AnyView, App, Entity, FocusHandle, Focusable, Render, Window};

pub use draft::{Autosave, Draft};
pub(crate) use registry::search_text;
pub use registry::{
    ContentKind, ContentMatch, ContentRecognizer, MatchKind, PluginMatch, PluginRegistry,
};
pub use search::SearchPanels;

/// Static metadata the shell uses to list and match a plugin.
pub struct PluginManifest {
    /// Stable identifier; also names the plugin's data directory.
    pub id: &'static str,
    /// Translation key of the display name (`locales/app.yml`).
    pub title: &'static str,
    /// Translation key of the one-line description shown under the title.
    pub description: &'static str,
    /// Launcher row and plugin header icon.
    pub icon: IconName,
    /// Exact command words supplied by the plugin developer.
    pub triggers: &'static [&'static str],
    /// Extra search keywords; the title always matches too.
    pub keywords: &'static [&'static str],
}

/// Everything a plugin receives when the shell opens it.
pub struct Launch {
    /// Content-match text (search text or pasted content); `None` when opened by keyword.
    pub input: Option<String>,
    /// The plugin's private data directory; it already exists.
    pub data_dir: PathBuf,
}

/// A plugin's root view together with the focus handle that should receive
/// keyboard input whenever the shell shows it.
pub struct PluginView {
    /// The view to mount.
    pub view: AnyView,
    /// Where keyboard focus goes when the view is shown.
    pub focus: FocusHandle,
}

impl PluginView {
    /// Wraps a focusable root view.
    pub fn new<V: Render + Focusable>(entity: Entity<V>, cx: &App) -> Self {
        let focus = entity.read(cx).focus_handle(cx);
        Self {
            view: entity.into(),
            focus,
        }
    }
}

/// A built-in plugin.
pub trait Plugin: 'static {
    /// Returns the plugin's static metadata.
    fn manifest(&self) -> &PluginManifest;

    /// Returns a pure matcher that can also run off the window thread on a large paste.
    fn recognizer(&self) -> Option<ContentRecognizer> {
        None
    }

    /// Builds the plugin root view.
    fn open(&self, launch: Launch, window: &mut Window, cx: &mut App) -> PluginView;

    /// Registers the plugin's key bindings, once at startup; most plugins
    /// have none of their own.
    fn bind_keys(&self, _cx: &mut App) {}
}
