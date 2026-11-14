//! The contract between the rtools shell and its built-in plugins (spec §3).
//!
//! A plugin describes itself with a [`PluginManifest`], optionally recognizes
//! pasted or typed content through [`Plugin::accepts`], and builds its root view
//! in [`Plugin::open`]. The shell finds plugins through a [`PluginRegistry`].
//! Plugins read their user preferences (their table in `settings.toml`) from the
//! [`crate::settings::SettingsStore`] global while rendering.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::{AnyView, App, Entity, FocusHandle, Focusable, Render, SharedString, Window};
use rust_i18n::t;

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
    /// Extra search keywords; the title always matches too.
    pub keywords: &'static [&'static str],
}

impl PluginManifest {
    /// Whether the lowercase `query` is a substring of the localized title or a keyword.
    pub(crate) fn matches_keyword(&self, query: &str) -> bool {
        t!(self.title).to_lowercase().contains(query)
            || self
                .keywords
                .iter()
                .any(|keyword| keyword.to_lowercase().contains(query))
    }
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

    /// Cheap content match: `Some(reason)` (e.g. "Recognized as JSON") when the plugin
    /// can handle the raw text, shown as the row's secondary line.
    fn accepts(&self, _input: &str) -> Option<SharedString> {
        None
    }

    /// Builds the plugin root view.
    fn open(&self, launch: Launch, window: &mut Window, cx: &mut App) -> PluginView;
}

/// Why a plugin appears in search results.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchKind {
    /// The title or a keyword contains the query (or the query is empty).
    Keyword,
    /// The plugin recognized the text; carries the plugin's reason.
    Content(SharedString),
}

/// One search hit, pointing back into the registry.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginMatch {
    /// Position of the plugin in the registry.
    pub index: usize,
    /// Why the plugin matched.
    pub kind: MatchKind,
}

/// All plugins compiled into the binary, in registration order.
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl FromIterator<Box<dyn Plugin>> for PluginRegistry {
    /// Registers plugins in iteration order.
    fn from_iter<I: IntoIterator<Item = Box<dyn Plugin>>>(iter: I) -> Self {
        Self {
            plugins: iter.into_iter().collect(),
        }
    }
}

impl PluginRegistry {
    /// The plugin at `index`; indexes come from [`Self::search`], so a bad one is a bug.
    pub fn plugin(&self, index: usize) -> &dyn Plugin {
        self.plugins[index].as_ref()
    }

    /// Keyword hits first, then content hits; a plugin appears at most once.
    /// An empty query lists every plugin as a keyword hit.
    pub fn search(&self, query: &str) -> Vec<PluginMatch> {
        let needle = query.trim().to_lowercase();
        let keyword: Vec<PluginMatch> = self
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, plugin)| plugin.manifest().matches_keyword(&needle))
            .map(|(index, _)| PluginMatch {
                index,
                kind: MatchKind::Keyword,
            })
            .collect();
        let content = self
            .search_content(query)
            .into_iter()
            .filter(|hit| !keyword.iter().any(|seen| seen.index == hit.index));
        keyword.iter().cloned().chain(content).collect()
    }

    /// Content hits only, used for pasted text where keywords are meaningless.
    pub fn search_content(&self, text: &str) -> Vec<PluginMatch> {
        self.plugins
            .iter()
            .enumerate()
            .filter_map(|(index, plugin)| {
                plugin.accepts(text).map(|reason| PluginMatch {
                    index,
                    kind: MatchKind::Content(reason),
                })
            })
            .collect()
    }
}
