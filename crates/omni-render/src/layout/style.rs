//! The CSS of each design node, as Taffy styles, by the mapping in
//! `docs/formats/layout.md`.

use crate::text::literal;
use omni_core::design::{self, Length, Node, NodeData, Sizing};
use taffy::prelude::{TaffyAuto, TaffyGridLine, fr, length};
use taffy::{
    AlignItems, AlignSelf, Dimension, Display, FlexDirection, FlexWrap, GridPlacement,
    GridTemplateComponent, JustifyContent, LengthPercentage, LengthPercentageAuto, Line, MaybeMath,
    Position, Rect, Size, Style,
};

/// How a parent places a child.
#[derive(Clone, Copy)]
pub(super) enum Flow {
    /// Laid out on its own: a board on its page, or the root of a layout.
    Root,
    /// At its `x` and `y` from the parent's top-left corner: children of
    /// `none` layouts and absolute children of flex and grid layouts.
    Positioned,
    /// In a flex layout whose main axis is horizontal (`row`) or vertical.
    Flex { row: bool },
    /// In a grid layout.
    Grid,
}

impl Flow {
    /// How a layout that places its children by `self` places `child`.
    pub(super) fn of(self, child: &Node) -> Flow {
        match self {
            Flow::Flex { .. } | Flow::Grid if child.placement().is_absolute() => Flow::Positioned,
            flow => flow,
        }
    }
}

/// The CSS of `node` as a child placed by `flow`: its size, and how its
/// parent's layout sizes and places it.
pub(super) fn item(node: &Node, flow: Flow) -> Style {
    let sizes = Sizes::of(node);
    let mut style = Style {
        size: Size {
            width: dimension(sizes.width, sizes.min_width, sizes.max_width),
            height: dimension(sizes.height, sizes.min_height, sizes.max_height),
        },
        min_size: Size {
            width: bound(sizes.min_width),
            height: bound(sizes.min_height),
        },
        max_size: Size {
            width: bound(sizes.max_width),
            height: bound(sizes.max_height),
        },
        ..Style::default()
    };
    let fills = |sizing: Option<&Sizing>| matches!(sizing, Some(Sizing::Fill));
    match flow {
        Flow::Root => {}
        Flow::Positioned => {
            let placement = node.placement();
            let offset = |offset: Option<f64>| length(offset.unwrap_or(0.0) as f32);
            // `fill` stretches the box to the parent's far edge.
            let far_edge = |sizing| {
                if fills(sizing) {
                    length(0.0)
                } else {
                    LengthPercentageAuto::auto()
                }
            };
            style.position = Position::Absolute;
            style.inset = Rect {
                left: offset(placement.x),
                right: far_edge(sizes.width),
                top: offset(placement.y),
                bottom: far_edge(sizes.height),
            };
        }
        Flow::Flex { row } => {
            let (main, cross) = if row {
                (sizes.width, sizes.height)
            } else {
                (sizes.height, sizes.width)
            };
            match main {
                Some(Sizing::Fixed(_)) => style.flex_shrink = 0.0,
                Some(Sizing::Fill) => {
                    style.flex_grow = 1.0;
                    style.flex_basis = length(0.0);
                }
                // `hug` keeps CSS's initial `flex: 0 1 auto`.
                Some(Sizing::Hug) | None => {}
            }
            if fills(cross) {
                style.align_self = Some(AlignSelf::STRETCH);
            }
        }
        Flow::Grid => {
            let self_alignment = |sizing| {
                if fills(sizing) {
                    AlignSelf::STRETCH
                } else {
                    AlignSelf::START
                }
            };
            style.justify_self = Some(self_alignment(sizes.width));
            style.align_self = Some(self_alignment(sizes.height));
            let placement = node.placement();
            if let Some(column) = &placement.grid_column {
                style.grid_column = grid_lines(column);
            }
            if let Some(row) = &placement.grid_row {
                style.grid_row = grid_lines(row);
            }
        }
    }
    style
}

/// Sets the CSS of a board's or frame's `layout` on its `style`, and returns
/// how the layout places the children.
pub(super) fn container(style: &mut Style, layout: &design::Layout) -> Flow {
    // A board or frame is the containing block of its positioned children.
    if style.position == Position::Static {
        style.position = Position::Relative;
    }
    match layout {
        design::Layout::None => {
            style.display = Display::Block;
            Flow::Positioned
        }
        design::Layout::Flex(flex) => {
            let direction = flex.direction.unwrap_or(design::FlexDirection::Row);
            style.display = Display::Flex;
            style.flex_direction = match direction {
                design::FlexDirection::Row => FlexDirection::Row,
                design::FlexDirection::Column => FlexDirection::Column,
                design::FlexDirection::RowReverse => FlexDirection::RowReverse,
                design::FlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
            };
            style.flex_wrap = match flex.wrap {
                None | Some(design::FlexWrap::NoWrap) => FlexWrap::NoWrap,
                Some(design::FlexWrap::Wrap) => FlexWrap::Wrap,
                Some(design::FlexWrap::WrapReverse) => FlexWrap::WrapReverse,
            };
            style.gap = gap(flex.gap.as_ref());
            style.padding = padding(flex.padding.as_ref());
            // `start` and `end` are the CSS keywords, not `flex-start` and
            // `flex-end`.
            style.align_items = match flex.align_items {
                None => AlignItems::NORMAL,
                Some(design::AlignItems::Start) => AlignItems::START,
                Some(design::AlignItems::Center) => AlignItems::CENTER,
                Some(design::AlignItems::End) => AlignItems::END,
                Some(design::AlignItems::Stretch) => AlignItems::STRETCH,
                Some(design::AlignItems::Baseline) => AlignItems::BASELINE,
            };
            style.justify_content = match flex.justify_content {
                None => JustifyContent::NORMAL,
                Some(design::JustifyContent::Start) => JustifyContent::START,
                Some(design::JustifyContent::Center) => JustifyContent::CENTER,
                Some(design::JustifyContent::End) => JustifyContent::END,
                Some(design::JustifyContent::SpaceBetween) => JustifyContent::SPACE_BETWEEN,
                Some(design::JustifyContent::SpaceAround) => JustifyContent::SPACE_AROUND,
                Some(design::JustifyContent::SpaceEvenly) => JustifyContent::SPACE_EVENLY,
            };
            Flow::Flex {
                row: matches!(
                    direction,
                    design::FlexDirection::Row | design::FlexDirection::RowReverse
                ),
            }
        }
        design::Layout::Grid(grid) => {
            style.display = Display::Grid;
            style.grid_template_columns = tracks(grid.columns.as_deref());
            style.grid_template_rows = tracks(grid.rows.as_deref());
            style.gap = gap(grid.gap.as_ref());
            style.padding = padding(grid.padding.as_ref());
            Flow::Grid
        }
    }
}

/// A node's sizing on each axis, and the bounds of its size. An instance may
/// leave an axis to its component.
struct Sizes<'a> {
    width: Option<&'a Sizing>,
    height: Option<&'a Sizing>,
    min_width: Option<&'a Length>,
    max_width: Option<&'a Length>,
    min_height: Option<&'a Length>,
    max_height: Option<&'a Length>,
}

impl<'a> Sizes<'a> {
    fn of(node: &'a Node) -> Self {
        match node.data() {
            NodeData::Board(frame) | NodeData::Frame(frame) => Self::new(&frame.size),
            NodeData::Text(text) => Self::new(&text.size),
            NodeData::Image(image) => Self::new(&image.size),
            NodeData::Vector(vector) => Self::new(&vector.size),
            NodeData::Instance(instance) => Self {
                width: instance.size.width.as_ref(),
                height: instance.size.height.as_ref(),
                ..Self::bounds(&instance.size)
            },
            // Pages are not laid out.
            NodeData::Page(_) => Self {
                width: None,
                height: None,
                min_width: None,
                max_width: None,
                min_height: None,
                max_height: None,
            },
        }
    }

    fn new(size: &'a design::Size) -> Self {
        Self {
            width: Some(&size.width),
            height: Some(&size.height),
            ..Self::bounds(size)
        }
    }

    fn bounds<S>(size: &'a design::Size<S>) -> Self {
        Self {
            width: None,
            height: None,
            min_width: size.min_width.as_ref(),
            max_width: size.max_width.as_ref(),
            min_height: size.min_height.as_ref(),
            max_height: size.max_height.as_ref(),
        }
    }
}

/// The CSS `width` or `height` of a sizing: the length of a fixed size, or
/// `auto` for `hug` and `fill`, which the parent's layout resolves.
///
/// A fixed length comes bounded by `min` and `max`, a min beating a max, as
/// CSS uses it. Taffy would otherwise count it unbounded in places, as in the
/// height of a column that hugs a fixed child with a max height.
fn dimension(sizing: Option<&Sizing>, min: Option<&Length>, max: Option<&Length>) -> Dimension {
    match sizing {
        Some(Sizing::Fixed(size)) => literal(size).map_or(Dimension::auto(), |size| {
            let [min, max] = [min, max].map(|bound| bound.and_then(literal));
            Dimension::length(size.maybe_clamp(min, max))
        }),
        Some(Sizing::Hug | Sizing::Fill) | None => Dimension::auto(),
    }
}

/// A CSS min or max size: the length, or none when it is left out.
fn bound(size: Option<&Length>) -> LengthPercentageAuto {
    size.and_then(literal)
        .map_or(LengthPercentageAuto::auto(), LengthPercentageAuto::length)
}

/// CSS `row-gap` and `column-gap`.
fn gap(gap: Option<&design::Gap>) -> Size<LengthPercentage> {
    let space = |space: Option<&Length>| length(space.and_then(literal).unwrap_or(0.0));
    Size {
        width: space(gap.and_then(|gap| gap.column.as_ref())),
        height: space(gap.and_then(|gap| gap.row.as_ref())),
    }
}

/// CSS `padding`.
fn padding(padding: Option<&design::Padding>) -> Rect<LengthPercentage> {
    let side = |side: fn(&design::Padding) -> Option<&Length>| {
        length(padding.and_then(side).and_then(literal).unwrap_or(0.0))
    };
    Rect {
        left: side(|padding| padding.left.as_ref()),
        right: side(|padding| padding.right.as_ref()),
        top: side(|padding| padding.top.as_ref()),
        bottom: side(|padding| padding.bottom.as_ref()),
    }
}

/// CSS `grid-template-columns` or `grid-template-rows`.
fn tracks(tracks: Option<&[design::Track]>) -> Vec<GridTemplateComponent<String>> {
    tracks
        .unwrap_or_default()
        .iter()
        .map(|track| match track {
            design::Track::Fixed(size) => literal(size).map_or(GridTemplateComponent::AUTO, length),
            design::Track::Fr(fraction) => fr(*fraction as f32),
            design::Track::Auto => GridTemplateComponent::AUTO,
        })
        .collect()
}

/// CSS `grid-column` or `grid-row`: `<start> / span <span>`, where a start
/// or span left out is `auto`.
fn grid_lines(placement: &design::GridPlacement) -> Line<GridPlacement> {
    let span = placement
        .span
        .map_or(GridPlacement::Auto, GridPlacement::Span);
    match placement.start {
        Some(start) => Line {
            start: GridPlacement::from_line_index(start),
            end: span,
        },
        None => Line {
            start: span,
            end: GridPlacement::Auto,
        },
    }
}
