//! The operation engine: one edit engine for every doc type, so that edits,
//! conflicts and history behave the same everywhere (docs/formats/operations.md).
//!
//! An [`EditDoc`] is a doc under edit: its blocks, each with a revision, in an
//! order kept by fractional indexes. [`EditDoc::apply`] runs a batch of
//! [`Operation`]s, all or nothing, and validates and cleans the content of
//! every operation with the doc type's rules (for HTML docs, those of
//! [`crate::html_doc`]).

use crate::design::NodeId;
use crate::html_doc::{Block, BlockKind, HtmlDoc, MAX_DEPTH, ParseError, can_hold, describe};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Where a node goes among the children of its parent: right after the child
/// `after`, or first when `after` is `null`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    #[serde(deserialize_with = "present")]
    pub after: Option<NodeId>,
}

/// One edit: insert, replace, set, move or delete, read from its JSON form. A
/// `null` parent is the top level.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Insert {
        #[serde(deserialize_with = "present")]
        parent: Option<NodeId>,
        position: Position,
        content: String,
    },
    Replace {
        node: NodeId,
        rev: u64,
        content: String,
    },
    Set {
        node: NodeId,
        rev: u64,
        properties: Map<String, Value>,
    },
    Move {
        node: NodeId,
        rev: u64,
        #[serde(deserialize_with = "present")]
        parent: Option<NodeId>,
        position: Position,
    },
    Delete {
        node: NodeId,
        rev: u64,
    },
}

/// Reads a field that may be `null` but must be there: serde reads a missing
/// `Option` field as `None`, unless the field has its own deserializer.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<NodeId>, D::Error> {
    Option::deserialize(deserializer)
}

/// What one applied operation did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpResult {
    /// The node the operation inserted, replaced, set, moved or deleted.
    pub node: NodeId,
    /// The node's revision after the operation. A deleted node keeps the
    /// revision it had.
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::Conflict(conflict) => write!(
                f,
                "operation {}: conflict on `{}`: it is at revision {}, not {}",
                conflict.index, conflict.node, conflict.current_revision, conflict.expected
            ),
            ApplyError::Invalid { index, message } => write!(f, "operation {index}: {message}"),
        }
    }
}

impl std::error::Error for ApplyError {}

fn invalid(index: usize, message: impl Into<String>) -> ApplyError {
    ApplyError::Invalid {
        index,
        message: message.into(),
    }
}

/// A node of a doc under edit: its block without the blocks inside it, which
/// are the node's `children`, in the order of their sort keys.
#[derive(Clone, Debug)]
struct Node {
    block: Block,
    parent: Option<NodeId>,
    key: String,
    rev: u64,
    children: Vec<NodeId>,
}

/// A doc under edit.
#[derive(Clone, Debug)]
pub struct EditDoc {
    nodes: HashMap<NodeId, Node>,
    top: Vec<NodeId>,
}

impl EditDoc {
    /// Starts from HTML in which every block carries a `data-node-id`. Every
    /// node starts at revision 1.
    pub fn from_html(html: &str) -> Result<EditDoc, ParseError> {
        let parsed = HtmlDoc::parse_identified(html)?;
        let mut doc = EditDoc {
            nodes: HashMap::new(),
            top: Vec::new(),
        };
        let mut keep_id = |_: &mut Block| Ok(1);
        doc.top = doc
            .graft_all(parsed.blocks().to_vec(), None, &mut keep_id)
            .expect("a parsed doc has an ID on every block");
        Ok(doc)
    }

    /// The doc as HTML, with `data-node-id` on every block.
    pub fn to_html(&self) -> String {
        HtmlDoc::from_blocks(self.top.iter().map(|id| self.block(*id)).collect()).to_html()
    }

    /// The doc as HTML, with `data-node-id` and `data-rev` (the node's
    /// revision) on every block: what an agent reads before it edits.
    pub fn to_html_with_revisions(&self) -> String {
        let revision = |id: &NodeId| self.revision(*id);
        HtmlDoc::from_blocks(self.top.iter().map(|id| self.block(*id)).collect())
            .to_html_with(Some(&revision))
    }

    /// One node and its descendants as HTML.
    pub fn node_html(&self, id: NodeId) -> Option<String> {
        self.nodes
            .contains_key(&id)
            .then(|| self.block(id).to_html())
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn revision(&self, id: NodeId) -> Option<u64> {
        self.nodes.get(&id).map(|node| node.rev)
    }

    /// The children of a node in order, or the top-level nodes for `None`.
    /// `None` when the node is not in the doc.
    pub fn children(&self, parent: Option<NodeId>) -> Option<Vec<NodeId>> {
        match parent {
            None => Some(self.top.clone()),
            Some(id) => self.nodes.get(&id).map(|node| node.children.clone()),
        }
    }

    /// The parent of a node: `Some(None)` for a top-level node, `None` when the
    /// node is not in the doc.
    pub fn parent(&self, id: NodeId) -> Option<Option<NodeId>> {
        self.nodes.get(&id).map(|node| node.parent)
    }

    /// Whether the node is a section, which an agent may claim (US-023).
    pub fn is_section(&self, id: NodeId) -> bool {
        self.nodes
            .get(&id)
            .is_some_and(|node| node.block.kind() == BlockKind::Section)
    }

    /// The fractional index that orders a node among its siblings.
    pub fn sort_key(&self, id: NodeId) -> Option<String> {
        self.nodes.get(&id).map(|node| node.key.clone())
    }

    /// Applies a batch all or nothing. `new_ids` supplies the ID of each new
    /// node.
    pub fn apply(
        &mut self,
        ops: &[Operation],
        new_ids: &mut dyn FnMut() -> NodeId,
    ) -> Result<Applied, ApplyError> {
        // The batch runs on a copy, which replaces the doc only when every
        // operation applied.
        let mut next = self.clone();
        let mut results = Vec::with_capacity(ops.len());
        for (index, op) in ops.iter().enumerate() {
            results.push(next.apply_one(index, op, new_ids)?);
        }
        *self = next;
        Ok(Applied { ops: results })
    }

    // --- Reading the tree ---

    /// A node and its descendants as a block tree.
    fn block(&self, id: NodeId) -> Block {
        let node = &self.nodes[&id];
        let mut block = node.block.clone();
        block.set_children(
            node.children
                .iter()
                .map(|child| self.block(*child))
                .collect(),
        );
        block
    }

    fn siblings(&self, parent: Option<NodeId>) -> &Vec<NodeId> {
        match parent {
            None => &self.top,
            Some(id) => &self.nodes[&id].children,
        }
    }

    fn siblings_mut(&mut self, parent: Option<NodeId>) -> &mut Vec<NodeId> {
        match parent {
            None => &mut self.top,
            Some(id) => {
                &mut self
                    .nodes
                    .get_mut(&id)
                    .expect("the parent is in the doc")
                    .children
            }
        }
    }

    fn kind_of(&self, id: Option<NodeId>) -> Option<BlockKind> {
        id.map(|id| self.nodes[&id].block.kind())
    }

    /// How many levels deep a node sits: 1 for a top-level node, 0 for `None`.
    fn level(&self, id: Option<NodeId>) -> usize {
        let mut level = 0;
        let mut at = id;
        while let Some(id) = at {
            level += 1;
            at = self.nodes[&id].parent;
        }
        level
    }

    /// How many levels a node and its descendants take up: 1 for a leaf.
    fn height(&self, id: NodeId) -> usize {
        1 + self.nodes[&id]
            .children
            .iter()
            .map(|child| self.height(*child))
            .max()
            .unwrap_or(0)
    }

    /// A node and its descendants, in document order.
    fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        let mut ids = vec![id];
        for child in &self.nodes[&id].children {
            ids.extend(self.subtree(*child));
        }
        ids
    }

    // --- Checking operations ---

    fn lookup(&self, index: usize, id: NodeId, role: &str) -> Result<&Node, ApplyError> {
        self.nodes
            .get(&id)
            .ok_or_else(|| invalid(index, format!("{role} `{id}` is not in the doc")))
    }

    /// The node an operation changes, which must be at revision `rev`.
    fn expect_rev(&self, index: usize, id: NodeId, rev: u64) -> Result<&Node, ApplyError> {
        let node = self.lookup(index, id, "the node")?;
        if node.rev != rev {
            return Err(ApplyError::Conflict(Conflict {
                index,
                node: id,
                expected: rev,
                current_revision: node.rev,
                current_html: self.block(id).to_html(),
            }));
        }
        Ok(node)
    }

    /// Checks that `parent` and the child `after` that a position names are
    /// in the doc.
    fn check_place(
        &self,
        index: usize,
        parent: Option<NodeId>,
        after: Option<NodeId>,
    ) -> Result<(), ApplyError> {
        if let Some(parent) = parent {
            self.lookup(index, parent, "the parent")?;
        }
        if let Some(after) = after {
            self.lookup(index, after, "the node to go after")?;
            if self.nodes[&after].parent != parent {
                let place = parent.map_or("the top level".to_string(), |id| format!("`{id}`"));
                return Err(invalid(
                    index,
                    format!("`{after}` is not a child of {place}, so a node cannot go after it"),
                ));
            }
        }
        Ok(())
    }

    /// The place in the children of `parent` right after `after` (first when
    /// `None`), as the index and the fractional index that goes there.
    fn place(&self, parent: Option<NodeId>, after: Option<NodeId>) -> (usize, String) {
        let siblings = self.siblings(parent);
        let at = after.map_or(0, |after| {
            siblings
                .iter()
                .position(|id| *id == after)
                .expect("after is a child")
                + 1
        });
        let before = at
            .checked_sub(1)
            .map(|i| self.nodes[&siblings[i]].key.as_str());
        let next = siblings.get(at).map(|id| self.nodes[id].key.as_str());
        (at, between(before.unwrap_or(""), next))
    }

    // --- Applying operations ---

    fn apply_one(
        &mut self,
        index: usize,
        op: &Operation,
        new_ids: &mut dyn FnMut() -> NodeId,
    ) -> Result<OpResult, ApplyError> {
        match op {
            Operation::Insert {
                parent,
                position,
                content,
            } => self.insert(index, *parent, position.after, content, new_ids),
            Operation::Replace { node, rev, content } => {
                self.replace(index, *node, *rev, content, new_ids)
            }
            Operation::Set {
                node,
                rev,
                properties,
            } => self.set(index, *node, *rev, properties),
            Operation::Move {
                node,
                rev,
                parent,
                position,
            } => self.move_node(index, *node, *rev, *parent, position.after),
            Operation::Delete { node, rev } => self.delete(index, *node, *rev),
        }
    }

    fn insert(
        &mut self,
        index: usize,
        parent: Option<NodeId>,
        after: Option<NodeId>,
        content: &str,
        new_ids: &mut dyn FnMut() -> NodeId,
    ) -> Result<OpResult, ApplyError> {
        self.check_place(index, parent, after)?;
        let block = HtmlDoc::parse_block(content, self.kind_of(parent), self.level(parent))
            .map_err(|error| invalid(index, error.message()))?;
        let (at, key) = self.place(parent, after);
        let mut created = Vec::new();
        let mut assign = |block: &mut Block| {
            if let Some(id) = block.id() {
                return Err(format!(
                    "inserted content must not carry `data-node-id` (`{id}`): the server assigns the IDs of new nodes"
                ));
            }
            let id = new_ids();
            block.set_id(id);
            created.push(id);
            Ok(1)
        };
        let node = self
            .graft(block, parent, key, &mut assign)
            .map_err(|message| invalid(index, message))?;
        self.siblings_mut(parent).insert(at, node);
        Ok(OpResult {
            node,
            revision: 1,
            created,
            removed: Vec::new(),
        })
    }

    fn replace(
        &mut self,
        index: usize,
        id: NodeId,
        rev: u64,
        content: &str,
        new_ids: &mut dyn FnMut() -> NodeId,
    ) -> Result<OpResult, ApplyError> {
        let node = self.expect_rev(index, id, rev)?;
        let parent = node.parent;
        let parsed = HtmlDoc::parse_block(content, self.kind_of(parent), self.level(parent))
            .map_err(|error| invalid(index, error.message()))?;
        if let Some(root) = parsed.id() {
            if *root != id {
                return Err(invalid(
                    index,
                    format!(
                        "the content's `data-node-id` `{root}` is not the replaced node `{id}`: replace keeps the node's ID"
                    ),
                ));
            }
        }
        let old_order = self.subtree(id);
        let own: HashSet<NodeId> = old_order[1..].iter().copied().collect();
        let mut inner = Vec::new();
        collect_ids(parsed.children(), &mut inner);
        for kept in inner {
            if !own.contains(&kept) {
                let message = if self.nodes.contains_key(&kept) {
                    format!(
                        "`{kept}` belongs to another node of the doc: replacing `{id}` may reuse only its own descendants' IDs"
                    )
                } else {
                    format!("`{kept}` is not in the doc: the server assigns the IDs of new nodes")
                };
                return Err(invalid(index, message));
            }
        }

        // Take the old descendants out; those that the content includes come
        // back with their revisions.
        let old: HashMap<NodeId, Node> = old_order[1..]
            .iter()
            .map(|old| {
                (
                    *old,
                    self.nodes.remove(old).expect("a descendant is in the doc"),
                )
            })
            .collect();
        let mut block = parsed;
        let children = block.take_children();
        block.set_id(id);
        let root = self.nodes.get_mut(&id).expect("the node is in the doc");
        root.block = block;
        root.rev += 1;
        let revision = root.rev;
        let mut created = Vec::new();
        let mut assign = |block: &mut Block| match block.id() {
            Some(kept) => {
                let before = &old[kept];
                Ok(before.rev + u64::from(before.block != *block))
            }
            None => {
                let id = new_ids();
                block.set_id(id);
                created.push(id);
                Ok(1)
            }
        };
        let kids = self
            .graft_all(children, Some(id), &mut assign)
            .map_err(|message| invalid(index, message))?;
        self.nodes
            .get_mut(&id)
            .expect("the node is in the doc")
            .children = kids;
        let removed = old_order
            .into_iter()
            .filter(|old| !self.nodes.contains_key(old))
            .collect();
        Ok(OpResult {
            node: id,
            revision,
            created,
            removed,
        })
    }

    fn set(
        &mut self,
        index: usize,
        id: NodeId,
        rev: u64,
        properties: &Map<String, Value>,
    ) -> Result<OpResult, ApplyError> {
        self.expect_rev(index, id, rev)?;
        if properties.is_empty() {
            return Err(invalid(index, "set needs at least one property"));
        }
        let depth = self.level(Some(id)) - 1;
        let node = self.nodes.get_mut(&id).expect("the node is in the doc");
        for (name, value) in properties {
            node.block
                .set_property(name, value, depth)
                .map_err(|message| invalid(index, message))?;
        }
        node.rev += 1;
        Ok(OpResult {
            node: id,
            revision: node.rev,
            created: Vec::new(),
            removed: Vec::new(),
        })
    }

    fn move_node(
        &mut self,
        index: usize,
        id: NodeId,
        rev: u64,
        parent: Option<NodeId>,
        after: Option<NodeId>,
    ) -> Result<OpResult, ApplyError> {
        let node = self.expect_rev(index, id, rev)?;
        let (old_parent, kind) = (node.parent, node.block.kind());
        self.check_place(index, parent, after)?;
        if after == Some(id) {
            return Err(invalid(index, format!("`{id}` cannot go after itself")));
        }
        let mut above = parent;
        while let Some(ancestor) = above {
            if ancestor == id {
                return Err(invalid(
                    index,
                    format!("`{id}` cannot move into itself or one of its descendants"),
                ));
            }
            above = self.nodes[&ancestor].parent;
        }
        let parent_kind = self.kind_of(parent);
        if !can_hold(parent_kind, kind) {
            let place = parent_kind.map_or("the top level".to_string(), |kind| {
                describe(kind).to_string()
            });
            return Err(invalid(
                index,
                format!("`{id}` is {} and cannot go inside {place}", describe(kind)),
            ));
        }
        if self.level(parent) + self.height(id) > MAX_DEPTH {
            return Err(invalid(
                index,
                format!("moving `{id}` there nests blocks more than {MAX_DEPTH} levels deep"),
            ));
        }
        self.siblings_mut(old_parent)
            .retain(|sibling| *sibling != id);
        let (at, key) = self.place(parent, after);
        self.siblings_mut(parent).insert(at, id);
        let node = self.nodes.get_mut(&id).expect("the node is in the doc");
        node.parent = parent;
        node.key = key;
        node.rev += 1;
        Ok(OpResult {
            node: id,
            revision: node.rev,
            created: Vec::new(),
            removed: Vec::new(),
        })
    }

    fn delete(&mut self, index: usize, id: NodeId, rev: u64) -> Result<OpResult, ApplyError> {
        let node = self.expect_rev(index, id, rev)?;
        let parent = node.parent;
        let removed = self.subtree(id);
        for gone in &removed {
            self.nodes.remove(gone);
        }
        self.siblings_mut(parent).retain(|sibling| *sibling != id);
        Ok(OpResult {
            node: id,
            revision: rev,
            created: Vec::new(),
            removed,
        })
    }

    // --- Building the tree ---

    /// Puts `blocks` into the doc as the children of `parent`, with fresh
    /// fractional indexes in order, and returns their IDs.
    fn graft_all(
        &mut self,
        blocks: Vec<Block>,
        parent: Option<NodeId>,
        assign: &mut dyn FnMut(&mut Block) -> Result<u64, String>,
    ) -> Result<Vec<NodeId>, String> {
        let keys = initial_keys(blocks.len());
        blocks
            .into_iter()
            .zip(keys)
            .map(|(block, key)| self.graft(block, parent, key, assign))
            .collect()
    }

    /// Puts a block and the blocks inside it into the doc as nodes, the block
    /// under `parent` with the sort key `key` (the caller adds it to the
    /// parent's children). `assign` gives each block, in document order, its ID
    /// if it has none, and says its revision.
    fn graft(
        &mut self,
        mut block: Block,
        parent: Option<NodeId>,
        key: String,
        assign: &mut dyn FnMut(&mut Block) -> Result<u64, String>,
    ) -> Result<NodeId, String> {
        let children = block.take_children();
        let rev = assign(&mut block)?;
        let id = *block.id().expect("assign gives every block an ID");
        if self.nodes.contains_key(&id) {
            return Err(format!("`{id}` is already a node of the doc"));
        }
        self.nodes.insert(
            id,
            Node {
                block,
                parent,
                key,
                rev,
                children: Vec::new(),
            },
        );
        let kids = self.graft_all(children, Some(id), assign)?;
        self.nodes.get_mut(&id).expect("just inserted").children = kids;
        Ok(id)
    }
}

/// The IDs that the blocks and their descendants carry, in document order.
fn collect_ids(blocks: &[Block], ids: &mut Vec<NodeId>) {
    for block in blocks {
        ids.extend(block.id().copied());
        collect_ids(block.children(), ids);
    }
}

// --- Fractional indexes ---

/// The digits of a fractional index, in order. An index is a fraction written
/// in these digits, such as `0i`, compared as text. It never ends in the first
/// digit, so there is always room before and after it.
const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn digit_value(digit: u8) -> usize {
    DIGITS
        .iter()
        .position(|d| *d == digit)
        .expect("an index is written in DIGITS")
}

/// `count` indexes in increasing order, for a list built in one go: counters
/// of one width, followed by the middle digit so that none ends in a `0`.
fn initial_keys(count: usize) -> Vec<String> {
    let mut width = 1;
    let mut reach = DIGITS.len();
    while reach < count {
        width += 1;
        reach *= DIGITS.len();
    }
    (0..count)
        .map(|n| {
            let mut key = vec![0; width];
            let mut rest = n;
            for slot in key.iter_mut().rev() {
                *slot = DIGITS[rest % DIGITS.len()];
                rest /= DIGITS.len();
            }
            key.push(DIGITS[DIGITS.len() / 2]);
            String::from_utf8(key).expect("DIGITS are ASCII")
        })
        .collect()
}

/// An index strictly between `before` (empty for the start of the list) and
/// `after` (`None` for the end), which must be greater than `before`.
fn between(before: &str, after: Option<&str>) -> String {
    let key = midpoint(before.as_bytes(), after.map(str::as_bytes));
    String::from_utf8(key).expect("DIGITS are ASCII")
}

fn midpoint(before: &[u8], after: Option<&[u8]>) -> Vec<u8> {
    if let Some(after) = after {
        // Keep the digits that both share, reading `before` as padded with zeros.
        let shared = (0..)
            .take_while(|n| Some(&before.get(*n).copied().unwrap_or(DIGITS[0])) == after.get(*n))
            .count();
        if shared > 0 {
            let mut key = after[..shared].to_vec();
            key.extend(midpoint(
                before.get(shared..).unwrap_or(&[]),
                Some(&after[shared..]),
            ));
            return key;
        }
    }
    let low = before.first().map_or(0, |digit| digit_value(*digit));
    let high = after.map_or(DIGITS.len(), |after| digit_value(after[0]));
    if high - low > 1 {
        vec![DIGITS[(low + high).div_ceil(2)]]
    } else if let Some(after) = after.filter(|after| after.len() > 1) {
        vec![after[0]]
    } else {
        // The first digits are neighbours, and `after` is at most that digit:
        // keep the lower one and find room after the rest of `before`.
        let mut key = vec![DIGITS[low]];
        key.extend(midpoint(before.get(1..).unwrap_or(&[]), None));
        key
    }
}
