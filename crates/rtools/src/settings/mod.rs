//! User preferences persisted to `settings.toml` (spec §4.6, §4.7), and the
//! window that edits them.

pub mod window;

use std::fmt;
use std::fs;
use std::io;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Global, Window, WindowAppearance};
use serde::{Deserialize, Serialize};

/// Accepted UI font sizes in pixels; values outside are clamped.
pub const FONT_SIZES: RangeInclusive<f32> = 12.0..=20.0;
/// Accepted window corner radii in pixels; values outside are clamped.
pub const WINDOW_RADII: RangeInclusive<f32> = 0.0..=24.0;

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
    /// Corner radius of the launcher and the other client-drawn windows, within
    /// [`WINDOW_RADII`].
    pub window_radius: f32,
    /// Interface language.
    pub language: Language,
    /// Color mode shared by the interface and code editors.
    pub appearance: Appearance,
    /// Registered theme names used in each color mode.
    pub light_theme: String,
    pub dark_theme: String,
    /// The `[json]` table; tables come last because TOML writes them after
    /// plain keys.
    pub json: JsonSettings,
    /// The `[diff]` table, changed only from the diff plugin's compare view.
    pub diff: DiffSettings,
}

/// Options of the JSON editor plugin.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JsonSettings {
    /// Wrap long lines in both editors; wrapping measures every line, which
    /// stalls on very large documents.
    pub soft_wrap: bool,
}

/// Options of the text diff plugin's compare view.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffSettings {
    /// Side-by-side or single-column diff.
    pub layout: DiffLayout,
    /// Lines that differ only in whitespace count as equal.
    pub ignore_whitespace: bool,
    /// Lines that differ only in case count as equal.
    pub ignore_case: bool,
    /// Compare both sides as key-sorted, formatted JSON when both parse.
    pub format_json: bool,
    /// Emphasize changed characters instead of changed words.
    pub char_level: bool,
    /// Fold unchanged lines to a few lines of context around each change;
    /// off shows every line.
    pub fold_unchanged: bool,
}

/// How the compare view lays out a diff.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffLayout {
    /// Original and modified side by side.
    #[default]
    Split,
    /// One column with removed lines above added ones.
    Unified,
}

/// The app's color preference; system mode follows the operating system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    /// Resolves the effective theme without allowing OS changes to override an explicit choice.
    pub fn resolve(self, system: WindowAppearance) -> ThemeMode {
        match self {
            Self::System => system.into(),
            Self::Light => ThemeMode::Light,
            Self::Dark => ThemeMode::Dark,
        }
    }
}

impl Default for Settings {
    /// Theme fonts, 16 px, 4 px window corners, system language and appearance,
    /// no soft wrap and a split word-level diff with every comparison switch off.
    fn default() -> Self {
        Self {
            ui_font: None,
            mono_font: None,
            font_size: 16.,
            window_radius: 4.,
            language: Language::System,
            appearance: Appearance::System,
            light_theme: "Default Light".into(),
            dark_theme: "Default Dark".into(),
            json: JsonSettings::default(),
            diff: DiffSettings::default(),
        }
    }
}

impl FromStr for Settings {
    type Err = toml::de::Error;

    /// Parses settings TOML; missing fields take their defaults and sizes are
    /// clamped to their ranges.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut settings: Settings = toml::from_str(text)?;
        settings.font_size = settings
            .font_size
            .clamp(*FONT_SIZES.start(), *FONT_SIZES.end());
        settings.window_radius = settings
            .window_radius
            .clamp(*WINDOW_RADII.start(), *WINDOW_RADII.end());
        Ok(settings)
    }
}

impl fmt::Display for Settings {
    /// Writes the settings as TOML.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&toml::to_string(self).map_err(|_| fmt::Error)?)
    }
}

/// The settings file, remembering whether its last read failed to parse: the
/// defaults then stand in for it, and the first save keeps the user's text as
/// `settings.toml.broken` instead of overwriting it.
pub struct SettingsFile {
    path: PathBuf,
    broken: bool,
}

impl SettingsFile {
    /// The file at `path`; nothing is read yet.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            broken: false,
        }
    }

    /// Where the settings persist.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the file; a missing file gives the defaults, and so does a broken
    /// one (with a warning), which is left untouched for the user to fix.
    pub fn load(&mut self) -> Settings {
        self.broken = false;
        match fs::read_to_string(&self.path) {
            Ok(text) => text.parse().unwrap_or_else(|err| {
                tracing::warn!(
                    "ignoring invalid settings file {}: {}",
                    self.path.display(),
                    err
                );
                self.broken = true;
                Settings::default()
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Settings::default(),
            Err(err) => {
                tracing::warn!(
                    "reading settings file {} failed: {}",
                    self.path.display(),
                    err
                );
                Settings::default()
            }
        }
    }

    /// Writes `settings` atomically (temp file, then rename). A file that
    /// failed to parse is first moved aside, so a change made while the
    /// defaults stand in never destroys the user's edits.
    pub fn save(&mut self, settings: &Settings) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        if self.broken {
            let aside = self.path.with_extension("toml.broken");
            fs::rename(&self.path, &aside)?;
            tracing::warn!("kept the invalid settings file as {}", aside.display());
            self.broken = false;
        }
        let temp = self.path.with_extension("toml.tmp");
        fs::write(&temp, settings.to_string())?;
        fs::rename(&temp, &self.path)
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
    pub file: SettingsFile,
    pub settings: Settings,
}

impl Global for SettingsStore {}

impl SettingsStore {
    /// Loads `<config dir>/rtools/settings.toml` and applies it; must run
    /// before any window opens.
    pub fn init(cx: &mut App) {
        Self::register_themes(cx);
        let mut file = SettingsFile::new(
            dirs::config_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("rtools")
                .join("settings.toml"),
        );
        let settings = file.load();
        cx.set_global(Self { file, settings });
        Self::apply(cx, None);
    }

    /// Keeps every live window in sync with OS appearance changes while following the system.
    pub fn observe_appearance(window: &Window) {
        window
            .observe_window_appearance(|window, cx| {
                let settings = &cx.global::<Self>().settings;
                // A shared theme needs only one update even when several windows observe the change.
                if settings.appearance == Appearance::System
                    && settings.appearance.resolve(window.appearance()) != Theme::global(cx).mode
                {
                    settings.clone().apply_theme(cx);
                }
            })
            .detach();
    }

    /// Re-reads manual edits and applies changed preferences or system appearance.
    /// The file is never written here.
    pub fn reload(cx: &mut App) {
        let store = cx.global_mut::<Self>();
        let settings = store.file.load();
        let before = std::mem::replace(&mut store.settings, settings);
        Self::apply(cx, Some(&before));
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
        if store.settings.appearance != before.appearance
            || store.settings.light_theme != before.light_theme
            || store.settings.dark_theme != before.dark_theme
        {
            tracing::info!(
                "theme preferences changed: mode {:?}, light {}, dark {}",
                store.settings.appearance,
                store.settings.light_theme,
                store.settings.dark_theme
            );
        }
        if let Err(err) = store.file.save(&store.settings) {
            tracing::warn!(
                "saving settings to {} failed: {}",
                store.file.path().display(),
                err
            );
        }
        Self::apply(cx, Some(&before));
    }

    /// Applies changed global preferences, loading the theme unconditionally at startup.
    fn apply(cx: &mut App, before: Option<&Settings>) {
        let settings = cx.global::<Self>().settings.clone();
        let locale = settings.language.resolve();
        let locale_changed = &*rust_i18n::locale() != locale;
        if locale_changed {
            tracing::info!("applying interface language {}", locale);
            rust_i18n::set_locale(locale);
        }
        if before.is_none_or(|before| settings.theme_changed(before))
            || settings.appearance.resolve(cx.window_appearance()) != Theme::global(cx).mode
        {
            settings.apply_theme(cx);
        } else if locale_changed {
            // Labels in windows without a settings subscription must also be redrawn.
            cx.refresh_windows();
        }
    }
}
