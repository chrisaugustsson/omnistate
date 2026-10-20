//! The tag format: designs as short, JSX-like tags that agents read and write
//! (Q26, FR-33). Storage stays JSON, and [`print`] and [`parse`] convert
//! between the two without loss.
//!
//! - One tag is one node, named after its type: `Page`, `Board`, `Frame`,
//!   `Text`, `Image`, `Vector` and `Instance`. A tag's children are the node's
//!   children, in order.
//! - Attributes are the node's properties, written `name="text"` or
//!   `name={expression}`, such as
//!   `<Frame id="k3f9x2qa" name="Card" dir="col" gap="$space.md" pad={16} fill="$color.surface">`.
//!   A tag without children may close itself: `<Frame name="Card"/>`.
//! - `id` refers to an existing node. A tag written without an `id` is a new
//!   node: the server assigns its ID (US-031), never the agent.
//! - [`print`] writes every property a node sets, and every number as the same
//!   double, so parsing what it prints gives back the same node.
//! - [`parse`] rejects unknown tags, unknown attributes and invalid values with
//!   a [`TagError`] naming the line, the column and how to fix it. Lines and
//!   columns count from 1, and columns count characters. An error points at
//!   the first character of the tag name, the attribute name or the value
//!   (its opening `"` or `{`) that has the problem.
//!
//! `docs/formats/design-tags.md` documents every tag and attribute.

use super::{Node, NodeId, NodeKind};
use std::fmt;

/// A tag of the format and the attributes it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    /// The tag's name, such as `Frame`.
    pub name: &'static str,
    /// Every attribute the tag takes, such as `id`, `name` and `dir`.
    pub attributes: &'static [&'static str],
}

/// Every tag of the format, one per node type.
pub fn all() -> &'static [Tag] {
    todo!("US-027: the tags of the format")
}

/// Prints `node` and its subtree in the tag format, one tag per node, with
/// each node's `id`.
pub fn print(node: &Node) -> String {
    let _ = node;
    todo!("US-027: print a node in the tag format")
}

/// Parses one tag and its subtree.
pub fn parse(source: &str) -> Result<TagNode, TagError> {
    let _ = source;
    todo!("US-027: parse the tag format")
}

/// A node read from the tag format, and its subtree. A node written without
/// an `id` is new until the server assigns it one.
#[derive(Clone, Debug, PartialEq)]
pub struct TagNode {
    _todo: (),
}

impl TagNode {
    /// The node's ID, or `None` for a new node.
    pub fn id(&self) -> Option<&NodeId> {
        todo!("US-027")
    }

    /// Whether the node was written without an `id`.
    pub fn is_new(&self) -> bool {
        todo!("US-027")
    }

    pub fn kind(&self) -> NodeKind {
        todo!("US-027")
    }

    pub fn name(&self) -> &str {
        todo!("US-027")
    }

    /// The node's children, in order.
    pub fn children(&self) -> &[TagNode] {
        todo!("US-027")
    }

    /// The node with an ID on every node: each new node, in document order,
    /// takes the next ID from `next`, and every other node keeps its own.
    pub fn assign_ids(self, next: impl FnMut() -> NodeId) -> Node {
        let _ = next;
        todo!("US-027")
    }
}

/// Why text is not valid in the tag format: where the problem is, what it is
/// and how to fix it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagError {
    line: usize,
    column: usize,
    message: String,
    fix: String,
}

impl TagError {
    /// The line of the problem, counting from 1.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The column of the problem, counting characters from 1.
    pub fn column(&self) -> usize {
        self.column
    }

    /// The problem, such as "`Box` is not a tag".
    pub fn message(&self) -> &str {
        &self.message
    }

    /// How to fix it, such as "use one of `Page`, `Board`, `Frame`, ...".
    pub fn fix(&self) -> &str {
        &self.fix
    }
}

impl fmt::Display for TagError {
    /// `line <line>, column <column>: <message>`, then how to fix it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!("US-027")
    }
}

impl std::error::Error for TagError {}
