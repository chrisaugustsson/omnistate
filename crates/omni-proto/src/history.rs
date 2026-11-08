//! The boards a prototype came from, and how it moved from each.

use crate::scroll::Scrolls;
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
/// navigate that left it and the scroll offsets it had.
#[derive(Clone, Debug, Default)]
pub(crate) struct History {
    boards: Vec<NodeId>,
    plays: Vec<Play>,
    scrolls: Vec<Scrolls>,
}

impl History {
    pub(crate) fn push(&mut self, board: NodeId, play: Play, scrolls: Scrolls) {
        self.boards.push(board);
        self.plays.push(play);
        self.scrolls.push(scrolls);
    }

    pub(crate) fn pop(&mut self) -> Option<(NodeId, Play, Scrolls)> {
        Some((self.boards.pop()?, self.plays.pop()?, self.scrolls.pop()?))
    }

    pub(crate) fn boards(&self) -> &[NodeId] {
        &self.boards
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        self.boards.truncate(len);
        self.plays.truncate(len);
        self.scrolls.truncate(len);
    }

    pub(crate) fn clear(&mut self) {
        self.boards.clear();
        self.plays.clear();
        self.scrolls.clear();
    }
}
