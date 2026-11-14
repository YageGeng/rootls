//! Double-tap Ctrl detection (spec §4.3), independent of where key events come from.

use std::time::{Duration, Instant};

/// Which key an event is about; only Ctrl matters, everything else interrupts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Ctrl,
    Other,
}

/// Physical key transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Release,
    /// Auto-repeat while the key is held.
    Repeat,
}

/// One keyboard event fed to the detector. Carries no key identity beyond
/// Ctrl/other, so nothing typed can be reconstructed from it.
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub kind: KeyKind,
    pub action: KeyAction,
    pub at: Instant,
}

/// Recognizes two lone Ctrl taps in quick succession.
#[derive(Default)]
pub struct DoubleTap {
    /// When the current Ctrl press started.
    pressed_at: Option<Instant>,
    /// Another key went down while Ctrl was held (a combo such as Ctrl+C).
    interrupted: bool,
    /// Release time of the previous lone tap.
    last_tap: Option<Instant>,
    /// The current press started soon enough after `last_tap` to complete a double tap.
    armed: bool,
}

impl DoubleTap {
    /// Longest press that still counts as a tap.
    pub const TAP_MAX: Duration = Duration::from_millis(300);
    /// Longest gap between the first release and the second press.
    pub const GAP_MAX: Duration = Duration::from_millis(350);

    /// Feeds one key event; returns `true` when it completes a double tap of Ctrl.
    pub fn feed(&mut self, event: KeyEvent) -> bool {
        match (event.kind, event.action) {
            (KeyKind::Ctrl, KeyAction::Press) => {
                self.armed = self
                    .last_tap
                    .is_some_and(|tap| event.at.saturating_duration_since(tap) <= Self::GAP_MAX);
                self.pressed_at = Some(event.at);
                self.interrupted = false;
                false
            }
            (KeyKind::Ctrl, KeyAction::Repeat) => false,
            (KeyKind::Ctrl, KeyAction::Release) => {
                let lone_tap = !self.interrupted
                    && self.pressed_at.take().is_some_and(|press| {
                        event.at.saturating_duration_since(press) <= Self::TAP_MAX
                    });
                let completed = lone_tap && self.armed;
                self.armed = false;
                // A completed double tap starts over, so a third tap needs a fourth.
                self.last_tap = (lone_tap && !completed).then_some(event.at);
                completed
            }
            (KeyKind::Other, KeyAction::Press | KeyAction::Repeat) => {
                self.interrupted = true;
                self.armed = false;
                self.last_tap = None;
                false
            }
            (KeyKind::Other, KeyAction::Release) => false,
        }
    }
}
