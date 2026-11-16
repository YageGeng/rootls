//! App-wide shell state: the plugin registry, the launcher entity and its popup window.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use rust_i18n::t;

use crate::cli::ShellCommand;
use crate::launcher::Launcher;
use crate::plugin::{PluginRegistry, PluginView};
use crate::settings::SettingsStore;
use crate::settings::window::SettingsWindow;

/// Wayland application id of every rtools window; matches
/// `packaging/linux/dev.rtools.rtools.desktop` so the desktop can name and group them.
pub(crate) const APP_ID: &str = "dev.rtools.rtools";

/// A toggle this soon after the popup closed is the same gesture that closed it:
/// clicking the tray icon first blurs (and so closes) the popup, then toggles.
const TOGGLE_AFTER_CLOSE: Duration = Duration::from_millis(300);

/// Default size of the settings window (a platform window boundary).
const SETTINGS_SIZE: Size<Pixels> = Size {
    width: px(960.),
    height: px(620.),
};

/// Default size of a detached plugin window (a platform window boundary).
const DETACHED_SIZE: Size<Pixels> = Size {
    width: px(960.),
    height: px(680.),
};

/// Global owner of the launcher popup.
pub struct Shell {
    registry: Rc<PluginRegistry>,
    data_root: PathBuf,
    /// Survives popup close/reopen so query and plugin state are kept (spec §4.2).
    launcher: Option<Entity<Launcher>>,
    popup: Option<AnyWindowHandle>,
    /// When the popup last closed, see [`TOGGLE_AFTER_CLOSE`].
    closed_at: Option<Instant>,
    /// The settings window, while it is open.
    settings: Option<AnyWindowHandle>,
}

impl Global for Shell {}

impl Shell {
    /// Installs the shell; `data_root` holds one directory per plugin.
    pub fn init(registry: PluginRegistry, data_root: PathBuf, cx: &mut App) {
        cx.set_global(Shell {
            registry: Rc::new(registry),
            data_root,
            launcher: None,
            popup: None,
            closed_at: None,
            settings: None,
        });
    }

    /// Carries out a command from the command line, the tray or the hotkey.
    pub fn run(command: ShellCommand, cx: &mut App) {
        match command {
            ShellCommand::Toggle => Self::toggle(cx),
            ShellCommand::Settings => Self::open_settings(cx),
            ShellCommand::Quit => {
                tracing::info!("rtools quitting");
                cx.quit();
            }
        }
    }

    /// Hides the popup when it is open, otherwise shows it.
    pub fn toggle(cx: &mut App) {
        let shell = cx.global::<Shell>();
        match shell.popup {
            Some(popup) => {
                let _ = popup.update(cx, |_, window, _| window.remove_window());
                Self::forget_popup(cx);
            }
            None if shell
                .closed_at
                .is_some_and(|closed| closed.elapsed() < TOGGLE_AFTER_CLOSE) => {}
            None => Self::open_popup(cx),
        }
    }

    /// Records that the popup window is gone (closed by the launcher itself).
    pub(crate) fn forget_popup(cx: &mut App) {
        let shell = cx.global_mut::<Shell>();
        shell.popup = None;
        shell.closed_at = Some(Instant::now());
    }

    /// Opens the settings window, or brings it forward when it is already open.
    pub fn open_settings(cx: &mut App) {
        // The macOS launcher is a non-activating panel; normal windows need app activation.
        cx.activate(true);
        if let Some(handle) = cx.global::<Shell>().settings
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            tracing::info!("activated the existing settings window");
            return;
        }
        let options = Self::framed_window(SETTINGS_SIZE, cx);
        let opened = open_window(options, cx, |window, cx| {
            SettingsStore::observe_appearance(window);
            window.set_window_title(&t!("settings.window_title"));
            let view = cx.new(|cx| SettingsWindow::new(window, cx));
            window.focus(&view.focus_handle(cx), cx);
            view
        });
        match opened {
            Ok((handle, _)) => {
                tracing::info!("opened the settings window");
                cx.global_mut::<Shell>().settings = Some(handle);
            }
            Err(err) => tracing::error!("opening the settings window failed: {}", err),
        }
    }

    /// Opens a plugin view in a normal window with the client-side title bar.
    pub(crate) fn detach(view: PluginView, title: SharedString, cx: &mut App) {
        // The launcher does not activate the app, so bring detached windows forward explicitly.
        cx.activate(true);
        let options = Self::framed_window(DETACHED_SIZE, cx);
        let opened = open_window(options, cx, |window, cx| {
            SettingsStore::observe_appearance(window);
            window.set_window_title(&title);
            window.focus(&view.focus, cx);
            cx.new(|_| Detached { view })
        });
        match opened {
            Ok(_) => tracing::info!("detached plugin {}", title),
            Err(err) => tracing::error!("opening the detached window failed: {}", err),
        }
    }

    /// A normal centered window of `size` that draws the client-side title bar.
    fn framed_window(size: Size<Pixels>, cx: &App) -> WindowOptions {
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
            window_decorations: Some(WindowDecorations::Client),
            app_id: Some(APP_ID.into()),
            ..TitleBar::window_options()
        }
    }

    /// Opens the launcher, reusing its entity from earlier openings: on Wayland a
    /// full-screen transparent overlay the launcher draws a movable panel into,
    /// elsewhere a borderless popup around it.
    fn open_popup(cx: &mut App) {
        // The settings file and the system appearance may have changed while hidden.
        SettingsStore::reload(cx);
        let shell = cx.global::<Shell>();
        let registry = Rc::clone(&shell.registry);
        let data_root = shell.data_root.clone();
        let existing = shell.launcher.clone();
        let overlay = Self::overlay_options(cx);
        let in_overlay = overlay.is_some();
        let build = move |window: &mut Window, cx: &mut App| {
            SettingsStore::observe_appearance(window);
            let launcher = existing
                .unwrap_or_else(|| cx.new(|cx| Launcher::new(registry, data_root, window, cx)));
            launcher.update(cx, |launcher, cx| launcher.attach(in_overlay, window, cx));
            launcher
        };
        let opened = match overlay {
            // Without a GPUI Kit root, which would paint an opaque surface and a
            // window frame across the whole screen; the launcher draws its own panel.
            Some(options) => {
                let mut built = None;
                cx.open_window(options, |window, cx| {
                    let launcher = build(window, cx);
                    built = Some(launcher.clone());
                    launcher
                })
                .map(|handle| {
                    let launcher = built.expect("open_window ran its build closure");
                    (handle.into(), launcher)
                })
            }
            None => open_window(Self::popup_options(cx), cx, build),
        };
        match opened {
            Ok((popup, launcher)) => {
                let shell = cx.global_mut::<Shell>();
                shell.popup = Some(popup);
                shell.launcher = Some(launcher);
            }
            Err(err) => tracing::error!("opening the launcher failed: {}", err),
        }
    }

    /// A borderless, transparent, client-decorated popup, sized to the launcher
    /// once it lays out.
    fn popup_options(cx: &App) -> WindowOptions {
        WindowOptions {
            kind: WindowKind::PopUp,
            titlebar: None,
            window_background: WindowBackgroundAppearance::Transparent,
            window_decorations: Some(WindowDecorations::Client),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                Launcher::INITIAL_SIZE,
                cx,
            ))),
            app_id: Some(APP_ID.into()),
            ..Default::default()
        }
    }

    /// Wayland: a transparent layer-shell surface over the whole primary screen,
    /// above the panels. `None` elsewhere, where a popup can be placed and moved.
    #[cfg(target_os = "linux")]
    fn overlay_options(cx: &App) -> Option<WindowOptions> {
        use gpui_kit::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
        if cx.compositor_name() != "Wayland" {
            return None;
        }
        // GPUI on Wayland reports no primary display, so the first output is used.
        // ponytail: follow the focused output if multi-monitor setups need it.
        let Some(display) = cx.displays().into_iter().next() else {
            tracing::warn!("no display reported; the launcher falls back to a fixed popup");
            return None;
        };
        Some(WindowOptions {
            kind: WindowKind::LayerShell(LayerShellOptions {
                namespace: "rtools".into(),
                layer: Layer::Overlay,
                anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                // Ignores the panels' reserved space, so the launcher centers on the screen.
                exclusive_zone: Some(px(-1.)),
                keyboard_interactivity: KeyboardInteractivity::OnDemand,
                ..Default::default()
            }),
            titlebar: None,
            window_background: WindowBackgroundAppearance::Transparent,
            window_bounds: Some(WindowBounds::Windowed(display.bounds())),
            display_id: Some(display.id()),
            app_id: Some(APP_ID.into()),
            ..Default::default()
        })
    }

    /// Other platforms place and move a popup window themselves.
    #[cfg(not(target_os = "linux"))]
    fn overlay_options(_: &App) -> Option<WindowOptions> {
        None
    }
}

/// Root of a detached plugin window: title bar above the plugin view.
struct Detached {
    view: PluginView,
}

impl Render for Detached {
    /// Fills the window below the title bar with the plugin view.
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(TitleBar::new())
            .child(div().flex_1().min_h_0().child(self.view.view.clone()))
    }
}
