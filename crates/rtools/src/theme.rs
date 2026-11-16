//! Bundled themes and product overrides shared by every rtools window.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeRegistry};
use gpui_kit::{App, px};

use crate::settings::{Settings, SettingsStore};

/// Official GPUI Kit v0.9.1 presets, embedded so app bundles work from any directory.
const PRESETS: &[(&str, &str)] = &[
    (
        "adventure.json",
        include_str!("../../../assets/themes/adventure.json"),
    ),
    (
        "alduin.json",
        include_str!("../../../assets/themes/alduin.json"),
    ),
    (
        "asciinema.json",
        include_str!("../../../assets/themes/asciinema.json"),
    ),
    (
        "aurora.json",
        include_str!("../../../assets/themes/aurora.json"),
    ),
    ("ayu.json", include_str!("../../../assets/themes/ayu.json")),
    (
        "catppuccin.json",
        include_str!("../../../assets/themes/catppuccin.json"),
    ),
    (
        "everforest.json",
        include_str!("../../../assets/themes/everforest.json"),
    ),
    (
        "fahrenheit.json",
        include_str!("../../../assets/themes/fahrenheit.json"),
    ),
    (
        "flexoki.json",
        include_str!("../../../assets/themes/flexoki.json"),
    ),
    (
        "gruvbox.json",
        include_str!("../../../assets/themes/gruvbox.json"),
    ),
    (
        "harper.json",
        include_str!("../../../assets/themes/harper.json"),
    ),
    (
        "hybrid.json",
        include_str!("../../../assets/themes/hybrid.json"),
    ),
    (
        "jellybeans.json",
        include_str!("../../../assets/themes/jellybeans.json"),
    ),
    (
        "kibble.json",
        include_str!("../../../assets/themes/kibble.json"),
    ),
    (
        "macos-classic.json",
        include_str!("../../../assets/themes/macos-classic.json"),
    ),
    (
        "mellifluous.json",
        include_str!("../../../assets/themes/mellifluous.json"),
    ),
    (
        "molokai.json",
        include_str!("../../../assets/themes/molokai.json"),
    ),
    (
        "solarized.json",
        include_str!("../../../assets/themes/solarized.json"),
    ),
    (
        "spaceduck.json",
        include_str!("../../../assets/themes/spaceduck.json"),
    ),
    (
        "tokyonight.json",
        include_str!("../../../assets/themes/tokyonight.json"),
    ),
    (
        "twilight.json",
        include_str!("../../../assets/themes/twilight.json"),
    ),
];

impl SettingsStore {
    /// Registers bundled presets before settings are applied or windows are created.
    pub(crate) fn register_themes(cx: &mut App) {
        let registry = ThemeRegistry::global_mut(cx);
        for (file, content) in PRESETS {
            if let Err(err) = registry.load_themes_from_str(content) {
                tracing::error!("loading bundled theme {} failed: {}", file, err);
            }
        }
        tracing::info!("registered {} available themes", registry.themes().len());
    }
}

impl Settings {
    /// Detects changes to shared theme inputs independently of language and plugin options.
    pub(crate) fn theme_changed(&self, before: &Self) -> bool {
        self.appearance != before.appearance
            || self.light_theme != before.light_theme
            || self.dark_theme != before.dark_theme
            || self.ui_font != before.ui_font
            || self.mono_font != before.mono_font
            || self.font_size != before.font_size
            || self.window_radius != before.window_radius
    }

    /// Resolves a configured name, preserving the expected mode on invalid manual edits.
    fn selected_theme(
        &self,
        registry: &ThemeRegistry,
        fallback: &Rc<ThemeConfig>,
    ) -> Rc<ThemeConfig> {
        let name = if fallback.mode.is_dark() {
            &self.dark_theme
        } else {
            &self.light_theme
        };
        match registry
            .themes()
            .get(name.as_str())
            .filter(|config| config.mode == fallback.mode)
        {
            Some(config) => Rc::clone(config),
            None => {
                tracing::warn!(
                    "theme {:?} is unavailable for {} mode; using {}",
                    name,
                    fallback.mode.name(),
                    fallback.name
                );
                Rc::clone(fallback)
            }
        }
    }

    /// Applies the selected preset, then restores user fonts, scale and window corners.
    pub(crate) fn apply_theme(&self, cx: &mut App) {
        let mode = self.appearance.resolve(cx.window_appearance());
        let registry = ThemeRegistry::global(cx);
        let light = self.selected_theme(registry, registry.default_light_theme());
        let dark = self.selected_theme(registry, registry.default_dark_theme());
        let selected = if mode.is_dark() { &dark } else { &light };
        let is_default = selected.is_default;
        if Theme::global(cx).theme_name() != &selected.name {
            tracing::info!("applying {} theme ({})", selected.name, mode.name());
        }
        // Register both presets without projecting an intermediate theme; `change` syncs it below.
        let theme = Theme::global_mut(cx);
        theme.light_theme = light;
        theme.dark_theme = dark;
        // A preset with no shadow preference must not inherit the previous preset's choice.
        theme.shadow = Theme::default().shadow;
        Theme::change(mode, None, cx);
        Theme::update(cx, |theme| {
            // Re-reading the theme keeps a previously chosen family (the built-in
            // themes name no fonts), so "theme default" is set explicitly from GPUI
            // Kit's defaults, which resolve to real families.
            let defaults = Theme::default();
            theme.font_family = self
                .ui_font
                .clone()
                .map_or(defaults.font_family, Into::into);
            theme.mono_font_family = self
                .mono_font
                .clone()
                .map_or(defaults.mono_font_family, Into::into);
            // Editors size their text from `mono_font_size`; keep its default ratio to the rem.
            theme.mono_font_size =
                defaults.mono_font_size * (self.font_size / f32::from(defaults.font_size));
            theme.font_size = px(self.font_size);
            theme.window_radius = px(self.window_radius);
            // Keep the default appearance, while letting presets own every syntax color and surface.
            if is_default {
                let mut highlight = (*theme.highlight_theme).clone();
                highlight.style.editor_background = Some(theme.background);
                highlight.style.editor_gutter_background = Some(theme.background);
                highlight.style.syntax.property = highlight.style.syntax.tag;
                theme.highlight_theme = Arc::new(highlight);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use gpui_kit::component::ThemeSet;

    use super::*;

    /// Plugin options and language changes must not rebuild the shared theme.
    #[test]
    fn theme_changes_track_only_appearance_preferences() {
        let before = Settings::default();
        for text in [
            "",
            "language = \"zh-CN\"",
            "[json]\nsoft_wrap = true",
            "[diff]\nlayout = \"unified\"\nignore_whitespace = true\nignore_case = true\nformat_json = true\nchar_level = true\nfold_unchanged = true",
        ] {
            let settings: Settings = text.parse().unwrap();
            assert!(!settings.theme_changed(&before), "{text}");
        }
        for text in [
            "appearance = \"dark\"",
            "light_theme = \"Ayu Light\"",
            "dark_theme = \"Ayu Dark\"",
            "ui_font = \"Inter\"",
            "mono_font = \"JetBrains Mono\"",
            "font_size = 18",
            "window_radius = 8",
        ] {
            let settings: Settings = text.parse().unwrap();
            assert!(settings.theme_changed(&before), "{text}");
            assert!(before.theme_changed(&settings), "resetting {text}");
        }
    }

    /// Every shipped file must load without duplicate names or missing editor colors.
    #[test]
    fn bundled_presets_are_complete_and_loadable() {
        let mut names = HashSet::new();
        let mut registry = ThemeRegistry::default();
        for (file, content) in PRESETS {
            let set: ThemeSet = serde_json::from_str(content).expect(file);
            assert!(!set.themes.is_empty(), "{file}");
            for config in set.themes {
                assert!(
                    names.insert(config.name.clone()),
                    "duplicate {}",
                    config.name
                );
                assert!(!config.is_default, "{} replaces a default", config.name);
                let style = config.highlight.as_ref().expect("preset syntax colors");
                assert!(style.syntax.string.is_some(), "{} strings", config.name);
                assert!(style.syntax.number.is_some(), "{} numbers", config.name);
            }
            registry.load_themes_from_str(content).expect(file);
        }
        assert_eq!(PRESETS.len(), 21);
        assert_eq!(registry.themes().len(), 36);
        for name in [
            "Catppuccin Latte",
            "Catppuccin Mocha",
            "Tokyo Night",
            "Solarized Light",
        ] {
            assert!(registry.themes().contains_key(name), "missing {name}");
        }
    }

    /// Invalid names and presets of the other mode use only the matching fallback.
    #[test]
    fn selected_presets_preserve_the_requested_mode() {
        let mut registry = ThemeRegistry::default();
        registry
            .load_themes_from_str(include_str!("../../../assets/themes/catppuccin.json"))
            .unwrap();
        let light: Rc<ThemeConfig> = Rc::new(
            serde_json::from_str(r#"{"name":"Default Light","mode":"light","is_default":true}"#)
                .unwrap(),
        );
        let dark: Rc<ThemeConfig> = Rc::new(
            serde_json::from_str(r#"{"name":"Default Dark","mode":"dark","is_default":true}"#)
                .unwrap(),
        );
        let mut settings = Settings {
            light_theme: "Catppuccin Latte".into(),
            dark_theme: "Catppuccin Mocha".into(),
            ..Settings::default()
        };
        assert_eq!(
            settings.selected_theme(&registry, &light).name,
            "Catppuccin Latte"
        );
        assert_eq!(
            settings.selected_theme(&registry, &dark).name,
            "Catppuccin Mocha"
        );
        for (light_name, dark_name) in [("missing", ""), ("Catppuccin Mocha", "Catppuccin Latte")] {
            settings.light_theme = light_name.into();
            settings.dark_theme = dark_name.into();
            assert!(Rc::ptr_eq(
                &settings.selected_theme(&registry, &light),
                &light
            ));
            assert!(Rc::ptr_eq(
                &settings.selected_theme(&registry, &dark),
                &dark
            ));
        }
    }
}
