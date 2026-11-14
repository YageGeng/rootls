//! Global launcher shortcut (spec §4.3): double-tap Ctrl only. No key-combo
//! fallback is registered — Alt+Space belongs to KRunner on KDE.

pub mod double_tap;
#[cfg(target_os = "linux")]
mod evdev_source;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod rdev_source;

#[cfg(target_os = "linux")]
pub use evdev_source::start;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use rdev_source::start;
