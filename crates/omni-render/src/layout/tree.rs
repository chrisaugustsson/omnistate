//! The Taffy tree of a design: one Taffy node per design node, laid out by
//! Taffy's flexbox, grid and block algorithms, with text measured by Skia's
//! paragraph layout.
//!
//! Layout owns the tree, rather than using Taffy's `TaffyTree`, to size flex
//! containers and their items the way Chrome does where Taffy does not (see
//! [`LayoutTree::auto_width`], [`LayoutTree::auto_height`],
//! [`LayoutTree::flex_inputs`], [`LayoutTree::without_guessed_cross_size`] and
//! [`LayoutTree::finish_flex_lines`]), to keep every box at least as
//! large as its padding (see [`LayoutTree::compute_layout`]), and to keep
//! every size it measures (see its [`CacheTree`] implementation).

mod lines;

use crate::text::ShapedText;
use std::iter::Copied;
use std::rc::Rc;
use std::slice;
use taffy::prelude::TaffyMaxContent;
use taffy::{
    AlignItems, AvailableSpace, Baselines, Cache, CacheTree, Dimension, Display, FlexDirection,
    FlexWrap, Layout, LayoutBlockContainer, LayoutContainingBlock, LayoutFlexboxContainer,
    LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree, LengthPercentage,
    LengthPercentageAuto, Line, MaybeMath, NodeId, Position, Rect, RequestedAxis, ResolveOrZero,
    RunMode, Size, SizingMode, Style, TraversePartialTree, TraverseTree, compute_block_layout,
    compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_hidden_layout,
    compute_leaf_layout, compute_oof_layout, compute_root_layout,
};

/// How far the height that a parent hands back to a column that wraps may be
/// from the column's own height and still count as that height.
const SAME_HEIGHT: f32 = 0.01;

/// How far two sizes or places may be apart and still count as the same when
/// the items of a flex container are sorted into their lines.
const SAME_PLACE: f32 = 0.01;

/// The Taffy tree of a design.
pub(super) struct LayoutTree {
    nodes: Vec<TreeNode>,
}

/// A node of the tree.
struct TreeNode {
    style: Style,
    children: Vec<NodeId>,
    /// The text of a text node, which sizes it.
    text: Option<Rc<ShapedText>>,
    /// Where the node sits in its parent, and its size.
    layout: Layout,
    /// Taffy's cache of the node's layout.
    cache: Cache,
    /// Every size measured for the node, by the inputs it was measured with:
    /// see [`CacheTree`].
    measured: Vec<(LayoutInput, LayoutOutput)>,
    /// The max-content and min-content widths of the node's content, by the
    /// height they were measured at: see [`LayoutTree::content_widths`].
    content_widths: Vec<(Option<f32>, [f32; 2])>,
    /// The node that the node is a child of.
    parent: Option<NodeId>,
    /// The inputs that the node's layout is laying it out with, while it lays
    /// out its children: see [`LayoutTree::without_guessed_cross_size`].
    laying_out: Option<LayoutInput>,
    /// The inputs of the last layout that the node's parent placed it with:
    /// see [`LayoutTree::finish_flex_lines`].
    placed_with: Option<LayoutInput>,
    /// How far the first baseline of the node is below its top, as of that
    /// layout, where it has one.
    baseline: Option<f32>,
}

/// How a flex container places its items: in a row or a column, and on one
/// line or wrapping onto several.
#[derive(Clone, Copy)]
struct Flex {
    row: bool,
    wraps: bool,
}

impl Flex {
    /// How `node` places its items, or `None` when it is not a flex
    /// container. Taffy lays out a node without children as a leaf, whatever
    /// its display, as it does texts.
    fn of(node: &TreeNode) -> Option<Flex> {
        let style = &node.style;
        (style.display == Display::Flex && !node.children.is_empty()).then(|| Flex {
            row: matches!(
                style.flex_direction,
                FlexDirection::Row | FlexDirection::RowReverse
            ),
            wraps: style.flex_wrap != FlexWrap::NoWrap,
        })
    }
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
        text: Option<Rc<ShapedText>>,
    ) -> NodeId {
        let node = NodeId::from(self.nodes.len());
        for &child in &children {
            self.node_mut(child).parent = Some(node);
        }
        self.nodes.push(TreeNode {
            style,
            children,
            text,
            layout: Layout::new(),
            cache: Cache::new(),
            measured: Vec::new(),
            content_widths: Vec::new(),
            parent: None,
            laying_out: None,
            placed_with: None,
            baseline: None,
        });
        node
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

    /// The width of a flex container or a grid whose content decides it, as
    /// Chrome sizes it before the node lays out its items. CSS gives such a
    /// container its max-content width under a max-content constraint, its
    /// min-content width under a min-content one, and otherwise its
    /// fit-content width: the max-content width, but no wider than the space
    /// available unless the min-content width is wider still, as where a
    /// column aligns it to the start. The container's own min and max widths
    /// then bound it, also where Taffy sizes its content alone, because its
    /// items wrap at the bounded width. Only an item of a row that is measured
    /// for its flex base size is not bounded, as CSS leaves the bounds to its
    /// hypothetical size: see [`LayoutTree::is_measured_for_its_flex_base_size`].
    ///
    /// Taffy sizes these containers otherwise (see
    /// [`LayoutTree::content_widths`] for what Chrome counts). It counts the
    /// items of a row at their flex bases, so a fill item counts as nothing
    /// and a fixed one ignores its max width. It makes a column that wraps as
    /// wide as all its lines, however narrow its space. And it measures the
    /// items of a column before it knows the column's width, so a text in
    /// the column may break its lines at another width than the one it gets.
    ///
    /// Taffy gives a grid its max-content width, whatever the space available,
    /// where CSS fits it into that space like a flex container: in a grid area,
    /// on the cross axis of a column that does not stretch it or that wraps, and
    /// where it is positioned.
    ///
    /// Returns `None` for every other node, and for a container whose width
    /// is already known.
    fn auto_width(&mut self, node: NodeId, inputs: &LayoutInput) -> Option<f32> {
        if self.node(node).style.display != Display::Grid {
            Flex::of(self.node(node))?;
        }
        let style = &self.node(node).style;
        let sized = inputs.known_dimensions.width.is_some()
            || (inputs.sizing_mode == SizingMode::InherentSize && !style.size.width.is_auto());
        if sized {
            return None;
        }
        // Chrome measures the content at the container's height only where
        // that height is definite, as Taffy marks it, such as a fixed height
        // or the height that a row of fixed height stretches the container
        // to. A column that hugs its height gives the container the height of
        // its content, bounded by its max height, which is not definite, so a
        // column that the container stretches to that height keeps its
        // children on one line there.
        let definite_height = inputs
            .known_dimensions
            .height
            .filter(|_| inputs.known_dimensions_are_definite.height);
        let height = definite_height.or_else(|| {
            fixed(
                style.size.height,
                style.min_size.height,
                style.max_size.height,
            )
        });
        let [max_content, min_content] = self.content_widths(node, height);
        let width = match inputs.available_space.width {
            AvailableSpace::Definite(available) => max_content.min(min_content.max(available)),
            AvailableSpace::MaxContent => max_content,
            AvailableSpace::MinContent => min_content,
        };
        // CSS clamps an item's width by its min and max widths in its
        // hypothetical size, not in its flex base size, which is its content's
        // width however wide that is: a row that overflows shrinks its items
        // from their flex base sizes. A column's item, or any other box, is
        // bounded where its width is fit into its space.
        if self.is_measured_for_its_flex_base_size(node, inputs) {
            return Some(width);
        }
        let style = &self.node(node).style;
        Some(bounded(width, style.min_size.width, style.max_size.width))
    }

    /// Whether `inputs` measure `node`, an item of a row, for its flex base size
    /// or its automatic minimum size: its content's width under a max-content
    /// or min-content constraint, where CSS ignores its min and max widths.
    fn is_measured_for_its_flex_base_size(&self, node: NodeId, inputs: &LayoutInput) -> bool {
        let TreeNode { parent, style, .. } = self.node(node);
        let in_row =
            parent.is_some_and(|parent| Flex::of(self.node(parent)).is_some_and(|flex| flex.row));
        in_row
            && style.position != Position::Absolute
            && inputs.run_mode == RunMode::ComputeSize
            && inputs.sizing_mode == SizingMode::ContentSize
            && !matches!(inputs.available_space.width, AvailableSpace::Definite(_))
    }

    /// The max-content and min-content widths of the content of `node`,
    /// padding included, when `node` is `height` px tall (`None` when its
    /// content decides its height), as Chrome measures them. They leave out
    /// the node's own width and its min and max widths.
    ///
    /// - A row's items take their max-content contributions side by side,
    ///   with the gaps between them. At its narrowest, a row that wraps is as
    ///   wide as its widest item at its narrowest, and one that does not is
    ///   as wide as all its items at their narrowest, with the gaps.
    /// - A column is as wide as its widest item, and at its narrowest as its
    ///   widest item at its narrowest. A column that wraps takes the lines
    ///   it wraps its items into, at its height or else at its max height,
    ///   less its padding, side by side with the gaps between them: each line
    ///   is as wide as its widest item.
    /// - A text's widths are those of its widest line and its widest piece.
    /// - A grid's widths are Taffy's, also where it has no items: the tracks
    ///   that it declares still take space.
    /// - Any other node has no content that takes space.
    ///
    /// An item's contribution is its fixed width or the widths of its content,
    /// bounded by its min and max widths and never narrower than its padding.
    /// An item that a row stretches to its height is measured at that height.
    ///
    /// Designs have no percentages and no aspect ratios, so the space around
    /// a node never changes these widths, and each is measured once per
    /// layout for every height the node is measured at. Widths that Taffy
    /// measures are measured without the node's own min and max widths, and
    /// are kept on the node instead of in its cache, where they would stand
    /// in for sizes measured with the bounds.
    fn content_widths(&mut self, node: NodeId, height: Option<f32>) -> [f32; 2] {
        let measured = &self.node(node).content_widths;
        if let Some(&(_, widths)) = measured.iter().find(|(at, _)| *at == height) {
            return widths;
        }
        let style = &self.node(node).style;
        let padding = style.padding.resolve_or_zero(None, |_, _| 0.0);
        let widths = match (style.display, Flex::of(self.node(node))) {
            (_, Some(Flex { row: true, wraps })) => {
                // A row stretches its items to its height only on one line.
                let stretched_to = height
                    .filter(|_| !wraps)
                    .map(|height| (height - padding.vertical_axis_sum()).max(0.0));
                let gap = style.gap.width.resolve_or_zero(None, |_, _| 0.0);
                let items = self.items(node);
                let mut widths: Option<[f32; 2]> = None;
                for item in items {
                    let item_height = self.item_height(node, item, stretched_to);
                    let [max, min] = self.contribution(item, item_height);
                    // The items of a line add up as Taffy adds them to find
                    // where the line breaks.
                    widths = Some(match widths {
                        None => [max, min],
                        Some([line, narrowest]) if wraps => {
                            [line + (max + gap), narrowest.max(min)]
                        }
                        Some([line, narrowest]) => [line + (max + gap), narrowest + (min + gap)],
                    });
                }
                let [line, narrowest] = widths.unwrap_or_default();
                let inset = padding.horizontal_axis_sum();
                // The items of a row that wraps stay on one line at its
                // max-content width, even where the inset takes a rounding
                // error from the line.
                let mut max_content = line + inset;
                while wraps && max_content - inset < line {
                    max_content = max_content.next_up();
                }
                [max_content, narrowest + inset]
            }
            (_, Some(Flex { row: false, wraps })) => {
                let inset = padding.horizontal_axis_sum();
                let mut widest = [0.0f32; 2];
                for item in self.items(node) {
                    let item_height = self.item_height(node, item, None);
                    let [max, min] = self.contribution(item, item_height);
                    widest = [widest[0].max(max), widest[1].max(min)];
                }
                if wraps {
                    let lines = self.measure_width(node, height, AvailableSpace::MaxContent);
                    [lines, widest[1] + inset]
                } else {
                    widest.map(|width| width + inset)
                }
            }
            (Display::Grid, None) => [AvailableSpace::MaxContent, AvailableSpace::MinContent]
                .map(|width| self.measure_width(node, height, width)),
            _ => {
                let text = self.node(node).text.as_ref();
                let widths = text.map_or([0.0; 2], |text| {
                    [text.max_content_width(), text.min_content_width()]
                });
                widths.map(|width| width + padding.horizontal_axis_sum())
            }
        };
        let widths = [widths[0].max(widths[1]), widths[1]];
        self.node_mut(node).content_widths.push((height, widths));
        widths
    }

    /// The max-content and min-content contributions of `item` to the width
    /// of its parent, when it is `height` px tall: its fixed width or the
    /// widths of its content, bounded by its min and max widths, and never
    /// narrower than its padding.
    fn contribution(&mut self, item: NodeId, height: Option<f32>) -> [f32; 2] {
        let style = &self.node(item).style;
        let widths = match style.size.width.into_option() {
            Some(width) => [width; 2],
            None => self.content_widths(item, height),
        };
        let style = &self.node(item).style;
        let padding = style
            .padding
            .resolve_or_zero(None, |_, _| 0.0)
            .horizontal_axis_sum();
        widths.map(|width| bounded(width, style.min_size.width, style.max_size.width).max(padding))
    }

    /// The height of `item` of the flex container `node` while its width is
    /// measured: its fixed height, or else `stretched_to`, the inner height of
    /// a row that stretches it, either bounded by its min and max heights and
    /// never smaller than its padding. `None` when its content decides its
    /// height.
    fn item_height(&self, node: NodeId, item: NodeId, stretched_to: Option<f32>) -> Option<f32> {
        let style = &self.node(item).style;
        let height = style
            .size
            .height
            .into_option()
            .or(stretched_to.filter(|_| self.stretches(node, item, true)))?;
        let padding = style
            .padding
            .resolve_or_zero(None, |_, _| 0.0)
            .vertical_axis_sum();
        Some(bounded(height, style.min_size.height, style.max_size.height).max(padding))
    }

    /// Whether the flex container `node`, a row if `row`, stretches its child
    /// `item` across its line: whether `item` hugs its cross axis and aligns
    /// as `stretch`, the alignment that `normal` stands for in a flex layout.
    fn stretches(&self, node: NodeId, item: NodeId, row: bool) -> bool {
        let style = &self.node(item).style;
        let align = style
            .align_self
            .unwrap_or(self.node(node).style.align_items);
        let cross = if row {
            style.size.height
        } else {
            style.size.width
        };
        cross.is_auto() && (align == AlignItems::STRETCH || align == AlignItems::NORMAL)
    }

    /// `inputs` without the cross size that Taffy guesses for `node` while it
    /// measures `node` as an item of a flex container. Taffy makes an item
    /// that the container stretches as large as the space across the
    /// container while it finds the item's flex basis, as CSS does where the
    /// container keeps to one line and its cross size is definite. Elsewhere
    /// CSS fits the item's content into that space instead, as Chrome does,
    /// and the item stretches only once its line's cross size is known.
    fn without_guessed_cross_size(&self, node: NodeId, mut inputs: LayoutInput) -> LayoutInput {
        let TreeNode { parent, style, .. } = self.node(node);
        let Some(parent) = *parent else {
            return inputs;
        };
        let container = self.node(parent);
        let (Some(flex), Some(laid_out_with)) = (Flex::of(container), container.laying_out) else {
            return inputs;
        };
        let measured = inputs.run_mode == RunMode::ComputeSize;
        let positioned = style.position == Position::Absolute;
        if !measured || positioned || !self.stretches(parent, node, flex.row) {
            return inputs;
        }
        let [known, definite, styled] = if flex.row {
            [
                laid_out_with.known_dimensions.height.is_some(),
                laid_out_with.known_dimensions_are_definite.height,
                !container.style.size.height.is_auto(),
            ]
        } else {
            [
                laid_out_with.known_dimensions.width.is_some(),
                laid_out_with.known_dimensions_are_definite.width,
                !container.style.size.width.is_auto(),
            ]
        };
        let inherent = laid_out_with.sizing_mode == SizingMode::InherentSize;
        if flex.wraps || !(known && definite || inherent && styled) {
            if flex.row {
                inputs.known_dimensions.height = None;
            } else {
                inputs.known_dimensions.width = None;
            }
        }
        inputs
    }

    /// The children of `node` that its layout places: neither hidden nor
    /// positioned.
    fn items(&self, node: NodeId) -> Vec<NodeId> {
        let children = self.node(node).children.iter().copied();
        children
            .filter(|&child| {
                let style = &self.node(child).style;
                style.display != Display::None && style.position != Position::Absolute
            })
            .collect()
    }

    /// The width Taffy gives `node` when it is `height` px tall and its width
    /// is sized under the constraint `width`, without the node's own min and
    /// max widths.
    fn measure_width(&mut self, node: NodeId, height: Option<f32>, width: AvailableSpace) -> f32 {
        let style = &mut self.node_mut(node).style;
        let bounds = [style.min_size.width, style.max_size.width];
        style.min_size.width = LengthPercentageAuto::auto();
        style.max_size.width = LengthPercentageAuto::auto();
        let inputs = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::ContentSize,
            axis: RequestedAxis::Horizontal,
            known_dimensions: Size {
                width: None,
                height,
            },
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: Size::NONE,
            available_space: Size {
                width,
                height: height.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
            },
            vertical_margins_are_collapsible: Line::FALSE,
        };
        // Where its content decides its height, a column that wraps at its max
        // height breaks its lines inside its padding.
        let padded = self
            .padded_wrapping_column(node)
            .filter(|_| height.is_none());
        let measured = match padded {
            Some(padding) => self
                .inside_vertical_padding(node, padding, |tree| tree.compute_uncached(node, inputs)),
            None => self.compute_uncached(node, inputs),
        };
        let style = &mut self.node_mut(node).style;
        [style.min_size.width, style.max_size.width] = bounds;
        measured.size.width
    }

    /// The inputs that Taffy lays out the flex container `node` with, in place
    /// of `inputs`, so that it sizes the container as Chrome does:
    ///
    /// - A row's lines break at the width it is given, whatever decided that
    ///   width. Taffy keeps the items of a row whose width its parent took
    ///   from the row's content on one line.
    /// - A column whose content decides its height is as tall as its content,
    ///   whatever the space available and whatever constraint it is sized
    ///   under. In definite space, Taffy makes such a column as tall as the
    ///   space when it wraps onto several lines, and counts a fill item in it
    ///   at its flex basis, nothing. Under a min-content constraint, it puts
    ///   each item of a column that wraps on a line of its own.
    /// - A column that wraps at its max height keeps the lines it wraps into
    ///   when its parent hands that height back to it. Taffy would put all
    ///   its items on one line. A column that its parent makes taller or
    ///   shorter than its content wraps at that height, as in Chrome.
    fn flex_inputs(&mut self, node: NodeId, mut inputs: LayoutInput) -> LayoutInput {
        let Some(flex) = Flex::of(self.node(node)) else {
            return inputs;
        };
        let style = &self.node(node).style;
        if flex.row {
            if inputs.known_dimensions.width.is_some() {
                inputs.known_dimensions_are_definite.width = true;
            }
            return inputs;
        }
        let content_height =
            inputs.sizing_mode == SizingMode::ContentSize || style.size.height.is_auto();
        let wraps_at_max_height = flex.wraps && !style.max_size.height.is_auto();
        match inputs.known_dimensions.height {
            None if content_height => inputs.available_space.height = AvailableSpace::MaxContent,
            Some(height) if wraps_at_max_height && !inputs.known_dimensions_are_definite.height => {
                let own = self.compute_layout(
                    node,
                    LayoutInput {
                        run_mode: RunMode::ComputeSize,
                        axis: RequestedAxis::Vertical,
                        known_dimensions: Size {
                            height: None,
                            ..inputs.known_dimensions
                        },
                        ..inputs
                    },
                );
                if (own.size.height - height).abs() <= SAME_HEIGHT {
                    inputs.known_dimensions.height = None;
                    inputs.available_space.height = AvailableSpace::MaxContent;
                } else {
                    inputs.known_dimensions_are_definite.height = true;
                }
            }
            _ => {}
        }
        inputs
    }

    /// The height of a column whose content decides it, as Chrome sizes it: the
    /// sum of the hypothetical main sizes of its items, each its flex base size
    /// clamped by its min and max heights, with the gaps between them, and the
    /// padding. The column's own min and max heights then bound it. Where the
    /// column wraps at its max height, it is as tall as its longest line.
    ///
    /// Taffy sizes such a column in two ways that differ from Chrome's:
    ///
    /// - It takes the intrinsic main size of CSS's flexbox specification,
    ///   where an item that grows counts at its content's height: a `fill`
    ///   item, whose flex base size is 0, adds the height of its content
    ///   however small its `minHeight` is, so a text that fills a column and
    ///   is 30 px tall at its `minHeight` adds the 168 px that it needs in 60
    ///   px of width. The items that do not grow are sized the same way in
    ///   both, so the column is measured once with its items kept from
    ///   growing, which leaves each at its hypothetical size, and laid out at
    ///   the height that gives, where its `fill` items grow into whatever its
    ///   own min height leaves them.
    /// - It breaks the lines of a column that wraps at its max height there,
    ///   where Chrome breaks them at the max height less the padding, so the
    ///   column is measured inside its padding (see
    ///   [`LayoutTree::inside_vertical_padding`]), and laid out at the height
    ///   that gives, plus the padding, where Taffy takes that height less
    ///   the padding for the room that breaks the lines.
    ///
    /// Returns that height and whether Taffy may break the lines at it, which
    /// it does only for a column that wraps at its max height. Returns `None`
    /// for every other node, for a column that has the height it is laid out
    /// at, and for one that needs neither.
    fn auto_height(&mut self, node: NodeId, inputs: &LayoutInput) -> Option<(f32, bool)> {
        let flex = Flex::of(self.node(node)).filter(|flex| !flex.row)?;
        let style = &self.node(node).style;
        let content_height =
            inputs.sizing_mode == SizingMode::ContentSize || style.size.height.is_auto();
        if inputs.known_dimensions.height.is_some() || !content_height {
            return None;
        }
        let wraps_at_max_height = flex.wraps && !style.max_size.height.is_auto();
        let [min, max] = [style.min_size.height, style.max_size.height];
        let padding = self.padded_wrapping_column(node);
        let items = self.items(node);
        let unchanged: Vec<(f32, LengthPercentageAuto)> = items
            .iter()
            .map(|&item| {
                let style = &self.node(item).style;
                (style.flex_grow, style.min_size.height)
            })
            .collect();
        let grows = unchanged.iter().any(|&(grow, _)| grow != 0.0);
        if !grows && padding.is_none() {
            return None;
        }
        if grows {
            for &item in &items {
                let style = &mut self.node_mut(item).style;
                style.flex_grow = 0.0;
                // An item is never shorter than its padding, whatever its min
                // height says: Taffy counts the item at its min height alone
                // where it adds up its content's height, for an item that hugs.
                let padding = style
                    .padding
                    .resolve_or_zero(None, |_, _| 0.0)
                    .vertical_axis_sum();
                if let Some(min) = style.min_size.height.resolve_to_option(0.0, |_, _| 0.0) {
                    style.min_size.height = LengthPercentageAuto::length(min.max(padding));
                }
            }
        }
        let measure = |tree: &mut Self| {
            tree.compute_uncached(
                node,
                LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    axis: RequestedAxis::Vertical,
                    ..*inputs
                },
            )
        };
        let measured = match padding {
            Some(padding) => self.inside_vertical_padding(node, padding, measure),
            None => measure(self),
        };
        for (&item, (grow, min_height)) in items.iter().zip(unchanged) {
            let style = &mut self.node_mut(item).style;
            style.flex_grow = grow;
            style.min_size.height = min_height;
        }
        let height = bounded(measured.size.height + padding.unwrap_or(0.0), min, max);
        Some((height, wraps_at_max_height))
    }

    /// The vertical padding of the column `node`, if it wraps at its max height
    /// and has any. Chrome breaks the lines of such a column at its max height
    /// less that padding, the room that its items have. Taffy breaks them at the
    /// max height itself, so its lines are too long by the padding, and a line
    /// that fits Chrome's room exactly is moved to a line of its own.
    fn padded_wrapping_column(&self, node: NodeId) -> Option<f32> {
        Flex::of(self.node(node)).filter(|flex| !flex.row && flex.wraps)?;
        let style = &self.node(node).style;
        let padding = style
            .padding
            .resolve_or_zero(None, |_, _| 0.0)
            .vertical_axis_sum();
        (!style.max_size.height.is_auto() && padding > 0.0).then_some(padding)
    }

    /// The result of `measure`, which sizes the column `node` that
    /// [`LayoutTree::padded_wrapping_column`] finds, as if the room that its
    /// items have were all its height: its vertical padding is taken off, and
    /// its min and max heights are reduced by it. Taffy then breaks the lines
    /// where Chrome does.
    fn inside_vertical_padding<R>(
        &mut self,
        node: NodeId,
        padding: f32,
        measure: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let style = &mut self.node_mut(node).style;
        let kept = (
            style.padding.top,
            style.padding.bottom,
            style.min_size.height,
            style.max_size.height,
        );
        // A min or max height less the padding, which a size that is left out
        // is not.
        let inside = |bound: LengthPercentageAuto| {
            bound
                .resolve_to_option(0.0, |_, _| 0.0)
                .map_or(bound, |bound| {
                    LengthPercentageAuto::length((bound - padding).max(0.0))
                })
        };
        style.padding.top = LengthPercentage::length(0.0);
        style.padding.bottom = LengthPercentage::length(0.0);
        style.min_size.height = inside(kept.2);
        style.max_size.height = inside(kept.3);
        let measured = measure(self);
        let style = &mut self.node_mut(node).style;
        (
            style.padding.top,
            style.padding.bottom,
            style.min_size.height,
            style.max_size.height,
        ) = kept;
        measured
    }

    /// Lays out `node` with the inputs that its parent's layout gives it,
    /// through the cache.
    fn compute_layout(&mut self, node: NodeId, inputs: LayoutInput) -> LayoutOutput {
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
        let inputs = self.flex_inputs(node, inputs);
        compute_cached_layout(self, node, inputs, |tree, node, inputs| {
            if let Some(width) = tree.auto_width(node, &inputs) {
                return tree.compute_layout(
                    node,
                    LayoutInput {
                        known_dimensions: Size {
                            width: Some(width),
                            ..inputs.known_dimensions
                        },
                        known_dimensions_are_definite: Size {
                            width: true,
                            ..inputs.known_dimensions_are_definite
                        },
                        ..inputs
                    },
                );
            }
            if let Some((height, definite)) = tree.auto_height(node, &inputs) {
                return tree.compute_layout(
                    node,
                    LayoutInput {
                        known_dimensions: Size {
                            height: Some(height),
                            ..inputs.known_dimensions
                        },
                        // The height comes from the column's own content, so
                        // percentages ignore it, and so does the wrapping of
                        // its items, unless it is the height that they wrap at.
                        known_dimensions_are_definite: Size {
                            height: definite,
                            ..inputs.known_dimensions_are_definite
                        },
                        ..inputs
                    },
                );
            }
            let mut output = tree.compute_uncached(node, inputs);
            if inputs.run_mode == RunMode::PerformLayout {
                tree.finish_flex_lines(node, &mut output);
                tree.finish_grid_baseline(node, &mut output);
                compute_oof_layout(tree, node, &mut output);
            }
            output
        })
    }

    /// Lays out `node` by its CSS `display`, without the cache.
    fn compute_uncached(&mut self, node: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let outer = self.node_mut(node).laying_out.replace(inputs);
        let output = self.compute_by_display(node, inputs);
        self.node_mut(node).laying_out = outer;
        output
    }

    fn compute_by_display(&mut self, node: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let TreeNode {
            style, children, ..
        } = self.node(node);
        match (style.display, children.is_empty()) {
            (Display::None, _) => compute_hidden_layout(self, node),
            // An empty grid still has the tracks that it declares.
            (Display::Grid, true) => compute_grid_layout(self, node, inputs),
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

/// A fixed `size` bounded by `min` and `max`, or `None` when the size is not
/// fixed.
fn fixed(size: Dimension, min: LengthPercentageAuto, max: LengthPercentageAuto) -> Option<f32> {
    size.into_option().map(|size| bounded(size, min, max))
}

/// `size` bounded by `min` and `max`, where a min beats a max, as in CSS.
fn bounded(size: f32, min: LengthPercentageAuto, max: LengthPercentageAuto) -> f32 {
    let [min, max] = [min, max].map(|bound| bound.resolve_to_option(0.0, |_, _| 0.0));
    size.maybe_clamp(min, max)
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
        let placed = inputs.run_mode == RunMode::PerformLayout;
        if placed {
            self.node_mut(node).placed_with = Some(inputs);
        }
        let inputs = self.without_guessed_cross_size(node, inputs);
        let output = self.compute_layout(node, inputs);
        if placed {
            self.node_mut(node).baseline = output.baselines.first;
        }
        output
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
        node.content_widths.clear();
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
