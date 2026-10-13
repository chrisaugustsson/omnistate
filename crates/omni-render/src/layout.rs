//! Layout: the box of every node of a design, computed with Taffy by the CSS
//! rules that `docs/formats/layout.md` maps each layout property to.

mod style;
mod tree;

use crate::text::{Fonts, ShapedText};
use omni_core::design::{Node, NodeData, NodeId};
use std::collections::HashMap;
use style::Flow;
use taffy::Display;
use tree::LayoutTree;

/// Where a node sits and how large it is, in CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutBox {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Lays out `root` and every node below it, measuring text with `fonts`, and
/// returns the box of each node by its ID.
///
/// Boxes are in page coordinates: the root sits at its own `x` and `y` (0
/// when left out), so a board's box is where the board sits on its page, and
/// every node below it is offset by the positions of its ancestors.
pub fn compute(root: &Node, fonts: &Fonts) -> HashMap<NodeId, LayoutBox> {
    let mut boxes = HashMap::new();
    let roots = match root.data() {
        // A page is a canvas with no size: its box is empty at its origin,
        // and each of its boards is laid out on its own.
        NodeData::Page(page) => {
            boxes.insert(*root.id(), LayoutBox::default());
            page.children.as_slice()
        }
        _ => std::slice::from_ref(root),
    };
    let mut tree = LayoutTree::new();
    for root in roots {
        let tree_root = add(&mut tree, root, Flow::Root, fonts);
        tree.lay_out(tree_root);
        let placement = root.placement();
        let origin = [placement.x, placement.y].map(|offset| offset.unwrap_or(0.0) as f32);
        collect(&tree, tree_root, root, origin, &mut boxes);
    }
    boxes
}

/// Adds `node` and its subtree to `tree`, placed in its parent by `flow`, and
/// returns the tree node of `node`.
fn add(tree: &mut LayoutTree, node: &Node, flow: Flow, fonts: &Fonts) -> taffy::NodeId {
    let mut style = style::item(node, flow);
    let (children, text) = match node.data() {
        NodeData::Board(frame) | NodeData::Frame(frame) => {
            let children_flow = style::container(&mut style, &frame.layout);
            let children = frame
                .children
                .iter()
                .map(|child| add(tree, child, children_flow.of(child), fonts))
                .collect();
            (children, None)
        }
        NodeData::Text(text) => (Vec::new(), Some(ShapedText::new(text, fonts))),
        NodeData::Page(_) | NodeData::Image(_) | NodeData::Vector(_) | NodeData::Instance(_) => {
            (Vec::new(), None)
        }
    };
    // A hidden node takes no space: CSS `display: none`.
    if !node.visible() {
        style.display = Display::None;
    }
    tree.add(style, children, text)
}

/// Records the box of `node`, whose tree node is `tree_node`, and of every
/// node below it, in page coordinates. `[parent_x, parent_y]` is where the
/// parent's box starts.
fn collect(
    tree: &LayoutTree,
    tree_node: taffy::NodeId,
    node: &Node,
    [parent_x, parent_y]: [f32; 2],
    boxes: &mut HashMap<NodeId, LayoutBox>,
) {
    let layout = tree.layout(tree_node);
    let node_box = LayoutBox {
        x: parent_x + layout.location.x,
        y: parent_y + layout.location.y,
        width: layout.size.width,
        height: layout.size.height,
    };
    boxes.insert(*node.id(), node_box);
    for (child, &tree_child) in node.children().iter().zip(tree.children(tree_node)) {
        collect(tree, tree_child, child, [node_box.x, node_box.y], boxes);
    }
}
