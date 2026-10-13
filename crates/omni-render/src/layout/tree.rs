//! The Taffy tree of a design: one Taffy node per design node, laid out by
//! Taffy's flexbox, grid and block algorithms, with text measured by Skia's
//! paragraph layout.
//!
//! Layout owns the tree, rather than using Taffy's `TaffyTree`, to size rows
//! the way CSS does where Taffy does not (see
//! [`LayoutTree::fit_content_width`]), to keep every box at least as large as
//! its padding (see its [`LayoutPartialTree`] implementation), and to keep
//! every size it measures (see its [`CacheTree`] implementation).

use crate::text::ShapedText;
use std::iter::Copied;
use std::slice;
use taffy::prelude::TaffyMaxContent;
use taffy::{
    AvailableSpace, Baselines, Cache, CacheTree, Dimension, Display, FlexDirection, Layout,
    LayoutBlockContainer, LayoutContainingBlock, LayoutFlexboxContainer, LayoutGridContainer,
    LayoutInput, LayoutOutput, LayoutPartialTree, LengthPercentageAuto, Line, NodeId, Rect,
    RequestedAxis, ResolveOrZero, RunMode, Size, SizingMode, Style, TraversePartialTree,
    TraverseTree, compute_block_layout, compute_cached_layout, compute_flexbox_layout,
    compute_grid_layout, compute_hidden_layout, compute_leaf_layout, compute_oof_layout,
    compute_root_layout,
};

/// The Taffy tree of a design.
pub(super) struct LayoutTree {
    nodes: Vec<TreeNode>,
}

/// A node of the tree.
struct TreeNode {
    style: Style,
    children: Vec<NodeId>,
    /// The text of a text node, which sizes it.
    text: Option<ShapedText>,
    /// Where the node sits in its parent, and its size.
    layout: Layout,
    /// Taffy's cache of the node's layout.
    cache: Cache,
    /// Every size measured for the node, by the inputs it was measured with:
    /// see [`CacheTree`].
    measured: Vec<(LayoutInput, LayoutOutput)>,
    /// The max-content and min-content widths of a row's content, once they
    /// are measured: see [`LayoutTree::content_widths`].
    content_widths: Option<[f32; 2]>,
}

impl LayoutTree {
    pub(super) fn new() -> Self {
        LayoutTree { nodes: Vec::new() }
    }

    /// Adds a node with `style` and `children`, sized by `text` if it is a
    /// text node, and returns its ID.
    pub(super) fn add(
        &mut self,
        style: Style,
        children: Vec<NodeId>,
        text: Option<ShapedText>,
    ) -> NodeId {
        self.nodes.push(TreeNode {
            style,
            children,
            text,
            layout: Layout::new(),
            cache: Cache::new(),
            measured: Vec::new(),
            content_widths: None,
        });
        NodeId::from(self.nodes.len() - 1)
    }

    /// Lays out `root` and every node below it on a canvas with no size, so
    /// a size that the root's content decides is its max-content size.
    pub(super) fn lay_out(&mut self, root: NodeId) {
        compute_root_layout(self, root, Size::MAX_CONTENT);
    }

    /// Where `node` sits in its parent, and its size.
    pub(super) fn layout(&self, node: NodeId) -> &Layout {
        &self.node(node).layout
    }

    pub(super) fn children(&self, node: NodeId) -> &[NodeId] {
        &self.node(node).children
    }

    fn node(&self, node: NodeId) -> &TreeNode {
        &self.nodes[usize::from(node)]
    }

    fn node_mut(&mut self, node: NodeId) -> &mut TreeNode {
        &mut self.nodes[usize::from(node)]
    }

    /// The width of a row flex container whose width the space available
    /// decides, such as a row that hugs its content in a column that aligns
    /// it to the start. Taffy makes a single-line row as wide as its items'
    /// flex bases add up to, even when they could shrink to fit the space or
    /// grow beyond their bases, and a row that wraps as wide as its widest
    /// line, even when an item on it is wider than the space. CSS gives the
    /// row its fit-content width: its max-content width, but no wider than
    /// the space available unless its min-content width is wider still, and
    /// then bounds it by the row's own min and max widths. The bounds apply
    /// even where Taffy sizes the row's content alone, as it does to find
    /// how tall the row is in a column, because the row's items wrap at the
    /// bounded width.
    ///
    /// Returns `None` for every other node, and for a row whose width is
    /// already known.
    fn fit_content_width(&mut self, node: NodeId, inputs: &LayoutInput) -> Option<f32> {
        let AvailableSpace::Definite(available) = inputs.available_space.width else {
            return None;
        };
        let TreeNode {
            style, children, ..
        } = self.node(node);
        let row = style.display == Display::Flex
            && matches!(
                style.flex_direction,
                FlexDirection::Row | FlexDirection::RowReverse
            )
            && !children.is_empty();
        let sized = inputs.known_dimensions.width.is_some()
            || (inputs.sizing_mode == SizingMode::InherentSize && !style.size.width.is_auto());
        if !row || sized {
            return None;
        }
        let [max_content, min_content] = self.content_widths(node);
        let width = max_content.min(min_content.max(available));
        let style = &self.node(node).style;
        let [min, max] = [style.min_size.width, style.max_size.width]
            .map(|bound| bound.resolve_to_option(0.0, |_, _| 0.0));
        // A min beats a max, as in CSS.
        let width = max.map_or(width, |max| width.min(max));
        Some(min.map_or(width, |min| width.max(min)))
    }

    /// The max-content and min-content widths of the content of the row
    /// `node`, which CSS measures without the row's own min and max widths.
    /// Taffy wraps the items of a row at its max width while it measures
    /// them, so the row's bounds are left out of its style meanwhile, and
    /// the widths are kept on the node instead of in its cache, where they
    /// would stand in for sizes measured with the bounds.
    ///
    /// Designs have no percentages and no aspect ratios, so neither the size
    /// of the parent nor the height of a row changes these widths, and each
    /// is measured once per layout, however many sizes the row is measured
    /// at.
    fn content_widths(&mut self, node: NodeId) -> [f32; 2] {
        if let Some(widths) = self.node(node).content_widths {
            return widths;
        }
        let style = &mut self.node_mut(node).style;
        let bounds = [style.min_size.width, style.max_size.width];
        style.min_size.width = LengthPercentageAuto::auto();
        style.max_size.width = LengthPercentageAuto::auto();
        let widths = [AvailableSpace::MaxContent, AvailableSpace::MinContent].map(|width| {
            let inputs = LayoutInput {
                run_mode: RunMode::ComputeSize,
                sizing_mode: SizingMode::ContentSize,
                axis: RequestedAxis::Horizontal,
                known_dimensions: Size::NONE,
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size: Size::NONE,
                available_space: Size {
                    width,
                    height: AvailableSpace::MaxContent,
                },
                vertical_margins_are_collapsible: Line::FALSE,
            };
            self.compute_uncached(node, inputs).size.width
        });
        let node = self.node_mut(node);
        [node.style.min_size.width, node.style.max_size.width] = bounds;
        node.content_widths = Some(widths);
        widths
    }

    /// Lays out `node` by its CSS `display`, without the cache.
    fn compute_uncached(&mut self, node: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let TreeNode {
            style, children, ..
        } = self.node(node);
        match (style.display, children.is_empty()) {
            (Display::None, _) => compute_hidden_layout(self, node),
            (_, true) => {
                let TreeNode { style, text, .. } = self.node(node);
                match text {
                    Some(text) => measure_text(inputs, style, text),
                    // An empty frame, an image, a vector or an instance has no
                    // content to size it.
                    None => compute_leaf_layout(inputs, style, |_, _| 0.0, |_, _| Size::ZERO),
                }
            }
            (Display::Block | Display::FlowRoot, false) => {
                compute_block_layout(self, node, inputs, None)
            }
            (Display::Flex, false) => compute_flexbox_layout(self, node, inputs),
            (Display::Grid, false) => compute_grid_layout(self, node, inputs),
        }
    }
}

/// Sizes a text node. A width that the text's own size or its parent's layout
/// sets is the width its lines break at; otherwise the text takes the CSS
/// fit-content width of the space available, bounded by its min and max
/// widths. Its height is as many lines as its content breaks into at that
/// width.
fn measure_text(inputs: LayoutInput, style: &Style, text: &ShapedText) -> LayoutOutput {
    // Sizing the content alone ignores the text's own size and its bounds.
    let inherent = inputs.sizing_mode == SizingMode::InherentSize;
    let styled = |size: Dimension| size.into_option().filter(|_| inherent);
    let clamp = |size: f32, min: LengthPercentageAuto, max: LengthPercentageAuto, bounded: bool| {
        let bound = |bound: LengthPercentageAuto| {
            bound.resolve_to_option(0.0, |_, _| 0.0).filter(|_| bounded)
        };
        // A min beats a max, as in CSS.
        let size = bound(max).map_or(size, |max| size.min(max));
        bound(min).map_or(size, |min| size.max(min))
    };
    let width = inputs.known_dimensions.width.unwrap_or_else(|| {
        let width =
            styled(style.size.width).unwrap_or_else(|| match inputs.available_space.width {
                AvailableSpace::Definite(available) => available
                    .max(text.min_content_width())
                    .min(text.max_content_width()),
                AvailableSpace::MinContent => text.min_content_width(),
                AvailableSpace::MaxContent => text.max_content_width(),
            });
        // CSS bounds a fit-content width before it breaks the lines there, so
        // the bounds apply even where Taffy sizes the content alone in the
        // space available, as it sizes an absolute text, whose max width
        // only narrows that space.
        let fit_content = matches!(inputs.available_space.width, AvailableSpace::Definite(_));
        clamp(
            width,
            style.min_size.width,
            style.max_size.width,
            inherent || fit_content,
        )
    });
    let height = inputs.known_dimensions.height.unwrap_or_else(|| {
        let height = styled(style.size.height).unwrap_or_else(|| text.height(width));
        clamp(
            height,
            style.min_size.height,
            style.max_size.height,
            inherent,
        )
    });
    LayoutOutput::from_sizes_and_baselines(
        Size { width, height },
        Rect::ZERO,
        Baselines::from_first(text.first_baseline()),
    )
}

impl TraversePartialTree for LayoutTree {
    type ChildIter<'a> = Copied<slice::Iter<'a, NodeId>>;

    fn child_ids(&self, node: NodeId) -> Self::ChildIter<'_> {
        self.node(node).children.iter().copied()
    }

    fn child_count(&self, node: NodeId) -> usize {
        self.node(node).children.len()
    }

    fn get_child_id(&self, node: NodeId, index: usize) -> NodeId {
        self.node(node).children[index]
    }
}

impl TraverseTree for LayoutTree {}

impl LayoutPartialTree for LayoutTree {
    type CoreContainerStyle<'a> = &'a Style;
    type CustomIdent = String;

    fn get_core_container_style(&self, node: NodeId) -> &Style {
        &self.node(node).style
    }

    fn set_unrounded_layout(&mut self, node: NodeId, layout: &Layout) {
        self.node_mut(node).layout = *layout;
    }

    fn compute_child_layout(&mut self, node: NodeId, inputs: LayoutInput) -> LayoutOutput {
        // A node below a hidden node is hidden too, whatever its own display.
        if inputs.run_mode == RunMode::PerformHiddenLayout {
            return compute_hidden_layout(self, node);
        }
        // CSS never makes a box smaller than its padding, even where its
        // parent's layout makes it smaller, as a column narrower than the
        // padding does when it stretches a child. Taffy lets such a size
        // through.
        let padding = self
            .node(node)
            .style
            .padding
            .resolve_or_zero(None, |_, _| 0.0)
            .sum_axes();
        let inputs = LayoutInput {
            known_dimensions: inputs.known_dimensions.zip_map(padding, |known, padding| {
                known.map(|known| known.max(padding))
            }),
            ..inputs
        };
        compute_cached_layout(self, node, inputs, |tree, node, inputs| {
            if let Some(width) = tree.fit_content_width(node, &inputs) {
                let known_dimensions = Size {
                    width: Some(width),
                    ..inputs.known_dimensions
                };
                return tree.compute_child_layout(
                    node,
                    LayoutInput {
                        known_dimensions,
                        ..inputs
                    },
                );
            }
            let mut output = tree.compute_uncached(node, inputs);
            if inputs.run_mode == RunMode::PerformLayout {
                compute_oof_layout(tree, node, &mut output);
            }
            output
        })
    }
}

/// Taffy's cache keeps 9 sizes per node. Nested layouts measure a node at more
/// sizes than that, so with Taffy's cache alone they measure the same sizes
/// again and again, at a cost that grows with every level of nesting. The tree
/// lives for one layout, so it also keeps every size it measures.
impl CacheTree for LayoutTree {
    fn cache_get(&mut self, node: NodeId, inputs: &LayoutInput) -> Option<LayoutOutput> {
        let node = self.node_mut(node);
        if inputs.run_mode == RunMode::ComputeSize {
            let measured = node
                .measured
                .iter()
                .find(|(measured, _)| measured == inputs);
            if let Some((_, output)) = measured {
                return Some(output.clone());
            }
        }
        node.cache.get(inputs)
    }

    fn cache_store(&mut self, node: NodeId, inputs: &LayoutInput, output: LayoutOutput) {
        let node = self.node_mut(node);
        if inputs.run_mode == RunMode::ComputeSize {
            node.measured.push((*inputs, output.clone()));
        }
        node.cache.store(inputs, output);
    }

    fn cache_clear(&mut self, node: NodeId) {
        let node = self.node_mut(node);
        node.cache.clear();
        node.measured.clear();
        node.content_widths = None;
    }
}

impl LayoutContainingBlock for LayoutTree {
    type OofItemStyle<'a> = &'a Style;

    fn get_oof_item_style(&self, node: NodeId) -> &Style {
        &self.node(node).style
    }

    // Every positioned node's box is read through its parent, which is always
    // its containing block, so the tree keeps no lists of them.
    fn clear_hoisted_children(&mut self, _node: NodeId) {}

    fn add_hoisted_children(&mut self, _node: NodeId, _hoisted: &[NodeId]) {}
}

impl LayoutFlexboxContainer for LayoutTree {
    type FlexboxContainerStyle<'a> = &'a Style;
    type FlexboxItemStyle<'a> = &'a Style;

    fn get_flexbox_container_style(&self, node: NodeId) -> &Style {
        &self.node(node).style
    }

    fn get_flexbox_child_style(&self, child: NodeId) -> &Style {
        &self.node(child).style
    }
}

impl LayoutGridContainer for LayoutTree {
    type GridContainerStyle<'a> = &'a Style;
    type GridItemStyle<'a> = &'a Style;

    fn get_grid_container_style(&self, node: NodeId) -> &Style {
        &self.node(node).style
    }

    fn get_grid_child_style(&self, child: NodeId) -> &Style {
        &self.node(child).style
    }
}

impl LayoutBlockContainer for LayoutTree {
    type BlockContainerStyle<'a> = &'a Style;
    type BlockItemStyle<'a> = &'a Style;

    fn get_block_container_style(&self, node: NodeId) -> &Style {
        &self.node(node).style
    }

    fn get_block_child_style(&self, child: NodeId) -> &Style {
        &self.node(child).style
    }
}
