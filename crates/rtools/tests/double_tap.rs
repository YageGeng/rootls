use rtools::hotkey::double_tap::{DoubleTap, KeyAction, KeyEvent, KeyKind};
use std::time::{Duration, Instant};

/// Replays `(kind, action, millis since start)` and counts triggers.
fn triggers(events: &[(KeyKind, KeyAction, u64)]) -> usize {
    let start = Instant::now();
    let mut detector = DoubleTap::default();
    events
        .iter()
        .filter(|(kind, action, ms)| {
            detector.feed(KeyEvent {
                kind: *kind,
                action: *action,
                at: start + Duration::from_millis(*ms),
            })
        })
        .count()
}

use KeyAction::{Press, Release, Repeat};
use KeyKind::{Ctrl, Other};

#[test]
fn two_quick_taps_trigger_once() {
    assert_eq!(
        triggers(&[
            (Ctrl, Press, 0),
            (Ctrl, Release, 80),
            (Ctrl, Press, 200),
            (Ctrl, Release, 280)
        ]),
        1
    );
}

#[test]
fn combos_do_not_count_as_taps() {
    // Ctrl+C, then a single tap.
    assert_eq!(
        triggers(&[
            (Ctrl, Press, 0),
            (Other, Press, 40),
            (Other, Release, 60),
            (Ctrl, Release, 90),
            (Ctrl, Press, 200),
            (Ctrl, Release, 260),
        ]),
        0
    );
}

#[test]
fn slow_taps_do_not_trigger() {
    assert_eq!(
        triggers(&[
            (Ctrl, Press, 0),
            (Ctrl, Release, 80),
            (Ctrl, Press, 600),
            (Ctrl, Release, 680)
        ]),
        0
    );
}

#[test]
fn a_long_hold_is_not_a_tap() {
    assert_eq!(
        triggers(&[
            (Ctrl, Press, 0),
            (Ctrl, Release, 600),
            (Ctrl, Press, 700),
            (Ctrl, Release, 760)
        ]),
        0
    );
}

#[test]
fn auto_repeat_while_held_is_ignored() {
    assert_eq!(
        triggers(&[
            (Ctrl, Press, 0),
            (Ctrl, Repeat, 30),
            (Ctrl, Repeat, 60),
            (Ctrl, Release, 90),
            (Ctrl, Press, 200),
            (Ctrl, Release, 260),
        ]),
        1
    );
}

#[test]
fn triple_tap_triggers_once() {
    assert_eq!(
        triggers(&[
            (Ctrl, Press, 0),
            (Ctrl, Release, 60),
            (Ctrl, Press, 150),
            (Ctrl, Release, 210),
            (Ctrl, Press, 300),
            (Ctrl, Release, 360),
        ]),
        1
    );
}
