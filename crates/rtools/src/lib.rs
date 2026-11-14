//! rtools: a resident desktop launcher.
//!
//! The shell (`cli`, `instance`, `hotkey`, `tray`, `shell`, `launcher`,
//! `settings`, `theme`, `assets`) hosts built-in plugins through the contract in
//! `plugin`; the plugins themselves live under `plugins`. User-facing text comes
//! from `locales/app.yml` through `rust_i18n::t!`.

rust_i18n::i18n!("locales", fallback = "en");

pub mod assets;
pub mod cli;
pub mod hotkey;
pub mod instance;
pub mod launcher;
pub mod plugin;
pub mod plugins;
pub mod settings;
pub mod shell;
pub mod theme;
pub mod tray;
