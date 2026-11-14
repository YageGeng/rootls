use rtools::settings::{Language, Settings};
use std::fs;

fn temp_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rtools-settings-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir.join("settings.toml")
}

#[test]
fn defaults_follow_the_theme_and_system() {
    let settings = Settings::default();
    assert_eq!(settings.ui_font, None);
    assert_eq!(settings.mono_font, None);
    assert_eq!(settings.font_size, 16.);
    assert_eq!(settings.language, Language::System);
}

#[test]
fn toml_round_trips() {
    let settings = Settings {
        ui_font: Some("Inter".into()),
        mono_font: Some("JetBrains Mono".into()),
        font_size: 14.,
        language: Language::ZhCn,
    };
    let text = settings.to_string();
    assert!(text.contains("language = \"zh-CN\""), "{text}");
    assert_eq!(text.parse::<Settings>().unwrap(), settings);
}

#[test]
fn missing_fields_take_defaults() {
    let settings = "font_size = 14".parse::<Settings>().unwrap();
    assert_eq!(
        settings,
        Settings {
            font_size: 14.,
            ..Settings::default()
        }
    );
}

#[test]
fn font_size_is_clamped() {
    assert_eq!("font_size = 40".parse::<Settings>().unwrap().font_size, 20.);
    assert_eq!("font_size = 2".parse::<Settings>().unwrap().font_size, 12.);
}

#[test]
fn missing_or_corrupt_file_loads_defaults_and_is_kept() {
    let path = temp_file("corrupt");
    assert_eq!(Settings::load(&path), Settings::default());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "font_size = [").unwrap();
    assert_eq!(Settings::load(&path), Settings::default());
    assert_eq!(fs::read_to_string(&path).unwrap(), "font_size = [");
}

#[test]
fn save_then_load_round_trips() {
    let path = temp_file("save");
    let settings = Settings {
        language: Language::En,
        ..Settings::default()
    };
    settings.save(&path).unwrap();
    assert_eq!(Settings::load(&path), settings);
}

#[test]
fn system_locale_maps_to_a_shipped_language() {
    assert_eq!(Language::from("zh-Hans-CN"), Language::ZhCn);
    assert_eq!(Language::from("zh_TW"), Language::ZhCn);
    assert_eq!(Language::from("en-US"), Language::En);
    assert_eq!(Language::from("fr-FR"), Language::En);
    assert_eq!(Language::ZhCn.resolve(), "zh-CN");
    assert_eq!(Language::En.resolve(), "en");
    assert!(["zh-CN", "en"].contains(&Language::System.resolve()));
}
