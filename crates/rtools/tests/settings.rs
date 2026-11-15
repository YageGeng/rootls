use rtools::settings::{DiffLayout, DiffSettings, JsonSettings, Language, Settings, SettingsFile};
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
    assert_eq!(settings.window_radius, 4.);
    assert_eq!(settings.language, Language::System);
    assert!(!settings.json.soft_wrap);
    assert_eq!(settings.diff, DiffSettings::default());
    assert_eq!(settings.diff.layout, DiffLayout::Split);
    assert!(!settings.diff.char_level);
    // Every unchanged line shows until the user folds them.
    assert!(!settings.diff.fold_unchanged);
}

#[test]
fn toml_round_trips() {
    let settings = Settings {
        ui_font: Some("Inter".into()),
        mono_font: Some("JetBrains Mono".into()),
        font_size: 14.,
        window_radius: 6.,
        language: Language::ZhCn,
        json: JsonSettings { soft_wrap: true },
        diff: DiffSettings {
            layout: DiffLayout::Unified,
            ignore_whitespace: true,
            ignore_case: false,
            format_json: true,
            char_level: true,
            fold_unchanged: true,
        },
    };
    let text = settings.to_string();
    assert!(text.contains("language = \"zh-CN\""), "{text}");
    assert!(text.contains("[json]\nsoft_wrap = true"), "{text}");
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
fn window_radius_is_clamped() {
    let radius = |text: &str| text.parse::<Settings>().unwrap().window_radius;
    assert_eq!(radius("window_radius = 99"), 24.);
    assert_eq!(radius("window_radius = -3"), 0.);
}

#[test]
fn missing_or_corrupt_file_loads_defaults_and_is_kept() {
    let path = temp_file("corrupt");
    let mut file = SettingsFile::new(path.clone());
    assert_eq!(file.load(), Settings::default());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "font_size = [").unwrap();
    assert_eq!(file.load(), Settings::default());
    assert_eq!(fs::read_to_string(&path).unwrap(), "font_size = [");
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn save_then_load_round_trips() {
    let path = temp_file("save");
    let mut file = SettingsFile::new(path.clone());
    let settings = Settings {
        language: Language::En,
        ..Settings::default()
    };
    file.save(&settings).unwrap();
    assert_eq!(file.path(), path);
    assert_eq!(file.load(), settings);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn saving_over_a_broken_file_keeps_it_aside_once() {
    let path = temp_file("broken");
    let broken = "language = \"fr\"\nfont_size = 18\n";
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, broken).unwrap();
    let mut file = SettingsFile::new(path.clone());
    assert_eq!(file.load(), Settings::default());

    let changed = Settings {
        window_radius: 8.,
        ..Settings::default()
    };
    file.save(&changed).unwrap();
    let aside = path.with_extension("toml.broken");
    assert_eq!(fs::read_to_string(&aside).unwrap(), broken);
    assert_eq!(file.load(), changed);

    // Later saves write normally and leave the kept copy alone.
    file.save(&Settings::default()).unwrap();
    assert_eq!(fs::read_to_string(&aside).unwrap(), broken);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn saving_a_valid_file_keeps_nothing_aside() {
    let path = temp_file("valid");
    let mut file = SettingsFile::new(path.clone());
    file.save(&Settings::default()).unwrap();
    file.load();
    file.save(&Settings {
        font_size: 14.,
        ..Settings::default()
    })
    .unwrap();
    assert!(!path.with_extension("toml.broken").exists());
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
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

#[test]
fn diff_table_fills_missing_keys() {
    let settings = "[diff]\nlayout = \"unified\"\nignore_case = true\n"
        .parse::<Settings>()
        .unwrap();
    assert_eq!(
        settings.diff,
        DiffSettings {
            layout: DiffLayout::Unified,
            ignore_case: true,
            ..Default::default()
        }
    );
}

#[test]
fn diff_layout_is_written_lowercase() {
    let text = Settings::default().to_string();
    assert!(text.contains("[diff]"), "{text}");
    assert!(text.contains("layout = \"split\""), "{text}");
}
