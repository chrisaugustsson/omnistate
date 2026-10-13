//! Layout: the box of every node of a design, computed with Taffy by the CSS
//! rules that `docs/formats/layout.md` maps each layout property to.

use crate::text::Fonts;
use omni_core::design::{Node, NodeId};
use std::collections::HashMap;

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
    let _ = fonts;
    todo!("US-003: lay out the tree of `{}` with Taffy", root.id())
}
