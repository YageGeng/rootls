//! Linux tray through StatusNotifierItem (KDE and other SNI hosts), via `ksni`.

use std::fs;
use std::path::Path;

use futures::channel::mpsc::UnboundedSender;
use gpui_kit::{App, AppContext as _, Task};
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip, TrayMethods as _};
use rust_i18n::t;

use super::{TITLE, TrayError};
use crate::cli::ShellCommand;

/// Monochrome panel glyph; KDE recolors its `ColorScheme-Text` class.
const TRAY_SVG: &str = include_str!("../../../../assets/icons/rtools-tray.svg");
/// Colour fallbacks for hosts that cannot resolve the themed icon.
const PIXMAPS: [&[u8]; 2] = [
    include_bytes!("../../../../assets/icons/rtools-22.png"),
    include_bytes!("../../../../assets/icons/rtools-44.png"),
];
/// Icon name inside the private icon theme directory.
const ICON_NAME: &str = "dev.rtools.rtools-tray";

/// The StatusNotifierItem exported on D-Bus.
struct StatusItem {
    commands: UnboundedSender<ShellCommand>,
    theme_path: String,
    pixmaps: Vec<Icon>,
}

impl StatusItem {
    /// Forwards a command to the app; a closed channel means the app is exiting.
    fn send(&self, command: ShellCommand) {
        let _ = self.commands.unbounded_send(command);
    }
}

impl ksni::Tray for StatusItem {
    /// Stable SNI id.
    fn id(&self) -> String {
        "rtools".into()
    }

    /// Accessible name of the item.
    fn title(&self) -> String {
        TITLE.into()
    }

    /// Directory searched for `icon_name`, so no system-wide install is needed.
    fn icon_theme_path(&self) -> String {
        self.theme_path.clone()
    }

    /// Themed (recolorable) panel glyph.
    fn icon_name(&self) -> String {
        ICON_NAME.into()
    }

    /// Pixmap fallback used when the themed icon cannot be found.
    fn icon_pixmap(&self) -> Vec<Icon> {
        self.pixmaps.clone()
    }

    /// Hover text.
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: TITLE.into(),
            description: t!("tray.hint").into_owned(),
            ..Default::default()
        }
    }

    /// Left click toggles the launcher.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(ShellCommand::Toggle);
    }

    /// Right-click menu, in the language in effect when the host asks for it.
    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: t!("tray.open").into_owned(),
                activate: Box::new(|item: &mut Self| item.send(ShellCommand::Toggle)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: t!("tray.settings").into_owned(),
                activate: Box::new(|item: &mut Self| item.send(ShellCommand::Settings)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: t!("tray.quit").into_owned(),
                activate: Box::new(|item: &mut Self| item.send(ShellCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Keeps the tray alive while held: owns the task holding the D-Bus service handle.
pub struct Tray {
    _task: Task<()>,
}

/// Shows the tray icon; clicks and menu items arrive as commands on `commands`.
/// Writes the themed icon under `data_root`, then registers the item on the
/// session bus in the background.
pub fn start(
    commands: UnboundedSender<ShellCommand>,
    data_root: &Path,
    cx: &mut App,
) -> Result<Tray, TrayError> {
    let theme_root = data_root.join("icons");
    let apps = theme_root.join("hicolor/scalable/apps");
    fs::create_dir_all(&apps)?;
    fs::write(apps.join(format!("{ICON_NAME}.svg")), TRAY_SVG)?;
    let pixmaps = PIXMAPS
        .iter()
        .map(|png| argb_icon(png))
        .collect::<Result<Vec<_>, _>>()?;
    let item = StatusItem {
        commands,
        theme_path: theme_root.to_string_lossy().into_owned(),
        pixmaps,
    };
    let task = cx.background_spawn(async move {
        match item.spawn().await {
            Ok(_handle) => {
                tracing::info!("system tray ready");
                // The service lives as long as its handle; hold it for the app's lifetime.
                std::future::pending::<()>().await;
            }
            Err(err) => tracing::warn!("system tray unavailable: {}", err),
        }
    });
    Ok(Tray { _task: task })
}

/// Decodes a PNG into the ARGB32 (network byte order) pixmap SNI expects.
fn argb_icon(png: &[u8]) -> Result<Icon, TrayError> {
    let rgba = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.to_rgba8();
    let (width, height) = rgba.dimensions();
    let data = rgba
        .pixels()
        .flat_map(|pixel| {
            let [r, g, b, a] = pixel.0;
            [a, r, g, b]
        })
        .collect();
    Ok(Icon {
        width: width as i32,
        height: height as i32,
        data,
    })
}
