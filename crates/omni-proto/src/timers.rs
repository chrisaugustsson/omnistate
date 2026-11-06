//! Waits that have not run out: an `afterDelay` trigger counting from the
//! moment its board or overlay was shown, and the `delayMs` of a mouse
//! trigger counting from its gesture. Ported from the timer rules of
//! FigCraft's `proto_run.rs` (MIT OR Apache-2.0; see `NOTICE`).

use crate::player::Rect;
use omni_core::design::NodeId;
use omni_core::design::proto::Interaction;

/// The box of the layer a wait acts for, which opening an overlay is placed
/// from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Target {
    Rect(Rect),
    /// A layer of `board`, looked up when the wait runs out; `origin` is
    /// where the board sits on the screen.
    Layer {
        board: NodeId,
        id: NodeId,
        origin: [f64; 2],
    },
}

/// A wait. It is dropped when the board or overlay it was started on goes
/// away.
#[derive(Clone, Debug, PartialEq)]
pub struct Timer {
    /// The visit of the board or overlay it was started on.
    pub(crate) visit: u64,
    pub(crate) interaction: Interaction,
    pub(crate) holder: Option<NodeId>,
    pub(crate) target: Target,
    since: f64,
    delay_ms: u32,
    /// Whether it began at an event time that may be older than the page's
    /// clock; `Player::restart_timers` counts it again from a fresh time.
    pub(crate) fresh: bool,
}

impl Timer {
    pub(crate) fn new(
        visit: u64,
        interaction: Interaction,
        holder: Option<NodeId>,
        target: Target,
        since: f64,
        delay_ms: u32,
    ) -> Timer {
        Timer {
            visit,
            interaction,
            holder,
            target,
            since,
            delay_ms,
            fresh: true,
        }
    }

    /// When the wait runs out, in milliseconds.
    pub fn due(&self) -> f64 {
        self.since + f64::from(self.delay_ms)
    }

    pub(crate) fn restart(&mut self, at: f64) {
        self.since = at;
        self.fresh = false;
    }
}
