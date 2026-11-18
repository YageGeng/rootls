//! Paste interception for the launcher's single-line search box (spec §4.2).
//!
//! A single-line input drops newlines (breaking YAML) and renders huge text
//! slowly, so multi-line or large pastes are kept aside and shown as a chip.

use gpui_kit::SharedString;
use std::sync::Arc;

/// Single-line pastes up to this many bytes stay in the search box.
pub const PASTE_INLINE_LIMIT: usize = 4 * 1024;

/// Number of characters shown in the chip's preview line.
const PREVIEW_CHARS: usize = 80;

/// Pasted text held outside the search box.
pub struct PastedInput {
    pub(super) text: Arc<String>,
    preview: SharedString,
}

impl PastedInput {
    /// `Some` for multi-line text or text larger than [`PASTE_INLINE_LIMIT`] bytes.
    pub fn intercept(text: String) -> Option<Self> {
        if text.len() <= PASTE_INLINE_LIMIT && !text.contains(['\n', '\r']) {
            return None;
        }
        // Build the chip once. A whitespace run only takes a column if another word follows it.
        let mut preview = String::new();
        let mut columns = 0;
        let mut space = false;
        for ch in text.trim_start_matches('\u{feff}').chars() {
            if ch.is_whitespace() {
                space = !preview.is_empty();
                continue;
            }
            columns += 1 + usize::from(space);
            if columns > PREVIEW_CHARS {
                preview.push('…');
                break;
            }
            if space {
                preview.push(' ');
                space = false;
            }
            preview.push(ch);
        }
        Some(Self {
            text: Arc::new(text),
            preview: preview.into(),
        })
    }

    /// The pasted text, unchanged.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Consumes the paste, returning the text.
    pub fn into_text(self) -> String {
        Arc::unwrap_or_clone(self.text)
    }

    /// The paste's 1024-based size for the chip, e.g. "3.2 KB".
    pub fn size_label(&self) -> String {
        const KIB: f64 = 1024.0;
        let bytes = self.text.len();
        let size = bytes as f64;
        if size < KIB {
            format!("{bytes} B")
        } else if size < KIB * KIB {
            format!("{:.1} KB", size / KIB)
        } else {
            format!("{:.1} MB", size / (KIB * KIB))
        }
    }

    /// Shares the cached, whitespace-collapsed preview with the rendered chip.
    pub fn preview(&self) -> SharedString {
        self.preview.clone()
    }
}
