//! The operation engine: one edit engine for every doc type, so that edits,
//! conflicts and history behave the same everywhere (docs/formats/operations.md).
//!
//! An [`EditDoc`] is a doc under edit: its nodes, each with a revision, in an
//! order kept by fractional indexes. [`EditDoc::apply`] runs a batch of
//! [`Operation`]s, all or nothing. For HTML docs the nodes are the block-level
//! elements of free-form HTML ([`crate::html_doc`]): the content of an
//! operation is any HTML, and an operation whose result the HTML parser would
//! read differently is refused, so a doc always reads back as the same tree.

use crate::design::NodeId;
use crate::html_doc::{Context, Kind, MAX_DEPTH, Ns, ParseError, Print, Tree};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fmt;

mod rebase;

pub use rebase::Rebased;

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
    /// The IDs of the new nodes, in document order.
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
    /// The node's current state, as HTML with `data-omni-id`.
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

/// What the doc knows of a node beyond the element: where it is in the tree,
/// its revision and the fractional index that orders it among its siblings.
#[derive(Clone, Debug)]
struct Meta {
    item: usize,
    rev: u64,
    key: String,
}

/// A node of a doc as a saved version indexes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedNode {
    pub id: NodeId,
    /// The node's type: `section`, `heading`, `paragraph` and so on.
    pub kind: &'static str,
    /// `None` for a top-level node.
    pub parent: Option<NodeId>,
    /// The node's own content (its tag, attributes and what is inside it
    /// outside the nodes in it), which does not change when a child or the
    /// node's place does. The fingerprint is a hash of this text.
    pub content: String,
}

/// A doc under edit: free-form HTML whose block-level elements are nodes. A
/// node's parent is the nearest node above it; the elements between (a
/// `<tbody>`, a `<span>`) belong to the content.
#[derive(Clone, Debug)]
pub struct EditDoc {
    tree: Tree,
    nodes: HashMap<NodeId, Meta>,
}

impl EditDoc {
    /// Starts from HTML in which every block-level element carries a
    /// `data-omni-id`. A node starts at the revision of its `data-rev` (what
    /// `to_html_with_revisions` writes), else at revision 1.
    pub fn from_html(html: &str) -> Result<EditDoc, ParseError> {
        let mut tree = Tree::parse_fragment(html, Context::BODY)?;
        tree.identify_strict()?;
        let mut doc = EditDoc {
            tree,
            nodes: HashMap::new(),
        };
        for ix in doc.tree.subtree(0) {
            if let Some(element) = doc.tree.element(ix) {
                if let Some(id) = element.id {
                    let rev = element.rev.unwrap_or(1);
                    doc.nodes.insert(
                        id,
                        Meta {
                            item: ix,
                            rev,
                            key: String::new(),
                        },
                    );
                }
            }
        }
        doc.rekey_groups(0);
        Ok(doc)
    }

    /// The doc as HTML, with `data-omni-id` on every block-level element.
    pub fn to_html(&self) -> String {
        self.tree.html(0, &Print::PLAIN)
    }

    /// The doc as HTML, with `data-omni-id` and `data-rev` (the node's
    /// revision) on every block-level element: what an agent reads before it
    /// edits.
    pub fn to_html_with_revisions(&self) -> String {
        let revision = |id: NodeId| self.revision(id);
        self.tree.html(
            0,
            &Print {
                ids: true,
                revisions: Some(&revision),
                own: None,
            },
        )
    }

    /// Every node of the doc in document order, as the node index of a saved
    /// version holds it.
    pub fn index(&self) -> Vec<IndexedNode> {
        let mut nodes = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<(usize, Option<NodeId>)> = self.tree.items[0]
            .children
            .iter()
            .rev()
            .map(|child| (*child, None))
            .collect();
        while let Some((ix, parent)) = stack.pop() {
            let mut below = parent;
            if let Some(element) = self.tree.element(ix) {
                if let Some(id) = element.id {
                    nodes.push(IndexedNode {
                        id,
                        kind: element.kind_name(),
                        parent,
                        content: self.own_content(ix),
                    });
                    below = Some(id);
                }
            }
            stack.extend(
                self.tree.items[ix]
                    .children
                    .iter()
                    .rev()
                    .map(|child| (*child, below)),
            );
        }
        nodes
    }

    /// The content of the node at `ix`: its element and what is inside it
    /// outside the nodes in it.
    fn own_content(&self, ix: usize) -> String {
        self.tree.html(
            ix,
            &Print {
                ids: false,
                revisions: None,
                own: Some(ix),
            },
        )
    }

    /// One node and its descendants as HTML.
    pub fn node_html(&self, id: NodeId) -> Option<String> {
        self.nodes
            .get(&id)
            .map(|node| self.tree.html(node.item, &Print::PLAIN))
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
        let container = match parent {
            None => 0,
            Some(id) => self.nodes.get(&id)?.item,
        };
        Some(
            self.ided_children(container)
                .into_iter()
                .filter_map(|ix| self.tree.id_of(ix))
                .collect(),
        )
    }

    /// The parent of a node: `Some(None)` for a top-level node, `None` when the
    /// node is not in the doc.
    pub fn parent(&self, id: NodeId) -> Option<Option<NodeId>> {
        self.nodes.get(&id).map(|node| self.parent_of(node.item))
    }

    /// Whether the node is a section, which an agent may claim (US-023).
    pub fn is_section(&self, id: NodeId) -> bool {
        self.nodes.get(&id).is_some_and(|node| {
            self.tree
                .element(node.item)
                .is_some_and(|element| element.name == "section")
        })
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

    /// The nearest node above an item: `None` at the top level.
    fn parent_of(&self, ix: usize) -> Option<NodeId> {
        let mut at = self.tree.items[ix].parent;
        while let Some(parent) = at {
            if let Some(id) = self.tree.id_of(parent) {
                return Some(id);
            }
            at = self.tree.items[parent].parent;
        }
        None
    }

    /// The nodes directly inside an item (the root for the top level), in
    /// order, as items: those with no other node between them and the item.
    fn ided_children(&self, container: usize) -> Vec<usize> {
        let mut found = Vec::new();
        let mut stack: Vec<usize> = self.tree.items[container]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(ix) = stack.pop() {
            if self.tree.id_of(ix).is_some() {
                found.push(ix);
            } else {
                stack.extend(self.tree.items[ix].children.iter().rev().copied());
            }
        }
        found
    }

    fn sibling_ids(&self, parent: Option<NodeId>) -> Vec<NodeId> {
        let container = parent.map_or(0, |id| self.nodes[&id].item);
        self.ided_children(container)
            .into_iter()
            .filter_map(|ix| self.tree.id_of(ix))
            .collect()
    }

    /// A node and the nodes inside it, in document order.
    fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        self.tree
            .subtree(self.nodes[&id].item)
            .into_iter()
            .filter_map(|ix| self.tree.id_of(ix))
            .collect()
    }

    /// The element a fragment inside an item is parsed in.
    fn context_of(&self, ix: usize) -> (String, Ns) {
        match self.tree.element(ix) {
            Some(element) => (element.name.clone(), element.ns),
            None => ("body".to_string(), Ns::Html),
        }
    }

    // --- Keys ---

    /// Gives the nodes under every node of the subtree at `top` (and under
    /// `top`) fresh fractional indexes, in order. `top` itself keeps its own.
    fn rekey_groups(&mut self, top: usize) {
        let containers: Vec<usize> = self
            .tree
            .subtree(top)
            .into_iter()
            .filter(|ix| *ix == top || self.tree.id_of(*ix).is_some())
            .collect();
        for container in containers {
            let kids = self.ided_children(container);
            let keys = initial_keys(kids.len());
            for (kid, key) in kids.into_iter().zip(keys) {
                let id = self.tree.id_of(kid).expect("a child is a node");
                self.nodes.get_mut(&id).expect("a node has its meta").key = key;
            }
        }
    }

    /// The fractional index for a node that goes right after `after` among
    /// the children of `parent`.
    fn key_for(&self, parent: Option<NodeId>, after: Option<NodeId>) -> String {
        let siblings = self.sibling_ids(parent);
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
        between(before.unwrap_or(""), next)
    }

    // --- Checking operations ---

    fn lookup(&self, index: usize, id: NodeId, role: &str) -> Result<&Meta, ApplyError> {
        self.nodes
            .get(&id)
            .ok_or_else(|| invalid(index, format!("{role} `{id}` is not in the doc")))
    }

    /// The node an operation changes, which must be at revision `rev`.
    fn expect_rev(&self, index: usize, id: NodeId, rev: u64) -> Result<&Meta, ApplyError> {
        let node = self.lookup(index, id, "the node")?;
        if node.rev != rev {
            return Err(ApplyError::Conflict(Conflict {
                index,
                node: id,
                expected: rev,
                current_revision: node.rev,
                current_html: self.tree.html(node.item, &Print::PLAIN),
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
            let node = self.lookup(index, after, "the node to go after")?;
            if self.parent_of(node.item) != parent {
                let place = parent.map_or("the top level".to_string(), |id| format!("`{id}`"));
                return Err(invalid(
                    index,
                    format!("`{after}` is not a child of {place}, so a node cannot go after it"),
                ));
            }
        }
        Ok(())
    }

    /// Where a new child of `parent` goes right after `after` (first when
    /// `None`), as the item that holds it and its place among that item's
    /// children. After a sibling it goes right behind it; first, it goes in
    /// front of the first node inside the parent, else at the end of the
    /// parent.
    fn locate(&self, parent: Option<NodeId>, after: Option<NodeId>) -> (usize, usize) {
        if let Some(after) = after {
            let item = self.nodes[&after].item;
            let holder = self.tree.items[item].parent.expect("a node has a holder");
            return (holder, self.tree.position(item) + 1);
        }
        let container = parent.map_or(0, |id| self.nodes[&id].item);
        match self.ided_children(container).first() {
            Some(first) => {
                let holder = self.tree.items[*first].parent.expect("a node has a holder");
                (holder, self.tree.position(*first))
            }
            None => (container, self.tree.items[container].children.len()),
        }
    }

    /// Checks that the item, put where it is now, reads back as itself: the
    /// HTML of the item inside the elements above it, parsed by the HTML
    /// parser, gives that same HTML again. An element the parser would close,
    /// move or drop there (a `<div>` in a `<p>`, a `<tr>` in a `<section>`)
    /// is an error.
    fn reads_back(&self, ix: usize) -> Result<(), String> {
        let chain = self.tree.ancestors(ix);
        let tag = |ix: usize| {
            self.tree
                .element(ix)
                .map_or("the content".to_string(), |element| {
                    format!("<{}>", element.name)
                })
        };
        if chain.len() + self.tree.height(ix) > MAX_DEPTH {
            return Err(format!(
                "{} nests elements more than {MAX_DEPTH} levels deep",
                tag(ix)
            ));
        }
        let holder = self.tree.items[ix].parent.expect("the item is in the doc");
        if let Some(element) = self.tree.element(ix) {
            let needs: &[&str] = match (element.ns, element.name.as_str()) {
                (Ns::Html, "li") => &["ul", "ol", "menu"],
                (Ns::Html, "tr") => &["table", "thead", "tbody", "tfoot"],
                (Ns::Html, "td" | "th") => &["tr"],
                _ => &[],
            };
            let held = self
                .tree
                .element(holder)
                .is_some_and(|above| above.ns == Ns::Html && needs.contains(&above.name.as_str()));
            if !needs.is_empty() && !held {
                let names: Vec<String> = needs.iter().map(|name| format!("<{name}>")).collect();
                return Err(format!(
                    "{} can only go inside {}",
                    tag(ix),
                    names.join(", ")
                ));
            }
        }
        let blocks = self.tree.block_level(&[ix], self.tree.within_svg(holder));
        let ided: Vec<usize> = self
            .tree
            .subtree(ix)
            .into_iter()
            .filter(|item| self.tree.id_of(*item).is_some())
            .collect();
        if blocks != ided {
            return Err(format!(
                "{} holds a node that is not a block-level element where it goes",
                tag(ix)
            ));
        }
        let mut text = String::new();
        for above in &chain {
            let element = self
                .tree
                .element(*above)
                .expect("an ancestor is an element");
            self.tree
                .print_open(element, *above, &Print::WITHOUT_IDS, &mut text);
        }
        self.tree.print(ix, &Print::WITHOUT_IDS, &mut text);
        for above in chain.iter().rev() {
            let element = self
                .tree
                .element(*above)
                .expect("an ancestor is an element");
            text.push_str("</");
            text.push_str(&element.name);
            text.push('>');
        }
        let place = chain
            .last()
            .map_or("the top level".to_string(), |above| tag(*above));
        let parsed =
            Tree::parse_fragment(&text, Context::BODY).map_err(|e| e.message().to_string())?;
        if parsed.html(0, &Print::WITHOUT_IDS) != text {
            return Err(format!(
                "{} cannot go inside {place}: the HTML parser would read the result differently, so the doc would not read back as the same HTML",
                tag(ix)
            ));
        }
        Ok(())
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
            } => self.set(index, *node, *rev, properties, new_ids),
            Operation::Move {
                node,
                rev,
                parent,
                position,
            } => self.move_node(index, *node, *rev, *parent, position.after),
            Operation::Delete { node, rev } => self.delete(index, *node, *rev),
        }
    }

    /// Parses the content of an insert or a replace: one block-level element,
    /// to go inside the item `holder`.
    fn parse_content(
        &self,
        index: usize,
        content: &str,
        holder: usize,
    ) -> Result<(Tree, usize), ApplyError> {
        let (name, ns) = self.context_of(holder);
        let frag = Tree::parse_fragment(content, Context { name: &name, ns })
            .map_err(|error| invalid(index, error.message()))?;
        let top = one_element(&frag).map_err(|message| invalid(index, message))?;
        let element = frag.element(top).expect("one_element finds an element");
        if !element.is_block_level(self.tree.within_svg(holder)) {
            return Err(invalid(
                index,
                format!(
                    "<{}> is not a block-level element, so it cannot be a node: insert or replace with a block-level element such as <div>, <p>, <section>, <li> or <tr>",
                    element.name
                ),
            ));
        }
        Ok((frag, top))
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
        let (holder, at) = self.locate(parent, after);
        let (mut frag, top) = self.parse_content(index, content, holder)?;
        let blocks = {
            let nodes = &self.nodes;
            frag.identify(
                &[top],
                self.tree.within_svg(holder),
                &[],
                None,
                &|id| !nodes.contains_key(&id),
                &mut || new_ids(),
            )
        };
        let created: Vec<NodeId> = blocks.iter().filter_map(|ix| frag.id_of(*ix)).collect();
        let key = self.key_for(parent, after);
        let item = self.tree.copy_from(&frag, top, false);
        self.tree.attach(holder, at, item);
        self.register(item);
        let node = created[0];
        self.nodes.get_mut(&node).expect("just registered").key = key;
        self.rekey_groups(item);
        self.reads_back(item)
            .map_err(|message| invalid(index, message))?;
        Ok(OpResult {
            node,
            revision: 1,
            created,
            removed: Vec::new(),
        })
    }

    /// Lists the nodes of the subtree at `top` among the doc's nodes, as new
    /// ones at revision 1.
    fn register(&mut self, top: usize) {
        for ix in self.tree.subtree(top) {
            if let Some(id) = self.tree.id_of(ix) {
                self.nodes.insert(
                    id,
                    Meta {
                        item: ix,
                        rev: 1,
                        key: String::new(),
                    },
                );
            }
        }
    }

    fn replace(
        &mut self,
        index: usize,
        id: NodeId,
        rev: u64,
        content: &str,
        new_ids: &mut dyn FnMut() -> NodeId,
    ) -> Result<OpResult, ApplyError> {
        let old_item = self.expect_rev(index, id, rev)?.item;
        let holder = self.tree.items[old_item]
            .parent
            .expect("a node has a holder");
        let (mut frag, top) = self.parse_content(index, content, holder)?;
        let old_order = self.subtree(id);
        let own: HashSet<NodeId> = old_order[1..].iter().copied().collect();
        let blocks = {
            let nodes = &self.nodes;
            frag.identify(
                &[top],
                self.tree.within_svg(holder),
                &[],
                Some(id),
                &|other| own.contains(&other) || !nodes.contains_key(&other),
                &mut || new_ids(),
            )
        };
        let ids: Vec<NodeId> = blocks.iter().filter_map(|ix| frag.id_of(*ix)).collect();
        let before = self.states_of(ids.iter().filter(|other| own.contains(*other)));

        let new_item = self.tree.copy_from(&frag, top, false);
        let at = self.tree.position(old_item);
        self.tree.detach(old_item);
        self.tree.attach(holder, at, new_item);
        for gone in &old_order[1..] {
            self.nodes.remove(gone);
        }
        let node = self.nodes.get_mut(&id).expect("the node is in the doc");
        node.item = new_item;
        node.rev += 1;
        let revision = node.rev;
        self.register_kept(new_item, &before);
        self.rekey_groups(new_item);
        self.reads_back(new_item)
            .map_err(|message| invalid(index, message))?;
        let created = ids
            .into_iter()
            .skip(1)
            .filter(|other| !own.contains(other))
            .collect();
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

    /// The revision and the content of each of `ids`, before an operation
    /// rebuilds the nodes.
    fn states_of<'a>(
        &self,
        ids: impl Iterator<Item = &'a NodeId>,
    ) -> HashMap<NodeId, (u64, String)> {
        ids.map(|id| {
            let node = &self.nodes[id];
            (*id, (node.rev, self.own_content(node.item)))
        })
        .collect()
    }

    /// Lists the nodes below `top` among the doc's nodes (not `top`): a node
    /// that was there before keeps its revision, and gets one more when its
    /// content changed; a new node starts at revision 1.
    fn register_kept(&mut self, top: usize, before: &HashMap<NodeId, (u64, String)>) {
        for ix in self.tree.subtree(top).into_iter().skip(1) {
            let Some(id) = self.tree.id_of(ix) else {
                continue;
            };
            let rev = match before.get(&id) {
                Some((rev, content)) => *rev + u64::from(*content != self.own_content(ix)),
                None => 1,
            };
            self.nodes.insert(
                id,
                Meta {
                    item: ix,
                    rev,
                    key: String::new(),
                },
            );
        }
    }

    fn set(
        &mut self,
        index: usize,
        id: NodeId,
        rev: u64,
        properties: &Map<String, Value>,
        new_ids: &mut dyn FnMut() -> NodeId,
    ) -> Result<OpResult, ApplyError> {
        let item = self.expect_rev(index, id, rev)?.item;
        if properties.is_empty() {
            return Err(invalid(index, "set needs at least one property"));
        }
        if let Some(name) = properties.keys().find(|name| *name != "html") {
            return Err(invalid(
                index,
                format!(
                    "`{name}` is not a property of an element: set takes one property, `html`, the inner HTML of the element"
                ),
            ));
        }
        let Some(html) = properties["html"].as_str() else {
            return Err(invalid(
                index,
                "`html` must be text: the inner HTML of the element",
            ));
        };
        let element = self.tree.element(item).expect("a node is an element");
        let (name, ns, void) = (element.name.clone(), element.ns, element.is_void());
        if void && !html.trim().is_empty() {
            return Err(invalid(
                index,
                format!("<{name}> holds nothing, so it has no inner HTML to set"),
            ));
        }
        let mut frag = Tree::parse_fragment(html, Context { name: &name, ns })
            .map_err(|error| invalid(index, error.message()))?;
        let roots = frag.items[0].children.clone();
        let old_order = self.subtree(id);
        let own: HashSet<NodeId> = old_order[1..].iter().copied().collect();
        let blocks = {
            let nodes = &self.nodes;
            frag.identify(
                &roots,
                self.tree.within_svg(item),
                &[id],
                None,
                &|other| own.contains(&other) || !nodes.contains_key(&other),
                &mut || new_ids(),
            )
        };
        let ids: Vec<NodeId> = blocks.iter().filter_map(|ix| frag.id_of(*ix)).collect();
        let before = self.states_of(ids.iter().filter(|other| own.contains(*other)));

        for child in self.tree.items[item].children.clone() {
            self.tree.detach(child);
        }
        for gone in &old_order[1..] {
            self.nodes.remove(gone);
        }
        for root in roots {
            let copy = self.tree.copy_from(&frag, root, false);
            let at = self.tree.items[item].children.len();
            self.tree.attach(item, at, copy);
        }
        self.register_kept(item, &before);
        self.rekey_groups(item);
        let node = self.nodes.get_mut(&id).expect("the node is in the doc");
        node.rev += 1;
        let revision = node.rev;
        self.reads_back(item)
            .map_err(|message| invalid(index, message))?;
        let created = ids
            .into_iter()
            .filter(|other| !own.contains(other))
            .collect();
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

    fn move_node(
        &mut self,
        index: usize,
        id: NodeId,
        rev: u64,
        parent: Option<NodeId>,
        after: Option<NodeId>,
    ) -> Result<OpResult, ApplyError> {
        let item = self.expect_rev(index, id, rev)?.item;
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
            above = self.parent_of(self.nodes[&ancestor].item);
        }
        self.tree.detach(item);
        let (holder, at) = self.locate(parent, after);
        let key = self.key_for(parent, after);
        self.tree.attach(holder, at, item);
        self.reads_back(item)
            .map_err(|message| invalid(index, message))?;
        let node = self.nodes.get_mut(&id).expect("the node is in the doc");
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
        let item = self.expect_rev(index, id, rev)?.item;
        let removed = self.subtree(id);
        for gone in &removed {
            self.nodes.remove(gone);
        }
        self.tree.detach(item);
        Ok(OpResult {
            node: id,
            revision: rev,
            created: Vec::new(),
            removed,
        })
    }
}

/// The one element of parsed content: white space and comments around it are
/// ignored, and anything else outside it is an error.
fn one_element(tree: &Tree) -> Result<usize, String> {
    let mut found = None;
    for child in &tree.items[0].children {
        match &tree.items[*child].kind {
            Kind::Element(_) => {
                if found.replace(*child).is_some() {
                    return Err("the content holds more than one element: an operation inserts or replaces one element, so wrap them in one".to_string());
                }
            }
            Kind::Text(text) if text.trim().is_empty() => {}
            Kind::Comment(_) => {}
            Kind::Text(text) => {
                let shown: String = text.trim().chars().take(30).collect();
                return Err(format!(
                    "the content has text outside an element (`{shown}`): an operation inserts or replaces one element"
                ));
            }
            _ => {}
        }
    }
    found.ok_or_else(|| {
        "the content holds no element the HTML parser can place there: an operation inserts or replaces one element".to_string()
    })
}

/// Which items of `values` belong to one longest strictly increasing
/// subsequence of them.
pub fn longest_increasing(values: Vec<usize>) -> Vec<bool> {
    // tails[k]: the index of the smallest last value of an increasing run of
    // length k + 1; previous[i]: the item before i in its run.
    let mut tails: Vec<usize> = Vec::new();
    let mut previous: Vec<Option<usize>> = vec![None; values.len()];
    for (i, &v) in values.iter().enumerate() {
        let k = tails.partition_point(|&t| values[t] < v);
        previous[i] = k.checked_sub(1).map(|j| tails[j]);
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut stays = vec![false; values.len()];
    let mut at = tails.last().copied();
    while let Some(i) = at {
        stays[i] = true;
        at = previous[i];
    }
    stays
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

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(html: &str) -> EditDoc {
        EditDoc::from_html(html).expect("the doc parses")
    }

    fn id(text: &str) -> NodeId {
        text.parse().expect("a node ID")
    }

    #[test]
    fn the_index_lists_nodes_in_document_order_with_their_parents() {
        let doc = doc(concat!(
            "<section data-omni-id=\"b0000001\"><h2 data-omni-id=\"b0000002\">Scope</h2>",
            "<p data-omni-id=\"b0000003\">Text</p></section>",
            "<p data-omni-id=\"b0000004\">After</p>"
        ));
        let index = doc.index();
        let listed: Vec<_> = index
            .iter()
            .map(|n| (n.id.to_string(), n.kind, n.parent.map(|p| p.to_string())))
            .collect();
        assert_eq!(
            listed,
            [
                ("b0000001".to_string(), "section", None),
                (
                    "b0000002".to_string(),
                    "heading",
                    Some("b0000001".to_string())
                ),
                (
                    "b0000003".to_string(),
                    "paragraph",
                    Some("b0000001".to_string())
                ),
                ("b0000004".to_string(), "paragraph", None),
            ]
        );
    }

    #[test]
    fn a_nodes_content_leaves_out_its_children_and_its_place() {
        let one = doc(concat!(
            "<section data-omni-id=\"b0000001\"><p data-omni-id=\"b0000003\">One</p></section>",
            "<p data-omni-id=\"b0000004\">Same</p>"
        ));
        let two = doc(concat!(
            "<section data-omni-id=\"b0000001\"><p data-omni-id=\"b0000003\">Other</p>",
            "<p data-omni-id=\"b0000004\">Same</p></section>"
        ));
        let content = |doc: &EditDoc, node: &str| {
            doc.index()
                .into_iter()
                .find(|n| n.id == id(node))
                .map(|n| n.content)
                .expect("the node is indexed")
        };
        assert_eq!(content(&one, "b0000001"), content(&two, "b0000001"));
        assert_eq!(content(&one, "b0000004"), content(&two, "b0000004"));
        assert_ne!(content(&one, "b0000003"), content(&two, "b0000003"));
    }
}
