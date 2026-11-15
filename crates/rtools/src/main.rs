//! rtools entry point: forwards commands to a running instance or becomes it.

use std::thread;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::*;
use rtools::assets::AppAssets;
use rtools::cli::ShellCommand;
use rtools::instance::InstanceName;
use rtools::plugin::{Plugin, PluginRegistry};
use rtools::plugins::diff::ui::DiffPlugin;
use rtools::plugins::json::ui::JsonPlugin;
use rtools::settings::SettingsStore;
use rtools::shell::Shell;
use rtools::{hotkey, launcher, settings, tray, ui};
use tracing_subscriber::EnvFilter;

/// Parses the command, hands it to a running instance, or starts the app.
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let command = match std::env::args().nth(1) {
        None => ShellCommand::Toggle,
        Some(arg) => match arg.parse::<ShellCommand>() {
            Ok(command) => command,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(2);
            }
        },
    };

    let name = InstanceName::for_current_user();
    if name.send(command).is_ok() {
        return;
    }
    if command == ShellCommand::Quit {
        eprintln!("rtools is not running");
        return;
    }
    let server = match name.bind() {
        Ok(server) => server,
        Err(err) => {
            tracing::error!("listening on the instance socket failed: {}", err);
            std::process::exit(1);
        }
    };
    tracing::info!("rtools started");

    // Commands from later `rtools` invocations (blocking thread), the tray icon
    // and the double-tap detector share one channel.
    let (commands, mut received) = mpsc::unbounded();
    let instance_commands = commands.clone();
    thread::Builder::new()
        .name("rtools-instance".into())
        .spawn(move || {
            for command in server.incoming() {
                match command {
                    Ok(command) => {
                        if instance_commands.unbounded_send(command).is_err() {
                            break;
                        }
                    }
                    Err(err) => tracing::warn!("ignoring an invalid instance command: {}", err),
                }
            }
        })
        .expect("spawning the instance listener thread");

    application()
        .with_assets(AppAssets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            ui::register_languages();
            SettingsStore::init(cx);
            launcher::bind_keys(cx);
            settings::window::bind_keys(cx);

            let registry: PluginRegistry = [
                Box::new(JsonPlugin) as Box<dyn Plugin>,
                Box::new(DiffPlugin),
            ]
            .into_iter()
            .collect();
            registry.bind_keys(cx);
            let data_root = dirs::data_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("rtools");
            let tray = tray::start(commands.clone(), &data_root, cx)
                .map_err(|err| tracing::warn!("system tray unavailable: {}", err))
                .ok();
            Shell::init(registry, data_root, cx);

            cx.spawn(async move |cx| {
                // Holding the tray here keeps it alive for as long as commands are handled.
                let _tray = tray;
                while let Some(command) = received.next().await {
                    cx.update(|cx| Shell::run(command, cx));
                }
            })
            .detach();

            // Double-tap Ctrl; without it the launcher is reachable through `rtools toggle`.
            match hotkey::start(commands) {
                Ok(()) => tracing::info!("double-tap Ctrl enabled"),
                Err(err) => tracing::warn!(
                    "double-tap Ctrl unavailable ({}); on Linux add yourself to the input group (`sudo usermod -aG input $USER`), on macOS grant Accessibility access, or bind a system shortcut to `rtools toggle`",
                    err
                ),
            }
            // A fresh start carries out the command it was started with (never
            // `Quit`, which returned above).
            Shell::run(command, cx);
        });
}
