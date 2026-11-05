//! The boards a prototype came from, and how it moved from each.

use omni_core::design::NodeId;
use omni_core::design::proto::{Easing, Transition};

/// How a change of board plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    pub transition: Transition,
    pub duration_ms: u32,
    pub easing: Easing,
}

/// The boards that navigate left, newest last, each with the play of the
/// navigate that left it.
#[derive(Clone, Debug, Default)]
pub(crate) struct History {
    boards: Vec<NodeId>,
    plays: Vec<Play>,
}

impl History {
    pub(crate) fn push(&mut self, board: NodeId, play: Play) {
        self.boards.push(board);
        self.plays.push(play);
    }

    pub(crate) fn pop(&mut self) -> Option<(NodeId, Play)> {
        Some((self.boards.pop()?, self.plays.pop()?))
    }

    pub(crate) fn boards(&self) -> &[NodeId] {
        &self.boards
    }

    pub(crate) fn clear(&mut self) {
        self.boards.clear();
        self.plays.clear();
    }
}
