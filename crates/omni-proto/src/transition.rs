//! How far a running transition has come.

use crate::player::Running;
use omni_core::design::proto::{Direction, Easing, Transition};

/// Where a transition is at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// The eased progress: 0 at the start and exactly 1 at the end; a back
    /// easing or a spring leaves 0..=1 in between.
    pub t: f64,
    pub done: bool,
}

/// How long `running` takes, in milliseconds: a spring's own length replaces
/// the transition's duration.
pub fn duration_ms(running: &Running) -> f64 {
    f64::from(
        running
            .easing
            .spring_duration_ms()
            .unwrap_or(running.duration_ms),
    )
}

/// The progress of `running` at `now`, in milliseconds. A spring's own length
/// replaces the transition's duration.
pub fn progress(running: &Running, now: f64) -> Progress {
    progress_of(running.easing, duration_ms(running), running.started, now)
}

/// The progress of something that began at `started` and takes `duration`
/// milliseconds under `easing`, at `now`.
pub fn progress_of(easing: Easing, duration: f64, started: f64, now: f64) -> Progress {
    let linear = if duration == 0.0 {
        1.0
    } else {
        ((now - started) / duration).clamp(0.0, 1.0)
    };
    Progress {
        t: easing.eval(linear),
        done: linear >= 1.0,
    }
}

/// The transition that undoes `transition`: in becomes out and the other way
/// round, and a direction turns to the opposite one.
pub fn reversed(transition: Transition) -> Transition {
    match transition {
        Transition::MoveIn { direction } => Transition::MoveOut {
            direction: opposite(direction),
        },
        Transition::MoveOut { direction } => Transition::MoveIn {
            direction: opposite(direction),
        },
        Transition::SlideIn { direction } => Transition::SlideOut {
            direction: opposite(direction),
        },
        Transition::SlideOut { direction } => Transition::SlideIn {
            direction: opposite(direction),
        },
        Transition::Push { direction } => Transition::Push {
            direction: opposite(direction),
        },
        other => other,
    }
}

fn opposite(direction: Direction) -> Direction {
    match direction {
        Direction::Left => Direction::Right,
        Direction::Right => Direction::Left,
        Direction::Top => Direction::Bottom,
        Direction::Bottom => Direction::Top,
    }
}
