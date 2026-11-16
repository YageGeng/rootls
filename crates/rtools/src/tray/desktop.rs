//! macOS / Windows tray via `tray-icon`; must be created on the main thread.
//! Not verified on this machine (Linux only).

use std::path::Path;

use futures::channel::mpsc::UnboundedSender;
use gpui_kit::App;
use rust_i18n::t;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::{TITLE, TrayError};
use crate::cli::ShellCommand;

/// macOS: black glyph used as a template image (the system recolors it).
#[cfg(target_os = "macos")]
const ICON_PNG: &[u8] = include_bytes!("../../../../assets/icons/rtools-tray-template-44.png");
/// Windows: the colour app icon at notification-area size.
#[cfg(target_os = "windows")]
const ICON_PNG: &[u8] = include_bytes!("../../../../assets/icons/rtools-32.png");

/// Keeps the tray icon alive while held.
pub struct Tray {
    _icon: TrayIcon,
}

/// Shows the tray icon; clicks and menu items arrive as commands on `commands`.
/// The icon is embedded, so `data_root` is unused here.
pub fn start(
    commands: UnboundedSender<ShellCommand>,
    _data_root: &Path,
    _cx: &mut App,
) -> Result<Tray, TrayError> {
    let rgba = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)?.to_rgba8();
    let (width, height) = rgba.dimensions();
    let icon = Icon::from_rgba(rgba.into_raw(), width, height)?;

    // ponytail: labels are fixed at startup; a language change shows after a restart.
    let open = MenuItem::new(t!("tray.open"), true, None);
    let settings = MenuItem::new(t!("tray.settings"), true, None);
    let quit = MenuItem::new(t!("tray.quit"), true, None);
    let menu = Menu::new();
    menu.append_items(&[&open, &settings, &PredefinedMenuItem::separator(), &quit])?;

    let (open_id, settings_id, quit_id) =
        (open.id().clone(), settings.id().clone(), quit.id().clone());
    let menu_commands = commands.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let command = if event.id == open_id {
            ShellCommand::Toggle
        } else if event.id == settings_id {
            ShellCommand::Settings
        } else if event.id == quit_id {
            ShellCommand::Quit
        } else {
            return;
        };
        let _ = menu_commands.unbounded_send(command);
    }));
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let _ = commands.unbounded_send(ShellCommand::Toggle);
        }
    }));

    // The template setter is macOS-only; Windows keeps the full-color icon.
    #[cfg(target_os = "macos")]
    let builder = TrayIconBuilder::new().with_icon_templated(icon);
    #[cfg(target_os = "windows")]
    let builder = TrayIconBuilder::new().with_icon(icon);

    let icon = builder
        .with_tooltip(format!("{TITLE} — {}", t!("tray.hint")))
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?;
    Ok(Tray { _icon: icon })
}
