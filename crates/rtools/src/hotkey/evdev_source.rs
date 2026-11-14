//! Linux key source: keyboards under `/dev/input` (works on Wayland and X11;
//! the user must be in the `input` group).

use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use evdev::{Device, EventSummary, KeyCode};
use futures::channel::mpsc::UnboundedSender;

use super::double_tap::{DoubleTap, KeyAction, KeyEvent, KeyKind};
use crate::cli::ShellCommand;

impl From<(KeyCode, i32)> for KeyEvent {
    /// Maps an evdev key code and value (0 release, 1 press, 2 repeat) to a
    /// Ctrl/other event; the concrete key is discarded here.
    fn from((code, value): (KeyCode, i32)) -> Self {
        let kind = if code == KeyCode::KEY_LEFTCTRL || code == KeyCode::KEY_RIGHTCTRL {
            KeyKind::Ctrl
        } else {
            KeyKind::Other
        };
        let action = match value {
            0 => KeyAction::Release,
            2 => KeyAction::Repeat,
            _ => KeyAction::Press,
        };
        KeyEvent {
            kind,
            action,
            at: Instant::now(),
        }
    }
}

/// Starts double-tap Ctrl detection on every readable keyboard; each activation
/// sends [`ShellCommand::Toggle`]. Fails when no keyboard can be opened.
pub fn start(commands: UnboundedSender<ShellCommand>) -> io::Result<()> {
    let keyboards: Vec<Device> = evdev::enumerate()
        .map(|(_, device)| device)
        .filter(is_keyboard)
        .collect();
    if keyboards.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "no readable keyboard device under /dev/input",
        ));
    }
    let (events, received) = mpsc::channel::<KeyEvent>();
    for mut keyboard in keyboards {
        let events = events.clone();
        thread::Builder::new()
            .name("rtools-keyboard".into())
            .spawn(move || {
                // ponytail: devices are enumerated once; hot-plugged keyboards need a restart.
                let name = keyboard.name().unwrap_or("unknown").to_string();
                loop {
                    let batch = match keyboard.fetch_events() {
                        Ok(batch) => batch,
                        Err(err) => {
                            // Typically the device was unplugged; other keyboards keep working.
                            tracing::warn!("stopped reading keyboard {}: {}", name, err);
                            return;
                        }
                    };
                    for event in batch {
                        if let EventSummary::Key(_, code, value) = event.destructure()
                            && events.send((code, value).into()).is_err()
                        {
                            return;
                        }
                    }
                }
            })?;
    }
    thread::Builder::new()
        .name("rtools-double-ctrl".into())
        .spawn(move || {
            let mut detector = DoubleTap::default();
            for event in received {
                if detector.feed(event) && commands.unbounded_send(ShellCommand::Toggle).is_err() {
                    break;
                }
            }
        })?;
    Ok(())
}

/// A keyboard is a device that has both Ctrl and letter keys (not a mouse or power button).
fn is_keyboard(device: &Device) -> bool {
    device
        .supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::KEY_LEFTCTRL) && keys.contains(KeyCode::KEY_A))
}
