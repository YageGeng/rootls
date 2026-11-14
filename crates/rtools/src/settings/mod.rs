//! User preferences persisted to `settings.toml` (spec §4.6, §4.7), and the
//! window that edits them.

pub mod window;

use std::fmt;
use std::fs;
use std::io;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

use crate::theme;

/// Accepted UI font sizes in pixels; values outside are clamped.
pub const FONT_SIZES: RangeInclusive<f32> = 12.0..=20.0;

/// Everything the user can change in the settings window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// UI font family; `None` follows the theme.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_font: Option<String>,
    /// Editor (monospace) font family; `None` follows the theme.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mono_font: Option<String>,
    /// Base UI font size in pixels (the rem size), within [`FONT_SIZES`].
    pub font_size: f32,
    /// Interface language.
    pub language: Language,
}

impl Default for Settings {
    /// Theme fonts, 16 px and the system language.
    fn default() -> Self {
        Self {
            ui_font: None,
            mono_font: None,
            font_size: 16.,
            language: Language::System,
        }
    }
}

impl FromStr for Settings {
    type Err = toml::de::Error;

    /// Parses settings TOML; missing fields take their defaults.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut settings: Settings = toml::from_str(text)?;
        settings.font_size = settings
            .font_size
            .clamp(*FONT_SIZES.start(), *FONT_SIZES.end());
        Ok(settings)
    }
}

impl fmt::Display for Settings {
    /// Writes the settings as TOML.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&toml::to_string(self).map_err(|_| fmt::Error)?)
    }
}

impl Settings {
    /// Reads `path`; a missing file gives the defaults, and so does a broken one
    /// (with a warning), which is left untouched for the user to fix.
    pub fn load(path: &Path) -> Settings {
        match fs::read_to_string(path) {
            Ok(text) => text.parse().unwrap_or_else(|err| {
                tracing::warn!("ignoring invalid settings file {}: {}", path.display(), err);
                Settings::default()
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Settings::default(),
            Err(err) => {
                tracing::warn!("reading settings file {} failed: {}", path.display(), err);
                Settings::default()
            }
        }
    }

    /// Writes the settings to `path` atomically (temp file, then rename).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("toml.tmp");
        fs::write(&temp, self.to_string())?;
        fs::rename(&temp, path)
    }
}

/// Interface language; `System` follows the operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en")]
    En,
}

impl Language {
    /// The rust-i18n locale to use, reading the OS language for `System`.
    pub fn resolve(self) -> &'static str {
        match self {
            Self::System => {
                Self::from(sys_locale::get_locale().as_deref().unwrap_or("en")).resolve()
            }
            Self::ZhCn => "zh-CN",
            Self::En => "en",
        }
    }
}

impl From<&str> for Language {
    /// Maps an OS locale tag (`zh-Hans-CN`, `zh_TW`, `en-US`) to a shipped
    /// language: any Chinese variant reads Simplified Chinese, the rest English.
    fn from(tag: &str) -> Self {
        if tag.to_ascii_lowercase().starts_with("zh") {
            Self::ZhCn
        } else {
            Self::En
        }
    }
}

/// The settings in effect and the file they persist to; an app global.
pub struct SettingsStore {
    pub path: PathBuf,
    pub settings: Settings,
}

impl Global for SettingsStore {}

impl SettingsStore {
    /// Loads `<config dir>/rtools/settings.toml` and applies it; must run
    /// before any window opens.
    pub fn init(cx: &mut App) {
        let path = dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("rtools")
            .join("settings.toml");
        let settings = Settings::load(&path);
        cx.set_global(Self { path, settings });
        Self::apply(cx);
    }

    /// Re-reads the settings file so edits made by hand take effect, and
    /// re-applies it, which also picks up a changed system appearance. The
    /// file is never written here.
    pub fn reload(cx: &mut App) {
        let store = cx.global_mut::<Self>();
        store.settings = Settings::load(&store.path);
        Self::apply(cx);
    }

    /// Changes the settings, applies them to every window right away and
    /// writes the file; a failed write is logged and the change still applies.
    pub fn update(cx: &mut App, change: impl FnOnce(&mut Settings)) {
        let store = cx.global_mut::<Self>();
        let before = store.settings.clone();
        change(&mut store.settings);
        if store.settings == before {
            return;
        }
        if let Err(err) = store.settings.save(&store.path) {
            tracing::warn!(
                "saving settings to {} failed: {}",
                store.path.display(),
                err
            );
        }
        Self::apply(cx);
    }

    /// Puts the settings in effect: the interface language, then the fonts and
    /// theme tokens, which refreshes every window.
    fn apply(cx: &mut App) {
        let settings = cx.global::<Self>().settings.clone();
        rust_i18n::set_locale(settings.language.resolve());
        theme::apply(&settings, cx);
    }
}
