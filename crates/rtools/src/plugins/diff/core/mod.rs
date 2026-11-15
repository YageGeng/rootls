//! Pure comparison logic for the diff plugin; must not depend on gpui.

mod compare;
mod language;

pub use compare::{CompareOptions, Comparison, FormatJson, Side, Texts};
pub use language::Language;
