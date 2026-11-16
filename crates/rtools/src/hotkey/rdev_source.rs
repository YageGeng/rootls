//! Windows key source: `rdev`'s global listener.

use std::io;
use std::thread;
use std::time::Instant;

use futures::channel::mpsc::UnboundedSender;
use rdev::{EventType, Key};

use super::double_tap::{DoubleTap, KeyAction, KeyEvent, KeyKind};
use crate::cli::ShellCommand;

/// Starts double-tap Ctrl detection on its own thread; each activation sends
/// [`ShellCommand::Toggle`]. Listener failures (no permission) are only logged.
pub fn start(commands: UnboundedSender<ShellCommand>) -> io::Result<()> {
    thread::Builder::new()
        .name("rtools-double-ctrl".into())
        .spawn(move || {
            let mut detector = DoubleTap::default();
            // rdev reports auto-repeat as more presses; remember the held state.
            let mut ctrl_down = false;
            let result = rdev::listen(move |event| {
                let (kind, action) = match event.event_type {
                    EventType::KeyPress(Key::ControlLeft | Key::ControlRight) => {
                        let action = if ctrl_down {
                            KeyAction::Repeat
                        } else {
                            KeyAction::Press
                        };
                        ctrl_down = true;
                        (KeyKind::Ctrl, action)
                    }
                    EventType::KeyRelease(Key::ControlLeft | Key::ControlRight) => {
                        ctrl_down = false;
                        (KeyKind::Ctrl, KeyAction::Release)
                    }
                    EventType::KeyPress(_) => (KeyKind::Other, KeyAction::Press),
                    EventType::KeyRelease(_) => (KeyKind::Other, KeyAction::Release),
                    _ => return,
                };
                let at = Instant::now();
                if detector.feed(KeyEvent { kind, action, at }) {
                    let _ = commands.unbounded_send(ShellCommand::Toggle);
                }
            });
            if let Err(err) = result {
                tracing::warn!(
                    "double-tap Ctrl unavailable ({:?}); bind a shortcut to `rtools toggle`",
                    err
                );
            }
        })?;
    Ok(())
}
