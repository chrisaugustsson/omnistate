//! Picking nodes: the node under a point, the selection rules built on it, the
//! events a host page hears, and the outlines it draws over the canvas.
//!
//! These are the types of `Scene`'s selection methods (`view.rs`); the bodies
//! of those methods are US-007's to write.

use omni_core::design::NodeId;
use std::fmt;
use std::str::FromStr;

use crate::layout::LayoutBox;

/// Where a node is, as comments pin to it: `<id>` for a node, or
/// `<instanceId>/<componentNodeId>` for a node inside a component instance.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeAddress {
    instance: Option<NodeId>,
    node: NodeId,
}

impl NodeAddress {
    pub fn new(instance: Option<NodeId>, node: NodeId) -> Self {
        Self { instance, node }
    }

    /// The instance that holds the node, for an address inside one.
    pub fn instance(&self) -> Option<&NodeId> {
        self.instance.as_ref()
    }

    /// The addressed node: the component's node for an address inside an
    /// instance.
    pub fn node(&self) -> &NodeId {
        &self.node
    }
}

/// Why a string is not a node address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidNodeAddress;

impl fmt::Display for NodeAddress {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("US-007: write `<id>` or `<instanceId>/<componentNodeId>`")
    }
}

impl FromStr for NodeAddress {
    type Err = InvalidNodeAddress;

    fn from_str(_address: &str) -> Result<Self, Self::Err> {
        todo!("US-007: read `<id>` or `<instanceId>/<componentNodeId>`")
    }
}

/// The keys held during a click or a pointer move: Ctrl or Cmd reaches the
/// deepest node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub meta: bool,
}

/// What a host page hears when hover or selection changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The hovered node changed; `None` when the pointer is over no board.
    Hover(Option<NodeAddress>),
    /// The selected node changed; `None` when the selection was cleared.
    Select(Option<NodeAddress>),
}

/// Which node an outline marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineKind {
    Hover,
    Selection,
}

/// An outline for the host page to draw over the canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub kind: OutlineKind,
    pub node: NodeAddress,
    /// The node's name, drawn next to the outline.
    pub label: String,
    /// The node's box in screen CSS pixels, in the viewport the outline was
    /// asked for.
    pub rect: LayoutBox,
}
