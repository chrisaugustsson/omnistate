//! Design files: pages, boards, frames, text, images, vectors and component
//! instances, as typed nodes with explicit design properties, and the JSON
//! form that the renderer, the API and the CLI share.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::str::FromStr;

/// The type of a design node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// A page of a design doc. Its children are boards.
    Page,
    /// A top-level frame on a page.
    Board,
    Frame,
    Text,
    Image,
    Vector,
    /// A placed component, defined in the design system doc.
    Instance,
}

/// A node ID: 8 lowercase base32 characters, unique within a project.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {}

impl NodeId {
    /// The ID as written.
    pub fn as_str(&self) -> &str {
        todo!("US-002: node IDs")
    }
}

/// Why a string is not a node ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidNodeId {}

impl FromStr for NodeId {
    type Err = InvalidNodeId;

    fn from_str(_id: &str) -> Result<Self, Self::Err> {
        todo!("US-002: node IDs")
    }
}

/// A design node and its subtree.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {}

impl Node {
    pub fn id(&self) -> &NodeId {
        todo!("US-002: the design node model")
    }

    pub fn name(&self) -> &str {
        todo!("US-002: the design node model")
    }

    pub fn kind(&self) -> NodeKind {
        todo!("US-002: the design node model")
    }

    pub fn visible(&self) -> bool {
        todo!("US-002: the design node model")
    }

    /// The children of a page, board or frame, in order. Other nodes have none.
    pub fn children(&self) -> &[Node] {
        todo!("US-002: the design node model")
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        todo!("US-002: the JSON form of design nodes")
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        todo!("US-002: the JSON form of design nodes")
    }
}
