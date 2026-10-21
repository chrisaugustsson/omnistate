//! Picking nodes: the node under a point, the selection rules built on it, the
//! events a host page hears, and the outlines it draws over the canvas.
//!
//! These are the types of `Scene`'s selection methods (`view.rs`), and the
//! index of node boxes that answers which node is at a point.

use omni_core::design::{Node, NodeData, NodeId};
use std::collections::{HashMap, HashSet};

use crate::layout::LayoutBox;

pub use omni_core::design::{InvalidNodeAddress, NodeAddress};

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

/// A node of a page as the design viewer's layers panel lists it: a board
/// and everything below it, nodes inside instances included, by their
/// addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    /// The node's ID, or `<instanceId>/<componentNodeId>` for a node inside
    /// an instance.
    pub address: NodeAddress,
    pub name: String,
    /// The node's type as the design has it: `board`, `frame`, `text`,
    /// `image`, `vector` or `instance`.
    pub kind: String,
    /// The node's children in document order.
    pub children: Vec<Layer>,
}

/// A rectangle of the page, in the half-open sense of a hit: it holds its
/// left and top edges and not its right and bottom ones.
#[derive(Clone, Copy)]
struct Extent {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Extent {
    fn of(rect: LayoutBox) -> Self {
        Self {
            left: rect.x,
            top: rect.y,
            right: rect.x + rect.width,
            bottom: rect.y + rect.height,
        }
    }

    fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn contains(&self, (x, y): (f32, f32)) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// A node of the index.
struct Entry {
    /// Where the node is: its ID, or the instance and the component node that
    /// it stands for inside an instance.
    address: NodeAddress,
    name: String,
    /// The node's type as the design has it.
    kind: &'static str,
    parent: Option<usize>,
    /// The board that holds the node, which is the node itself for a board.
    board: usize,
    visible: bool,
    rect: LayoutBox,
    /// Where the node or anything below it can be hit: its own box, plus
    /// that of every visible descendant unless the node clips.
    reach: Extent,
    children: Vec<usize>,
}

/// The boxes of every node of a page, as a tree that answers which node is
/// at a point. Nodes are named by their position in the index, which is
/// stable for the life of the scene.
pub(crate) struct Index {
    entries: Vec<Entry>,
    by_address: HashMap<NodeAddress, usize>,
    /// The boards, in page order.
    boards: Vec<usize>,
}

impl Index {
    /// Indexes `boards` and everything below them, with the `boxes` that
    /// layout gave them. A node that `addresses` names is a node inside an
    /// instance, and has that address; other nodes are addressed by their IDs.
    /// A node that `instances` names is an instance that was expanded into a
    /// frame.
    pub(crate) fn new(
        boards: &[Node],
        boxes: &HashMap<NodeId, LayoutBox>,
        addresses: &HashMap<NodeId, NodeAddress>,
        instances: &HashSet<NodeId>,
    ) -> Self {
        let mut index = Self {
            entries: Vec::new(),
            by_address: HashMap::new(),
            boards: Vec::new(),
        };
        for board in boards {
            let at = index.add(board, None, boxes, addresses, instances);
            index.boards.push(at);
        }
        index
    }

    fn add(
        &mut self,
        node: &Node,
        parent: Option<usize>,
        boxes: &HashMap<NodeId, LayoutBox>,
        addresses: &HashMap<NodeId, NodeAddress>,
        instances: &HashSet<NodeId>,
    ) -> usize {
        let at = self.entries.len();
        let rect = boxes.get(node.id()).copied().unwrap_or_default();
        let clips = matches!(
            node.data(),
            NodeData::Board(frame) | NodeData::Frame(frame) if frame.clip == Some(true)
        );
        let address = addresses
            .get(node.id())
            .cloned()
            .unwrap_or_else(|| NodeAddress::new(None, *node.id()));
        self.entries.push(Entry {
            address: address.clone(),
            name: node.name().to_string(),
            kind: if instances.contains(node.id()) {
                "instance"
            } else {
                node.kind().as_str()
            },
            parent,
            board: parent.map_or(at, |parent| self.entries[parent].board),
            visible: node.visible(),
            rect,
            reach: Extent::of(rect),
            children: Vec::new(),
        });
        self.by_address.insert(address, at);

        let mut reach = Extent::of(rect);
        let mut children = Vec::with_capacity(node.children().len());
        for child in node.children() {
            let child_at = self.add(child, Some(at), boxes, addresses, instances);
            if !clips && child.visible() {
                reach = reach.union(self.entries[child_at].reach);
            }
            children.push(child_at);
        }
        let entry = &mut self.entries[at];
        entry.reach = reach;
        entry.children = children;
        at
    }

    /// The topmost visible node at the page point `point`: the deepest node
    /// whose box holds it, later siblings above earlier ones.
    pub(crate) fn node_at(&self, point: (f32, f32)) -> Option<usize> {
        self.boards
            .iter()
            .rev()
            .find_map(|&board| self.hit(board, point))
    }

    fn hit(&self, at: usize, point: (f32, f32)) -> Option<usize> {
        let entry = &self.entries[at];
        // A node that clips has no reach beyond its box, so a point outside
        // a clipping node is out here too.
        if !entry.visible || !entry.reach.contains(point) {
            return None;
        }
        let inside = Extent::of(entry.rect).contains(point);
        entry
            .children
            .iter()
            .rev()
            .find_map(|&child| self.hit(child, point))
            .or(inside.then_some(at))
    }

    pub(crate) fn lookup(&self, address: &NodeAddress) -> Option<usize> {
        self.by_address.get(address).copied()
    }

    /// The ID of the node: for a node inside an instance, the ID of the node
    /// of the component.
    pub(crate) fn id(&self, at: usize) -> NodeId {
        *self.entries[at].address.node()
    }

    pub(crate) fn name(&self, at: usize) -> &str {
        &self.entries[at].name
    }

    pub(crate) fn rect(&self, at: usize) -> LayoutBox {
        self.entries[at].rect
    }

    pub(crate) fn parent(&self, at: usize) -> Option<usize> {
        self.entries[at].parent
    }

    /// The board that holds the node.
    pub(crate) fn board(&self, at: usize) -> usize {
        self.entries[at].board
    }

    /// The node whose children a click selects among when `at` is selected:
    /// its parent, or the board itself.
    pub(crate) fn context(&self, at: usize) -> usize {
        self.entries[at].parent.unwrap_or(at)
    }

    /// The child of `ancestor` that is `descendant` or holds it, if
    /// `descendant` is below `ancestor`.
    pub(crate) fn child_of(&self, descendant: usize, ancestor: usize) -> Option<usize> {
        let mut at = descendant;
        while let Some(parent) = self.entries[at].parent {
            if parent == ancestor {
                return Some(at);
            }
            at = parent;
        }
        None
    }

    /// The boards with every node below them, as layers.
    pub(crate) fn layers(&self) -> Vec<Layer> {
        self.boards.iter().map(|&board| self.layer(board)).collect()
    }

    fn layer(&self, at: usize) -> Layer {
        let entry = &self.entries[at];
        Layer {
            address: entry.address.clone(),
            name: entry.name.clone(),
            kind: entry.kind.to_string(),
            children: entry
                .children
                .iter()
                .map(|&child| self.layer(child))
                .collect(),
        }
    }

    /// The address of the node.
    pub(crate) fn address(&self, at: usize) -> NodeAddress {
        self.entries[at].address.clone()
    }
}
