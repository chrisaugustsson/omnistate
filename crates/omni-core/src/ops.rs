//! The operation engine: one edit engine for every doc type, so that edits,
//! conflicts and history behave the same everywhere (docs/formats/operations.md).
//!
//! An [`EditDoc`] is a doc under edit: its blocks, each with a revision, in an
//! order kept by fractional indexes. [`EditDoc::apply`] runs a batch of
//! [`Operation`]s, all or nothing, and validates and cleans the content of
//! every operation with the doc type's rules.

use crate::design::NodeId;
use crate::html_doc::ParseError;
use serde::{Deserialize, Deserializer};
use std::fmt;

/// One edit: insert, replace, set, move or delete, read from its JSON form.
#[derive(Clone, Debug, PartialEq)]
pub struct Operation {
    _private: (),
}

impl<'de> Deserialize<'de> for Operation {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        todo!("read an operation from its JSON form")
    }
}

/// What one applied operation did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpResult {
    /// The node the operation inserted, replaced, set, moved or deleted.
    pub node: NodeId,
    /// The node's revision after the operation.
    pub revision: u64,
    /// The IDs the engine assigned to new nodes, in document order.
    pub created: Vec<NodeId>,
    /// The nodes the operation removed from the doc, in document order.
    pub removed: Vec<NodeId>,
}

/// What an applied batch did, one result per operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub ops: Vec<OpResult>,
}

/// An operation that expected another revision of a node than the doc has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The position of the operation in the batch, counting from 0.
    pub index: usize,
    pub node: NodeId,
    /// The revision the operation listed.
    pub expected: u64,
    /// The revision the node has now.
    pub current_revision: u64,
    /// The node's current state, as HTML with `data-node-id`.
    pub current_html: String,
}

/// Why a batch changed nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyError {
    Conflict(Conflict),
    Invalid { index: usize, message: String },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("describe the error")
    }
}

impl std::error::Error for ApplyError {}

/// A doc under edit.
#[derive(Clone, Debug)]
pub struct EditDoc {
    _private: (),
}

impl EditDoc {
    /// Starts from HTML in which every block carries a `data-node-id`. Every
    /// node starts at revision 1.
    pub fn from_html(_html: &str) -> Result<EditDoc, ParseError> {
        todo!("start a doc under edit")
    }

    /// The doc as HTML, with `data-node-id` on every block.
    pub fn to_html(&self) -> String {
        todo!("print the doc")
    }

    /// One node and its descendants as HTML.
    pub fn node_html(&self, _id: NodeId) -> Option<String> {
        todo!("print one node")
    }

    pub fn contains(&self, _id: NodeId) -> bool {
        todo!("look a node up")
    }

    pub fn revision(&self, _id: NodeId) -> Option<u64> {
        todo!("look a revision up")
    }

    /// The children of a node in order, or the top-level nodes for `None`.
    /// `None` when the node is not in the doc.
    pub fn children(&self, _parent: Option<NodeId>) -> Option<Vec<NodeId>> {
        todo!("list children")
    }

    /// The fractional index that orders a node among its siblings.
    pub fn sort_key(&self, _id: NodeId) -> Option<String> {
        todo!("look a sort key up")
    }

    /// Applies a batch all or nothing. `new_ids` supplies the ID of each new
    /// node.
    pub fn apply(
        &mut self,
        _ops: &[Operation],
        _new_ids: &mut dyn FnMut() -> NodeId,
    ) -> Result<Applied, ApplyError> {
        todo!("apply the batch")
    }
}
