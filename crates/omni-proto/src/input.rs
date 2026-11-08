//! What a pointer is doing: the layers it hovers, the button held down, a
//! drag under way, and what each undoes when it ends. The rules follow
//! FigCraft's `proto_run.rs` (MIT OR Apache-2.0; see `NOTICE`).

use crate::history::Play;
use crate::player::{OverlayState, Rect};
use omni_core::design::proto::{Direction, Interaction};
use omni_core::design::{NodeAddress, NodeId};

/// What a hover or press puts back when it ends.
#[derive(Clone, Debug)]
pub(crate) enum Revert {
    /// A navigate: back to `from`, as the navigate left it and without a
    /// history step; skipped when another board has been shown since.
    Board {
        to: NodeId,
        from: NodeId,
        visit: u64,
        history_len: usize,
        play: Play,
    },
    /// A change to: the variant the instance, by its address on `board`,
    /// showed before.
    Variant {
        board: NodeId,
        instance: NodeAddress,
        previous: Option<NodeId>,
    },
    /// An open overlay: closed again.
    Overlay { visit: u64 },
    /// A swap overlay: the overlay as it was.
    Swap { visit: u64, before: OverlayState },
}

/// An instance, by where it is, and the component it shows now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Holder {
    pub(crate) address: NodeAddress,
    pub(crate) shown: NodeId,
}

/// Where a layer is: its board and the instances that hold it, or that it is,
/// the nearest last.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Site {
    pub(crate) board: NodeId,
    pub(crate) holders: Vec<Holder>,
}

/// A layer the pointer is over.
#[derive(Clone, Debug)]
pub(crate) struct Hover {
    pub(crate) id: NodeId,
    pub(crate) depth: usize,
    /// Its box on the screen, as it was on the board it was entered on: the
    /// hotspot whatever the board shows there later.
    pub(crate) rect: Rect,
    pub(crate) site: Site,
    /// What leaving runs.
    pub(crate) leave: Vec<Interaction>,
    pub(crate) reverts: Vec<Revert>,
    /// The showing of the board it was entered on.
    pub(crate) board_visit: u64,
    /// The showing of the board or overlay it was hit on: what its delayed
    /// leave waits on.
    pub(crate) source_visit: u64,
}

/// A layer that reacts to a drag.
#[derive(Clone, Debug)]
pub(crate) struct DragTarget {
    pub(crate) interactions: Vec<Interaction>,
    pub(crate) site: Site,
    pub(crate) rect: Rect,
}

/// A transition that follows the pointer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Follow {
    /// The way the drag has to go for the transition to come on.
    pub(crate) direction: Direction,
    /// The linear progress the pointer has dragged it to, 0 to 1.
    pub(crate) fraction: f64,
}

/// The button being down.
pub(crate) struct Press {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) moved: bool,
    /// The layer a click on this press would land on.
    pub(crate) layer: Option<NodeId>,
    /// The visit of the board or overlay the button went down on.
    pub(crate) visit: u64,
    pub(crate) drag: Option<DragTarget>,
    /// Whether a drag began something.
    pub(crate) dragged: bool,
    pub(crate) follow: Option<Follow>,
    pub(crate) reverts: Vec<Revert>,
}
