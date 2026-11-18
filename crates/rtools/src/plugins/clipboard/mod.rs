//! Opt-in, process-wide clipboard history with ordered, durable image groups.

mod content;
mod history;
mod platform;
pub mod service;
mod view;

use crate::plugin::{Launch, Plugin, PluginManifest, PluginView};
use gpui_kit::assets::IconName;
use gpui_kit::{App, AppContext as _, Window};

type Result<T> = std::result::Result<T, Error>;

/// User-visible failure classes keep copied data and source paths out of diagnostics.
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("user data directory unavailable")]
    DataDirectory,
    #[error("clipboard storage failure ({:?})", .0.sqlite_error_code())]
    Database(#[from] rusqlite::Error),
    #[error("clipboard file operation failed ({:?})", .0.kind())]
    Io(#[from] std::io::Error),
    #[error("invalid image data")]
    Image(#[from] image::ImageError),
    #[error("clipboard content exceeds the size limit")]
    Limit,
    #[error("clipboard item unavailable")]
    Missing,
    #[error("clipboard data or format invalid")]
    Invalid,
    #[error("system clipboard unavailable")]
    Platform,
    #[error("{} members remain available", .0.len())]
    Partial(Vec<i64>),
}

impl Error {
    /// Selects a localized explanation without inserting sensitive native diagnostics.
    fn key(&self) -> &'static str {
        match self {
            Self::Limit => "clipboard.limit",
            Self::Platform => "clipboard.unavailable",
            Self::Missing => "clipboard.missing",
            Self::Partial(_) => "clipboard.partial",
            Self::Image(image::ImageError::Limits(_)) => "clipboard.image_decode_limit",
            Self::Image(_) | Self::Invalid => "clipboard.invalid",
            Self::Database(_) | Self::Io(_) | Self::DataDirectory => "clipboard.storage_error",
        }
    }
}

/// Launcher entry for browsing history even while capture is stopped.
pub struct ClipboardPlugin;

impl Plugin for ClipboardPlugin {
    /// Identifies the plugin for search and its private data directory.
    fn manifest(&self) -> &PluginManifest {
        static MANIFEST: PluginManifest = PluginManifest {
            id: "clipboard",
            title: "clipboard.title",
            description: "clipboard.description",
            icon: IconName::Clipboard,
            triggers: &["jqb"],
            keywords: &["clipboard", "history", "剪贴板", "剪切板"],
        };
        &MANIFEST
    }

    /// Opens a browser over the resident recorder.
    fn open(&self, _: Launch, window: &mut Window, cx: &mut App) -> PluginView {
        PluginView::new(cx.new(|cx| view::HistoryView::new(window, cx)), cx)
    }

    /// Installs history navigation and copy actions.
    fn bind_keys(&self, cx: &mut App) {
        view::HistoryView::bind_keys(cx);
    }
}
