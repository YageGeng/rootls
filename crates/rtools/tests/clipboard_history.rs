use gpui_kit::{AssetSource as _, assets::IconName};
use rtools::assets::AppAssets;
use rtools::plugin::Plugin as _;
use rtools::plugins::clipboard::ClipboardPlugin;
use rtools::settings::{Settings, SettingsFile, SettingsStore};

/// Existing settings opt out of capture and the launcher can resolve the plugin icon.
#[test]
fn clipboard_is_opt_in_with_a_loadable_launcher_icon() {
    assert!(!"".parse::<Settings>().unwrap().clipboard.enabled);
    let settings = "[clipboard]\nenabled = true".parse::<Settings>().unwrap();
    assert!(
        settings
            .to_string()
            .parse::<Settings>()
            .unwrap()
            .clipboard
            .enabled
    );
    let manifest = ClipboardPlugin.manifest();
    assert_eq!(manifest.id, "clipboard");
    for icon in [
        manifest.icon,
        IconName::Copy,
        IconName::Trash,
        IconName::Eye,
        IconName::ArrowLeft,
        IconName::ChevronLeft,
        IconName::ChevronRight,
    ] {
        assert!(
            AppAssets.load(&icon.path()).unwrap().is_some(),
            "missing {}",
            icon.path()
        );
    }
}

/// A failed settings write must never re-enable capture when the launcher reloads the file.
#[test]
fn failed_opt_out_keeps_this_session_paused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut store = SettingsStore {
        file: SettingsFile::new(tmp.path().join("settings.toml")),
        settings: Settings::default(),
        credential_revision: 0,
    };
    store.set_clipboard_enabled(true);
    let obstacle = tmp.path().join("settings.toml.tmp");
    std::fs::create_dir(&obstacle).unwrap();
    store.set_clipboard_enabled(false);
    assert!(!store.settings.clipboard.enabled);
    assert!(!store.file.load().clipboard.enabled);
    std::fs::remove_dir(obstacle).unwrap();
    store.set_clipboard_enabled(false);
    assert!(
        !SettingsFile::new(tmp.path().join("settings.toml"))
            .load()
            .clipboard
            .enabled
    );
}
