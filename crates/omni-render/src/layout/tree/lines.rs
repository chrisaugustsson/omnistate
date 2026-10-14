//! The lines of flex containers, finished once Taffy has laid a container out.
//!
//! Where a flex container wraps its items into lines or runs them in reverse,
//! Taffy and Chrome differ in three places, and this module has the layout of
//! the container follow Chrome in all of them:
//!
//! - How wide an item is that hugs its width and that no alignment stretches,
//!   in a column that wraps (see [`LayoutTree::fit_items_to_their_lines`]).
//! - Where the lines or the items go when they overflow the container, which
//!   Taffy decides by the newer wording of CSS Box Alignment, and Chrome by the
//!   older one (see [`LayoutTree::place_overflowing_items`]).
//! - Which item a row that runs in reverse takes its first baseline from, and
//!   where that item is once items have moved (see
//!   [`LayoutTree::first_baseline`]).

use super::{Flex, LayoutTree, SAME_PLACE, TreeNode, bounded};
use std::ops::Range;
use taffy::{
    AlignItems, AvailableSpace, Display, FlexDirection, FlexWrap, JustifyContent, Layout,
    LayoutInput, LayoutOutput, LayoutPartialTree, NodeId, RequestedAxis, ResolveOrZero, RunMode,
    Size,
};

/// The items of a flex container as Taffy has just placed them.
struct Placed {
    /// The items that the container's layout places, in document order.
    items: Vec<NodeId>,
    /// Where Taffy put each item, and how large it made it.
    layouts: Vec<Layout>,
    /// The lines that the items are in, as ranges of `items`.
    lines: Vec<Range<usize>>,
    /// The size of the container inside its padding, along its main axis and
    /// across it.
    inner_main: f32,
    inner_cross: f32,
}

impl Placed {
    /// The place and the size of an item along the main axis.
    fn main(&self, row: bool, index: usize) -> [f32; 2] {
        main_axis(row, &self.layouts[index])
    }
}

fn main_axis(row: bool, layout: &Layout) -> [f32; 2] {
    if row {
        [layout.location.x, layout.size.width]
    } else {
        [layout.location.y, layout.size.height]
    }
}

fn cross_axis(row: bool, layout: &Layout) -> [f32; 2] {
    main_axis(!row, layout)
}

impl LayoutTree {
    /// Finishes the layout of the flex container `node` once Taffy has laid it
    /// out and made `output` of it.
    pub(super) fn finish_flex_lines(&mut self, node: NodeId, output: &mut LayoutOutput) {
        let size = output.size;
        let Some(flex) = Flex::of(self.node(node)) else {
            return;
        };
        let style = &self.node(node).style;
        let wraps_in_reverse = style.flex_wrap == FlexWrap::WrapReverse;
        let reversed = matches!(
            style.flex_direction,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        );
        let spaces_between = reversed && style.justify_content == JustifyContent::SPACE_BETWEEN;
        let fits_hug_items = !flex.row && flex.wraps;
        let runs_in_reverse_row = flex.row && reversed;
        if !fits_hug_items && !wraps_in_reverse && !spaces_between && !runs_in_reverse_row {
            return;
        }
        if fits_hug_items {
            let placed = self.placed(node, flex, size);
            self.fit_items_to_their_lines(node, &placed);
        }
        if wraps_in_reverse || spaces_between {
            let placed = self.placed(node, flex, size);
            self.place_overflowing_items(node, flex, &placed);
        }
        // Where items moved, or where a row runs in reverse, the container's
        // first baseline is no longer the one that Taffy found.
        let placed = self.placed(node, flex, size);
        if let Some(baseline) = self.first_baseline(node, flex, &placed) {
            output.baselines.first = Some(baseline);
        }
    }

    /// The items of the flex container `node`, which is `size` large, as Taffy
    /// has just placed them, and the lines they are in. Along the main axis, the
    /// items of a line follow one another, and each line starts the main axis
    /// again, so a line ends where the next item does not come after the item
    /// before it, where the main axis runs in reverse, the next item comes
    /// before. A line also ends where the next item no longer fits in it, which
    /// settles where items of no size are on the same line as the next item and
    /// where they are not.
    fn placed(&self, node: NodeId, flex: Flex, size: Size<f32>) -> Placed {
        let style = &self.node(node).style;
        let reversed = matches!(
            style.flex_direction,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        );
        let padding = style.padding.resolve_or_zero(None, |_, _| 0.0);
        let gap = style.gap.resolve_or_zero(Size::NONE, |_, _| 0.0);
        let (inner_width, inner_height) = (
            size.width - padding.horizontal_axis_sum(),
            size.height - padding.vertical_axis_sum(),
        );
        let (inner_main, inner_cross, main_gap) = if flex.row {
            (inner_width, inner_height, gap.width)
        } else {
            (inner_height, inner_width, gap.height)
        };
        let items = self.items(node);
        let layouts: Vec<Layout> = items.iter().map(|&item| self.node(item).layout).collect();
        let mut lines = Vec::new();
        let mut start = 0;
        let mut length = 0.0;
        // A container that does not wrap keeps all its items on one line.
        let breaking = layouts.iter().enumerate().filter(|_| flex.wraps);
        for (index, layout) in breaking {
            let [position, extent] = main_axis(flex.row, layout);
            if index > start {
                let previous = main_axis(flex.row, &layouts[index - 1]);
                let [before, after] = if reversed {
                    [[position, extent], previous]
                } else {
                    [previous, [position, extent]]
                };
                let follows = after[0] >= before[0] + before[1] - SAME_PLACE;
                let fits = length + main_gap + extent <= inner_main + SAME_PLACE;
                if !follows || !fits {
                    lines.push(start..index);
                    start = index;
                    length = 0.0;
                }
            }
            length += if index == start {
                extent
            } else {
                main_gap + extent
            };
        }
        if !layouts.is_empty() {
            lines.push(start..layouts.len());
        }
        Placed {
            items,
            layouts,
            lines,
            inner_main,
            inner_cross,
        }
    }

    /// Fits the items of the column `node`, which wraps, into their lines as
    /// Chrome does. Taffy sizes an item whose width is its content's, and that
    /// no alignment stretches, to fit into the width of the column, which is the
    /// space it is measured in. Chrome lays such an item out again in its flex
    /// line, and a line is as wide as its widest item, so a text that is 120 px
    /// wide where the column is, next to a box of 130 px, is as wide as it needs
    /// up to 130 px. The height of the item stays what it was measured at in the
    /// column, and the line, which the widest item sets, stays as wide.
    fn fit_items_to_their_lines(&mut self, node: NodeId, placed: &Placed) {
        let style = &self.node(node).style;
        let wraps_in_reverse = style.flex_wrap == FlexWrap::WrapReverse;
        for line in &placed.lines {
            let line_width = placed.layouts[line.clone()]
                .iter()
                .map(|layout| layout.size.width)
                .fold(0.0, f32::max);
            for index in line.clone() {
                let layout = placed.layouts[index];
                if layout.size.width >= line_width - SAME_PLACE {
                    continue;
                }
                let item = placed.items[index];
                let TreeNode {
                    style, placed_with, ..
                } = self.node(item);
                let container = self.node(node);
                let align = style.align_self.unwrap_or(container.style.align_items);
                if !style.size.width.is_auto() || self.stretches(node, item, false) {
                    continue;
                }
                let Some(placed_with) = *placed_with else {
                    continue;
                };
                // Where Taffy aligned the item in the line, it aligns the
                // wider item by the same share of the space that is left.
                let share = if align == AlignItems::CENTER {
                    0.5
                } else if align == AlignItems::END
                    || align == AlignItems::BASELINE && wraps_in_reverse
                {
                    1.0
                } else {
                    0.0
                };
                let padding = style
                    .padding
                    .resolve_or_zero(None, |_, _| 0.0)
                    .horizontal_axis_sum();
                let [min, max] = [style.min_size.width, style.max_size.width];
                let fit = |width: f32| bounded(width, min, max).max(padding);
                let measured = self.compute_layout(
                    item,
                    LayoutInput {
                        run_mode: RunMode::ComputeSize,
                        axis: RequestedAxis::Horizontal,
                        known_dimensions: Size {
                            width: None,
                            ..placed_with.known_dimensions
                        },
                        available_space: Size {
                            width: AvailableSpace::Definite(fit(line_width)),
                            ..placed_with.available_space
                        },
                        ..placed_with
                    },
                );
                let width = fit(measured.size.width);
                if width <= layout.size.width + SAME_PLACE {
                    continue;
                }
                let output = self.compute_layout(
                    item,
                    LayoutInput {
                        known_dimensions: Size {
                            width: Some(width),
                            ..placed_with.known_dimensions
                        },
                        ..placed_with
                    },
                );
                let mut fitted = layout;
                fitted.size = output.size;
                fitted.location.x -= (width - layout.size.width) * share;
                self.set_unrounded_layout(item, &fitted);
            }
        }
    }

    /// Places the items of the flex container `node` where they overflow it as
    /// Chrome does:
    ///
    /// - A container that wraps in reverse packs its lines against the far edge
    ///   (`flex-start`, where the cross axis is reversed), so lines that are
    ///   larger than the container overflow its near edge. Taffy packs them
    ///   against the near edge, and lets them overflow the far one.
    /// - A container that runs in reverse with `justifyContent: space-between`
    ///   packs a line that is larger than the container, or that has a single
    ///   item, against the main end of the container, as `flex-start` does
    ///   where the main axis is reversed. Taffy packs it against the near edge.
    /// - The items that `alignItems: baseline` aligns in a column that wraps in
    ///   reverse start together, as one group against the far edge of their
    ///   line, which is as wide as the widest of them. A column has no baseline
    ///   to align by, so the items fall back to `start`, and they do not keep
    ///   to the line's end one by one, as Taffy puts them.
    ///
    /// Chrome and Taffy agree everywhere else, including the other alignments
    /// of an overflow, so only these cases move items.
    fn place_overflowing_items(&mut self, node: NodeId, flex: Flex, placed: &Placed) {
        let style = &self.node(node).style;
        let wraps_in_reverse = style.flex_wrap == FlexWrap::WrapReverse;
        let reversed = matches!(
            style.flex_direction,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        );
        let spaces_between = reversed && style.justify_content == JustifyContent::SPACE_BETWEEN;
        let gap = style.gap.resolve_or_zero(Size::NONE, |_, _| 0.0);
        let main_gap = if flex.row { gap.width } else { gap.height };
        let layouts = &placed.layouts;
        // How far each item moves along the main and the cross axis.
        let mut shifts = vec![[0.0f32; 2]; placed.items.len()];
        if spaces_between {
            for line in &placed.lines {
                let sizes: f32 = line
                    .clone()
                    .map(|index| placed.main(flex.row, index)[1])
                    .sum();
                let free = placed.inner_main - (sizes + main_gap * (line.len() - 1) as f32);
                if free < -SAME_PLACE {
                    for index in line.clone() {
                        shifts[index][0] = free;
                    }
                }
            }
        }
        if wraps_in_reverse {
            // The lines follow one another, and each is as large as an item of
            // it, so the items cover the lines and their gaps.
            let start = layouts
                .iter()
                .map(|layout| cross_axis(flex.row, layout)[0])
                .fold(f32::INFINITY, f32::min);
            let end = layouts
                .iter()
                .map(|layout| {
                    let [position, extent] = cross_axis(flex.row, layout);
                    position + extent
                })
                .fold(f32::NEG_INFINITY, f32::max);
            let free = placed.inner_cross - (end - start);
            if free < -SAME_PLACE {
                for shift in &mut shifts {
                    shift[1] += free;
                }
            }
            if !flex.row {
                let aligns_by_baseline =
                    |index: usize| self.aligns_by_baseline(node, placed.items[index]);
                for line in &placed.lines {
                    let group_width = line
                        .clone()
                        .filter(|&index| aligns_by_baseline(index))
                        .map(|index| layouts[index].size.width)
                        .fold(0.0, f32::max);
                    for index in line.clone().filter(|&index| aligns_by_baseline(index)) {
                        shifts[index][1] += layouts[index].size.width - group_width;
                    }
                }
            }
        }
        for ((&item, &layout), [along, across]) in placed.items.iter().zip(layouts).zip(shifts) {
            if along == 0.0 && across == 0.0 {
                continue;
            }
            let (x, y) = if flex.row {
                (along, across)
            } else {
                (across, along)
            };
            let mut moved = layout;
            moved.location.x += x;
            moved.location.y += y;
            self.set_unrounded_layout(item, &moved);
        }
    }

    /// How far the first baseline of the flex container `node` is below its
    /// top, as Chrome finds it: the baseline of the first item of its first line
    /// that `alignItems: baseline` aligns, or else that of the item that is
    /// first along the main axis, which is the last item where the main axis
    /// runs in reverse. A line is the first where the cross axis starts, the
    /// last where it wraps in reverse. Taffy takes the first item of a row
    /// whichever way it runs.
    fn first_baseline(&self, node: NodeId, flex: Flex, placed: &Placed) -> Option<f32> {
        let style = &self.node(node).style;
        let wraps_in_reverse = style.flex_wrap == FlexWrap::WrapReverse;
        let reversed = matches!(
            style.flex_direction,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        );
        let line = if wraps_in_reverse {
            placed.lines.last()?
        } else {
            placed.lines.first()?
        };
        let first_along_main = if reversed { line.end - 1 } else { line.start };
        let index = if flex.row {
            line.clone()
                .find(|&index| self.aligns_by_baseline(node, placed.items[index]))
                .unwrap_or(first_along_main)
        } else {
            first_along_main
        };
        let layout = &placed.layouts[index];
        let item = self.node(placed.items[index]);
        Some(layout.location.y + item.baseline.unwrap_or(layout.size.height))
    }

    /// Gives the grid `node` the first baseline that Chrome gives it, once
    /// Taffy has laid it out and made `output` of it. A grid takes its first
    /// baseline from an item of its first row, as a flex container does from its
    /// first item: the first item that aligns by baseline, or else the first
    /// item, whose own first baseline it is. Taffy finds the item the same way,
    /// but it takes the bottom edge of an item that does not align by baseline
    /// where it has the item's baseline.
    pub(super) fn finish_grid_baseline(&mut self, node: NodeId, output: &mut LayoutOutput) {
        if self.node(node).style.display != Display::Grid {
            return;
        }
        let Some(taffys) = output.baselines.first else {
            return;
        };
        let chosen = self.items(node).into_iter().find(|&item| {
            let layout = &self.node(item).layout;
            (layout.location.y + layout.size.height - taffys).abs() <= SAME_PLACE
        });
        if let Some(item) = chosen.map(|item| self.node(item)) {
            if let Some(baseline) = item.baseline {
                output.baselines.first = Some(item.layout.location.y + baseline);
            }
        }
    }

    /// Whether the flex container `node` aligns `item` by baseline.
    fn aligns_by_baseline(&self, node: NodeId, item: NodeId) -> bool {
        let style = &self.node(item).style;
        let align = style
            .align_self
            .unwrap_or(self.node(node).style.align_items);
        align == AlignItems::BASELINE
    }
}
