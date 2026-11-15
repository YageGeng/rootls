//! Pure JSON logic shared by the plugin UI; must not depend on gpui.

pub mod convert;
mod detect;
mod error;
mod filter;
mod format;

pub use detect::{InputKind, Normalized};
pub use error::JsonError;
pub use filter::{FILTER_TIMEOUT, FilterEngine};
pub(crate) use format::INDENT;
pub use format::{Json, Pretty, Rewrite};
