//! System tray icon (spec §4.5): left click toggles the launcher, the menu
//! opens the launcher or the settings, or quits. Events become `ShellCommand`s.

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod desktop;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use desktop::{Tray, start};
#[cfg(target_os = "linux")]
pub use linux::{Tray, start};

/// Hover title.
const TITLE: &str = "rtools";

/// Why the tray could not be shown.
#[derive(Debug, thiserror::Error)]
pub enum TrayError {
    #[error("preparing the tray icon failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("decoding the tray icon failed: {0}")]
    Image(#[from] image::ImageError),
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[error("the tray icon image was rejected: {0}")]
    Icon(#[from] tray_icon::BadIcon),
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[error("building the tray menu failed: {0}")]
    Menu(#[from] tray_icon::menu::Error),
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[error("creating the tray icon failed: {0}")]
    Tray(#[from] tray_icon::Error),
}
