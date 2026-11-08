//! Which layers of one board are the same layer on another, for smart animate.

use omni_core::design::{Node, NodeId};

/// The layers of `from` that `to` has again: a layer pairs with the first
/// layer of the same name under the layer its parent paired with, each layer
/// pairing once, so that two siblings of one name pair first with first. The
/// boards themselves are not paired and their names are not part of the
/// match: the chain starts at their children. The pairs come in the document
/// order of `from`: a parent before its children, siblings in order.
pub fn match_layers(from: &Node, to: &Node) -> Vec<(NodeId, NodeId)> {
    let mut pairs = Vec::new();
    pair_children(from, to, &mut pairs);
    pairs
}

fn pair_children(from: &Node, to: &Node, pairs: &mut Vec<(NodeId, NodeId)>) {
    let mut taken = vec![false; to.children().len()];
    for old in from.children() {
        let found = to
            .children()
            .iter()
            .enumerate()
            .find(|(at, new)| !taken[*at] && new.name() == old.name());
        if let Some((at, new)) = found {
            taken[at] = true;
            pairs.push((*old.id(), *new.id()));
            pair_children(old, new, pairs);
        }
    }
}
