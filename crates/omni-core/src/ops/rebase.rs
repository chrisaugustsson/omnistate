//! The three-way merge behind a rebase (US-026): the changes of one doc that a
//! change set made from a base version, replayed onto the mainline version the
//! doc has now. The merge works node by node, as a comparison of versions does:
//! a node is changed when it is removed, when its own content differs, or when
//! its place differs (another parent, or a reorder among the siblings that keep
//! theirs). A node that both sides changed differently is a conflict and keeps
//! its mainline state; everything else of the change set is replayed with the
//! node IDs it has. Additions and moves are replayed before removals, so that a
//! container the change set emptied and removed is judged by what is still in
//! it, and a moved node goes after a sibling that the mainline left in place,
//! so that a reorder on each side keeps both.

use super::{EditDoc, MAX_DEPTH, Node, can_hold, longest_increasing};
use crate::design::NodeId;
use std::collections::{HashMap, HashSet};

/// What a rebase made.
pub struct Rebased {
    /// The mainline doc with the change set's changes replayed onto it.
    pub doc: EditDoc,
    /// The nodes that both sides changed and that the replay left as the
    /// mainline has them: those of the base in its document order, then the
    /// nodes the change set added that found no place.
    pub conflicts: Vec<NodeId>,
    /// The nodes the change set added that found no place: the tail of
    /// `conflicts`, which the mainline never had a state of.
    pub unplaced: Vec<NodeId>,
}

/// The nodes of a doc in document order with what a comparison reads of each.
struct Shape {
    order: Vec<NodeId>,
    parent: HashMap<NodeId, Option<NodeId>>,
    content: HashMap<NodeId, String>,
}

impl Shape {
    fn of(doc: &EditDoc) -> Shape {
        let nodes = doc.index();
        Shape {
            order: nodes.iter().map(|n| n.id).collect(),
            parent: nodes.iter().map(|n| (n.id, n.parent)).collect(),
            content: nodes.into_iter().map(|n| (n.id, n.content)).collect(),
        }
    }

    /// The nearest sibling before the node that is one of `among`; `None` when
    /// it is first among those.
    fn follows(&self, id: &NodeId, among: &HashSet<NodeId>) -> Option<NodeId> {
        let parent = self.parent[id];
        self.order
            .iter()
            .filter(|sibling| self.parent[*sibling] == parent)
            .take_while(|sibling| *sibling != id)
            .filter(|sibling| among.contains(*sibling))
            .last()
            .copied()
    }
}

/// What one side changed of the base's nodes.
#[derive(Default)]
struct Changes {
    removed: HashSet<NodeId>,
    content: HashSet<NodeId>,
    place: HashSet<NodeId>,
}

impl Changes {
    fn of(base: &Shape, side: &Shape) -> Changes {
        let mut changes = Changes::default();
        for id in &base.order {
            match side.parent.get(id) {
                None => {
                    changes.removed.insert(*id);
                }
                Some(parent) => {
                    if side.content[id] != base.content[id] {
                        changes.content.insert(*id);
                    }
                    if *parent != base.parent[id] {
                        changes.place.insert(*id);
                    }
                }
            }
        }
        changes.place.extend(reordered(base, side));
        changes
    }

    fn touches(&self, id: &NodeId) -> bool {
        self.removed.contains(id) || self.content.contains(id) || self.place.contains(id)
    }
}

/// The nodes that keep their parent but change their order among the siblings
/// that also keep it: for each parent, those outside a longest common
/// subsequence of the two orders, the fewest nodes that account for the
/// reorder.
fn reordered(base: &Shape, side: &Shape) -> HashSet<NodeId> {
    let position: HashMap<NodeId, usize> = base
        .order
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();
    // The siblings that keep their parent, in the side's order, each with its
    // position in the base.
    let mut kept: HashMap<Option<NodeId>, Vec<NodeId>> = HashMap::new();
    for id in &side.order {
        if base.parent.get(id) == Some(&side.parent[id]) {
            kept.entry(side.parent[id]).or_default().push(*id);
        }
    }
    let mut out = HashSet::new();
    for siblings in kept.values() {
        let stays = longest_increasing(siblings.iter().map(|id| position[id]).collect());
        out.extend(
            siblings
                .iter()
                .zip(stays)
                .filter(|&(_, stays)| !stays)
                .map(|(id, _)| *id),
        );
    }
    out
}

impl EditDoc {
    /// The nodes that `side` changed from `base`, as a comparison of versions
    /// sees it: first those of the base that are removed, edited or placed
    /// elsewhere, in the base's document order, then the nodes added, in the
    /// side's. Empty when the two are the same doc.
    pub fn changed_from(base: &EditDoc, side: &EditDoc) -> Vec<NodeId> {
        let (b, s) = (Shape::of(base), Shape::of(side));
        let changes = Changes::of(&b, &s);
        let kept = b.order.iter().filter(|id| changes.touches(id));
        let added = s.order.iter().filter(|id| !b.parent.contains_key(id));
        kept.chain(added).copied().collect()
    }

    /// Replays the changes that `changes` made from `base` onto `mainline`.
    /// All three hold the same doc at different versions: `base` is where the
    /// change set started, `changes` is the change set's latest version and
    /// `mainline` is the mainline version now.
    pub fn rebase(base: &EditDoc, mainline: &EditDoc, changes: &EditDoc) -> Rebased {
        let (b, m, t) = (Shape::of(base), Shape::of(mainline), Shape::of(changes));
        let mine = Changes::of(&b, &t);
        let theirs = Changes::of(&b, &m);

        // Both sides changed the node, unless they came to the same state. A
        // node that both sides placed differently, under the same parent, is
        // in the same state only when it follows the same sibling.
        let kept: HashSet<NodeId> = b
            .order
            .iter()
            .filter(|id| t.parent.contains_key(id) && m.parent.contains_key(id))
            .copied()
            .collect();
        let mut conflicts: HashSet<NodeId> = b
            .order
            .iter()
            .filter(|id| mine.touches(id) && theirs.touches(id))
            .filter(|id| match (t.parent.get(id), m.parent.get(id)) {
                (None, None) => false,
                (Some(mine_parent), Some(theirs_parent)) => {
                    mine_parent != theirs_parent
                        || t.content[id] != m.content[id]
                        || (mine.place.contains(id)
                            && theirs.place.contains(id)
                            && t.follows(id, &kept) != m.follows(id, &kept))
                }
                _ => true,
            })
            .copied()
            .collect();

        let mut doc = mainline.clone();
        for id in &t.order {
            if mine.content.contains(id) && !conflicts.contains(id) {
                if let Some(node) = doc.nodes.get_mut(id) {
                    node.block = changes.nodes[id].block.clone();
                    node.rev += 1;
                }
            }
        }

        // Added and moved nodes, in the change set's document order, so that
        // the sibling a node follows and the parent it goes under are placed
        // before it.
        let mut unplaced = Vec::new();
        for id in &t.order {
            if !b.parent.contains_key(id) {
                if !doc.replay_added(changes, &t, *id) {
                    unplaced.push(*id);
                }
            } else if mine.place.contains(id)
                && !conflicts.contains(id)
                && doc.nodes.contains_key(id)
            {
                // A sibling that the mainline moved, or that keeps its
                // mainline place as a conflict, is not where the change set
                // put it, so the moved node does not follow it.
                let settled = |sibling: &NodeId| {
                    !theirs.place.contains(sibling)
                        && (!mine.place.contains(sibling) || !conflicts.contains(sibling))
                };
                if !doc.replay_moved(changes, &t, *id, &settled) {
                    conflicts.insert(*id);
                }
            }
        }
        doc.remove_removed(&m, &mine, &mut conflicts);

        let mut listed: Vec<NodeId> = b
            .order
            .iter()
            .filter(|id| conflicts.contains(id))
            .copied()
            .collect();
        listed.extend(unplaced.iter().copied());
        Rebased {
            doc,
            conflicts: listed,
            unplaced,
        }
    }

    /// Removes the nodes that the change set removed and the mainline left
    /// alone, once the change set's additions and moves are in place. A
    /// removal takes a whole subtree, so a node stays, as a conflict, when
    /// something under it stays: a node the mainline added or moved there, or
    /// a conflict. The children that the change set moved elsewhere are gone
    /// from it by then.
    fn remove_removed(&mut self, m: &Shape, mine: &Changes, conflicts: &mut HashSet<NodeId>) {
        let mut gone: HashSet<NodeId> = mine
            .removed
            .iter()
            .filter(|id| m.parent.contains_key(id) && !conflicts.contains(id))
            .copied()
            .collect();
        let order: Vec<NodeId> = self.index().into_iter().map(|n| n.id).collect();
        // Children follow their parent in document order, so in reverse a
        // node is judged after the nodes under it.
        for id in order.iter().rev() {
            let stays = |children: &[NodeId]| children.iter().any(|child| !gone.contains(child));
            if gone.contains(id) && stays(&self.nodes[id].children) {
                gone.remove(id);
                conflicts.insert(*id);
            }
        }
        for id in &order {
            if gone.contains(id) && self.nodes.contains_key(id) {
                self.detach(*id);
                for removed in self.subtree(*id) {
                    self.nodes.remove(&removed);
                }
            }
        }
    }

    /// Puts a node the change set added into the doc: under the parent it had
    /// there or, when the mainline removed that, under the nearest ancestor
    /// that is here and can hold it, else at the top level. `false` when it
    /// finds no place.
    fn replay_added(&mut self, changes: &EditDoc, t: &Shape, id: NodeId) -> bool {
        let block = changes.nodes[&id].block.clone();
        let mut candidate = t.parent[&id];
        loop {
            if candidate.is_none_or(|parent| self.nodes.contains_key(&parent))
                && can_hold(self.kind_of(candidate), block.kind())
                && self.level(candidate) < MAX_DEPTH
            {
                break;
            }
            match candidate {
                Some(parent) => candidate = t.parent[&parent],
                None => return false,
            }
        }
        let after = self.preceding(changes, t, id, candidate, &|_| true);
        let (at, key) = self.place(candidate, after);
        self.nodes.insert(
            id,
            Node {
                block,
                parent: candidate,
                key,
                rev: 1,
                children: Vec::new(),
            },
        );
        self.siblings_mut(candidate).insert(at, id);
        true
    }

    /// Moves a node the change set moved to the place it has there, after the
    /// nearest of its preceding siblings there that is `settled`. `false` when
    /// that place is not possible here: its parent is gone, the move would put
    /// the node under itself, or the parent cannot hold it.
    fn replay_moved(
        &mut self,
        changes: &EditDoc,
        t: &Shape,
        id: NodeId,
        settled: &dyn Fn(&NodeId) -> bool,
    ) -> bool {
        let parent = t.parent[&id];
        if parent.is_some_and(|parent| !self.nodes.contains_key(&parent)) {
            return false;
        }
        let mut above = parent;
        while let Some(ancestor) = above {
            if ancestor == id {
                return false;
            }
            above = self.nodes[&ancestor].parent;
        }
        if !can_hold(self.kind_of(parent), self.nodes[&id].block.kind())
            || self.level(parent) + self.height(id) > MAX_DEPTH
        {
            return false;
        }
        self.detach(id);
        let after = self.preceding(changes, t, id, parent, settled);
        let (at, key) = self.place(parent, after);
        self.siblings_mut(parent).insert(at, id);
        let node = self.nodes.get_mut(&id).expect("the node is in the doc");
        node.parent = parent;
        node.key = key;
        node.rev += 1;
        true
    }

    /// The sibling that `id` follows in the change set's doc and that is a
    /// child of `parent` in this one: the nearest of the siblings before it
    /// that is `settled`. `None` puts the node first.
    fn preceding(
        &self,
        changes: &EditDoc,
        t: &Shape,
        id: NodeId,
        parent: Option<NodeId>,
        settled: &dyn Fn(&NodeId) -> bool,
    ) -> Option<NodeId> {
        let siblings = changes
            .children(t.parent[&id])
            .expect("the parent is in the change set's doc");
        let at = siblings.iter().position(|s| *s == id)?;
        siblings[..at].iter().rev().copied().find(|sibling| {
            settled(sibling)
                && self
                    .nodes
                    .get(sibling)
                    .is_some_and(|node| node.parent == parent)
        })
    }

    /// Takes a node out of its parent's children; the node stays in the doc.
    fn detach(&mut self, id: NodeId) {
        let parent = self.nodes[&id].parent;
        self.siblings_mut(parent).retain(|sibling| *sibling != id);
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

    /// Paragraphs `a0000000`... with the texts, at the top level.
    fn paragraphs(items: &[(&str, &str)]) -> EditDoc {
        doc(&items
            .iter()
            .map(|(id, text)| format!("<p data-node-id=\"{id}\">{text}</p>"))
            .collect::<String>())
    }

    fn order(doc: &EditDoc) -> Vec<String> {
        doc.index().iter().map(|n| n.id.to_string()).collect()
    }

    #[test]
    fn changes_to_different_nodes_are_replayed_with_their_ids() {
        let base = paragraphs(&[("a0000000", "A"), ("b0000000", "B"), ("c0000000", "C")]);
        let mainline = paragraphs(&[("a0000000", "A2"), ("b0000000", "B"), ("c0000000", "C")]);
        let changes = paragraphs(&[
            ("a0000000", "A"),
            ("b0000000", "B2"),
            ("d0000000", "D"),
            ("c0000000", "C"),
        ]);
        let rebased = EditDoc::rebase(&base, &mainline, &changes);
        assert!(rebased.conflicts.is_empty());
        assert_eq!(
            order(&rebased.doc),
            ["a0000000", "b0000000", "d0000000", "c0000000"]
        );
        let html = rebased.doc.to_html();
        assert!(html.contains("A2") && html.contains("B2") && html.contains(">D<"));
    }

    #[test]
    fn a_node_changed_on_both_sides_keeps_the_mainline_state() {
        let base = paragraphs(&[("a0000000", "A"), ("b0000000", "B"), ("c0000000", "C")]);
        // The mainline edits A and removes B; the change set edits all three.
        let mainline = paragraphs(&[("a0000000", "A mainline"), ("c0000000", "C")]);
        let changes = paragraphs(&[
            ("a0000000", "A mine"),
            ("b0000000", "B mine"),
            ("c0000000", "C mine"),
        ]);
        let rebased = EditDoc::rebase(&base, &mainline, &changes);
        assert_eq!(rebased.conflicts, [id("a0000000"), id("b0000000")]);
        assert_eq!(order(&rebased.doc), ["a0000000", "c0000000"]);
        let html = rebased.doc.to_html();
        assert!(html.contains("A mainline") && html.contains("C mine"));
        assert!(!html.contains("A mine") && !html.contains("B mine"));
    }

    #[test]
    fn both_sides_coming_to_the_same_state_is_no_conflict() {
        let base = paragraphs(&[("a0000000", "A"), ("b0000000", "B")]);
        let mainline = paragraphs(&[("a0000000", "Same")]);
        let changes = paragraphs(&[("a0000000", "Same")]);
        let rebased = EditDoc::rebase(&base, &mainline, &changes);
        assert!(rebased.conflicts.is_empty());
        assert_eq!(order(&rebased.doc), ["a0000000"]);
    }

    #[test]
    fn a_reorder_is_replayed_as_the_moves_that_account_for_it() {
        let base = paragraphs(&[
            ("a0000000", "A"),
            ("b0000000", "B"),
            ("c0000000", "C"),
            ("d0000000", "D"),
        ]);
        // The mainline edits D; the change set moves A behind C.
        let mainline = paragraphs(&[
            ("a0000000", "A"),
            ("b0000000", "B"),
            ("c0000000", "C"),
            ("d0000000", "D2"),
        ]);
        let changes = paragraphs(&[
            ("b0000000", "B"),
            ("c0000000", "C"),
            ("a0000000", "A"),
            ("d0000000", "D"),
        ]);
        let rebased = EditDoc::rebase(&base, &mainline, &changes);
        assert!(rebased.conflicts.is_empty());
        assert_eq!(
            order(&rebased.doc),
            ["b0000000", "c0000000", "a0000000", "d0000000"]
        );
        assert!(rebased.doc.to_html().contains("D2"));
    }

    #[test]
    fn changed_from_lists_what_a_side_removed_edited_moved_and_added() {
        let base = paragraphs(&[("a0000000", "A"), ("b0000000", "B"), ("c0000000", "C")]);
        assert!(EditDoc::changed_from(&base, &base).is_empty());
        let side = paragraphs(&[("c0000000", "C"), ("a0000000", "A2"), ("n0000000", "N")]);
        assert_eq!(
            EditDoc::changed_from(&base, &side),
            [
                id("a0000000"),
                id("b0000000"),
                id("c0000000"),
                id("n0000000")
            ]
        );
    }

    #[test]
    fn a_removed_section_stays_when_the_mainline_added_to_it() {
        let section = |children: &str| {
            doc(&format!(
                "<section data-node-id=\"s0000000\">{children}</section>"
            ))
        };
        let child = |id: &str| format!("<p data-node-id=\"{id}\">{id}</p>");
        let base = section(&child("a0000000"));
        let mainline = section(&format!("{}{}", child("a0000000"), child("b0000000")));
        let changes = doc("");
        let rebased = EditDoc::rebase(&base, &mainline, &changes);
        // The change set removed the section and A; the mainline added B to it.
        assert_eq!(rebased.conflicts, [id("s0000000")]);
        assert_eq!(order(&rebased.doc), ["s0000000", "b0000000"]);
    }

    #[test]
    fn a_removed_section_stays_when_the_mainline_moved_a_node_into_it() {
        let base = doc(concat!(
            "<section data-node-id=\"s0000000\"><p data-node-id=\"a0000000\">A</p></section>",
            "<p data-node-id=\"b0000000\">B</p>"
        ));
        let mainline = doc(concat!(
            "<section data-node-id=\"s0000000\"><p data-node-id=\"a0000000\">A</p>",
            "<p data-node-id=\"b0000000\">B</p></section>"
        ));
        let changes = doc("<p data-node-id=\"b0000000\">B</p>");
        let rebased = EditDoc::rebase(&base, &mainline, &changes);
        assert_eq!(rebased.conflicts, [id("s0000000")]);
        assert_eq!(order(&rebased.doc), ["s0000000", "b0000000"]);
    }

    #[test]
    fn a_node_added_under_a_section_the_mainline_removed_goes_up_a_level() {
        let section =
            doc("<section data-node-id=\"s0000000\"><p data-node-id=\"a0000000\">A</p></section>");
        let mainline = doc("");
        let changes = doc(concat!(
            "<section data-node-id=\"s0000000\"><p data-node-id=\"a0000000\">A</p>",
            "<p data-node-id=\"n0000000\">New</p></section>"
        ));
        let rebased = EditDoc::rebase(&section, &mainline, &changes);
        assert!(rebased.conflicts.is_empty());
        assert_eq!(order(&rebased.doc), ["n0000000"]);
        assert_eq!(rebased.doc.parent(id("n0000000")), Some(None));
    }
}
