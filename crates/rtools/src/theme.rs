//! Product theme tokens layered over the GPUI Kit theme (spec §5.7).

use std::sync::Arc;

use gpui_kit::component::Theme;
use gpui_kit::{App, px};

use crate::settings::Settings;

/// Follows the system light/dark appearance, then applies the user's fonts and
/// window radius and rtools' theme tokens on top; call again whenever either may
/// have changed, because a theme change rebuilds these fields from the base theme.
pub fn apply(settings: &Settings, cx: &mut App) {
    Theme::sync_system_appearance(None, cx);
    Theme::update(cx, |theme| {
        // Re-reading the theme keeps a previously chosen family (the built-in
        // themes name no fonts), so "theme default" is set explicitly from GPUI
        // Kit's defaults, which resolve to real families.
        let defaults = Theme::default();
        theme.font_family = settings
            .ui_font
            .clone()
            .map_or(defaults.font_family, Into::into);
        theme.mono_font_family = settings
            .mono_font
            .clone()
            .map_or(defaults.mono_font_family, Into::into);
        // Editors size their text from `mono_font_size`; keep its default ratio to the rem.
        theme.mono_font_size =
            defaults.mono_font_size * (settings.font_size / f32::from(defaults.font_size));
        theme.font_size = px(settings.font_size);
        theme.window_radius = px(settings.window_radius);
        // Editors and their line-number gutters share the window background so
        // the plugin reads as one surface.
        let mut highlight = (*theme.highlight_theme).clone();
        highlight.style.editor_background = Some(theme.background);
        highlight.style.editor_gutter_background = Some(theme.background);
        theme.highlight_theme = Arc::new(highlight);
    });
}
