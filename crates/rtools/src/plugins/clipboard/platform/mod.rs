//! Native monitoring and atomic publication of a single clipboard offer.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(super) use macos::{watch, write};
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(super) use linux::{watch, write};
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(super) use windows::{watch, write};

// AppKit rejects MIME strings as pasteboard types; its marker must be a valid UTI.
#[cfg(target_os = "macos")]
const OWNER: &str = "dev.rtools.clipboard.replay";
#[cfg(not(target_os = "macos"))]
const OWNER: &str = "application/x-rtools-history-replay";
