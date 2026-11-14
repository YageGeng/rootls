//! Paste interception for the launcher's single-line search box (spec §4.2).
//!
//! A single-line input drops newlines (breaking YAML) and renders huge text
//! slowly, so multi-line or large pastes are kept aside and shown as a chip.

/// Single-line pastes up to this many bytes stay in the search box.
pub const PASTE_INLINE_LIMIT: usize = 4 * 1024;

/// Number of characters shown in the chip's preview line.
const PREVIEW_CHARS: usize = 80;

/// Pasted text held outside the search box.
pub struct PastedInput {
    text: String,
}

impl PastedInput {
    /// `Some` for multi-line text or text larger than [`PASTE_INLINE_LIMIT`] bytes.
    pub fn intercept(text: String) -> Option<Self> {
        (text.contains('\n') || text.len() > PASTE_INLINE_LIMIT).then_some(Self { text })
    }

    /// The pasted text, unchanged.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Consumes the paste, returning the text.
    pub fn into_text(self) -> String {
        self.text
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

    /// First characters with whitespace runs collapsed to one space; "…" when cut.
    /// Stops scanning as soon as the preview is full, so huge pastes stay cheap.
    pub fn preview(&self) -> String {
        let mut preview = String::new();
        let mut count = 0;
        let mut pending_space = false;
        for ch in self.text.trim().chars() {
            if ch.is_whitespace() {
                pending_space = true;
                continue;
            }
            let needed = if pending_space { 2 } else { 1 };
            if count + needed > PREVIEW_CHARS {
                preview.push('…');
                return preview;
            }
            if pending_space {
                preview.push(' ');
                pending_space = false;
            }
            preview.push(ch);
            count += needed;
        }
        preview
    }
}
