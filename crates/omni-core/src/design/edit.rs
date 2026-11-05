//! A design under edit: the pages of a design doc with the revision of every
//! node, and the operations of docs/formats/operations.md applied to them,
//! all or nothing (US-031, FR-15, FR-16).
//!
//! The operations are those of the HTML engine in [`crate::ops`], on design
//! nodes: `insert` and `replace` take one tag of the tag format with its
//! children, `set` names properties as the attributes of the node's tag, and
//! `move` and `delete` take a node with its subtree along. The server assigns
//! the ID of every new node; a tag that carries one is refused. A node starts
//! at revision 1, an operation that changes a node raises that node's
//! revision by 1 and no other, and an insert leaves the parent's revision
//! alone.

use super::proto::{Action, Interaction};
use super::tags::{self, Host, TagNode};
use super::{Node, NodeData, NodeId, NodeKind};
use crate::ops::{Applied, ApplyError, Conflict, OpResult, Operation};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// The name and the variant set of a component of the design system, by its
/// ID; `None` when there is no such component.
pub type Variants<'a> = dyn Fn(NodeId) -> Option<(String, Vec<NodeId>)> + 'a;

/// Where a node sits in the design.
#[derive(Clone, Copy, Debug)]
struct Place {
    /// `None` for a page.
    parent: Option<NodeId>,
    kind: NodeKind,
}

/// The pages of a design under edit.
#[derive(Clone, Debug, Default)]
pub struct DesignDoc {
    pages: Vec<Node>,
    revisions: HashMap<NodeId, u64>,
    places: HashMap<NodeId, Place>,
}

fn invalid(index: usize, message: impl Into<String>) -> ApplyError {
    ApplyError::Invalid {
        index,
        message: message.into(),
    }
}

impl DesignDoc {
    /// A design with these pages. A node that `revisions` does not list is at
    /// revision 1.
    pub fn new(pages: Vec<Node>, revisions: HashMap<NodeId, u64>) -> DesignDoc {
        let mut doc = DesignDoc {
            pages,
            revisions,
            places: HashMap::new(),
        };
        let mut places = HashMap::new();
        for page in &doc.pages {
            index_subtree(page, None, &mut places);
        }
        doc.places = places;
        doc
    }

    pub fn pages(&self) -> &[Node] {
        &self.pages
    }

    pub fn revisions(&self) -> &HashMap<NodeId, u64> {
        &self.revisions
    }

    /// The node's revision, `None` when it is not in the design.
    pub fn revision(&self, id: &NodeId) -> Option<u64> {
        self.places
            .contains_key(id)
            .then(|| self.revisions.get(id).copied().unwrap_or(1))
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.places.contains_key(&id)
    }

    /// The parent of a node: `Some(None)` for a page, `None` when the node is
    /// not in the design.
    pub fn parent(&self, id: NodeId) -> Option<Option<NodeId>> {
        self.places.get(&id).map(|place| place.parent)
    }

    /// Whether the node is a board.
    pub fn is_board(&self, id: NodeId) -> bool {
        self.places
            .get(&id)
            .is_some_and(|place| place.kind == NodeKind::Board)
    }

    /// Checks the interactions of `node` against the doc (D17): a navigate,
    /// open overlay or swap overlay `dest` is a board of the doc, a scroll to
    /// `dest` a layer below the top-level board that holds `node`, and a
    /// change to is on an instance, with a `dest` in the variant set of the
    /// component it places. Conditional blocks are checked at any depth.
    pub fn check_interactions(
        &self,
        node: NodeId,
        interactions: &[Interaction],
        variants: &Variants<'_>,
    ) -> Result<(), String> {
        let mut pending: Vec<&Action> = interactions
            .iter()
            .flat_map(|interaction| interaction.actions.iter())
            .collect();
        pending.reverse();
        while let Some(action) = pending.pop() {
            if let Action::Conditional { blocks } = action {
                pending.extend(
                    blocks
                        .iter()
                        .rev()
                        .flat_map(|block| block.actions.iter().rev()),
                );
                continue;
            }
            let Some(text) = action.dest_text() else {
                continue;
            };
            if let Action::ChangeTo { .. } = action {
                self.check_change_to(node, text, variants)?;
                continue;
            }
            let dest = text
                .parse::<NodeId>()
                .ok()
                .filter(|dest| self.places.contains_key(dest))
                .ok_or_else(|| format!("`{text}` is not in the doc"))?;
            if let Action::ScrollTo { .. } = action {
                let board = self.path(node).and_then(|path| path.get(1).copied());
                let inside = self
                    .path(dest)
                    .is_some_and(|path| path.len() > 2 && Some(path[1]) == board);
                if !inside {
                    return Err(format!(
                        "`{text}` is not a layer of this board: scrollTo takes a layer of the board the trigger is on"
                    ));
                }
            } else if !self.is_board(dest) {
                return Err(format!(
                    "`{text}` is not a board: navigate, openOverlay and swapOverlay take a board of this design"
                ));
            }
        }
        Ok(())
    }

    fn check_change_to(&self, node: NodeId, text: &str, variants: &Variants) -> Result<(), String> {
        let component = match self.node(node).map(Node::data) {
            Some(NodeData::Instance(instance)) => instance.component,
            _ => return Err("changeTo is for instances and the layers of a component".to_string()),
        };
        let Some((name, set)) = variants(component) else {
            return Err(format!(
                "`{component}` is not a component of the design system, but the instance places it"
            ));
        };
        if text.parse::<NodeId>().is_ok_and(|dest| set.contains(&dest)) {
            Ok(())
        } else {
            Err(format!("`{text}` is not a variant of `{name}`"))
        }
    }

    /// Checks the interactions of `id` and of the nodes below it.
    fn check_tree(&self, index: usize, id: NodeId, variants: &Variants) -> Result<(), ApplyError> {
        for node in self.subtree(id) {
            let Some(found) = self.node(node) else {
                continue;
            };
            if found.interactions().is_empty() {
                continue;
            }
            self.check_interactions(node, found.interactions(), variants)
                .map_err(|message| invalid(index, message))?;
        }
        Ok(())
    }

    fn rev(&self, id: NodeId) -> u64 {
        self.revisions.get(&id).copied().unwrap_or(1)
    }

    // --- Reading the tree ---

    /// The IDs from the page down to the node, the node last.
    fn path(&self, id: NodeId) -> Option<Vec<NodeId>> {
        let mut path = vec![id];
        let mut at = self.places.get(&id)?.parent;
        while let Some(parent) = at {
            path.push(parent);
            at = self.places.get(&parent)?.parent;
        }
        path.reverse();
        Some(path)
    }

    fn node(&self, id: NodeId) -> Option<&Node> {
        let path = self.path(id)?;
        let (first, rest) = path.split_first()?;
        let mut node = self.pages.iter().find(|page| page.id() == first)?;
        for next in rest {
            node = node.children().iter().find(|child| child.id() == next)?;
        }
        Some(node)
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let path = self.path(id)?;
        let (first, rest) = path.split_first()?;
        let mut node = self.pages.iter_mut().find(|page| page.id() == first)?;
        for next in rest {
            node = node
                .children_mut()?
                .iter_mut()
                .find(|child| child.id() == next)?;
        }
        Some(node)
    }

    fn siblings(&self, parent: Option<NodeId>) -> &[Node] {
        match parent {
            None => &self.pages,
            Some(id) => self.node(id).map_or(&[], Node::children),
        }
    }

    fn siblings_mut(&mut self, parent: Option<NodeId>) -> &mut Vec<Node> {
        match parent {
            None => &mut self.pages,
            Some(id) => self
                .node_mut(id)
                .and_then(Node::children_mut)
                .expect("the parent is in the design and has children"),
        }
    }

    /// The host that a tag inside `parent` is read against.
    fn host_of(&self, parent: Option<NodeId>) -> Option<Host> {
        parent.and_then(|id| self.node(id)).map(Host::of)
    }

    /// A node and its descendants, in document order.
    fn subtree(&self, id: NodeId) -> Vec<NodeId> {
        let mut ids = Vec::new();
        if let Some(node) = self.node(id) {
            collect(node, &mut ids);
        }
        ids
    }

    /// The node and its descendants as tags, with their revisions.
    fn tags(&self, id: NodeId) -> String {
        let revision = |id: &NodeId| self.revisions.get(id).copied();
        self.node(id)
            .map(|node| tags::print_with(node, Some(&revision)))
            .unwrap_or_default()
    }

    // --- Checking operations ---

    fn lookup(&self, index: usize, id: NodeId, role: &str) -> Result<Place, ApplyError> {
        self.places
            .get(&id)
            .copied()
            .ok_or_else(|| invalid(index, format!("{role} `{id}` is not in the doc")))
    }

    /// The node an operation changes, which must be at revision `rev`.
    fn expect_rev(&self, index: usize, id: NodeId, rev: u64) -> Result<Place, ApplyError> {
        let place = self.lookup(index, id, "the node")?;
        let current = self.rev(id);
        if current != rev {
            return Err(ApplyError::Conflict(Conflict {
                index,
                node: id,
                expected: rev,
                current_revision: current,
                current_html: self.tags(id),
            }));
        }
        Ok(place)
    }

    /// Checks that `parent` and the child `after` that a position names are in
    /// the design.
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
            let place = self.lookup(index, after, "the node to go after")?;
            if place.parent != parent {
                let held = parent.map_or("the top level".to_string(), |id| format!("`{id}`"));
                return Err(invalid(
                    index,
                    format!("`{after}` is not a child of {held}, so a node cannot go after it"),
                ));
            }
        }
        Ok(())
    }

    /// Where among the children of `parent` a node goes that goes right after
    /// `after` (first when `None`).
    fn position(&self, parent: Option<NodeId>, after: Option<NodeId>) -> usize {
        after.map_or(0, |after| {
            self.siblings(parent)
                .iter()
                .position(|node| *node.id() == after)
                .expect("after is a child")
                + 1
        })
    }

    /// Keeps a batch inside one board: each node that the operation names must
    /// be the board or inside it.
    fn check_scope(&self, index: usize, op: &Operation, board: NodeId) -> Result<(), ApplyError> {
        let named: Vec<Option<NodeId>> = match op {
            Operation::Insert { parent, .. } => vec![*parent],
            Operation::Replace { node, .. }
            | Operation::Set { node, .. }
            | Operation::Delete { node, .. } => vec![Some(*node)],
            Operation::Move { node, parent, .. } => vec![Some(*node), *parent],
        };
        for named in named {
            let inside = match named {
                None => false,
                // The operation reports a node that is not in the design.
                Some(id) => self.path(id).is_none_or(|path| path.contains(&board)),
            };
            if !inside {
                let what = named.map_or("the top level".to_string(), |id| format!("`{id}`"));
                return Err(invalid(
                    index,
                    format!(
                        "{what} is not in the board `{board}`: this batch may change only that board"
                    ),
                ));
            }
        }
        Ok(())
    }

    // --- Applying operations ---

    /// Applies a batch all or nothing. `new_ids` supplies the ID of each new
    /// node; `scope` keeps the batch inside one board; `check` examines the
    /// new subtree of an insert or a replace, and an instance that a set
    /// changes (what it places is the caller's to know); `variants` names the
    /// variant set of a component, for the `changeTo` of an interaction.
    pub fn apply(
        &mut self,
        ops: &[Operation],
        scope: Option<NodeId>,
        new_ids: &mut dyn FnMut() -> NodeId,
        check: &dyn Fn(&[Node]) -> Result<(), String>,
        variants: &Variants<'_>,
    ) -> Result<Applied, ApplyError> {
        // The batch runs on a copy, which replaces the design only when every
        // operation applied.
        let mut next = self.clone();
        let mut results = Vec::with_capacity(ops.len());
        for (index, op) in ops.iter().enumerate() {
            if let Some(board) = scope {
                next.check_scope(index, op, board)?;
            }
            results.push(next.apply_one(index, op, new_ids, check, variants)?);
        }
        *self = next;
        Ok(Applied { ops: results })
    }

    fn apply_one(
        &mut self,
        index: usize,
        op: &Operation,
        new_ids: &mut dyn FnMut() -> NodeId,
        check: &dyn Fn(&[Node]) -> Result<(), String>,
        variants: &Variants<'_>,
    ) -> Result<OpResult, ApplyError> {
        match op {
            Operation::Insert {
                parent,
                position,
                content,
            } => self.insert(
                index,
                *parent,
                position.after,
                content,
                new_ids,
                check,
                variants,
            ),
            Operation::Replace { node, rev, content } => {
                self.replace(index, *node, *rev, content, new_ids, check, variants)
            }
            Operation::Set {
                node,
                rev,
                properties,
            } => self.set(index, *node, *rev, properties, check, variants),
            Operation::Move {
                node,
                rev,
                parent,
                position,
            } => self.move_node(index, *node, *rev, *parent, position.after),
            Operation::Delete { node, rev } => self.delete(index, *node, *rev),
        }
    }

    /// Reads the tag of an insert or a replace that goes inside `parent`.
    fn read_tag(
        &self,
        index: usize,
        parent: Option<NodeId>,
        content: &str,
    ) -> Result<TagNode, ApplyError> {
        let host = self.host_of(parent);
        let tag = tags::parse_in(content, host.as_ref())
            .map_err(|error| invalid(index, error.to_string()))?;
        if parent.is_none() && tag.kind() != NodeKind::Page {
            return Err(invalid(
                index,
                format!(
                    "only a page goes at the top of a design, but this is a {} named `{}`",
                    tag.kind(),
                    tag.name()
                ),
            ));
        }
        Ok(tag)
    }

    #[allow(clippy::too_many_arguments)]
    fn insert(
        &mut self,
        index: usize,
        parent: Option<NodeId>,
        after: Option<NodeId>,
        content: &str,
        new_ids: &mut dyn FnMut() -> NodeId,
        check: &dyn Fn(&[Node]) -> Result<(), String>,
        variants: &Variants<'_>,
    ) -> Result<OpResult, ApplyError> {
        self.check_place(index, parent, after)?;
        let tag = self.read_tag(index, parent, content)?;
        if let Some(id) = tag.ids().first() {
            return Err(invalid(
                index,
                format!(
                    "inserted content must not carry `id` (`{id}`): the server assigns the IDs of new nodes"
                ),
            ));
        }
        let mut created = Vec::new();
        let node = tag
            .try_assign_ids(|| {
                let id = new_ids();
                created.push(id);
                id
            })
            .map_err(|message| invalid(index, message))?;
        if let Some(taken) = created.iter().find(|id| self.places.contains_key(id)) {
            return Err(invalid(
                index,
                format!("`{taken}` is already a node of the doc"),
            ));
        }
        check(std::slice::from_ref(&node)).map_err(|message| invalid(index, message))?;
        check_flows(&node).map_err(|message| invalid(index, message))?;

        let at = self.position(parent, after);
        index_subtree(&node, parent, &mut self.places);
        for id in &created {
            self.revisions.insert(*id, 1);
        }
        self.siblings_mut(parent).insert(at, node);
        self.check_tree(index, created[0], variants)?;
        Ok(OpResult {
            node: created[0],
            revision: 1,
            created,
            removed: Vec::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn replace(
        &mut self,
        index: usize,
        id: NodeId,
        rev: u64,
        content: &str,
        new_ids: &mut dyn FnMut() -> NodeId,
        check: &dyn Fn(&[Node]) -> Result<(), String>,
        variants: &Variants<'_>,
    ) -> Result<OpResult, ApplyError> {
        let place = self.expect_rev(index, id, rev)?;
        let parsed = self.read_tag(index, place.parent, content)?;
        if let Some(root) = parsed.id() {
            if *root != id {
                return Err(invalid(
                    index,
                    format!(
                        "the content's `id` `{root}` is not the replaced node `{id}`: replace keeps the node's ID"
                    ),
                ));
            }
        }
        let old_order = self.subtree(id);
        let own: HashSet<NodeId> = old_order[1..].iter().copied().collect();
        for kept in parsed.children().iter().flat_map(TagNode::ids) {
            if !own.contains(&kept) {
                let message = if self.places.contains_key(&kept) {
                    format!(
                        "`{kept}` belongs to another node of the doc: replacing `{id}` may reuse only its own descendants' IDs"
                    )
                } else {
                    format!("`{kept}` is not in the doc: the server assigns the IDs of new nodes")
                };
                return Err(invalid(index, message));
            }
        }
        let mut created = Vec::new();
        let node = parsed
            .with_id(id)
            .try_assign_ids(|| {
                let id = new_ids();
                created.push(id);
                id
            })
            .map_err(|message| invalid(index, message))?;
        if let Some(taken) = created.iter().find(|id| self.places.contains_key(id)) {
            return Err(invalid(
                index,
                format!("`{taken}` is already a node of the doc"),
            ));
        }
        check(std::slice::from_ref(&node)).map_err(|message| invalid(index, message))?;
        check_flows(&node).map_err(|message| invalid(index, message))?;

        let before: HashMap<NodeId, u64> =
            old_order.iter().map(|id| (*id, self.rev(*id))).collect();
        let slot = self.node_mut(id).expect("the node is in the doc");
        let old = std::mem::replace(slot, node);
        let mut old_nodes = HashMap::new();
        detach(old, &mut old_nodes);

        // A kept descendant is one revision further when its own content
        // changed; the replaced node is one further always.
        let mut placed = Vec::new();
        let new = self.node(id).expect("the node is in the doc");
        place_all(new, place.parent, &mut |node, parent| {
            let node_id = *node.id();
            let revision = if node_id == id {
                before[&id] + 1
            } else {
                match old_nodes.get(&node_id) {
                    Some(was) => before[&node_id] + u64::from(!was.same_own(node)),
                    None => 1,
                }
            };
            placed.push((node_id, parent, node.kind(), revision));
        });
        for gone in &old_order {
            self.places.remove(gone);
            self.revisions.remove(gone);
        }
        let revision = before[&id] + 1;
        for (node_id, parent, kind, revision) in placed {
            self.places.insert(node_id, Place { parent, kind });
            self.revisions.insert(node_id, revision);
        }
        let removed = old_order
            .into_iter()
            .filter(|old| !self.places.contains_key(old))
            .collect();
        self.check_tree(index, id, variants)?;
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
        check: &dyn Fn(&[Node]) -> Result<(), String>,
        variants: &Variants<'_>,
    ) -> Result<OpResult, ApplyError> {
        let place = self.expect_rev(index, id, rev)?;
        if properties.is_empty() {
            return Err(invalid(index, "set needs at least one property"));
        }
        let host = self.host_of(place.parent);
        let node = self.node_mut(id).expect("the node is in the doc");
        tags::set_properties(node, host.as_ref(), properties)
            .map_err(|message| invalid(index, message))?;
        let instance = node.kind() == NodeKind::Instance
            && ["component", "props", "overrides"]
                .iter()
                .any(|name| properties.contains_key(*name));
        if instance {
            check(std::slice::from_ref(&*node)).map_err(|message| invalid(index, message))?;
        }
        if properties.contains_key("flows") {
            check_flows(node).map_err(|message| invalid(index, message))?;
        }
        if properties.contains_key("interactions") {
            self.check_tree(index, id, variants)?;
        }
        let revision = rev + 1;
        self.revisions.insert(id, revision);
        Ok(OpResult {
            node: id,
            revision,
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
        // A parent or a neighbour that is not in the design is named before a
        // stale revision is.
        self.lookup(index, id, "the node")?;
        self.check_place(index, parent, after)?;
        let place = self.expect_rev(index, id, rev)?;
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
            above = self.places[&ancestor].parent;
        }
        if parent.is_none() && place.kind != NodeKind::Page {
            return Err(invalid(
                index,
                format!(
                    "only a page goes at the top of a design, but `{id}` is a {}",
                    place.kind
                ),
            ));
        }
        // The node must fit its new parent: boards sit on pages only, and a
        // child that flows takes no position of its own.
        let host = self.host_of(parent);
        let node = self.node_mut(id).expect("the node is in the doc");
        tags::set_properties(node, host.as_ref(), &Map::new())
            .map_err(|message| invalid(index, message))?;

        let siblings = self.siblings_mut(place.parent);
        let at = siblings
            .iter()
            .position(|node| *node.id() == id)
            .expect("the node is a child of its parent");
        let node = siblings.remove(at);
        let at = self.position(parent, after);
        self.siblings_mut(parent).insert(at, node);
        if let Some(place) = self.places.get_mut(&id) {
            place.parent = parent;
        }
        let revision = rev + 1;
        self.revisions.insert(id, revision);
        Ok(OpResult {
            node: id,
            revision,
            created: Vec::new(),
            removed: Vec::new(),
        })
    }

    fn delete(&mut self, index: usize, id: NodeId, rev: u64) -> Result<OpResult, ApplyError> {
        let place = self.expect_rev(index, id, rev)?;
        let removed = self.subtree(id);
        self.siblings_mut(place.parent)
            .retain(|node| *node.id() != id);
        for gone in &removed {
            self.places.remove(gone);
            self.revisions.remove(gone);
        }
        Ok(OpResult {
            node: id,
            revision: rev,
            created: Vec::new(),
            removed,
        })
    }
}

/// The IDs of a node and its descendants, in document order.
/// Checks that every flow of a page starts at one of the page's boards. A
/// board deleted later leaves its flow as it was; only a write of `flows`
/// is checked.
fn check_flows(node: &Node) -> Result<(), String> {
    let NodeData::Page(page) = node.data() else {
        return Ok(());
    };
    match page
        .flows()
        .iter()
        .find(|flow| !page.children.iter().any(|board| board.id() == &flow.start))
    {
        Some(flow) => Err(format!(
            "`{}` is not a board of this page: a flow starts at one of its boards",
            flow.start
        )),
        None => Ok(()),
    }
}

fn collect(node: &Node, ids: &mut Vec<NodeId>) {
    ids.push(*node.id());
    for child in node.children() {
        collect(child, ids);
    }
}

/// Records where a node and its descendants sit; the node is under `parent`.
fn index_subtree(node: &Node, parent: Option<NodeId>, places: &mut HashMap<NodeId, Place>) {
    places.insert(
        *node.id(),
        Place {
            parent,
            kind: node.kind(),
        },
    );
    for child in node.children() {
        index_subtree(child, Some(*node.id()), places);
    }
}

/// Calls `each` with every node of the subtree in document order and the ID of
/// its parent; the subtree's root is under `parent`.
fn place_all(node: &Node, parent: Option<NodeId>, each: &mut dyn FnMut(&Node, Option<NodeId>)) {
    each(node, parent);
    for child in node.children() {
        place_all(child, Some(*node.id()), each);
    }
}

/// Takes a subtree apart into its nodes, each without children, by ID.
fn detach(mut node: Node, nodes: &mut HashMap<NodeId, Node>) {
    let children = node.take_children();
    nodes.insert(*node.id(), node);
    for child in children {
        detach(child, nodes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::tags;

    const DESIGN: &str = r#"<Page name="Checkout">
  <Board name="Cart" x={0} y={0} width={360} height={640} dir="col" gap={8}>
    <Frame name="Summary" dir="col" gap={4}>
      <Text name="Total" content="Summa 10 kr" fontFamily="Inter" fontSize={16}/>
      <Text name="Shipping" content="Frakt ingår" fontFamily="Inter" fontSize={14}/>
    </Frame>
    <Frame name="Actions" dir="row" gap={8}/>
  </Board>
  <Board name="Payment" x={400} y={0} width={360} height={640} dir="col"/>
</Page>"#;

    fn id(text: &str) -> NodeId {
        text.parse().expect("a node ID")
    }

    /// The design with IDs `00000001`, `00000002`, ... in document order.
    fn design() -> DesignDoc {
        let mut next = 0u32;
        let page = tags::parse(DESIGN).expect("the tags parse").assign_ids(|| {
            next += 1;
            id(&format!("{next:08x}"))
        });
        let revisions = HashMap::new();
        DesignDoc::new(vec![page], revisions)
    }

    fn fresh() -> impl FnMut() -> NodeId {
        let mut next = 0x100u32;
        move || {
            next += 1;
            id(&format!("{next:08x}"))
        }
    }

    fn run(doc: &mut DesignDoc, ops: &[Operation]) -> Result<Applied, ApplyError> {
        doc.apply(ops, None, &mut fresh(), &|_| Ok(()), &|_| None)
    }

    fn op(json: Value) -> Operation {
        serde_json::from_value(json).expect("an operation")
    }

    // IDs: 1 page, 2 Cart, 3 Summary, 4 Total, 5 Shipping, 6 Actions, 7 Payment.

    #[test]
    fn a_destination_is_checked_against_the_doc_on_a_set_of_interactions() {
        let set_to = |kind: &str, dest: &str| {
            op(serde_json::json!({
                "op": "set", "node": "00000004", "rev": 1, "properties": {
                    "interactions": [{ "actions": [{ "type": kind, "dest": dest }] }]
                }
            }))
        };
        let message = |kind: &str, dest: &str| match run(&mut design(), &[set_to(kind, dest)]) {
            Ok(_) => String::new(),
            Err(ApplyError::Invalid { message, .. }) => message,
            Err(other) => panic!("{other:?}"),
        };
        assert_eq!(message("navigate", "00000007"), "");
        assert_eq!(message("scrollTo", "00000005"), "");
        assert!(message("navigate", "00000003").contains("is not a board"));
        assert!(message("navigate", "xyz!").contains("`xyz!` is not in the doc"));
        assert!(message("scrollTo", "00000002").contains("is not a layer of this board"));
        assert!(message("scrollTo", "00000007").contains("is not a layer of this board"));
        assert!(message("changeTo", "00000005").contains("changeTo is for instances"));
    }

    #[test]
    fn an_insert_creates_nodes_at_revision_1_and_leaves_the_parent_alone() {
        let mut doc = design();
        let applied = run(
            &mut doc,
            &[op(serde_json::json!({
                "op": "insert", "parent": "00000006", "position": { "after": null },
                "content": "<Frame name=\"Row\" dir=\"row\"><Text name=\"Buy\" content=\"Köp\" fontFamily=\"Inter\" fontSize={14}/></Frame>"
            }))],
        )
        .expect("the insert applies");
        let result = &applied.ops[0];
        assert_eq!(result.created, vec![id("00000101"), id("00000102")]);
        assert_eq!(result.node, id("00000101"));
        assert_eq!(doc.revision(&id("00000101")), Some(1));
        assert_eq!(doc.revision(&id("00000006")), Some(1));
        assert_eq!(doc.parent(id("00000102")), Some(Some(id("00000101"))));
    }

    #[test]
    fn a_batch_that_fails_changes_nothing() {
        let mut doc = design();
        let before = tags::print(&doc.pages()[0]);
        let refused = run(
            &mut doc,
            &[
                op(serde_json::json!({
                    "op": "set", "node": "00000004", "rev": 1, "properties": { "content": "x" }
                })),
                op(serde_json::json!({ "op": "delete", "node": "00000007", "rev": 9 })),
            ],
        );
        assert!(matches!(
            refused,
            Err(ApplyError::Conflict(Conflict {
                index: 1,
                expected: 9,
                current_revision: 1,
                ..
            }))
        ));
        assert_eq!(tags::print(&doc.pages()[0]), before);
        assert_eq!(doc.revision(&id("00000004")), Some(1));
    }

    #[test]
    fn a_set_raises_only_the_revision_of_its_node() {
        let mut doc = design();
        run(
            &mut doc,
            &[op(serde_json::json!({
                "op": "set", "node": "00000004", "rev": 1,
                "properties": { "content": "Summa 12 kr", "fontSize": 18 }
            }))],
        )
        .expect("the set applies");
        assert_eq!(doc.revision(&id("00000004")), Some(2));
        assert_eq!(doc.revision(&id("00000003")), Some(1));
        let printed = tags::print(&doc.pages()[0]);
        assert!(printed.contains(r#"content="Summa 12 kr""#), "{printed}");
        assert!(printed.contains("fontSize={18}"), "{printed}");
    }

    #[test]
    fn a_set_names_the_attribute_the_node_does_not_take() {
        let mut doc = design();
        let refused = run(
            &mut doc,
            &[op(serde_json::json!({
                "op": "set", "node": "00000004", "rev": 1, "properties": { "colour": "red" }
            }))],
        );
        match refused {
            Err(ApplyError::Invalid { message, .. }) => assert!(message.contains("colour")),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_replace_keeps_named_descendants_and_removes_the_others() {
        let mut doc = design();
        let applied = run(
            &mut doc,
            &[op(serde_json::json!({
                "op": "replace", "node": "00000002", "rev": 1,
                "content": "<Board id=\"00000002\" name=\"Cart\" x={0} y={0} width={360} height={640} dir=\"col\" gap={8}><Frame id=\"00000003\" name=\"Summary\" dir=\"col\" gap={4}><Text id=\"00000004\" name=\"Total\" content=\"Summa 12 kr\" fontFamily=\"Inter\" fontSize={16}/><Text id=\"00000005\" name=\"Shipping\" content=\"Frakt ingår\" fontFamily=\"Inter\" fontSize={14}/></Frame><Text name=\"New\" content=\"Ny\" fontFamily=\"Inter\" fontSize={14}/></Board>"
            }))],
        )
        .expect("the replace applies");
        let result = &applied.ops[0];
        assert_eq!(result.revision, 2);
        assert_eq!(result.created, vec![id("00000101")]);
        assert_eq!(result.removed, vec![id("00000006")]);
        // Total's own content changed; Summary and Shipping are kept as they were.
        assert_eq!(doc.revision(&id("00000004")), Some(2));
        assert_eq!(doc.revision(&id("00000005")), Some(1));
        assert_eq!(doc.revision(&id("00000003")), Some(1));
        assert_eq!(doc.revision(&id("00000101")), Some(1));
        assert_eq!(doc.revision(&id("00000001")), Some(1));
        assert!(!doc.contains(id("00000006")));
    }

    #[test]
    fn a_move_raises_only_the_revision_of_the_moved_node() {
        let mut doc = design();
        run(
            &mut doc,
            &[op(serde_json::json!({
                "op": "move", "node": "00000005", "rev": 1,
                "parent": "00000006", "position": { "after": null }
            }))],
        )
        .expect("the move applies");
        assert_eq!(doc.revision(&id("00000005")), Some(2));
        assert_eq!(doc.revision(&id("00000003")), Some(1));
        assert_eq!(
            doc.revision(&id("00000006")),
            Some(1),
            "the new parent is not touched"
        );
        assert_eq!(doc.parent(id("00000005")), Some(Some(id("00000006"))));
        assert_eq!(doc.siblings(Some(id("00000003"))).len(), 1);
    }

    #[test]
    fn a_delete_lists_the_subtree_in_document_order() {
        let mut doc = design();
        let applied = run(
            &mut doc,
            &[op(
                serde_json::json!({ "op": "delete", "node": "00000003", "rev": 1 }),
            )],
        )
        .expect("the delete applies");
        assert_eq!(
            applied.ops[0].removed,
            vec![id("00000003"), id("00000004"), id("00000005")]
        );
        assert!(!doc.contains(id("00000004")));
    }

    #[test]
    fn pages_hold_only_boards_and_boards_sit_only_on_pages() {
        let mut doc = design();
        let frame_on_page = run(
            &mut doc,
            &[op(serde_json::json!({
                "op": "insert", "parent": "00000001", "position": { "after": null },
                "content": "<Frame name=\"Loose\"/>"
            }))],
        );
        assert!(matches!(frame_on_page, Err(ApplyError::Invalid { .. })));
        let board_in_frame = run(
            &mut doc,
            &[op(serde_json::json!({
                "op": "move", "node": "00000007", "rev": 1,
                "parent": "00000003", "position": { "after": null }
            }))],
        );
        assert!(matches!(board_in_frame, Err(ApplyError::Invalid { .. })));
    }

    #[test]
    fn a_scope_keeps_the_batch_inside_one_board() {
        let mut doc = design();
        let outside = doc.apply(
            &[op(serde_json::json!({
                "op": "set", "node": "00000007", "rev": 1, "properties": { "name": "Pay" }
            }))],
            Some(id("00000002")),
            &mut fresh(),
            &|_| Ok(()),
            &|_| None,
        );
        match outside {
            Err(ApplyError::Invalid { message, .. }) => assert!(message.contains("00000007")),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}
