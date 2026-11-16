//! Passive macOS keyboard events without input-source or character translation.

use std::cell::RefCell;
use std::ffi::c_void;
use std::io;
use std::ptr::NonNull;
use std::thread;
use std::time::Instant;

use futures::channel::mpsc::UnboundedSender;
use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventTapProxy, CGEventType, CGPreflightListenEventAccess,
};

use super::double_tap::{DoubleTap, KeyAction, KeyEvent, KeyKind};
use crate::cli::ShellCommand;

/// Owns the event tap and shortcut state on its dedicated run-loop thread.
struct Listener {
    commands: UnboundedSender<ShellCommand>,
    detector: DoubleTap,
    ctrl_down: bool,
    tap: Option<CFRetained<CFMachPort>>,
}

impl From<UnboundedSender<ShellCommand>> for Listener {
    /// Starts with no held keys and no native resources.
    fn from(commands: UnboundedSender<ShellCommand>) -> Self {
        Self {
            commands,
            detector: DoubleTap::default(),
            ctrl_down: false,
            tap: None,
        }
    }
}

impl Listener {
    /// Installs a listen-only tap; missing permission leaves the rest of the app usable.
    fn run(self) {
        tracing::info!("starting macOS keyboard event listener");
        // An ad-hoc rebuild can invalidate TCC access even while Settings shows it enabled.
        if !CGPreflightListenEventAccess() {
            tracing::warn!(
                "double-tap Ctrl unavailable: Input Monitoring is not granted to this build; quit rtools, remove its old entry in System Settings > Privacy & Security > Input Monitoring, add the installed rtools.app again and enable it, then restart; the menu bar remains available"
            );
            return;
        }
        let listener = RefCell::new(self);
        // The context stays in place on this thread; RefCell prevents aliased mutable access.
        let tap = unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::SessionEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                (1 << CGEventType::KeyDown.0) | (1 << CGEventType::FlagsChanged.0),
                Some(Self::callback),
                (&listener as *const RefCell<Self>).cast_mut().cast(),
            )
        };
        let Some(tap) = tap else {
            tracing::warn!(
                "macOS keyboard event tap creation failed despite Input Monitoring access; restart rtools and check its permission in System Settings > Privacy & Security > Input Monitoring; the menu bar remains available"
            );
            return;
        };
        let Some(source) = CFMachPort::new_run_loop_source(None, Some(&tap), 0) else {
            tracing::warn!("creating the macOS keyboard run-loop source failed");
            return;
        };
        let Some(run_loop) = CFRunLoop::current() else {
            tracing::warn!("getting the macOS keyboard run loop failed");
            return;
        };
        // Core Foundation owns this immutable global mode string.
        let mode = unsafe { kCFRunLoopCommonModes };
        run_loop.add_source(Some(&source), mode);
        CGEvent::tap_enable(&tap, true);
        if !CGEvent::tap_is_enabled(&tap) {
            tracing::warn!(
                "macOS keyboard event tap remains disabled; check Input Monitoring access and restart rtools"
            );
            run_loop.remove_source(Some(&source), mode);
            tap.invalidate();
            return;
        }
        listener.borrow_mut().tap = Some(tap);
        tracing::info!("macOS keyboard event listener enabled");
        CFRunLoop::run();
        run_loop.remove_source(Some(&source), mode);
        if let Some(tap) = listener.borrow_mut().tap.take() {
            tap.invalidate();
        }
        tracing::warn!("macOS keyboard event listener stopped");
    }

    /// Forwards raw events without altering them or consulting the keyboard layout.
    unsafe extern "C-unwind" fn callback(
        _proxy: CGEventTapProxy,
        event_type: CGEventType,
        event: NonNull<CGEvent>,
        context: *mut c_void,
    ) -> *mut CGEvent {
        // run() retains this context for the lifetime of the tap on the same thread.
        let listener = unsafe { &*context.cast::<RefCell<Self>>() };
        let mut listener = listener.borrow_mut();
        if matches!(
            event_type,
            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
        ) {
            // Events may have been lost, so an unfinished tap must not survive a restart.
            listener.detector = DoubleTap::default();
            listener.ctrl_down = false;
            if let Some(tap) = &listener.tap {
                tracing::warn!(
                    "macOS disabled the keyboard event tap ({:?}); re-enabling it",
                    event_type
                );
                CGEvent::tap_enable(tap, true);
            }
        } else {
            // Quartz owns the event and keeps it alive until this callback returns.
            listener.on_event(event_type, unsafe { event.as_ref() }, Instant::now());
        }
        event.as_ptr()
    }

    /// Reduces native keys to Ctrl/other transitions, discarding any typed content.
    fn on_event(&mut self, event_type: CGEventType, event: &CGEvent, at: Instant) {
        let flags = CGEvent::flags(Some(event));
        let (kind, action) = match event_type {
            CGEventType::FlagsChanged
                if matches!(
                    CGEvent::integer_value_field(Some(event), CGEventField::KeyboardEventKeycode),
                    0x3b | 0x3e
                ) =>
            {
                // The aggregate flag remains set until both physical Ctrl keys are released.
                let down = flags.contains(CGEventFlags::MaskControl);
                if down == self.ctrl_down {
                    return;
                }
                self.ctrl_down = down;
                (
                    KeyKind::Ctrl,
                    if down {
                        KeyAction::Press
                    } else {
                        KeyAction::Release
                    },
                )
            }
            CGEventType::KeyDown | CGEventType::FlagsChanged => (KeyKind::Other, KeyAction::Press),
            _ => return,
        };
        // A modifier already held before Ctrl is still a chord, not a lone Ctrl tap.
        if kind == KeyKind::Ctrl
            && flags.intersects(
                CGEventFlags::MaskShift
                    | CGEventFlags::MaskAlternate
                    | CGEventFlags::MaskCommand
                    | CGEventFlags::MaskSecondaryFn,
            )
        {
            self.detector.feed(KeyEvent {
                kind: KeyKind::Other,
                action: KeyAction::Press,
                at,
            });
            return;
        }
        if self.detector.feed(KeyEvent { kind, action, at }) {
            tracing::debug!("double-tap Ctrl detected; toggling launcher");
            if let Err(err) = self.commands.unbounded_send(ShellCommand::Toggle) {
                tracing::warn!("sending the macOS launcher toggle failed: {}", err);
                if let Some(run_loop) = CFRunLoop::current() {
                    run_loop.stop();
                }
            }
        }
    }
}

/// Starts raw keyboard monitoring without blocking the application's main run loop.
pub fn start(commands: UnboundedSender<ShellCommand>) -> io::Result<()> {
    thread::Builder::new()
        .name("rtools-double-ctrl".into())
        .spawn(move || Listener::from(commands).run())
        .inspect_err(|err| {
            tracing::warn!(
                "starting the macOS keyboard listener thread failed: {}",
                err
            );
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::channel::mpsc;

    use super::*;

    /// Replays native Quartz events on a worker and counts actual launcher commands.
    fn triggers(events: &[(CGEventType, u16, CGEventFlags, u64)]) -> usize {
        thread::scope(|scope| {
            scope
                .spawn(|| {
                    let (commands, mut received) = mpsc::unbounded();
                    let listener = RefCell::new(Listener::from(commands));
                    let start = Instant::now();
                    for &(event_type, code, flags, ms) in events {
                        let event = CGEvent::new_keyboard_event(None, code, true).unwrap();
                        CGEvent::set_flags(Some(&event), flags);
                        if matches!(
                            event_type,
                            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
                        ) {
                            // Exercise the native callback's recovery branch with a live context and event.
                            unsafe {
                                Listener::callback(
                                    std::ptr::null_mut(),
                                    event_type,
                                    NonNull::from(&*event),
                                    (&listener as *const RefCell<Listener>).cast_mut().cast(),
                                );
                            }
                        } else {
                            listener.borrow_mut().on_event(
                                event_type,
                                &event,
                                start + Duration::from_millis(ms),
                            );
                        }
                    }
                    let mut count = 0;
                    while let Ok(command) = received.try_recv() {
                        assert_eq!(command, ShellCommand::Toggle);
                        count += 1;
                    }
                    count
                })
                .join()
                .unwrap()
        })
    }

    /// Both physical Ctrl keys generate one toggle after two complete taps.
    #[test]
    fn left_and_right_ctrl_double_taps_toggle() {
        for code in [0x3b, 0x3e] {
            assert_eq!(
                triggers(&[
                    (
                        CGEventType::FlagsChanged,
                        code,
                        CGEventFlags::MaskControl,
                        0
                    ),
                    (CGEventType::FlagsChanged, code, CGEventFlags::empty(), 60),
                    (
                        CGEventType::FlagsChanged,
                        code,
                        CGEventFlags::MaskControl,
                        180
                    ),
                    (CGEventType::FlagsChanged, code, CGEventFlags::empty(), 240),
                ]),
                1
            );
        }
    }

    /// Ordinary keys must remain safe on the listener thread and never toggle the launcher.
    #[test]
    fn raw_key_down_events_need_no_input_source_translation() {
        let events: Vec<_> = (0..128)
            .map(|code| {
                (
                    CGEventType::KeyDown,
                    code,
                    CGEventFlags::empty(),
                    u64::from(code),
                )
            })
            .collect();
        assert_eq!(triggers(&events), 0);
    }

    /// A letter pressed with Ctrl invalidates that tap, including repeated key-down events.
    #[test]
    fn ctrl_shortcuts_do_not_toggle() {
        assert_eq!(
            triggers(&[
                (
                    CGEventType::FlagsChanged,
                    0x3b,
                    CGEventFlags::MaskControl,
                    0
                ),
                (CGEventType::KeyDown, 0x08, CGEventFlags::MaskControl, 30),
                (CGEventType::KeyDown, 0x08, CGEventFlags::MaskControl, 40),
                (CGEventType::FlagsChanged, 0x3b, CGEventFlags::empty(), 60),
                (
                    CGEventType::FlagsChanged,
                    0x3b,
                    CGEventFlags::MaskControl,
                    180
                ),
                (CGEventType::FlagsChanged, 0x3b, CGEventFlags::empty(), 240),
            ]),
            0
        );
    }

    /// Modifiers held before monitoring begins must not turn chords into lone Ctrl taps.
    #[test]
    fn already_held_modifiers_interrupt_ctrl_taps() {
        for modifier in [
            CGEventFlags::MaskShift,
            CGEventFlags::MaskAlternate,
            CGEventFlags::MaskCommand,
            CGEventFlags::MaskSecondaryFn,
        ] {
            assert_eq!(
                triggers(&[
                    (
                        CGEventType::FlagsChanged,
                        0x3b,
                        CGEventFlags::MaskControl | modifier,
                        0
                    ),
                    (CGEventType::FlagsChanged, 0x3b, modifier, 60),
                    (
                        CGEventType::FlagsChanged,
                        0x3b,
                        CGEventFlags::MaskControl | modifier,
                        180
                    ),
                    (CGEventType::FlagsChanged, 0x3b, modifier, 240),
                ]),
                0
            );
        }
    }

    /// Overlapping left/right Ctrl presses form one aggregate tap, not a double tap.
    #[test]
    fn overlapping_ctrl_keys_do_not_toggle() {
        assert_eq!(
            triggers(&[
                (
                    CGEventType::FlagsChanged,
                    0x3b,
                    CGEventFlags::MaskControl,
                    0
                ),
                (
                    CGEventType::FlagsChanged,
                    0x3e,
                    CGEventFlags::MaskControl,
                    20
                ),
                (
                    CGEventType::FlagsChanged,
                    0x3b,
                    CGEventFlags::MaskControl,
                    40
                ),
                (CGEventType::FlagsChanged, 0x3e, CGEventFlags::empty(), 60),
            ]),
            0
        );
    }

    /// Losing events must discard a pending tap before the native listener is re-enabled.
    #[test]
    fn disabled_taps_reset_pending_shortcuts() {
        for event_type in [
            CGEventType::TapDisabledByTimeout,
            CGEventType::TapDisabledByUserInput,
        ] {
            assert_eq!(
                triggers(&[
                    (
                        CGEventType::FlagsChanged,
                        0x3b,
                        CGEventFlags::MaskControl,
                        0
                    ),
                    (CGEventType::FlagsChanged, 0x3b, CGEventFlags::empty(), 60),
                    (event_type, 0, CGEventFlags::empty(), 100),
                    (
                        CGEventType::FlagsChanged,
                        0x3b,
                        CGEventFlags::MaskControl,
                        180
                    ),
                    (CGEventType::FlagsChanged, 0x3b, CGEventFlags::empty(), 240),
                ]),
                0
            );
        }
    }
}
