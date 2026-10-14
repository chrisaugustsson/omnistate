//! Designs and the boxes Chrome gives them, as `make chrome-boxes` prints them:
//! headless Chrome at a device pixel ratio of 2, laying the design out as HTML
//! with the CSS of `docs/formats/layout.md` and the bundled Inter loaded
//! through `@font-face`. The layout tests lay each design out and expect those
//! boxes, and `chrome_boxes.rs` expects the tool to print them, so the layout
//! and the tool are held to the same numbers.
//!
//! The design of a case is a board, which sits at its own `x` and `y` (0 when
//! left out). The boxes are in page coordinates.
//!
//! The groups name the findings of the checker that compared the layout with
//! Chrome on thousands of random designs, and the documented rules that a
//! small change of the layout code used to pass every test.

use super::{
    EPSILON, SIX_WORDS, absolute, at, board, fixed, flex, frame, grid, hugging, none, rect, sized,
    text, with, without,
};
use serde_json::{Value, json};

/// How far a box that text sizes may be from Chrome's, which keeps text widths
/// to 1/64 px.
pub const TEXT_TOLERANCE: f32 = 0.05;

/// A design and the boxes Chrome gives some of its nodes.
pub struct Case {
    /// What the case shows, for failure messages.
    pub name: String,
    /// A board, whose children are the rest of the design.
    pub design: Value,
    /// The boxes of nodes of the design, `[x, y, width, height]` by node ID.
    pub boxes: Vec<(&'static str, [f64; 4])>,
    /// How far a box may be from Chrome's.
    pub tolerance: f32,
}

/// A case whose boxes no text measures, so that layout gets them to within
/// [`EPSILON`].
pub fn case(name: impl Into<String>, design: Value, boxes: &[(&'static str, [f64; 4])]) -> Case {
    Case {
        name: name.into(),
        design,
        boxes: boxes.to_vec(),
        tolerance: EPSILON,
    }
}

/// A case whose widths Chrome rounds to 1/64 px, as it does those of texts and
/// of lengths that are not multiples of 1/64 px, so that they come to within
/// [`TEXT_TOLERANCE`].
pub fn rounded_case(
    name: impl Into<String>,
    design: Value,
    boxes: &[(&'static str, [f64; 4])],
) -> Case {
    Case {
        tolerance: TEXT_TOLERANCE,
        ..case(name, design, boxes)
    }
}

/// Every case of every group, which is every page of boxes that
/// `make chrome-boxes` must print right.
pub fn all() -> Vec<Case> {
    [
        fill_height_with_min_height(),
        max_width_while_shrinking(),
        wrapping_columns(),
        hug_rows_that_wrap(),
        nowrap_hug_row(),
        column_padding(),
        minimum_widths(),
        wrapping_column_overflow(),
        grids_in_rows(),
        auto_widths_for_columns(),
        clamped_fixed_sizes(),
        line_breaks(),
        tabs(),
        boards_sized_by_their_content(),
        none_frames_and_media_take_no_space(),
        hidden_nodes_get_empty_boxes(),
        fill_items(),
        boxes_are_never_smaller_than_their_padding(),
        absolute_children_and_empty_grid_lines(),
        text_style_leaves_boxes_alone(),
        token_references_count_as_left_out(),
        spaces_in_text(),
        weights_styles_and_families_measure_as_inter_regular(),
        normal_line_heights(),
        normal_line_heights_on_baselines(),
        css_mapping(),
        wrapping_columns_wrap_at_their_height(),
        wrapping_rows_at_their_max_content_width(),
        tabs_near_a_stop(),
        empty_texts_have_no_baseline(),
        hug_columns_take_the_height_their_items_need(),
        narrow_columns_wrap_their_texts(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

// A fill-height item with a min height below its content height.

const QUARTERLY: &str = "Quarterly report Supercalifragilistic Lorem ipsum dolor sit amet";

/// A column that hugs its height around a text and a 20 x 20 box. The text is
/// 60 px wide and fills its height, and its minHeight, 30 px, is below the
/// 168 px of height that its content needs at that width. Chrome gives it that
/// minHeight: the text is 60 x 30, the box is at y 30, and the column is 50 px
/// tall.
fn quarterly_column() -> Value {
    let quarterly = text("txt00001", QUARTERLY, 16.0, 1.5);
    let quarterly = with(&quarterly, "width", fixed(60.0));
    let quarterly = with(&quarterly, "height", json!("fill"));
    let quarterly = with(&quarterly, "minHeight", json!(30));
    frame(
        "frm00001",
        json!("hug"),
        json!("hug"),
        flex(json!({"direction": "column"})),
        vec![quarterly, rect("a0000001", 20.0, 20.0)],
    )
}

/// A fill-height item whose minHeight is below its content height stays at its
/// minHeight inside a column whose height comes from its content, as in
/// Chrome, wherever the column is.
pub fn fill_height_with_min_height() -> Vec<Case> {
    let in_row = |align: Option<&str>| {
        let layout = match align {
            Some(align) => flex(json!({"alignItems": align})),
            None => flex(json!({})),
        };
        board(
            "brd00001",
            fixed(300.0),
            fixed(100.0),
            layout,
            vec![quarterly_column()],
        )
    };
    vec![
        case(
            "the column in a board that hugs it",
            board(
                "brd00001",
                json!("hug"),
                json!("hug"),
                flex(json!({})),
                vec![quarterly_column()],
            ),
            &[
                ("brd00001", [0.0, 0.0, 60.0, 50.0]),
                ("frm00001", [0.0, 0.0, 60.0, 50.0]),
                ("txt00001", [0.0, 0.0, 60.0, 30.0]),
                ("a0000001", [0.0, 30.0, 20.0, 20.0]),
            ],
        ),
        case(
            "the column in a grid area",
            board(
                "brd00001",
                fixed(200.0),
                json!("hug"),
                grid(json!({"columns": [{"fixed": 150}, {"fr": 1}]})),
                vec![quarterly_column()],
            ),
            &[
                ("brd00001", [0.0, 0.0, 200.0, 50.0]),
                ("frm00001", [0.0, 0.0, 60.0, 50.0]),
                ("txt00001", [0.0, 0.0, 60.0, 30.0]),
                ("a0000001", [0.0, 30.0, 20.0, 20.0]),
            ],
        ),
        case(
            "the column absolutely positioned in a flex parent",
            board(
                "brd00001",
                fixed(200.0),
                fixed(200.0),
                flex(json!({})),
                vec![absolute(&quarterly_column(), 10.0, 10.0)],
            ),
            &[
                ("brd00001", [0.0, 0.0, 200.0, 200.0]),
                ("frm00001", [10.0, 10.0, 60.0, 50.0]),
                ("txt00001", [10.0, 10.0, 60.0, 30.0]),
                ("a0000001", [10.0, 40.0, 20.0, 20.0]),
            ],
        ),
        case(
            "the column in a `none` layout",
            board(
                "brd00001",
                fixed(200.0),
                fixed(200.0),
                none(),
                vec![at(&quarterly_column(), 10.0, 10.0)],
            ),
            &[
                ("brd00001", [0.0, 0.0, 200.0, 200.0]),
                ("frm00001", [10.0, 10.0, 60.0, 50.0]),
                ("txt00001", [10.0, 10.0, 60.0, 30.0]),
                ("a0000001", [10.0, 40.0, 20.0, 20.0]),
            ],
        ),
        case(
            "the column in a fixed-height row that aligns it to the start",
            in_row(Some("start")),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 100.0]),
                ("frm00001", [0.0, 0.0, 60.0, 50.0]),
                ("txt00001", [0.0, 0.0, 60.0, 30.0]),
                ("a0000001", [0.0, 30.0, 20.0, 20.0]),
            ],
        ),
        case(
            "the column in a fixed-height row that centers it",
            in_row(Some("center")),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 100.0]),
                ("frm00001", [0.0, 25.0, 60.0, 50.0]),
                ("txt00001", [0.0, 25.0, 60.0, 30.0]),
                ("a0000001", [0.0, 55.0, 20.0, 20.0]),
            ],
        ),
        case(
            "the column in a fixed-height row that aligns it to the end",
            in_row(Some("end")),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 100.0]),
                ("frm00001", [0.0, 50.0, 60.0, 50.0]),
                ("txt00001", [0.0, 50.0, 60.0, 30.0]),
                ("a0000001", [0.0, 80.0, 20.0, 20.0]),
            ],
        ),
        // A row that stretches the column makes it as tall as the row, and
        // the text fills the height that the box leaves.
        case(
            "the column in a fixed-height row that stretches it",
            in_row(None),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 100.0]),
                ("frm00001", [0.0, 0.0, 60.0, 100.0]),
                ("txt00001", [0.0, 0.0, 60.0, 80.0]),
                ("a0000001", [0.0, 80.0, 20.0, 20.0]),
            ],
        ),
    ]
}

// A max width that a row's items shrink to.

const ALPHA: &str = "alpha beta gamma delta epsilon";

/// A frame that hugs the five words of `ALPHA`, about 250 px on one line,
/// with `max_width` as its maxWidth.
fn alpha_frame(max_width: u32) -> Value {
    with(
        &frame(
            "frm00001",
            json!("hug"),
            json!("hug"),
            flex(json!({})),
            vec![text("txt00001", ALPHA, 16.0, 1.5)],
        ),
        "maxWidth",
        json!(max_width),
    )
}

/// CSS clamps an item's min and max width in its hypothetical size, not in
/// its flex basis: a row that overflows shrinks its items from the widths of
/// their content, an item that its max width stops keeps that width, and the
/// other items share the rest of the row.
pub fn max_width_while_shrinking() -> Vec<Case> {
    let zeta = || text("txt00002", "zeta eta theta", 16.0, 1.5);
    vec![
        rounded_case(
            "a hug frame with maxWidth 100 and a text in a 160 px row",
            board(
                "brd00001",
                fixed(160.0),
                json!("hug"),
                flex(json!({})),
                vec![alpha_frame(100), zeta()],
            ),
            &[
                ("brd00001", [0.0, 0.0, 160.0, 72.0]),
                ("frm00001", [0.0, 0.0, 100.0, 72.0]),
                ("txt00001", [0.0, 0.0, 100.0, 72.0]),
                ("txt00002", [100.0, 0.0, 60.0, 72.0]),
            ],
        ),
        rounded_case(
            "the same row with alignItems start",
            board(
                "brd00001",
                fixed(160.0),
                json!("hug"),
                flex(json!({"alignItems": "start"})),
                vec![alpha_frame(100), zeta()],
            ),
            &[
                ("brd00001", [0.0, 0.0, 160.0, 72.0]),
                ("frm00001", [0.0, 0.0, 100.0, 72.0]),
                ("txt00001", [0.0, 0.0, 100.0, 72.0]),
                ("txt00002", [100.0, 0.0, 60.0, 72.0]),
            ],
        ),
        rounded_case(
            "the same row at a fixed height of 100",
            board(
                "brd00001",
                fixed(160.0),
                fixed(100.0),
                flex(json!({})),
                vec![alpha_frame(100), zeta()],
            ),
            &[
                ("brd00001", [0.0, 0.0, 160.0, 100.0]),
                ("frm00001", [0.0, 0.0, 100.0, 100.0]),
                ("txt00001", [0.0, 0.0, 100.0, 100.0]),
                ("txt00002", [100.0, 0.0, 60.0, 100.0]),
            ],
        ),
        // The text cannot shrink below its narrowest piece, 38.32 px, so it
        // takes that much, and the two frames share the rest of the row.
        rounded_case(
            "two hug frames with maxWidth 100 and 70 and a text in a 160 px row",
            board(
                "brd00001",
                fixed(160.0),
                json!("hug"),
                flex(json!({})),
                vec![
                    alpha_frame(100),
                    with(
                        &frame(
                            "frm00003",
                            json!("hug"),
                            json!("hug"),
                            flex(json!({})),
                            vec![text("txt00003", ALPHA, 16.0, 1.5)],
                        ),
                        "maxWidth",
                        json!(70),
                    ),
                    zeta(),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 160.0, 120.0]),
                ("frm00001", [0.0, 0.0, 60.84375, 120.0]),
                ("txt00001", [0.0, 0.0, 60.84375, 120.0]),
                ("frm00003", [60.84375, 0.0, 60.8359375, 120.0]),
                ("txt00003", [60.84375, 0.0, 60.8359375, 120.0]),
                ("txt00002", [121.6796875, 0.0, 38.3203125, 120.0]),
            ],
        ),
        // A text that hugs its width bounds itself, and a fixed width is
        // bounded where the style is read.
        rounded_case(
            "a hug text with maxWidth 100 and a text in a 160 px row",
            board(
                "brd00001",
                fixed(160.0),
                json!("hug"),
                flex(json!({})),
                vec![
                    with(&text("txt00001", ALPHA, 16.0, 1.5), "maxWidth", json!(100)),
                    zeta(),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 160.0, 72.0]),
                ("txt00001", [0.0, 0.0, 100.0, 72.0]),
                ("txt00002", [100.0, 0.0, 60.0, 72.0]),
            ],
        ),
        rounded_case(
            "a fixed 200 px box with maxWidth 100 and a text in a 160 px row",
            board(
                "brd00001",
                fixed(160.0),
                json!("hug"),
                flex(json!({})),
                vec![
                    with(&rect("a0000001", 200.0, 20.0), "maxWidth", json!(100)),
                    zeta(),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 160.0, 72.0]),
                ("a0000001", [0.0, 0.0, 100.0, 20.0]),
                ("txt00002", [100.0, 0.0, 60.0, 72.0]),
            ],
        ),
        rounded_case(
            "a hug frame with minWidth 120 around \"Hi\" and a text in a 160 px row",
            board(
                "brd00001",
                fixed(160.0),
                json!("hug"),
                flex(json!({})),
                vec![
                    with(
                        &frame(
                            "frm00001",
                            json!("hug"),
                            json!("hug"),
                            flex(json!({})),
                            vec![text("txt00001", "Hi", 16.0, 1.5)],
                        ),
                        "minWidth",
                        json!(120),
                    ),
                    zeta(),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 160.0, 72.0]),
                ("frm00001", [0.0, 0.0, 120.0, 72.0]),
                ("txt00001", [0.0, 0.0, 15.765625, 72.0]),
                ("txt00002", [120.0, 0.0, 40.0, 72.0]),
            ],
        ),
    ]
}

// Hug items of a wrapping column.

const BROWN_FOX: &str = "brown fox jumps";
const BETA_GAMMA_DELTA: &str = "beta gamma delta";

/// A wrapping column 120 px wide that hugs its height, in a board that hugs
/// it, with `align` as its alignItems (stretching when left out).
fn wrapping_column(align: Option<&str>, children: Vec<Value>) -> Value {
    wrapping_column_of(align, json!("hug"), children)
}

fn wrapping_column_of(align: Option<&str>, height: Value, children: Vec<Value>) -> Value {
    let layout = match align {
        Some(align) => json!({"direction": "column", "wrap": "wrap", "alignItems": align}),
        None => json!({"direction": "column", "wrap": "wrap"}),
    };
    board(
        "brd00001",
        json!("hug"),
        json!("hug"),
        flex(json!({"direction": "column"})),
        vec![frame(
            "frm00001",
            fixed(120.0),
            height,
            flex(layout),
            children,
        )],
    )
}

/// In a wrapping column, hug items are sized against their flex line's cross
/// size, not the container's inner width, as in Chrome. A 130 px box makes the
/// line of the first items wider than the 120 px of the column, so a text
/// that hugs its width gets as much room as its content needs up to 130 px, or
/// up to the width of the widest unbreakable word. Its height stays what its
/// content needs in the 120 px that its flex basis was measured at.
pub fn wrapping_columns() -> Vec<Case> {
    let brown_fox = || text("txt00001", BROWN_FOX, 16.0, 1.5);
    let beta_gamma_delta = || text("txt00003", BETA_GAMMA_DELTA, 16.0, 1.5);
    let wide_box = || rect("a0000001", 130.0, 20.0);
    vec![
        rounded_case(
            "a hug text above a 130 px box in a start-aligned column",
            wrapping_column(Some("start"), vec![brown_fox(), wide_box()]),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00001", [0.0, 0.0, 120.0, 68.0]),
                ("txt00001", [0.0, 0.0, 125.421875, 48.0]),
                ("a0000001", [0.0, 48.0, 130.0, 20.0]),
            ],
        ),
        rounded_case(
            "the same, centered",
            wrapping_column(Some("center"), vec![brown_fox(), wide_box()]),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00001", [0.0, 0.0, 120.0, 68.0]),
                ("txt00001", [2.2890625, 0.0, 125.421875, 48.0]),
                ("a0000001", [0.0, 48.0, 130.0, 20.0]),
            ],
        ),
        rounded_case(
            "the same, aligned to the end",
            wrapping_column(Some("end"), vec![brown_fox(), wide_box()]),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00001", [0.0, 0.0, 120.0, 68.0]),
                ("txt00001", [4.578125, 0.0, 125.421875, 48.0]),
                ("a0000001", [0.0, 48.0, 130.0, 20.0]),
            ],
        ),
        rounded_case(
            "the 130 px box above the text",
            wrapping_column(Some("start"), vec![wide_box(), brown_fox()]),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00001", [0.0, 0.0, 120.0, 68.0]),
                ("a0000001", [0.0, 0.0, 130.0, 20.0]),
                ("txt00001", [0.0, 20.0, 125.421875, 48.0]),
            ],
        ),
        // The text needs 135.77 px on one line, so it fills the 130 px and
        // wraps there.
        rounded_case(
            "a hug text wider than the line, above a 130 px box",
            wrapping_column(Some("start"), vec![beta_gamma_delta(), wide_box()]),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00001", [0.0, 0.0, 120.0, 68.0]),
                ("txt00003", [0.0, 0.0, 130.0, 48.0]),
                ("a0000001", [0.0, 48.0, 130.0, 20.0]),
            ],
        ),
        // The word is 139.70 px wide, and the line grows to fit it.
        rounded_case(
            "a hug text above an unbreakable word wider than the column",
            wrapping_column(
                Some("start"),
                vec![
                    brown_fox(),
                    text("txt00002", "Supercalifragilistic", 16.0, 1.5),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 72.0]),
                ("frm00001", [0.0, 0.0, 120.0, 72.0]),
                ("txt00001", [0.0, 0.0, 125.421875, 48.0]),
                ("txt00002", [0.0, 48.0, 139.703125, 24.0]),
            ],
        ),
        rounded_case(
            "two hug texts above a 150 px box",
            wrapping_column(
                Some("start"),
                vec![
                    brown_fox(),
                    beta_gamma_delta(),
                    rect("a0000001", 150.0, 20.0),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 116.0]),
                ("frm00001", [0.0, 0.0, 120.0, 116.0]),
                ("txt00001", [0.0, 0.0, 125.421875, 48.0]),
                ("txt00003", [0.0, 48.0, 135.765625, 48.0]),
                ("a0000001", [0.0, 96.0, 150.0, 20.0]),
            ],
        ),
        rounded_case(
            "a hug row frame around the text, above a 130 px box",
            wrapping_column(
                Some("start"),
                vec![
                    frame(
                        "frm00003",
                        json!("hug"),
                        json!("hug"),
                        flex(json!({})),
                        vec![brown_fox()],
                    ),
                    wide_box(),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00003", [0.0, 0.0, 125.421875, 48.0]),
                ("txt00001", [0.0, 0.0, 125.421875, 48.0]),
                ("a0000001", [0.0, 48.0, 130.0, 20.0]),
            ],
        ),
        rounded_case(
            "the same row frame, centered",
            wrapping_column(
                Some("center"),
                vec![
                    frame(
                        "frm00003",
                        json!("hug"),
                        json!("hug"),
                        flex(json!({})),
                        vec![brown_fox()],
                    ),
                    wide_box(),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00003", [2.2890625, 0.0, 125.421875, 48.0]),
                ("txt00001", [2.2890625, 0.0, 125.421875, 48.0]),
                ("a0000001", [0.0, 48.0, 130.0, 20.0]),
            ],
        ),
        rounded_case(
            "a hug column frame around the text, above a 130 px box",
            wrapping_column(
                Some("start"),
                vec![
                    frame(
                        "frm00004",
                        json!("hug"),
                        json!("hug"),
                        flex(json!({"direction": "column"})),
                        vec![brown_fox()],
                    ),
                    wide_box(),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00004", [0.0, 0.0, 125.421875, 48.0]),
                ("txt00001", [0.0, 0.0, 125.421875, 24.0]),
                ("a0000001", [0.0, 48.0, 130.0, 20.0]),
            ],
        ),
        // The column has a fixed height of 100, so the 130 px box and the text
        // share the first line and the second text gets a line of its own.
        rounded_case(
            "the same in a column of fixed height",
            wrapping_column_of(Some("start"), fixed(100.0), vec![brown_fox(), wide_box()]),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 100.0]),
                ("frm00001", [0.0, 0.0, 120.0, 100.0]),
                ("txt00001", [0.0, 0.0, 125.421875, 48.0]),
                ("a0000001", [0.0, 48.0, 130.0, 20.0]),
            ],
        ),
        // The texts of the second line are sized against that line, which has
        // only them: 120 px, the width of the column, not the 130 px of the
        // first line.
        rounded_case(
            "two hug texts on the second line of a column of fixed height",
            wrapping_column_of(
                Some("start"),
                fixed(100.0),
                vec![
                    rect("a0000001", 130.0, 60.0),
                    brown_fox(),
                    beta_gamma_delta(),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 100.0]),
                ("frm00001", [0.0, 0.0, 120.0, 100.0]),
                ("a0000001", [0.0, 0.0, 130.0, 60.0]),
                ("txt00001", [130.0, 0.0, 120.0, 48.0]),
                ("txt00003", [130.0, 48.0, 120.0, 48.0]),
            ],
        ),
        // Items that stretch were already sized against their line.
        rounded_case(
            "a hug text above a 130 px box in a column that stretches it",
            wrapping_column(None, vec![beta_gamma_delta(), wide_box()]),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 68.0]),
                ("frm00001", [0.0, 0.0, 120.0, 68.0]),
                ("txt00003", [0.0, 0.0, 130.0, 48.0]),
                ("a0000001", [0.0, 48.0, 130.0, 20.0]),
            ],
        ),
    ]
}

// Hug rows that wrap.

/// A row that hugs its width, wraps, and holds a text.
fn wrapping_frame(id: &str, children: Vec<Value>) -> Value {
    frame(
        id,
        json!("hug"),
        json!("hug"),
        flex(json!({"wrap": "wrap"})),
        children,
    )
}

/// Items with a maxWidth below their unbreakable width, and fill items that
/// must shrink, are sized as in Chrome.
pub fn hug_rows_that_wrap() -> Vec<Case> {
    let input_output = || text("txt00001", "input/output", 14.0, 1.5);
    vec![
        // "input/output" never breaks and is 80.83 px wide, but the text's
        // maxWidth bounds it to 40 px, and so does the frame's.
        rounded_case(
            "a hug wrap frame around \"input/output\" with maxWidth 40 on the text, next to a 60 x 20 box in a 120 px row",
            board(
                "brd00001",
                fixed(120.0),
                json!("hug"),
                flex(json!({})),
                vec![
                    wrapping_frame(
                        "frm00001",
                        vec![with(&input_output(), "maxWidth", json!(40))],
                    ),
                    rect("a0000001", 60.0, 20.0),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 21.0]),
                ("frm00001", [0.0, 0.0, 40.0, 21.0]),
                ("txt00001", [0.0, 0.0, 40.0, 21.0]),
                ("a0000001", [40.0, 0.0, 60.0, 20.0]),
            ],
        ),
        rounded_case(
            "the same with maxWidth 40 on the frame",
            board(
                "brd00001",
                fixed(120.0),
                json!("hug"),
                flex(json!({})),
                vec![
                    with(
                        &wrapping_frame("frm00001", vec![input_output()]),
                        "maxWidth",
                        json!(40),
                    ),
                    rect("a0000001", 60.0, 20.0),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 21.0]),
                ("frm00001", [0.0, 0.0, 40.0, 21.0]),
                ("txt00001", [0.0, 0.0, 80.828125, 21.0]),
                ("a0000001", [40.0, 0.0, 60.0, 20.0]),
            ],
        ),
        // The frame is as wide as "Design" and its padding, 65.91 px, and the
        // long text shrinks to the 124.09 px that are left.
        rounded_case(
            "a 30 x 30 box, a long text and a hug wrap frame around a fill text in a 220 px row",
            board(
                "brd00001",
                fixed(220.0),
                json!("hug"),
                flex(json!({})),
                vec![
                    rect("a0000001", 30.0, 30.0),
                    text(
                        "txt00001",
                        "Design Design A much longer chip label that wraps",
                        12.0,
                        1.5,
                    ),
                    frame(
                        "frm00001",
                        json!("hug"),
                        json!("hug"),
                        flex(json!({
                            "wrap": "wrap",
                            "padding": {"top": 4, "right": 4, "bottom": 4, "left": 16}
                        })),
                        vec![with(
                            &text("txt00002", "Design", 14.0, 1.5),
                            "width",
                            json!("fill"),
                        )],
                    ),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 220.0, 54.0]),
                ("a0000001", [0.0, 0.0, 30.0, 30.0]),
                ("txt00001", [30.0, 0.0, 124.09375, 54.0]),
                ("frm00001", [154.09375, 0.0, 65.90625, 54.0]),
                ("txt00002", [170.09375, 4.0, 45.90625, 46.0]),
            ],
        ),
    ]
}

// The documented rules that small changes of the layout used to pass.

/// A column that hugs both axes.
fn hug_column(id: &str, layout: Value, children: Vec<Value>) -> Value {
    frame(id, json!("hug"), json!("hug"), flex(layout), children)
}

/// A board `width` px wide that is a column aligning its child to the start of
/// its cross axis and hugging its height: where CSS gives a child that hugs
/// its width the fit-content width.
fn start_column(width: f64, children: Vec<Value>) -> Value {
    board(
        "brd00001",
        fixed(width),
        json!("hug"),
        flex(json!({"direction": "column", "alignItems": "start"})),
        children,
    )
}

/// A board that hugs both axes and lays out its children as a row.
fn hug_board(layout: Value, children: Vec<Value>) -> Value {
    board("brd00001", json!("hug"), json!("hug"), layout, children)
}

/// At its narrowest, a row that does not wrap is as wide as its items at their
/// narrowest, with the gaps between them.
pub fn nowrap_hug_row() -> Vec<Case> {
    vec![
        // At its narrowest, a row that does not wrap is as wide as its items
        // at their narrowest, with the gaps: 41.10 + 8 + 55.81 px.
        rounded_case(
            "a nowrap hug row of two texts, 8 px apart, in a 50 px start-aligned column",
            start_column(
                50.0,
                vec![hug_column(
                    "frm00001",
                    json!({"gap": {"column": 8}}),
                    vec![
                        text("txt00001", "alpha beta", 16.0, 1.5),
                        text("txt00002", "gamma delta", 16.0, 1.5),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 50.0, 48.0]),
                ("frm00001", [0.0, 0.0, 104.9140625, 48.0]),
                ("txt00001", [0.0, 0.0, 41.1015625, 48.0]),
                ("txt00002", [49.1015625, 0.0, 55.8125, 48.0]),
            ],
        ),
    ]
}

/// A column's content width includes its padding.
pub fn column_padding() -> Vec<Case> {
    vec![rounded_case(
        "a hug column padded 10 left and 14 right around \"Hello world\", in a 400 px start-aligned column",
        start_column(
            400.0,
            vec![hug_column(
                "frm00001",
                json!({"direction": "column", "padding": {"left": 10, "right": 14}}),
                vec![text("txt00001", "Hello world", 16.0, 1.5)],
            )],
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 24.0]),
            ("frm00001", [0.0, 0.0, 109.40625, 24.0]),
            ("txt00001", [10.0, 0.0, 85.40625, 24.0]),
        ],
    )]
}

/// An item's contribution to the width of a row respects its minWidth.
pub fn minimum_widths() -> Vec<Case> {
    vec![
        rounded_case(
            "a hug row of \"Hi\" with minWidth 80 and a 20 px box, 4 px apart, in a board that hugs it",
            hug_board(
                flex(json!({})),
                vec![hug_column(
                    "frm00001",
                    json!({"gap": {"column": 4}}),
                    vec![
                        with(&text("txt00001", "Hi", 16.0, 1.5), "minWidth", json!(80)),
                        rect("a0000001", 20.0, 20.0),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 104.0, 24.0]),
                ("frm00001", [0.0, 0.0, 104.0, 24.0]),
                ("txt00001", [0.0, 0.0, 80.0, 24.0]),
                ("a0000001", [84.0, 0.0, 20.0, 20.0]),
            ],
        ),
        rounded_case(
            "the same row in a 50 px start-aligned column",
            start_column(
                50.0,
                vec![hug_column(
                    "frm00001",
                    json!({"gap": {"column": 4}}),
                    vec![
                        with(&text("txt00001", "Hi", 16.0, 1.5), "minWidth", json!(80)),
                        rect("a0000001", 20.0, 20.0),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 50.0, 24.0]),
                ("frm00001", [0.0, 0.0, 104.0, 24.0]),
                ("txt00001", [0.0, 0.0, 80.0, 24.0]),
                ("a0000001", [84.0, 0.0, 20.0, 20.0]),
            ],
        ),
        case(
            "a hug row of a 50 px box with minWidth 150 and maxWidth 100, in a 60 px start-aligned column",
            start_column(
                60.0,
                vec![hug_column(
                    "frm00001",
                    json!({}),
                    vec![bounded(
                        rect("a0000001", 50.0, 20.0),
                        &[("minWidth", 150), ("maxWidth", 100)],
                    )],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 60.0, 20.0]),
                ("frm00001", [0.0, 0.0, 150.0, 20.0]),
                ("a0000001", [0.0, 0.0, 150.0, 20.0]),
            ],
        ),
    ]
}

/// A column that wraps is as wide as the lines it wraps into, and at its narrowest
/// as its widest item: where the space is narrower, its lines overflow it.
pub fn wrapping_column_overflow() -> Vec<Case> {
    vec![
        case(
            "a wrapping column with maxHeight 70 around three 40 x 30 boxes with gap 10, in a 30 px start-aligned column",
            start_column(
                30.0,
                vec![with(
                    &hug_column(
                        "frm00001",
                        json!({
                            "direction": "column",
                            "wrap": "wrap",
                            "gap": {"row": 10, "column": 10}
                        }),
                        vec![
                            rect("a0000001", 40.0, 30.0),
                            rect("b0000001", 40.0, 30.0),
                            rect("c0000001", 40.0, 30.0),
                        ],
                    ),
                    "maxHeight",
                    json!(70),
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 30.0, 70.0]),
                ("frm00001", [0.0, 0.0, 40.0, 70.0]),
                ("a0000001", [0.0, 0.0, 40.0, 30.0]),
                ("b0000001", [0.0, 40.0, 40.0, 30.0]),
                ("c0000001", [50.0, 0.0, 40.0, 30.0]),
            ],
        ),
        case(
            "a wrapping column of fixed height 70 around the same boxes, in a board that hugs it",
            hug_board(
                flex(json!({"alignItems": "start"})),
                vec![frame(
                    "frm00001",
                    json!("hug"),
                    fixed(70.0),
                    flex(json!({
                        "direction": "column",
                        "wrap": "wrap",
                        "gap": {"row": 10, "column": 10}
                    })),
                    vec![
                        rect("a0000001", 40.0, 30.0),
                        rect("b0000001", 40.0, 30.0),
                        rect("c0000001", 40.0, 30.0),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 90.0, 70.0]),
                ("frm00001", [0.0, 0.0, 90.0, 70.0]),
                ("a0000001", [0.0, 0.0, 40.0, 30.0]),
                ("b0000001", [0.0, 40.0, 40.0, 30.0]),
                ("c0000001", [50.0, 0.0, 40.0, 30.0]),
            ],
        ),
    ]
}

/// A grid counts in the content widths of the row that holds it.
pub fn grids_in_rows() -> Vec<Case> {
    vec![
        rounded_case(
            "a hug row of a hug grid holding \"Grid cell\" and a 10 px box, 6 px apart, in a board that hugs it",
            hug_board(
                flex(json!({})),
                vec![hug_column(
                    "frm00002",
                    json!({"gap": {"column": 6}}),
                    vec![
                        frame(
                            "frm00001",
                            json!("hug"),
                            json!("hug"),
                            grid(json!({})),
                            vec![text("txt00001", "Grid cell", 16.0, 1.5)],
                        ),
                        rect("a0000001", 10.0, 10.0),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 78.5078125, 24.0]),
                ("frm00002", [0.0, 0.0, 78.5078125, 24.0]),
                ("frm00001", [0.0, 0.0, 62.5078125, 24.0]),
                ("txt00001", [0.0, 0.0, 62.5078125, 24.0]),
                ("a0000001", [68.5078125, 0.0, 10.0, 10.0]),
            ],
        ),
        rounded_case(
            "the same row in a 30 px start-aligned column",
            start_column(
                30.0,
                vec![hug_column(
                    "frm00002",
                    json!({"gap": {"column": 6}}),
                    vec![
                        frame(
                            "frm00001",
                            json!("hug"),
                            json!("hug"),
                            grid(json!({})),
                            vec![text("txt00001", "Grid cell", 16.0, 1.5)],
                        ),
                        rect("a0000001", 10.0, 10.0),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 30.0, 48.0]),
                ("frm00002", [0.0, 0.0, 47.7890625, 48.0]),
                ("frm00001", [0.0, 0.0, 31.7890625, 48.0]),
                ("txt00001", [0.0, 0.0, 31.7890625, 48.0]),
                ("a0000001", [37.7890625, 0.0, 10.0, 10.0]),
            ],
        ),
    ]
}

/// A column that hugs its width takes the width of its minWidth in a grid area.
pub fn auto_widths_for_columns() -> Vec<Case> {
    vec![rounded_case(
        "a hug column with minWidth 200 around six words, in a 120 px grid area",
        board(
            "brd00001",
            fixed(400.0),
            json!("hug"),
            grid(json!({"columns": [{"fixed": 120}, {"fr": 1}]})),
            vec![with(
                &hug_column(
                    "frm00001",
                    json!({"direction": "column"}),
                    vec![text("txt00001", SIX_WORDS, 16.0, 1.5)],
                ),
                "minWidth",
                json!(200),
            )],
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 48.0]),
            ("frm00001", [0.0, 0.0, 200.0, 48.0]),
            ("txt00001", [0.0, 0.0, 200.0, 48.0]),
        ],
    )]
}

/// Fixed sizes are bounded by min and max sizes, where the board that hugs them
/// counts them.
pub fn clamped_fixed_sizes() -> Vec<Case> {
    vec![
        case(
            "a board hugging a column of a 100 x 100 box with maxHeight 50 and a 20 x 20 box",
            hug_board(
                flex(json!({})),
                vec![hug_column(
                    "frm00001",
                    json!({"direction": "column"}),
                    vec![
                        with(&rect("a0000001", 100.0, 100.0), "maxHeight", json!(50)),
                        rect("b0000001", 20.0, 20.0),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 100.0, 70.0]),
                ("frm00001", [0.0, 0.0, 100.0, 70.0]),
                ("a0000001", [0.0, 0.0, 100.0, 50.0]),
                ("b0000001", [0.0, 50.0, 20.0, 20.0]),
            ],
        ),
        case(
            "a board hugging a 10 px high box with minHeight 30, a 100 px wide box with maxWidth 60 and a 20 x 20 box",
            hug_board(
                flex(json!({"direction": "column", "alignItems": "start"})),
                vec![
                    with(&rect("a0000001", 10.0, 10.0), "minHeight", json!(30)),
                    with(&rect("b0000001", 100.0, 10.0), "maxWidth", json!(60)),
                    rect("c0000001", 20.0, 20.0),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 60.0, 60.0]),
                ("a0000001", [0.0, 0.0, 10.0, 30.0]),
                ("b0000001", [0.0, 30.0, 60.0, 10.0]),
                ("c0000001", [0.0, 40.0, 20.0, 20.0]),
            ],
        ),
    ]
}

/// Lines break where Chrome breaks them.
pub fn line_breaks() -> Vec<Case> {
    vec![
        rounded_case(
            "a URL in a text 150 px wide",
            board(
                "brd00001",
                fixed(400.0),
                fixed(300.0),
                flex(json!({"alignItems": "start"})),
                vec![with(
                    &text(
                        "txt00001",
                        "See https://example.com/a/very/long/path/name for more",
                        16.0,
                        1.5,
                    ),
                    "width",
                    fixed(150.0),
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 300.0]),
                ("txt00001", [0.0, 0.0, 150.0, 72.0]),
            ],
        ),
        rounded_case(
            "\"state-of-the-art\" in a text 60 px wide",
            board(
                "brd00001",
                fixed(400.0),
                fixed(300.0),
                flex(json!({"alignItems": "start"})),
                vec![with(
                    &text("txt00001", "state-of-the-art", 16.0, 1.5),
                    "width",
                    fixed(60.0),
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 300.0]),
                ("txt00001", [0.0, 0.0, 60.0, 72.0]),
            ],
        ),
        rounded_case(
            "texts 10 px wide with hyphens and dashes after digits, at the start of a text and of a word",
            board(
                "brd00001",
                fixed(100.0),
                json!("hug"),
                flex(json!({"direction": "column", "alignItems": "start"})),
                [
                    // A hyphen breaks before a digit when a letter or a digit
                    // comes first.
                    "2020-12",
                    "1-2",
                    "x2-3",
                    // A hyphen or a dash that starts a word stays with the
                    // letter after it: at the start of a text, after a tab, but
                    // not after a zero-width space, which is a place to break,
                    // and a no-break space keeps the whole text together.
                    "-\u{e9}lan",
                    "\u{2013}word",
                    "a\t-\u{e9}lan",
                    "a\t\u{2013}word",
                    "word\t-\u{e9}lan",
                    "a\u{a0}-\u{e9}lan",
                    "a\u{200b}-\u{e9}lan",
                    "(-\u{e9}lan)",
                ]
                .iter()
                .enumerate()
                .map(|(index, content)| {
                    let id = format!("t{index:07}");
                    with(&text(&id, content, 16.0, 1.5), "width", fixed(10.0))
                })
                .collect(),
            ),
            &[
                ("brd00001", [0.0, 0.0, 100.0, 456.0]),
                ("t0000000", [0.0, 0.0, 10.0, 48.0]),
                ("t0000001", [0.0, 48.0, 10.0, 48.0]),
                ("t0000002", [0.0, 96.0, 10.0, 48.0]),
                ("t0000003", [0.0, 144.0, 10.0, 24.0]),
                ("t0000004", [0.0, 168.0, 10.0, 24.0]),
                ("t0000005", [0.0, 192.0, 10.0, 48.0]),
                ("t0000006", [0.0, 240.0, 10.0, 48.0]),
                ("t0000007", [0.0, 288.0, 10.0, 48.0]),
                ("t0000008", [0.0, 336.0, 10.0, 24.0]),
                ("t0000009", [0.0, 360.0, 10.0, 48.0]),
                ("t0000010", [0.0, 408.0, 10.0, 48.0]),
            ],
        ),
    ]
}

/// A tab moves its line on to the next tab stop, or to the one after it when
/// less than half a space is left.
pub fn tabs() -> Vec<Case> {
    vec![rounded_case(
        "tabs after \"Date\" (34.94 px, which leaves 1.06 px to the first stop), \"Item\" (32.95 px) and \"Total\" (36.84 px)",
        board(
            "brd00001",
            fixed(1000.0),
            fixed(300.0),
            flex(json!({"alignItems": "start", "direction": "column"})),
            vec![
                text("txt00001", "Date\t", 16.0, 1.5),
                text("txt00002", "Item\t", 16.0, 1.5),
                text("txt00003", "Total\tx", 16.0, 1.5),
                text("txt00004", "Date\tx", 16.0, 1.5),
                text("txt00005", "Item\tx", 16.0, 1.5),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 1000.0, 300.0]),
            ("txt00001", [0.0, 0.0, 72.0, 24.0]),
            ("txt00002", [0.0, 24.0, 36.0, 24.0]),
            ("txt00003", [0.0, 48.0, 80.734375, 24.0]),
            ("txt00004", [0.0, 72.0, 80.734375, 24.0]),
            ("txt00005", [0.0, 96.0, 44.734375, 24.0]),
        ],
    )]
}

fn image(id: &str, width: Value, height: Value) -> Value {
    json!({
        "id": id,
        "type": "image",
        "name": format!("Image {id}"),
        "width": width,
        "height": height,
        "image": "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd",
        "fit": "cover"
    })
}

fn vector(id: &str, size: f64) -> Value {
    json!({
        "id": id,
        "type": "vector",
        "name": format!("Vector {id}"),
        "width": fixed(size),
        "height": fixed(size),
        "path": "M0 0 L24 24"
    })
}

fn instance(id: &str) -> Value {
    json!({
        "id": id,
        "type": "instance",
        "name": format!("Instance {id}"),
        "component": "cmp00001"
    })
}

fn hello_world() -> Value {
    text("txt00001", "Hello world", 16.0, 1.5)
}

fn hidden(json: &Value) -> Value {
    with(json, "visible", json!(false))
}

/// A board has no parent to fill, so one that hugs or fills its width is as wide
/// as its content on one line.
pub fn boards_sized_by_their_content() -> Vec<Case> {
    vec![
        rounded_case(
            "a board that hugs six words",
            hug_board(
                flex(json!({})),
                vec![text("txt00001", SIX_WORDS, 16.0, 1.5)],
            ),
            &[
                ("brd00001", [0.0, 0.0, 250.21875, 24.0]),
                ("txt00001", [0.0, 0.0, 250.21875, 24.0]),
            ],
        ),
        rounded_case(
            "a board that fills both axes around six words",
            board(
                "brd00001",
                json!("fill"),
                json!("fill"),
                flex(json!({})),
                vec![text("txt00001", SIX_WORDS, 16.0, 1.5)],
            ),
            &[
                ("brd00001", [0.0, 0.0, 250.21875, 24.0]),
                ("txt00001", [0.0, 0.0, 250.21875, 24.0]),
            ],
        ),
    ]
}

/// A frame with a `none` layout, an image, a vector and an instance have no
/// content that takes space in their parent.
pub fn none_frames_and_media_take_no_space() -> Vec<Case> {
    vec![
        // A frame with a `none` layout has no content that takes space.
        case(
            "a none frame that hugs, between two 10 px boxes",
            hug_board(
                flex(json!({"alignItems": "start"})),
                vec![
                    rect("a0000001", 10.0, 10.0),
                    frame(
                        "frm00001",
                        json!("hug"),
                        json!("hug"),
                        none(),
                        vec![at(&rect("a0000002", 40.0, 40.0), 5.0, 5.0)],
                    ),
                    rect("a0000003", 10.0, 10.0),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 20.0, 10.0]),
                ("a0000001", [0.0, 0.0, 10.0, 10.0]),
                ("frm00001", [10.0, 0.0, 0.0, 0.0]),
                ("a0000002", [15.0, 5.0, 40.0, 40.0]),
                ("a0000003", [10.0, 0.0, 10.0, 10.0]),
            ],
        ),
        // An image, a vector and an instance have no content either.
        case(
            "images, a vector and instances",
            board(
                "brd00001",
                fixed(400.0),
                json!("hug"),
                flex(json!({"alignItems": "start"})),
                vec![
                    image("jpg00001", json!("hug"), json!("hug")),
                    image("jpg00002", fixed(64.0), fixed(48.0)),
                    vector("vec00001", 24.0),
                    instance("nst00001"),
                    with(&instance("nst00002"), "width", fixed(120.0)),
                    with(
                        &with(&instance("nst00003"), "width", fixed(30.0)),
                        "height",
                        fixed(40.0),
                    ),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 48.0]),
                ("jpg00001", [0.0, 0.0, 0.0, 0.0]),
                ("jpg00002", [0.0, 0.0, 64.0, 48.0]),
                ("vec00001", [64.0, 0.0, 24.0, 24.0]),
                ("nst00001", [88.0, 0.0, 0.0, 0.0]),
                ("nst00002", [88.0, 0.0, 120.0, 0.0]),
                ("nst00003", [208.0, 0.0, 30.0, 40.0]),
            ],
        ),
    ]
}

/// A hidden node, and every node below it, gets an empty box at the top-left
/// corner of its parent.
pub fn hidden_nodes_get_empty_boxes() -> Vec<Case> {
    vec![case(
        "a hidden frame with a child in a row at (50, 20)",
        at(
            &board(
                "brd00001",
                fixed(300.0),
                fixed(100.0),
                flex(json!({"gap": {"column": 10}})),
                vec![
                    rect("a0000001", 50.0, 20.0),
                    hidden(&frame(
                        "frm00001",
                        fixed(200.0),
                        fixed(200.0),
                        flex(json!({})),
                        vec![rect("a0000002", 100.0, 100.0)],
                    )),
                    rect("a0000003", 70.0, 20.0),
                ],
            ),
            50.0,
            20.0,
        ),
        &[
            ("brd00001", [50.0, 20.0, 300.0, 100.0]),
            ("a0000001", [50.0, 20.0, 50.0, 20.0]),
            ("frm00001", [50.0, 20.0, 0.0, 0.0]),
            ("a0000002", [50.0, 20.0, 0.0, 0.0]),
            ("a0000003", [110.0, 20.0, 70.0, 20.0]),
        ],
    )]
}

/// Fill items grow from a flex basis of 0 but never shrink below their
/// min-content width, and count at their content in the width of a row that
/// hugs them.
pub fn fill_items() -> Vec<Case> {
    vec![
        rounded_case(
            "a fill text and a fill box in a 100 px row, the text's word being 139.70 px wide",
            board(
                "brd00001",
                fixed(100.0),
                json!("hug"),
                flex(json!({"alignItems": "start"})),
                vec![
                    with(
                        &text("txt00001", "Supercalifragilistic", 16.0, 1.5),
                        "width",
                        json!("fill"),
                    ),
                    sized("a0000001", json!("fill"), fixed(20.0)),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 100.0, 24.0]),
                ("txt00001", [0.0, 0.0, 139.703125, 24.0]),
                ("a0000001", [139.703125, 0.0, 0.0, 20.0]),
            ],
        ),
        rounded_case(
            "a 50 px box and a fill text in a row that hugs them",
            hug_board(
                flex(json!({"alignItems": "start"})),
                vec![
                    rect("a0000001", 50.0, 10.0),
                    with(
                        &text("txt00001", "Hello", 16.0, 1.5),
                        "width",
                        json!("fill"),
                    ),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 88.5625, 24.0]),
                ("a0000001", [0.0, 0.0, 50.0, 10.0]),
                ("txt00001", [50.0, 0.0, 38.5625, 24.0]),
            ],
        ),
    ]
}

/// A box is never smaller than its padding, on either axis, also in the width
/// of the row that hugs it.
pub fn boxes_are_never_smaller_than_their_padding() -> Vec<Case> {
    vec![
        case(
            "a frame padded 15 px above and below it in a row 20 px tall",
            board(
                "brd00001",
                fixed(300.0),
                fixed(20.0),
                flex(json!({})),
                vec![frame(
                    "frm00001",
                    fixed(50.0),
                    json!("hug"),
                    flex(json!({"padding": {"top": 15, "bottom": 15}})),
                    vec![rect("a0000001", 4.0, 4.0)],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 20.0]),
                ("frm00001", [0.0, 0.0, 50.0, 30.0]),
                ("a0000001", [0.0, 15.0, 4.0, 4.0]),
            ],
        ),
        case(
            "a frame 10 px wide padded 15 px on both sides, next to a 10 px box, in a 20 px start-aligned column",
            start_column(
                20.0,
                vec![hug_column(
                    "frm00001",
                    json!({"gap": {"column": 4}}),
                    vec![
                        frame(
                            "frm00002",
                            fixed(10.0),
                            fixed(10.0),
                            flex(json!({"padding": {"left": 15, "right": 15}})),
                            Vec::new(),
                        ),
                        rect("a0000001", 10.0, 10.0),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 20.0, 10.0]),
                ("frm00001", [0.0, 0.0, 44.0, 10.0]),
                ("frm00002", [0.0, 0.0, 30.0, 10.0]),
                ("a0000001", [34.0, 0.0, 10.0, 10.0]),
            ],
        ),
    ]
}

/// An absolute child of a grid is placed by its `x` and `y` only, and a
/// placement without a start or a span is automatic.
pub fn absolute_children_and_empty_grid_lines() -> Vec<Case> {
    vec![
        // An absolute child is placed by its x and y only.
        case(
            "an absolute child with grid lines in a grid",
            board(
                "brd00001",
                fixed(300.0),
                fixed(200.0),
                grid(json!({
                    "columns": [{"fixed": 100}, {"fixed": 100}],
                    "rows": [{"fixed": 100}]
                })),
                vec![
                    sized("a0000001", json!("fill"), json!("fill")),
                    with(
                        &with(
                            &absolute(&rect("a0000002", 30.0, 30.0), 20.0, 10.0),
                            "gridColumn",
                            json!({"start": 2}),
                        ),
                        "gridRow",
                        json!({"start": 2}),
                    ),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 200.0]),
                ("a0000001", [0.0, 0.0, 100.0, 100.0]),
                ("a0000002", [20.0, 10.0, 30.0, 30.0]),
            ],
        ),
        // A placement without a start or a span is automatic.
        case(
            "children with empty grid lines in a grid",
            board(
                "brd00001",
                fixed(200.0),
                json!("hug"),
                grid(json!({
                    "columns": [{"fixed": 100}, {"fixed": 100}],
                    "rows": [{"fixed": 50}, {"fixed": 50}]
                })),
                vec![
                    with(
                        &with(
                            &sized("a0000001", json!("fill"), json!("fill")),
                            "gridColumn",
                            json!({}),
                        ),
                        "gridRow",
                        json!({}),
                    ),
                    with(
                        &sized("a0000002", json!("fill"), json!("fill")),
                        "gridColumn",
                        json!({"start": 1}),
                    ),
                    sized("a0000003", json!("fill"), json!("fill")),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 200.0, 100.0]),
                ("a0000001", [0.0, 0.0, 100.0, 50.0]),
                ("a0000002", [0.0, 50.0, 100.0, 50.0]),
                ("a0000003", [100.0, 50.0, 100.0, 50.0]),
            ],
        ),
    ]
}

/// `textAlign`, `textDecoration` and `color` do not change a box.
pub fn text_style_leaves_boxes_alone() -> Vec<Case> {
    vec![rounded_case(
        "texts with textAlign, textDecoration and color",
        board(
            "brd00001",
            fixed(300.0),
            json!("hug"),
            flex(json!({"direction": "column"})),
            vec![
                with(
                    &with(
                        &with(&hello_world(), "textAlign", json!("center")),
                        "textDecoration",
                        json!("underline"),
                    ),
                    "width",
                    fixed(200.0),
                ),
                with(
                    &with(
                        &with(
                            &text("txt00002", "Hello world", 16.0, 1.5),
                            "textAlign",
                            json!("right"),
                        ),
                        "color",
                        json!("#ff0000"),
                    ),
                    "width",
                    json!("fill"),
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 300.0, 48.0]),
            ("txt00001", [0.0, 0.0, 200.0, 24.0]),
            ("txt00002", [0.0, 24.0, 300.0, 24.0]),
        ],
    )]
}

/// A token reference counts as left out, so the property keeps its CSS initial
/// value.
pub fn token_references_count_as_left_out() -> Vec<Case> {
    vec![
        rounded_case(
            "token references for gaps, padding, bounds and text properties",
            board(
                "brd00001",
                fixed(300.0),
                json!("hug"),
                flex(json!({
                    "direction": "column",
                    "alignItems": "start",
                    "gap": {"row": "$space.md", "column": "$space.md"},
                    "padding": {"top": "$space.sm", "left": "$space.sm"}
                })),
                vec![
                    with(
                        &with(
                            &with(
                                &without(&text("txt00001", "Hello", 16.0, 1.5), "lineHeight"),
                                "fontSize",
                                json!("$font.size"),
                            ),
                            "lineHeight",
                            json!("$font.lineHeight"),
                        ),
                        "letterSpacing",
                        json!("$font.spacing"),
                    ),
                    with(
                        &with(
                            &rect("a0000001", 20.0, 20.0),
                            "minWidth",
                            json!("$size.min"),
                        ),
                        "maxWidth",
                        json!("$size.max"),
                    ),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 39.359375]),
                ("txt00001", [0.0, 0.0, 38.5625, 19.359375]),
                ("a0000001", [0.0, 19.359375, 20.0, 20.0]),
            ],
        ),
        rounded_case(
            "token references for a fixed size and a grid track",
            board(
                "brd00001",
                fixed(300.0),
                json!("hug"),
                grid(json!({"columns": [{"fixed": "$column.width"}, {"fr": 1}]})),
                vec![
                    text("txt00001", "Hello", 16.0, 1.5),
                    sized("a0000001", json!("fill"), fixed(20.0)),
                    with(
                        &hugging("a0000002", "a0000003", 30.0, 10.0),
                        "width",
                        json!({"fixed": "$size.width"}),
                    ),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 34.0]),
                ("txt00001", [0.0, 0.0, 38.5625, 24.0]),
                ("a0000001", [38.5625, 0.0, 261.4375, 20.0]),
                ("a0000002", [0.0, 24.0, 30.0, 10.0]),
                ("a0000003", [0.0, 24.0, 30.0, 10.0]),
            ],
        ),
    ]
}

/// The spaces at the end of a text count in its width, and the spaces after its
/// narrowest piece do not.
pub fn spaces_in_text() -> Vec<Case> {
    vec![
        rounded_case(
            "\"Hi   \" and \"Hi\" side by side",
            board(
                "brd00001",
                fixed(1000.0),
                json!("hug"),
                flex(json!({"alignItems": "start"})),
                vec![
                    text("txt00001", "Hi   ", 16.0, 1.5),
                    text("txt00002", "Hi", 16.0, 1.5),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 1000.0, 24.0]),
                ("txt00001", [0.0, 0.0, 29.265625, 24.0]),
                ("txt00002", [29.265625, 0.0, 15.765625, 24.0]),
            ],
        ),
        rounded_case(
            "texts of two words and a word followed by spaces in a 10 px start-aligned column",
            start_column(
                10.0,
                vec![
                    text("txt00001", "word    word", 16.0, 1.5),
                    text("txt00002", "word   ", 16.0, 1.5),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 10.0, 72.0]),
                ("txt00001", [0.0, 0.0, 37.953125, 48.0]),
                ("txt00002", [0.0, 48.0, 37.953125, 24.0]),
            ],
        ),
    ]
}

/// With Inter Regular the only face, every weight and style measures as Inter
/// Regular, and so does every family that is missing.
pub fn weights_styles_and_families_measure_as_inter_regular() -> Vec<Case> {
    vec![rounded_case(
        "bold, italic and Helvetica texts",
        board(
            "brd00001",
            fixed(1000.0),
            json!("hug"),
            flex(json!({"alignItems": "start"})),
            vec![
                with(&hello_world(), "fontWeight", json!(700)),
                with(
                    &text("txt00002", "Hello world", 16.0, 1.5),
                    "fontStyle",
                    json!("italic"),
                ),
                with(
                    &with(
                        &text("txt00003", "Hello world", 16.0, 1.5),
                        "fontFamily",
                        json!("Helvetica"),
                    ),
                    "fontWeight",
                    json!(300),
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 1000.0, 24.0]),
            ("txt00001", [0.0, 0.0, 85.40625, 24.0]),
            ("txt00002", [85.40625, 0.0, 85.40625, 24.0]),
            ("txt00003", [170.8125, 0.0, 85.40625, 24.0]),
        ],
    )]
}

// Lines of text without a lineHeight.

/// A row of texts that start at the left edge of a wide board.
fn text_row(children: Vec<Value>) -> Value {
    board(
        "brd00001",
        fixed(2000.0),
        fixed(500.0),
        flex(json!({"alignItems": "start"})),
        children,
    )
}

/// A baseline row: 400 x 100, with every first baseline lined up.
fn baseline_row(children: Vec<Value>) -> Value {
    board(
        "brd00001",
        fixed(400.0),
        fixed(100.0),
        flex(json!({"alignItems": "baseline"})),
        children,
    )
}

/// A text of `size` px without a lineHeight.
fn without_line_height(id: &str, content: &str, size: f64) -> Value {
    without(&text(id, content, size, 1.5), "lineHeight")
}

/// A text without a lineHeight is as tall as its font's normal line height,
/// ascent + descent + line gap, at full precision (decision Q44): 19.359375 px
/// for 16 px Inter, where Chrome's own `line-height: normal` rounds the
/// ascent and the descent to device pixels and gives 19.5 px at a device pixel
/// ratio of 2. The font sizes are those where rounding down and rounding to
/// the nearest 1/64 px agree.
pub fn normal_line_heights() -> Vec<Case> {
    vec![
        rounded_case(
            "\"Hello\" at 10, 12, 14, 16 and 32 px without a lineHeight",
            text_row(vec![
                without_line_height("txt00001", "Hello", 10.0),
                without_line_height("txt00002", "Hello", 12.0),
                without_line_height("txt00003", "Hello", 14.0),
                without_line_height("txt00004", "Hello", 16.0),
                without_line_height("txt00005", "Hello", 32.0),
            ]),
            &[
                ("brd00001", [0.0, 0.0, 2000.0, 500.0]),
                ("txt00001", [0.0, 0.0, 24.1015625, 12.09375]),
                ("txt00002", [24.1015625, 0.0, 28.921875, 14.515625]),
                ("txt00003", [53.0234375, 0.0, 33.7421875, 16.9375]),
                ("txt00004", [86.765625, 0.0, 38.5625, 19.359375]),
                ("txt00005", [125.328125, 0.0, 77.125, 38.71875]),
            ],
        ),
        rounded_case(
            "two lines of 16 px text without a lineHeight",
            text_row(vec![without_line_height("txt00001", "Hello\nworld", 16.0)]),
            &[
                ("brd00001", [0.0, 0.0, 2000.0, 500.0]),
                ("txt00001", [0.0, 0.0, 42.34375, 38.71875]),
            ],
        ),
    ]
}

/// The same text without a lineHeight, lined up with others on their first
/// baselines: the first baseline is the line's ascent, rounded to device
/// pixels, plus half the leading, rounded down to device pixels. At 16 px the
/// line is shorter than the rounded ascent and descent, so that half is
/// negative.
pub fn normal_line_heights_on_baselines() -> Vec<Case> {
    vec![
        rounded_case(
            "a 32 px and a 16 px text without a lineHeight and a 20 px box, on their baselines",
            baseline_row(vec![
                without_line_height("txt00001", "Hello", 32.0),
                without_line_height("txt00002", "Hello", 16.0),
                rect("a0000001", 20.0, 20.0),
            ]),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 100.0]),
                ("txt00001", [0.0, 0.0, 77.125, 38.71875]),
                ("txt00002", [77.125, 16.0, 38.5625, 19.359375]),
                ("a0000001", [115.6875, 11.0, 20.0, 20.0]),
            ],
        ),
        rounded_case(
            "texts of 32 px with lineHeight 1.5 and of 14, 12 and 10 px without one, and a 10 px box, on their baselines",
            baseline_row(vec![
                text("txt00001", "Hello", 32.0, 1.5),
                without_line_height("txt00002", "Hello", 14.0),
                without_line_height("txt00003", "Hello", 12.0),
                without_line_height("txt00004", "Hello", 10.0),
                rect("a0000001", 10.0, 10.0),
            ]),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 100.0]),
                ("txt00001", [0.0, 0.0, 77.125, 48.0]),
                ("txt00002", [77.125, 22.5, 33.7421875, 16.9375]),
                ("txt00003", [110.8671875, 24.0, 28.921875, 14.515625]),
                ("txt00004", [139.7890625, 26.0, 24.1015625, 12.09375]),
                ("a0000001", [163.890625, 25.5, 10.0, 10.0]),
            ],
        ),
        rounded_case(
            "texts of 13 px with lineHeight 1.5, 14 px without one and 24 px with lineHeight 1.2, and a 10 px box, on their baselines",
            baseline_row(vec![
                text("txt00001", "Hello", 13.0, 1.5),
                without_line_height("txt00002", "Hello", 14.0),
                text("txt00003", "Hello", 24.0, 1.2),
                rect("a0000001", 10.0, 10.0),
            ]),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 100.0]),
                ("txt00001", [0.0, 8.5, 31.3359375, 19.5]),
                ("txt00002", [31.3359375, 10.0, 33.7421875, 16.9375]),
                ("txt00003", [65.078125, 0.0, 57.84375, 28.796875]),
                ("a0000001", [122.921875, 13.0, 10.0, 10.0]),
            ],
        ),
        rounded_case(
            "16 px texts without a lineHeight and with lineHeight 1.5 and 1, on their baselines",
            baseline_row(vec![
                without_line_height("txt00001", "Hello", 16.0),
                text("txt00002", "Hello", 16.0, 1.5),
                text("txt00003", "Hello", 16.0, 1.0),
            ]),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 100.0]),
                ("txt00001", [0.0, 2.5, 38.5625, 19.359375]),
                ("txt00002", [38.5625, 0.0, 38.5625, 24.0]),
                ("txt00003", [77.125, 4.0, 38.5625, 16.0]),
            ],
        ),
        rounded_case(
            "12 and 16 px texts without a lineHeight, on their baselines",
            baseline_row(vec![
                without_line_height("txt00001", "Hello", 12.0),
                without_line_height("txt00002", "Hello", 16.0),
            ]),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 100.0]),
                ("txt00001", [0.0, 3.5, 28.921875, 14.515625]),
                ("txt00002", [28.921875, 0.0, 38.5625, 19.359375]),
            ],
        ),
        rounded_case(
            "10 and 32 px texts without a lineHeight, on their baselines",
            baseline_row(vec![
                without_line_height("txt00001", "Hello", 10.0),
                without_line_height("txt00002", "Hello", 32.0),
            ]),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 100.0]),
                ("txt00001", [0.0, 21.5, 24.1015625, 12.09375]),
                ("txt00002", [24.1015625, 0.0, 77.125, 38.71875]),
            ],
        ),
    ]
}

// The CSS that each layout property maps to.

/// Three fixed boxes of different sizes: 50 x 20, 80 x 30 and 30 x 40.
fn three_rects() -> Vec<Value> {
    vec![
        rect("a0000001", 50.0, 20.0),
        rect("b0000001", 80.0, 30.0),
        rect("c0000001", 30.0, 40.0),
    ]
}

/// Five fixed 80 x 30 boxes.
fn five_rects() -> Vec<Value> {
    ["a0000001", "b0000001", "c0000001", "d0000001", "e0000001"]
        .iter()
        .map(|id| rect(id, 80.0, 30.0))
        .collect()
}

/// An empty child that fills its grid area.
fn filling(id: &str) -> Value {
    sized(id, json!("fill"), json!("fill"))
}

/// A copy of `json` with each of `bounds`, such as `("maxWidth", 100)`, set.
fn bounded(json: Value, bounds: &[(&str, u32)]) -> Value {
    bounds
        .iter()
        .fold(json, |json, (key, value)| with(&json, key, json!(value)))
}

/// A copy of `json` placed on grid lines: `column` and `row` are
/// `{"start": <line>, "span": <tracks>}`.
fn placed(json: &Value, column: Value, row: Value) -> Value {
    let json = if column.is_null() {
        json.clone()
    } else {
        with(json, "gridColumn", column)
    };
    if row.is_null() {
        json
    } else {
        with(&json, "gridRow", row)
    }
}

/// One case for each value of each property of a layout, and for each way that
/// a parent sizes and places a child: every row of the tables of
/// `docs/formats/layout.md`.
pub fn css_mapping() -> Vec<Case> {
    let mut cases = Vec::new();
    for (direction, boxes) in [
        (
            "row",
            &[
                ("brd00001", [0.0, 0.0, 400.0, 300.0]),
                ("a0000001", [0.0, 0.0, 50.0, 20.0]),
                ("b0000001", [50.0, 0.0, 80.0, 30.0]),
                ("c0000001", [130.0, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            "column",
            &[
                ("brd00001", [0.0, 0.0, 400.0, 300.0]),
                ("a0000001", [0.0, 0.0, 50.0, 20.0]),
                ("b0000001", [0.0, 20.0, 80.0, 30.0]),
                ("c0000001", [0.0, 50.0, 30.0, 40.0]),
            ],
        ),
        (
            "row-reverse",
            &[
                ("brd00001", [0.0, 0.0, 400.0, 300.0]),
                ("a0000001", [350.0, 0.0, 50.0, 20.0]),
                ("b0000001", [270.0, 0.0, 80.0, 30.0]),
                ("c0000001", [240.0, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            "column-reverse",
            &[
                ("brd00001", [0.0, 0.0, 400.0, 300.0]),
                ("a0000001", [0.0, 280.0, 50.0, 20.0]),
                ("b0000001", [0.0, 250.0, 80.0, 30.0]),
                ("c0000001", [0.0, 210.0, 30.0, 40.0]),
            ],
        ),
    ] {
        cases.push(case(
            format!("three boxes in a 400 x 300 {direction}"),
            board(
                "brd00001",
                fixed(400.0),
                fixed(300.0),
                flex(json!({"direction": direction})),
                three_rects(),
            ),
            boxes,
        ));
    }
    for (wrap, boxes) in [
        (
            "wrap",
            &[
                ("brd00001", [0.0, 0.0, 200.0, 200.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [100.0, 0.0, 80.0, 30.0]),
                ("c0000001", [0.0, 70.0, 80.0, 30.0]),
                ("d0000001", [100.0, 70.0, 80.0, 30.0]),
                ("e0000001", [0.0, 140.0, 80.0, 30.0]),
            ],
        ),
        (
            "wrap-reverse",
            &[
                ("brd00001", [0.0, 0.0, 200.0, 200.0]),
                ("a0000001", [0.0, 170.0, 80.0, 30.0]),
                ("b0000001", [100.0, 170.0, 80.0, 30.0]),
                ("c0000001", [0.0, 100.0, 80.0, 30.0]),
                ("d0000001", [100.0, 100.0, 80.0, 30.0]),
                ("e0000001", [0.0, 30.0, 80.0, 30.0]),
            ],
        ),
        (
            "nowrap",
            &[
                ("brd00001", [0.0, 0.0, 200.0, 200.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [100.0, 0.0, 80.0, 30.0]),
                ("c0000001", [200.0, 0.0, 80.0, 30.0]),
                ("d0000001", [300.0, 0.0, 80.0, 30.0]),
                ("e0000001", [400.0, 0.0, 80.0, 30.0]),
            ],
        ),
    ] {
        cases.push(case(
            format!("five boxes in a 200 x 200 row with wrap {wrap} and gaps of 10 and 20"),
            board(
                "brd00001",
                fixed(200.0),
                fixed(200.0),
                flex(json!({"wrap": wrap, "gap": {"row": 10, "column": 20}})),
                five_rects(),
            ),
            boxes,
        ));
    }
    cases.push(case(
        "five boxes wrapping in a 300 x 100 column, whose lines share the free width",
        board(
            "brd00001",
            fixed(300.0),
            fixed(100.0),
            flex(json!({"direction": "column", "wrap": "wrap", "gap": {"row": 10, "column": 20}})),
            five_rects(),
        ),
        &[
            ("brd00001", [0.0, 0.0, 300.0, 100.0]),
            ("a0000001", [0.0, 0.0, 80.0, 30.0]),
            ("b0000001", [0.0, 40.0, 80.0, 30.0]),
            ("c0000001", [106.6640625, 0.0, 80.0, 30.0]),
            ("d0000001", [106.6640625, 40.0, 80.0, 30.0]),
            ("e0000001", [213.3359375, 0.0, 80.0, 30.0]),
        ],
    ));
    cases.push(case(
        "a padded 400 x 300 row packed to the end of both axes",
        board(
            "brd00001",
            fixed(400.0),
            fixed(300.0),
            flex(json!({
                "padding": {"top": 10, "right": 20, "bottom": 30, "left": 40},
                "justifyContent": "end",
                "alignItems": "end"
            })),
            three_rects(),
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 300.0]),
            ("a0000001", [220.0, 250.0, 50.0, 20.0]),
            ("b0000001", [270.0, 240.0, 80.0, 30.0]),
            ("c0000001", [350.0, 230.0, 30.0, 40.0]),
        ],
    ));
    for (align, boxes) in [
        (
            None,
            &[
                ("brd00001", [0.0, 0.0, 300.0, 100.0]),
                ("a0000001", [0.0, 0.0, 50.0, 20.0]),
                ("txt00001", [50.0, 0.0, 38.5625, 100.0]),
                ("b0000001", [88.5625, 0.0, 40.0, 100.0]),
                ("b1000001", [88.5625, 0.0, 10.0, 10.0]),
            ],
        ),
        (
            Some("start"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 100.0]),
                ("a0000001", [0.0, 0.0, 50.0, 20.0]),
                ("txt00001", [50.0, 0.0, 38.5625, 24.0]),
                ("b0000001", [88.5625, 0.0, 40.0, 10.0]),
                ("b1000001", [88.5625, 0.0, 10.0, 10.0]),
            ],
        ),
        (
            Some("center"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 100.0]),
                ("a0000001", [0.0, 40.0, 50.0, 20.0]),
                ("txt00001", [50.0, 38.0, 38.5625, 24.0]),
                ("b0000001", [88.5625, 45.0, 40.0, 10.0]),
                ("b1000001", [88.5625, 45.0, 10.0, 10.0]),
            ],
        ),
        (
            Some("end"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 100.0]),
                ("a0000001", [0.0, 80.0, 50.0, 20.0]),
                ("txt00001", [50.0, 76.0, 38.5625, 24.0]),
                ("b0000001", [88.5625, 90.0, 40.0, 10.0]),
                ("b1000001", [88.5625, 90.0, 10.0, 10.0]),
            ],
        ),
        (
            Some("stretch"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 100.0]),
                ("a0000001", [0.0, 0.0, 50.0, 20.0]),
                ("txt00001", [50.0, 0.0, 38.5625, 100.0]),
                ("b0000001", [88.5625, 0.0, 40.0, 100.0]),
                ("b1000001", [88.5625, 0.0, 10.0, 10.0]),
            ],
        ),
        (
            Some("baseline"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 100.0]),
                ("a0000001", [0.0, 0.0, 50.0, 20.0]),
                ("txt00001", [50.0, 2.5, 38.5625, 24.0]),
                ("b0000001", [88.5625, 10.0, 40.0, 10.0]),
                ("b1000001", [88.5625, 10.0, 10.0, 10.0]),
            ],
        ),
    ] {
        let layout = match align {
            Some(align) => flex(json!({"alignItems": align})),
            None => flex(json!({})),
        };
        cases.push(rounded_case(
            format!("a box, a text and a hug frame in a 300 x 100 row with alignItems {align:?}"),
            board(
                "brd00001",
                fixed(300.0),
                fixed(100.0),
                layout,
                vec![
                    rect("a0000001", 50.0, 20.0),
                    text("txt00001", "Hello", 16.0, 1.5),
                    with(
                        &hugging("b0000001", "b1000001", 10.0, 10.0),
                        "width",
                        fixed(40.0),
                    ),
                ],
            ),
            boxes,
        ));
    }
    for (align, boxes) in [
        (
            "start",
            &[
                ("brd00001", [0.0, 0.0, 300.0, 200.0]),
                ("a0000001", [0.0, 0.0, 50.0, 20.0]),
                ("b0000001", [0.0, 20.0, 10.0, 30.0]),
                ("b1000001", [0.0, 20.0, 10.0, 10.0]),
            ],
        ),
        (
            "center",
            &[
                ("brd00001", [0.0, 0.0, 300.0, 200.0]),
                ("a0000001", [125.0, 0.0, 50.0, 20.0]),
                ("b0000001", [145.0, 20.0, 10.0, 30.0]),
                ("b1000001", [145.0, 20.0, 10.0, 10.0]),
            ],
        ),
        (
            "end",
            &[
                ("brd00001", [0.0, 0.0, 300.0, 200.0]),
                ("a0000001", [250.0, 0.0, 50.0, 20.0]),
                ("b0000001", [290.0, 20.0, 10.0, 30.0]),
                ("b1000001", [290.0, 20.0, 10.0, 10.0]),
            ],
        ),
        (
            "stretch",
            &[
                ("brd00001", [0.0, 0.0, 300.0, 200.0]),
                ("a0000001", [0.0, 0.0, 50.0, 20.0]),
                ("b0000001", [0.0, 20.0, 300.0, 30.0]),
                ("b1000001", [0.0, 20.0, 10.0, 10.0]),
            ],
        ),
    ] {
        cases.push(case(
            format!("a box and a hug frame in a 300 x 200 column with alignItems {align}"),
            board(
                "brd00001",
                fixed(300.0),
                fixed(200.0),
                flex(json!({"direction": "column", "alignItems": align})),
                vec![
                    rect("a0000001", 50.0, 20.0),
                    with(
                        &hugging("b0000001", "b1000001", 10.0, 10.0),
                        "height",
                        fixed(30.0),
                    ),
                ],
            ),
            boxes,
        ));
    }
    for (align, boxes) in [
        (
            "start",
            &[
                ("brd00001", [0.0, 0.0, 200.0, 200.0]),
                ("a0000001", [0.0, 100.0, 80.0, 20.0]),
                ("b0000001", [100.0, 100.0, 80.0, 40.0]),
                ("c0000001", [0.0, 0.0, 80.0, 30.0]),
                ("d0000001", [100.0, 0.0, 80.0, 10.0]),
            ],
        ),
        (
            "end",
            &[
                ("brd00001", [0.0, 0.0, 200.0, 200.0]),
                ("a0000001", [0.0, 180.0, 80.0, 20.0]),
                ("b0000001", [100.0, 160.0, 80.0, 40.0]),
                ("c0000001", [0.0, 60.0, 80.0, 30.0]),
                ("d0000001", [100.0, 80.0, 80.0, 10.0]),
            ],
        ),
    ] {
        cases.push(case(
            format!("four boxes wrapping in reverse in a 200 x 200 row with alignItems {align}"),
            board(
                "brd00001",
                fixed(200.0),
                fixed(200.0),
                flex(json!({
                    "wrap": "wrap-reverse",
                    "alignItems": align,
                    "gap": {"row": 10, "column": 20}
                })),
                vec![
                    rect("a0000001", 80.0, 20.0),
                    rect("b0000001", 80.0, 40.0),
                    rect("c0000001", 80.0, 30.0),
                    rect("d0000001", 80.0, 10.0),
                ],
            ),
            boxes,
        ));
    }
    for (justify, boxes) in [
        (
            None,
            &[
                ("brd00001", [0.0, 0.0, 300.0, 50.0]),
                ("a0000001", [10.0, 0.0, 50.0, 20.0]),
                ("b0000001", [75.0, 0.0, 80.0, 30.0]),
                ("c0000001", [170.0, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            Some("start"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 50.0]),
                ("a0000001", [10.0, 0.0, 50.0, 20.0]),
                ("b0000001", [75.0, 0.0, 80.0, 30.0]),
                ("c0000001", [170.0, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            Some("center"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 50.0]),
                ("a0000001", [50.0, 0.0, 50.0, 20.0]),
                ("b0000001", [115.0, 0.0, 80.0, 30.0]),
                ("c0000001", [210.0, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            Some("end"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 50.0]),
                ("a0000001", [90.0, 0.0, 50.0, 20.0]),
                ("b0000001", [155.0, 0.0, 80.0, 30.0]),
                ("c0000001", [250.0, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            Some("space-between"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 50.0]),
                ("a0000001", [10.0, 0.0, 50.0, 20.0]),
                ("b0000001", [115.0, 0.0, 80.0, 30.0]),
                ("c0000001", [250.0, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            Some("space-around"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 50.0]),
                ("a0000001", [23.328125, 0.0, 50.0, 20.0]),
                ("b0000001", [114.9921875, 0.0, 80.0, 30.0]),
                ("c0000001", [236.6640625, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            Some("space-evenly"),
            &[
                ("brd00001", [0.0, 0.0, 300.0, 50.0]),
                ("a0000001", [30.0, 0.0, 50.0, 20.0]),
                ("b0000001", [115.0, 0.0, 80.0, 30.0]),
                ("c0000001", [230.0, 0.0, 30.0, 40.0]),
            ],
        ),
    ] {
        let layout = match justify {
            Some(justify) => flex(json!({
                "justifyContent": justify,
                "gap": {"column": 15},
                "padding": {"left": 10, "right": 20}
            })),
            None => flex(json!({"gap": {"column": 15}, "padding": {"left": 10, "right": 20}})),
        };
        cases.push(case(
            format!("three boxes in a padded 300 px row with gaps and justifyContent {justify:?}"),
            board("brd00001", fixed(300.0), fixed(50.0), layout, three_rects()),
            boxes,
        ));
    }
    for (direction, justify, boxes) in [
        (
            "row-reverse",
            "start",
            &[
                ("brd00001", [0.0, 0.0, 400.0, 300.0]),
                ("a0000001", [110.0, 0.0, 50.0, 20.0]),
                ("b0000001", [30.0, 0.0, 80.0, 30.0]),
                ("c0000001", [0.0, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            "row-reverse",
            "end",
            &[
                ("brd00001", [0.0, 0.0, 400.0, 300.0]),
                ("a0000001", [350.0, 0.0, 50.0, 20.0]),
                ("b0000001", [270.0, 0.0, 80.0, 30.0]),
                ("c0000001", [240.0, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            "column-reverse",
            "start",
            &[
                ("brd00001", [0.0, 0.0, 400.0, 300.0]),
                ("a0000001", [0.0, 70.0, 50.0, 20.0]),
                ("b0000001", [0.0, 40.0, 80.0, 30.0]),
                ("c0000001", [0.0, 0.0, 30.0, 40.0]),
            ],
        ),
        (
            "column-reverse",
            "end",
            &[
                ("brd00001", [0.0, 0.0, 400.0, 300.0]),
                ("a0000001", [0.0, 280.0, 50.0, 20.0]),
                ("b0000001", [0.0, 250.0, 80.0, 30.0]),
                ("c0000001", [0.0, 210.0, 30.0, 40.0]),
            ],
        ),
    ] {
        cases.push(case(
            format!("three boxes in a {direction} with justifyContent {justify}"),
            board(
                "brd00001",
                fixed(400.0),
                fixed(300.0),
                flex(json!({"direction": direction, "justifyContent": justify})),
                three_rects(),
            ),
            boxes,
        ));
    }
    // The main axis: fixed never shrinks, hug shrinks to its content's narrowest
    // size, and fill children share the free space.
    cases.push(case(
        "three 100 px boxes in a 200 px row, which never shrinks them",
        board(
            "brd00001",
            fixed(200.0),
            fixed(200.0),
            flex(json!({"gap": {"row": 7, "column": 3}})),
            vec![
                rect("a0000001", 100.0, 20.0),
                rect("b0000001", 100.0, 20.0),
                rect("c0000001", 100.0, 20.0),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 200.0, 200.0]),
            ("a0000001", [0.0, 0.0, 100.0, 20.0]),
            ("b0000001", [103.0, 0.0, 100.0, 20.0]),
            ("c0000001", [206.0, 0.0, 100.0, 20.0]),
        ],
    ));
    cases.push(case(
        "a 100 px box and two fill children in a 500 px row with 20 px gaps",
        board(
            "brd00001",
            fixed(500.0),
            fixed(300.0),
            flex(json!({"alignItems": "start", "gap": {"column": 20}})),
            vec![
                rect("a0000001", 100.0, 20.0),
                sized("b0000001", json!("fill"), fixed(20.0)),
                sized("c0000001", json!("fill"), fixed(20.0)),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 500.0, 300.0]),
            ("a0000001", [0.0, 0.0, 100.0, 20.0]),
            ("b0000001", [120.0, 0.0, 180.0, 20.0]),
            ("c0000001", [320.0, 0.0, 180.0, 20.0]),
        ],
    ));
    cases.push(case(
        "a 100 px box and two fill children in a 300 px column",
        board(
            "brd00001",
            fixed(100.0),
            fixed(300.0),
            flex(json!({"direction": "column", "alignItems": "start"})),
            vec![
                rect("a0000001", 20.0, 100.0),
                sized("b0000001", fixed(20.0), json!("fill")),
                sized("c0000001", fixed(20.0), json!("fill")),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 100.0, 300.0]),
            ("a0000001", [0.0, 0.0, 20.0, 100.0]),
            ("b0000001", [0.0, 100.0, 20.0, 100.0]),
            ("c0000001", [0.0, 200.0, 20.0, 100.0]),
        ],
    ));
    cases.push(case(
        "a fill height in a centered row with padding above and below",
        board(
            "brd00001",
            fixed(300.0),
            fixed(100.0),
            flex(json!({"alignItems": "center", "padding": {"top": 10, "bottom": 20}})),
            vec![
                sized("a0000001", fixed(50.0), json!("fill")),
                rect("b0000001", 50.0, 20.0),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 300.0, 100.0]),
            ("a0000001", [0.0, 10.0, 50.0, 70.0]),
            ("b0000001", [50.0, 35.0, 50.0, 20.0]),
        ],
    ));
    cases.push(case(
        "a fill width in an end-aligned column padded left and right",
        board(
            "brd00001",
            fixed(300.0),
            fixed(200.0),
            flex(json!({
                "direction": "column",
                "alignItems": "end",
                "padding": {"left": 10, "right": 30}
            })),
            vec![
                sized("c0000001", json!("fill"), fixed(20.0)),
                rect("d0000001", 50.0, 20.0),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 300.0, 200.0]),
            ("c0000001", [10.0, 0.0, 260.0, 20.0]),
            ("d0000001", [220.0, 20.0, 50.0, 20.0]),
        ],
    ));
    cases.push(case(
        "a hug row and a hug column that hold boxes, with gaps and padding",
        board(
            "brd00001",
            fixed(500.0),
            fixed(200.0),
            flex(json!({"alignItems": "start"})),
            vec![
                frame(
                    "frm00001",
                    json!("hug"),
                    json!("hug"),
                    flex(json!({
                        "gap": {"column": 5},
                        "padding": {"top": 10, "right": 20, "bottom": 30, "left": 40}
                    })),
                    vec![rect("a0000001", 50.0, 20.0), rect("b0000001", 30.0, 40.0)],
                ),
                frame(
                    "frm00002",
                    json!("hug"),
                    json!("hug"),
                    flex(json!({"direction": "column", "gap": {"row": 5}})),
                    vec![rect("c0000001", 50.0, 20.0), rect("d0000001", 30.0, 40.0)],
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 500.0, 200.0]),
            ("frm00001", [0.0, 0.0, 145.0, 80.0]),
            ("a0000001", [40.0, 10.0, 50.0, 20.0]),
            ("b0000001", [95.0, 10.0, 30.0, 40.0]),
            ("frm00002", [145.0, 0.0, 50.0, 65.0]),
            ("c0000001", [145.0, 0.0, 50.0, 20.0]),
            ("d0000001", [145.0, 25.0, 30.0, 40.0]),
        ],
    ));
    cases.push(case(
        "a board that hugs a row with padding 10 20 30 40",
        hug_board(
            flex(json!({"padding": {"top": 10, "right": 20, "bottom": 30, "left": 40}})),
            three_rects(),
        ),
        &[
            ("brd00001", [0.0, 0.0, 220.0, 80.0]),
            ("a0000001", [40.0, 10.0, 50.0, 20.0]),
            ("b0000001", [90.0, 10.0, 80.0, 30.0]),
            ("c0000001", [170.0, 10.0, 30.0, 40.0]),
        ],
    ));
    // Min and max sizes bound every sizing, and a min beats a max.
    cases.push(case(
        "fill children with max and min sizes in rows and a column",
        board(
            "brd00001",
            fixed(400.0),
            fixed(400.0),
            flex(json!({"alignItems": "start"})),
            vec![
                bounded(
                    sized("a0000001", json!("fill"), fixed(20.0)),
                    &[("maxWidth", 150)],
                ),
                bounded(
                    sized("b0000001", json!("fill"), fixed(20.0)),
                    &[("minWidth", 40)],
                ),
                bounded(
                    sized("c0000001", fixed(20.0), json!("fill")),
                    &[("maxHeight", 100)],
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 400.0]),
            ("a0000001", [0.0, 0.0, 150.0, 20.0]),
            ("b0000001", [150.0, 0.0, 230.0, 20.0]),
            ("c0000001", [380.0, 0.0, 20.0, 100.0]),
        ],
    ));
    cases.push(case(
        "fixed widths with max and min sizes, and a min that beats a max",
        board(
            "brd00001",
            fixed(700.0),
            fixed(100.0),
            flex(json!({"alignItems": "start"})),
            vec![
                bounded(rect("a0000001", 300.0, 20.0), &[("maxWidth", 200)]),
                bounded(rect("b0000001", 100.0, 20.0), &[("minWidth", 150)]),
                bounded(
                    rect("c0000001", 100.0, 20.0),
                    &[("minWidth", 250), ("maxWidth", 200)],
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 700.0, 100.0]),
            ("a0000001", [0.0, 0.0, 200.0, 20.0]),
            ("b0000001", [200.0, 0.0, 150.0, 20.0]),
            ("c0000001", [350.0, 0.0, 250.0, 20.0]),
        ],
    ));
    cases.push(rounded_case(
        "texts that hug their width with min and max widths, a min beating a max",
        board(
            "brd00001",
            fixed(600.0),
            json!("hug"),
            flex(json!({"alignItems": "start"})),
            vec![
                bounded(
                    text("txt00001", "word word", 16.0, 1.5),
                    &[("minWidth", 150), ("maxWidth", 100)],
                ),
                bounded(
                    text("txt00002", SIX_WORDS, 16.0, 1.5),
                    &[("minWidth", 60), ("maxWidth", 90)],
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 600.0, 72.0]),
            ("txt00001", [0.0, 0.0, 150.0, 24.0]),
            ("txt00002", [150.0, 0.0, 90.0, 72.0]),
        ],
    ));
    cases.push(case(
        "hug frames and a stretched frame with min and max sizes",
        board(
            "brd00001",
            fixed(400.0),
            fixed(200.0),
            flex(json!({})),
            vec![
                bounded(
                    hugging("a0000001", "a1000001", 10.0, 10.0),
                    &[("minWidth", 60), ("minHeight", 50)],
                ),
                bounded(
                    hugging("b0000001", "b1000001", 80.0, 30.0),
                    &[("maxWidth", 60), ("maxHeight", 20)],
                ),
                bounded(
                    with(
                        &hugging("c0000001", "c1000001", 10.0, 10.0),
                        "width",
                        fixed(50.0),
                    ),
                    &[("maxHeight", 40)],
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 200.0]),
            ("a0000001", [0.0, 0.0, 60.0, 200.0]),
            ("a1000001", [0.0, 0.0, 10.0, 10.0]),
            ("b0000001", [60.0, 0.0, 60.0, 20.0]),
            ("b1000001", [60.0, 0.0, 80.0, 30.0]),
            ("c0000001", [120.0, 0.0, 50.0, 40.0]),
            ("c1000001", [120.0, 0.0, 10.0, 10.0]),
        ],
    ));
    cases.push(case(
        "a box and three fill children sharing the other 100 px of a 110.25 px row",
        board(
            "brd00001",
            fixed(110.25),
            fixed(30.0),
            flex(json!({})),
            vec![
                rect("a0000001", 10.25, 20.75),
                sized("b0000001", json!("fill"), fixed(30.0)),
                sized("c0000001", json!("fill"), fixed(30.0)),
                sized("d0000001", json!("fill"), fixed(30.0)),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 110.25, 30.0]),
            ("a0000001", [0.0, 0.0, 10.25, 20.75]),
            ("b0000001", [10.25, 0.0, 33.328125, 30.0]),
            ("c0000001", [43.578125, 0.0, 33.3359375, 30.0]),
            ("d0000001", [76.9140625, 0.0, 33.3359375, 30.0]),
        ],
    ));
    // Grids.
    cases.push(case(
        "a 460 px grid of 100 px, 1fr and 2fr columns and a 50 px and an auto row, with gaps and padding",
        board(
            "brd00001",
            fixed(460.0),
            json!("hug"),
            grid(json!({
                "columns": [{"fixed": 100}, {"fr": 1}, {"fr": 2}],
                "rows": [{"fixed": 50}, "auto"],
                "gap": {"row": 10, "column": 20},
                "padding": {"top": 10, "right": 10, "bottom": 10, "left": 10}
            })),
            vec![
                filling("a0000001"),
                filling("b0000001"),
                filling("c0000001"),
                rect("d0000001", 60.0, 35.0),
                filling("e0000001"),
                filling("f0000001"),
            ],
        ),
        &[
    ("brd00001", [0.0, 0.0, 460.0, 115.0]),
    ("a0000001", [10.0, 10.0, 100.0, 50.0]),
    ("b0000001", [130.0, 10.0, 100.0, 50.0]),
    ("c0000001", [250.0, 10.0, 200.0, 50.0]),
    ("d0000001", [10.0, 70.0, 60.0, 35.0]),
    ("e0000001", [130.0, 70.0, 100.0, 35.0]),
    ("f0000001", [250.0, 70.0, 200.0, 35.0]),
],
    ));
    cases.push(case(
        "a 300 x 100 grid of an auto and a 100 px column, whose auto track stretches",
        board(
            "brd00001",
            fixed(300.0),
            fixed(100.0),
            grid(json!({"columns": ["auto", {"fixed": 100}], "rows": ["auto"]})),
            vec![rect("a0000001", 70.0, 20.0), filling("b0000001")],
        ),
        &[
            ("brd00001", [0.0, 0.0, 300.0, 100.0]),
            ("a0000001", [0.0, 0.0, 70.0, 20.0]),
            ("b0000001", [200.0, 0.0, 100.0, 100.0]),
        ],
    ));
    cases.push(case(
        "children on the grid lines of a 3 x 3 grid with gaps",
        board(
            "brd00001",
            fixed(320.0),
            fixed(320.0),
            grid(json!({
                "columns": [{"fixed": 100}, {"fixed": 100}, {"fixed": 100}],
                "rows": [{"fixed": 100}, {"fixed": 100}, {"fixed": 100}],
                "gap": {"row": 10, "column": 10}
            })),
            vec![
                placed(
                    &filling("a0000001"),
                    json!({"start": 2, "span": 2}),
                    json!({"start": 1, "span": 3}),
                ),
                placed(
                    &filling("b0000001"),
                    json!({"start": 1}),
                    json!({"start": 2}),
                ),
                placed(
                    &filling("c0000001"),
                    json!({"start": 1, "span": 1}),
                    json!({"start": 3}),
                ),
                filling("d0000001"),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 320.0, 320.0]),
            ("a0000001", [110.0, 0.0, 210.0, 320.0]),
            ("b0000001", [0.0, 110.0, 100.0, 100.0]),
            ("c0000001", [0.0, 220.0, 100.0, 100.0]),
            ("d0000001", [0.0, 0.0, 100.0, 100.0]),
        ],
    ));
    cases.push(case(
        "children on negative grid lines",
        board(
            "brd00001",
            fixed(300.0),
            fixed(100.0),
            grid(json!({
                "columns": [{"fixed": 100}, {"fixed": 100}, {"fixed": 100}],
                "rows": [{"fixed": 100}]
            })),
            vec![
                placed(
                    &filling("a0000001"),
                    json!({"start": -2}),
                    json!({"start": 1}),
                ),
                placed(
                    &filling("b0000001"),
                    json!({"start": -3}),
                    json!({"start": 1}),
                ),
                placed(
                    &filling("c0000001"),
                    json!({"start": -4}),
                    json!({"start": 1}),
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 300.0, 100.0]),
            ("a0000001", [200.0, 0.0, 100.0, 100.0]),
            ("b0000001", [100.0, 0.0, 100.0, 100.0]),
            ("c0000001", [0.0, 0.0, 100.0, 100.0]),
        ],
    ));
    cases.push(case(
        "children that flow into the next free cells and a new implicit row",
        board(
            "brd00001",
            fixed(200.0),
            json!("hug"),
            grid(json!({
                "columns": [{"fixed": 100}, {"fixed": 100}],
                "rows": [{"fixed": 100}, {"fixed": 100}]
            })),
            vec![
                placed(
                    &filling("a0000001"),
                    json!({"start": 2}),
                    json!({"start": 1}),
                ),
                filling("b0000001"),
                filling("c0000001"),
                filling("d0000001"),
                placed(
                    &sized("e0000001", json!("fill"), fixed(30.0)),
                    json!({"span": 2}),
                    Value::Null,
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 200.0, 230.0]),
            ("a0000001", [100.0, 0.0, 100.0, 100.0]),
            ("b0000001", [0.0, 0.0, 100.0, 100.0]),
            ("c0000001", [0.0, 100.0, 100.0, 100.0]),
            ("d0000001", [100.0, 100.0, 100.0, 100.0]),
            ("e0000001", [0.0, 200.0, 200.0, 30.0]),
        ],
    ));
    let two_cells = || {
        grid(json!({
            "columns": [{"fixed": 200}, {"fixed": 200}],
            "rows": [{"fixed": 100}]
        }))
    };
    cases.push(case(
        "fixed and hug children at the start of their grid areas, and fill children filling them",
        board(
            "brd00001",
            fixed(400.0),
            fixed(100.0),
            two_cells(),
            vec![
                rect("a0000001", 50.0, 30.0),
                hugging("b0000001", "b1000001", 40.0, 20.0),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 100.0]),
            ("a0000001", [0.0, 0.0, 50.0, 30.0]),
            ("b0000001", [200.0, 0.0, 40.0, 20.0]),
            ("b1000001", [200.0, 0.0, 40.0, 20.0]),
        ],
    ));
    cases.push(case(
        "a child that fills both axes, and one that fills its width and hugs a box, in grid areas",
        board(
            "brd00001",
            fixed(400.0),
            fixed(100.0),
            two_cells(),
            vec![
                filling("c0000001"),
                with(
                    &hugging("d0000001", "d1000001", 40.0, 20.0),
                    "width",
                    json!("fill"),
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 100.0]),
            ("c0000001", [0.0, 0.0, 200.0, 100.0]),
            ("d0000001", [200.0, 0.0, 200.0, 20.0]),
            ("d1000001", [200.0, 0.0, 40.0, 20.0]),
        ],
    ));
    cases.push(case(
        "a 60 px wide child filling its height, and a fill child with max sizes, in grid areas",
        board(
            "brd00001",
            fixed(400.0),
            fixed(100.0),
            two_cells(),
            vec![
                sized("e0000001", fixed(60.0), json!("fill")),
                bounded(filling("f0000001"), &[("maxWidth", 120), ("maxHeight", 50)]),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 100.0]),
            ("e0000001", [0.0, 0.0, 60.0, 100.0]),
            ("f0000001", [200.0, 0.0, 120.0, 50.0]),
        ],
    ));
    // Positioned children.
    cases.push(case(
        "children of a `none` layout at their x and y, and nested frames",
        board(
            "brd00001",
            fixed(400.0),
            fixed(300.0),
            none(),
            vec![
                at(&rect("a0000001", 50.0, 40.0), 10.0, 20.0),
                at(&rect("b0000001", 20.0, 20.0), -5.0, 30.5),
                rect("c0000001", 10.0, 10.0),
                at(&hugging("d0000001", "d1000001", 30.0, 20.0), 100.0, 100.0),
                at(
                    &frame(
                        "f0000001",
                        fixed(80.0),
                        fixed(80.0),
                        none(),
                        vec![at(&rect("g0000001", 5.0, 5.0), 10.0, 10.0)],
                    ),
                    300.0,
                    200.0,
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 300.0]),
            ("a0000001", [10.0, 20.0, 50.0, 40.0]),
            ("b0000001", [-5.0, 30.5, 20.0, 20.0]),
            ("c0000001", [0.0, 0.0, 10.0, 10.0]),
            ("d0000001", [100.0, 100.0, 30.0, 20.0]),
            ("d1000001", [100.0, 100.0, 30.0, 20.0]),
            ("f0000001", [300.0, 200.0, 80.0, 80.0]),
            ("g0000001", [310.0, 210.0, 5.0, 5.0]),
        ],
    ));
    cases.push(case(
        "children of a `none` layout that fill an axis, with min and max sizes",
        board(
            "brd00001",
            fixed(400.0),
            fixed(300.0),
            none(),
            vec![
                at(&sized("a0000001", json!("fill"), json!("fill")), 50.0, 40.0),
                at(&sized("b0000001", json!("fill"), fixed(20.0)), 10.0, 0.0),
                at(&sized("c0000001", fixed(30.0), json!("fill")), 0.0, 100.0),
                sized("d0000001", json!("fill"), json!("fill")),
                at(
                    &bounded(
                        sized("e0000001", json!("fill"), fixed(20.0)),
                        &[("maxWidth", 100)],
                    ),
                    50.0,
                    200.0,
                ),
                at(
                    &bounded(
                        sized("f0000001", json!("fill"), fixed(20.0)),
                        &[("minWidth", 500)],
                    ),
                    50.0,
                    250.0,
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 300.0]),
            ("a0000001", [50.0, 40.0, 350.0, 260.0]),
            ("b0000001", [10.0, 0.0, 390.0, 20.0]),
            ("c0000001", [0.0, 100.0, 30.0, 200.0]),
            ("d0000001", [0.0, 0.0, 400.0, 300.0]),
            ("e0000001", [50.0, 200.0, 100.0, 20.0]),
            ("f0000001", [50.0, 250.0, 500.0, 20.0]),
        ],
    ));
    cases.push(case(
        "absolute children of a padded flex layout, which take no space and sit at x and y from its edge",
        board(
            "brd00001",
            fixed(300.0),
            fixed(200.0),
            flex(json!({
                "gap": {"column": 10},
                "padding": {"top": 20, "right": 20, "bottom": 20, "left": 20}
            })),
            vec![
                rect("a0000001", 50.0, 50.0),
                absolute(&rect("b0000001", 40.0, 40.0), 5.0, 7.0),
                rect("c0000001", 60.0, 50.0),
                absolute(&hugging("d0000001", "d1000001", 30.0, 20.0), 200.0, 150.0),
            ],
        ),
        &[
    ("brd00001", [0.0, 0.0, 300.0, 200.0]),
    ("a0000001", [20.0, 20.0, 50.0, 50.0]),
    ("b0000001", [5.0, 7.0, 40.0, 40.0]),
    ("c0000001", [80.0, 20.0, 60.0, 50.0]),
    ("d0000001", [200.0, 150.0, 30.0, 20.0]),
    ("d1000001", [200.0, 150.0, 30.0, 20.0]),
],
    ));
    cases.push(case(
        "absolute children that fill an axis of a padded flex layout",
        board(
            "brd00001",
            fixed(300.0),
            fixed(200.0),
            flex(json!({"padding": {"top": 20, "right": 20, "bottom": 20, "left": 20}})),
            vec![
                rect("a0000001", 50.0, 50.0),
                absolute(&sized("b0000001", json!("fill"), fixed(10.0)), 5.0, 7.0),
                absolute(
                    &sized("c0000001", json!("fill"), json!("fill")),
                    100.0,
                    50.0,
                ),
                absolute(&sized("d0000001", fixed(40.0), json!("fill")), 0.0, 150.0),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 300.0, 200.0]),
            ("a0000001", [20.0, 20.0, 50.0, 50.0]),
            ("b0000001", [5.0, 7.0, 295.0, 10.0]),
            ("c0000001", [100.0, 50.0, 200.0, 150.0]),
            ("d0000001", [0.0, 150.0, 40.0, 50.0]),
        ],
    ));
    cases.push(case(
        "an absolute child between the cells of a padded grid",
        board(
            "brd00001",
            fixed(300.0),
            fixed(200.0),
            grid(json!({
                "columns": [{"fixed": 100}, {"fixed": 100}],
                "rows": [{"fixed": 100}],
                "padding": {"top": 20, "right": 20, "bottom": 20, "left": 20}
            })),
            vec![
                filling("a0000001"),
                absolute(&rect("b0000001", 30.0, 30.0), 250.0, 150.0),
                filling("c0000001"),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 300.0, 200.0]),
            ("a0000001", [20.0, 20.0, 100.0, 100.0]),
            ("b0000001", [250.0, 150.0, 30.0, 30.0]),
            ("c0000001", [120.0, 20.0, 100.0, 100.0]),
        ],
    ));
    cases.push(case(
        "a row that hugs two boxes and an absolute 500 x 500 box, and an absolute child of a nested frame",
        hug_board(
            flex(json!({"padding": {"top": 10, "right": 10, "bottom": 10, "left": 10}})),
            vec![
                rect("a0000001", 50.0, 20.0),
                absolute(&rect("b0000001", 500.0, 500.0), 0.0, 0.0),
                frame(
                    "f0000001",
                    fixed(200.0),
                    fixed(100.0),
                    flex(json!({"padding": {"top": 5, "right": 5, "bottom": 5, "left": 5}})),
                    vec![
                        rect("a0000002", 20.0, 20.0),
                        absolute(&rect("g0000001", 10.0, 10.0), 30.0, 40.0),
                    ],
                ),
            ],
        ),
        &[
    ("brd00001", [0.0, 0.0, 270.0, 120.0]),
    ("a0000001", [10.0, 10.0, 50.0, 20.0]),
    ("b0000001", [0.0, 0.0, 500.0, 500.0]),
    ("f0000001", [60.0, 10.0, 200.0, 100.0]),
    ("a0000002", [65.0, 15.0, 20.0, 20.0]),
    ("g0000001", [90.0, 50.0, 10.0, 10.0]),
],
    ));
    // Text.
    for (width, lines, boxes) in [
        (
            70.0,
            6,
            &[
                ("brd00001", [0.0, 0.0, 2000.0, 500.0]),
                ("txt00001", [0.0, 0.0, 70.0, 144.0]),
            ],
        ),
        (
            100.0,
            3,
            &[
                ("brd00001", [0.0, 0.0, 2000.0, 500.0]),
                ("txt00001", [0.0, 0.0, 100.0, 72.0]),
            ],
        ),
        (
            130.0,
            2,
            &[
                ("brd00001", [0.0, 0.0, 2000.0, 500.0]),
                ("txt00001", [0.0, 0.0, 130.0, 48.0]),
            ],
        ),
        (
            300.0,
            1,
            &[
                ("brd00001", [0.0, 0.0, 2000.0, 500.0]),
                ("txt00001", [0.0, 0.0, 300.0, 24.0]),
            ],
        ),
    ] {
        cases.push(rounded_case(
            format!("six words in a text {width} px wide, as {lines} lines"),
            board(
                "brd00001",
                fixed(2000.0),
                fixed(500.0),
                flex(json!({"alignItems": "start"})),
                vec![with(
                    &text("txt00001", SIX_WORDS, 16.0, 1.5),
                    "width",
                    fixed(width),
                )],
            ),
            boxes,
        ));
    }
    cases.push(rounded_case(
        "a text that fills a padded column",
        board(
            "brd00001",
            fixed(120.0),
            json!("hug"),
            flex(json!({
                "direction": "column",
                "padding": {"top": 10, "right": 10, "bottom": 10, "left": 10}
            })),
            vec![with(
                &text("txt00001", SIX_WORDS, 16.0, 1.5),
                "width",
                json!("fill"),
            )],
        ),
        &[
            ("brd00001", [0.0, 0.0, 120.0, 92.0]),
            ("txt00001", [10.0, 10.0, 100.0, 72.0]),
        ],
    ));
    cases.push(rounded_case(
        "a text that fills the 100 px a 50 px box leaves in a 150 px row",
        board(
            "brd00001",
            fixed(150.0),
            fixed(200.0),
            flex(json!({"alignItems": "start"})),
            vec![
                rect("a0000001", 50.0, 10.0),
                with(
                    &text("txt00001", SIX_WORDS, 16.0, 1.5),
                    "width",
                    json!("fill"),
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 150.0, 200.0]),
            ("a0000001", [0.0, 0.0, 50.0, 10.0]),
            ("txt00001", [50.0, 0.0, 100.0, 72.0]),
        ],
    ));
    cases.push(rounded_case(
        "a text that hugs its content in a 100 px column that aligns it to the start",
        start_column(100.0, vec![text("txt00001", SIX_WORDS, 16.0, 1.5)]),
        &[
            ("brd00001", [0.0, 0.0, 100.0, 72.0]),
            ("txt00001", [0.0, 0.0, 100.0, 72.0]),
        ],
    ));
    cases.push(rounded_case(
        "a text that hugs its content at x 100 in a 200 px board with no layout",
        board(
            "brd00001",
            fixed(200.0),
            fixed(200.0),
            none(),
            vec![at(&text("txt00001", SIX_WORDS, 16.0, 1.5), 100.0, 0.0)],
        ),
        &[
            ("brd00001", [0.0, 0.0, 200.0, 200.0]),
            ("txt00001", [100.0, 0.0, 100.0, 72.0]),
        ],
    ));
    cases.push(rounded_case(
        "a text that fills a 100 px grid column, and an auto column around a text",
        board(
            "brd00001",
            fixed(400.0),
            json!("hug"),
            grid(json!({"columns": [{"fixed": 100}, "auto"]})),
            vec![
                with(
                    &text("txt00001", SIX_WORDS, 16.0, 1.5),
                    "width",
                    json!("fill"),
                ),
                text("txt00002", "Hello", 16.0, 1.5),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 400.0, 72.0]),
            ("txt00001", [0.0, 0.0, 100.0, 72.0]),
            ("txt00002", [100.0, 0.0, 38.5625, 24.0]),
        ],
    ));
    // The example of docs/formats/layout.md: the card.
    cases.push(rounded_case(
        "the card of the example in docs/formats/layout.md",
        at(
            &board(
                "brd00001",
                fixed(320.0),
                json!("hug"),
                flex(json!({
                    "direction": "column",
                    "gap": {"row": 12},
                    "padding": {"top": 16, "right": 16, "bottom": 16, "left": 16}
                })),
                vec![
                    with(
                        &text("txt00001", "Quarterly report", 20.0, 1.2),
                        "width",
                        json!("fill"),
                    ),
                    absolute(
                        &frame("frm00001", fixed(24.0), fixed(24.0), none(), Vec::new()),
                        280.0,
                        8.0,
                    ),
                ],
            ),
            100.0,
            40.0,
        ),
        &[
            ("brd00001", [100.0, 40.0, 320.0, 56.0]),
            ("txt00001", [116.0, 56.0, 288.0, 24.0]),
            ("frm00001", [380.0, 48.0, 24.0, 24.0]),
        ],
    ));
    cases.push(rounded_case(
        "a text of two lines of 16 px with a line break",
        text_row(vec![text("txt00001", "Hello\nminimum", 16.0, 1.5)]),
        &[
            ("brd00001", [0.0, 0.0, 2000.0, 500.0]),
            ("txt00001", [0.0, 0.0, 68.7109375, 48.0]),
        ],
    ));
    cases
}

/// A column of the five 80 x 30 boxes that wraps, with gaps of 10 and 20.
fn five_box_column(height: Value) -> Value {
    frame(
        "frm00001",
        json!("hug"),
        height,
        flex(json!({"direction": "column", "wrap": "wrap", "gap": {"row": 10, "column": 20}})),
        five_rects(),
    )
}

/// A column wraps its children at its height when that is fixed, bounded by
/// its min and max heights and never below its padding, and at the height of a
/// row that does not wrap and stretches it, less the row's padding. A row that
/// wraps does not stretch it. The lines it wraps into set its width where
/// nothing else does.
pub fn wrapping_columns_wrap_at_their_height() -> Vec<Case> {
    let in_start_column = |row: Value| {
        board(
            "brd00001",
            fixed(600.0),
            json!("hug"),
            flex(json!({"direction": "column", "alignItems": "start"})),
            vec![row],
        )
    };
    vec![
        case(
            "a column of fixed height 200 with maxHeight 100, in a board that hugs it",
            hug_board(
                flex(json!({"alignItems": "start"})),
                vec![bounded(
                    five_box_column(fixed(200.0)),
                    &[("maxHeight", 100)],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 280.0, 100.0]),
                ("frm00001", [0.0, 0.0, 280.0, 100.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [0.0, 40.0, 80.0, 30.0]),
                ("c0000001", [100.0, 0.0, 80.0, 30.0]),
                ("d0000001", [100.0, 40.0, 80.0, 30.0]),
                ("e0000001", [200.0, 0.0, 80.0, 30.0]),
            ],
        ),
        case(
            "a column of fixed height 20 with minHeight 100, in a board that hugs it",
            hug_board(
                flex(json!({"alignItems": "start"})),
                vec![bounded(five_box_column(fixed(20.0)), &[("minHeight", 100)])],
            ),
            &[
                ("brd00001", [0.0, 0.0, 280.0, 100.0]),
                ("frm00001", [0.0, 0.0, 280.0, 100.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [0.0, 40.0, 80.0, 30.0]),
                ("c0000001", [100.0, 0.0, 80.0, 30.0]),
                ("d0000001", [100.0, 40.0, 80.0, 30.0]),
                ("e0000001", [200.0, 0.0, 80.0, 30.0]),
            ],
        ),
        case(
            "a column of fixed height 20 padded 15 px above and below, in a board that hugs it",
            hug_board(
                flex(json!({"alignItems": "start"})),
                vec![frame(
                    "frm00001",
                    json!("hug"),
                    fixed(20.0),
                    flex(json!({
                        "direction": "column",
                        "wrap": "wrap",
                        "padding": {"top": 15, "bottom": 15}
                    })),
                    vec![
                        rect("a0000001", 40.0, 10.0),
                        rect("b0000001", 40.0, 10.0),
                        rect("c0000001", 40.0, 10.0),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 120.0, 30.0]),
                ("frm00001", [0.0, 0.0, 120.0, 30.0]),
                ("a0000001", [0.0, 15.0, 40.0, 10.0]),
                ("b0000001", [40.0, 15.0, 40.0, 10.0]),
                ("c0000001", [80.0, 15.0, 40.0, 10.0]),
            ],
        ),
        case(
            "a column of fixed height 70 around three 40 x 30 boxes, in a 400 px start-aligned column",
            start_column(
                400.0,
                vec![frame(
                    "frm00001",
                    json!("hug"),
                    fixed(70.0),
                    flex(json!({
                        "direction": "column",
                        "wrap": "wrap",
                        "gap": {"row": 10, "column": 10}
                    })),
                    vec![
                        rect("a0000001", 40.0, 30.0),
                        rect("b0000001", 40.0, 30.0),
                        rect("c0000001", 40.0, 30.0),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 70.0]),
                ("frm00001", [0.0, 0.0, 90.0, 70.0]),
                ("a0000001", [0.0, 0.0, 40.0, 30.0]),
                ("b0000001", [0.0, 40.0, 40.0, 30.0]),
                ("c0000001", [50.0, 0.0, 40.0, 30.0]),
            ],
        ),
        // A row that does not wrap stretches the column to its height.
        case(
            "a column in a row of fixed height 100 that hugs it",
            in_start_column(frame(
                "frm00002",
                json!("hug"),
                fixed(100.0),
                flex(json!({})),
                vec![five_box_column(json!("hug"))],
            )),
            &[
                ("brd00001", [0.0, 0.0, 600.0, 100.0]),
                ("frm00002", [0.0, 0.0, 280.0, 100.0]),
                ("frm00001", [0.0, 0.0, 280.0, 100.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [0.0, 40.0, 80.0, 30.0]),
                ("c0000001", [100.0, 0.0, 80.0, 30.0]),
                ("d0000001", [100.0, 40.0, 80.0, 30.0]),
                ("e0000001", [200.0, 0.0, 80.0, 30.0]),
            ],
        ),
        case(
            "a column in a row of fixed height 110 padded 10 px above and below",
            in_start_column(frame(
                "frm00002",
                json!("hug"),
                fixed(110.0),
                flex(json!({"padding": {"top": 10, "bottom": 10}})),
                vec![five_box_column(json!("hug"))],
            )),
            &[
                ("brd00001", [0.0, 0.0, 600.0, 110.0]),
                ("frm00002", [0.0, 0.0, 280.0, 110.0]),
                ("frm00001", [0.0, 10.0, 280.0, 90.0]),
                ("a0000001", [0.0, 10.0, 80.0, 30.0]),
                ("b0000001", [0.0, 50.0, 80.0, 30.0]),
                ("c0000001", [100.0, 10.0, 80.0, 30.0]),
                ("d0000001", [100.0, 50.0, 80.0, 30.0]),
                ("e0000001", [200.0, 10.0, 80.0, 30.0]),
            ],
        ),
        // A row that wraps leaves the column as tall as its content.
        case(
            "a column in a row that wraps, of fixed height 100",
            in_start_column(frame(
                "frm00002",
                json!("hug"),
                fixed(100.0),
                flex(json!({"wrap": "wrap"})),
                vec![five_box_column(json!("hug"))],
            )),
            &[
                ("brd00001", [0.0, 0.0, 600.0, 100.0]),
                ("frm00002", [0.0, 0.0, 80.0, 100.0]),
                ("frm00001", [0.0, 0.0, 80.0, 190.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [0.0, 40.0, 80.0, 30.0]),
                ("c0000001", [0.0, 80.0, 80.0, 30.0]),
                ("d0000001", [0.0, 120.0, 80.0, 30.0]),
                ("e0000001", [0.0, 160.0, 80.0, 30.0]),
            ],
        ),
        case(
            "a column in a row that wraps, of fixed height 110, padded 10 px above and below",
            in_start_column(frame(
                "frm00002",
                json!("hug"),
                fixed(110.0),
                flex(json!({"wrap": "wrap", "padding": {"top": 10, "bottom": 10}})),
                vec![five_box_column(json!("hug"))],
            )),
            &[
                ("brd00001", [0.0, 0.0, 600.0, 110.0]),
                ("frm00002", [0.0, 0.0, 80.0, 110.0]),
                ("frm00001", [0.0, 10.0, 80.0, 190.0]),
                ("a0000001", [0.0, 10.0, 80.0, 30.0]),
                ("b0000001", [0.0, 50.0, 80.0, 30.0]),
                ("c0000001", [0.0, 90.0, 80.0, 30.0]),
                ("d0000001", [0.0, 130.0, 80.0, 30.0]),
                ("e0000001", [0.0, 170.0, 80.0, 30.0]),
            ],
        ),
        case(
            "a column with maxHeight 200 in a row of fixed height 100, which makes it shorter than its content",
            in_start_column(frame(
                "frm00002",
                json!("hug"),
                fixed(100.0),
                flex(json!({})),
                vec![bounded(
                    five_box_column(json!("hug")),
                    &[("maxHeight", 200)],
                )],
            )),
            &[
                ("brd00001", [0.0, 0.0, 600.0, 100.0]),
                ("frm00002", [0.0, 0.0, 280.0, 100.0]),
                ("frm00001", [0.0, 0.0, 280.0, 100.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [0.0, 40.0, 80.0, 30.0]),
                ("c0000001", [100.0, 0.0, 80.0, 30.0]),
                ("d0000001", [100.0, 40.0, 80.0, 30.0]),
                ("e0000001", [200.0, 0.0, 80.0, 30.0]),
            ],
        ),
        case(
            "a column with maxHeight 200 in a row of fixed height 250, which makes it as tall as its max height",
            in_start_column(frame(
                "frm00002",
                json!("hug"),
                fixed(250.0),
                flex(json!({})),
                vec![bounded(
                    five_box_column(json!("hug")),
                    &[("maxHeight", 200)],
                )],
            )),
            &[
                ("brd00001", [0.0, 0.0, 600.0, 250.0]),
                ("frm00002", [0.0, 0.0, 80.0, 250.0]),
                ("frm00001", [0.0, 0.0, 80.0, 200.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [0.0, 40.0, 80.0, 30.0]),
                ("c0000001", [0.0, 80.0, 80.0, 30.0]),
                ("d0000001", [0.0, 120.0, 80.0, 30.0]),
                ("e0000001", [0.0, 160.0, 80.0, 30.0]),
            ],
        ),
        case(
            "a column with maxHeight 100 in a row of fixed height 200",
            in_start_column(frame(
                "frm00002",
                json!("hug"),
                fixed(200.0),
                flex(json!({})),
                vec![bounded(
                    five_box_column(json!("hug")),
                    &[("maxHeight", 100)],
                )],
            )),
            &[
                ("brd00001", [0.0, 0.0, 600.0, 200.0]),
                ("frm00002", [0.0, 0.0, 280.0, 200.0]),
                ("frm00001", [0.0, 0.0, 280.0, 100.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [0.0, 40.0, 80.0, 30.0]),
                ("c0000001", [100.0, 0.0, 80.0, 30.0]),
                ("d0000001", [100.0, 40.0, 80.0, 30.0]),
                ("e0000001", [200.0, 0.0, 80.0, 30.0]),
            ],
        ),
        case(
            "a column with minHeight 120 in a row of fixed height 150",
            in_start_column(frame(
                "frm00002",
                json!("hug"),
                fixed(150.0),
                flex(json!({})),
                vec![bounded(
                    five_box_column(json!("hug")),
                    &[("minHeight", 120)],
                )],
            )),
            &[
                ("brd00001", [0.0, 0.0, 600.0, 150.0]),
                ("frm00002", [0.0, 0.0, 180.0, 150.0]),
                ("frm00001", [0.0, 0.0, 180.0, 150.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [0.0, 40.0, 80.0, 30.0]),
                ("c0000001", [0.0, 80.0, 80.0, 30.0]),
                ("d0000001", [0.0, 120.0, 80.0, 30.0]),
                ("e0000001", [100.0, 0.0, 80.0, 30.0]),
            ],
        ),
        case(
            "a column with minHeight 150 in a row of fixed height 100",
            in_start_column(frame(
                "frm00002",
                json!("hug"),
                fixed(100.0),
                flex(json!({})),
                vec![bounded(
                    five_box_column(json!("hug")),
                    &[("minHeight", 150)],
                )],
            )),
            &[
                ("brd00001", [0.0, 0.0, 600.0, 100.0]),
                ("frm00002", [0.0, 0.0, 180.0, 100.0]),
                ("frm00001", [0.0, 0.0, 180.0, 150.0]),
                ("a0000001", [0.0, 0.0, 80.0, 30.0]),
                ("b0000001", [0.0, 40.0, 80.0, 30.0]),
                ("c0000001", [0.0, 80.0, 80.0, 30.0]),
                ("d0000001", [0.0, 120.0, 80.0, 30.0]),
                ("e0000001", [100.0, 0.0, 80.0, 30.0]),
            ],
        ),
        case(
            "a column padded 15 px above and below in a row of fixed height 20",
            in_start_column(frame(
                "frm00002",
                json!("hug"),
                fixed(20.0),
                flex(json!({})),
                vec![frame(
                    "frm00001",
                    json!("hug"),
                    json!("hug"),
                    flex(json!({
                        "direction": "column",
                        "wrap": "wrap",
                        "padding": {"top": 15, "bottom": 15}
                    })),
                    vec![
                        rect("a0000001", 40.0, 10.0),
                        rect("b0000001", 40.0, 10.0),
                        rect("c0000001", 40.0, 10.0),
                    ],
                )],
            )),
            &[
                ("brd00001", [0.0, 0.0, 600.0, 20.0]),
                ("frm00002", [0.0, 0.0, 120.0, 20.0]),
                ("frm00001", [0.0, 0.0, 120.0, 30.0]),
                ("a0000001", [0.0, 15.0, 40.0, 10.0]),
                ("b0000001", [40.0, 15.0, 40.0, 10.0]),
                ("c0000001", [80.0, 15.0, 40.0, 10.0]),
            ],
        ),
    ]
}

/// A row that hugs its width and wraps keeps its items on one line at their
/// max-content widths, also where the widths add up to numbers that a float
/// cannot hold exactly and the row's padding gets added and taken off again.
pub fn wrapping_rows_at_their_max_content_width() -> Vec<Case> {
    let padded_row = |a: f64, b: f64, gap: f64, left: u32, right: u32| {
        hug_board(
            flex(json!({"alignItems": "start"})),
            vec![frame(
                "frm00001",
                json!("hug"),
                json!("hug"),
                flex(json!({
                    "wrap": "wrap",
                    "gap": {"column": gap},
                    "padding": {"left": left, "right": right}
                })),
                vec![rect("a0000001", a, 20.0), rect("b0000001", b, 20.0)],
            )],
        )
    };
    vec![
        rounded_case(
            "a wrapping row padded 12 px on both sides around a 32.4 px and an 83 px box",
            padded_row(32.4, 83.0, 0.0, 12, 12),
            &[
                ("brd00001", [0.0, 0.0, 139.3984375, 20.0]),
                ("frm00001", [0.0, 0.0, 139.3984375, 20.0]),
                ("a0000001", [12.0, 0.0, 32.3984375, 20.0]),
                ("b0000001", [44.3984375, 0.0, 83.0, 20.0]),
            ],
        ),
        rounded_case(
            "a wrapping row padded 6 and 8 px around an 89.2 px and a 30.35 px box, 3 px apart",
            padded_row(89.2, 30.35, 3.0, 6, 8),
            &[
                ("brd00001", [0.0, 0.0, 136.5390625, 20.0]),
                ("frm00001", [0.0, 0.0, 136.5390625, 20.0]),
                ("a0000001", [6.0, 0.0, 89.1953125, 20.0]),
                ("b0000001", [98.1953125, 0.0, 30.34375, 20.0]),
            ],
        ),
        rounded_case(
            "a wrapping row padded 4 and 24 px around a 39.57 px and a 50.89 px box, 12 px apart",
            padded_row(39.57, 50.89, 12.0, 4, 24),
            &[
                ("brd00001", [0.0, 0.0, 130.4453125, 20.0]),
                ("frm00001", [0.0, 0.0, 130.4453125, 20.0]),
                ("a0000001", [4.0, 0.0, 39.5625, 20.0]),
                ("b0000001", [55.5625, 0.0, 50.8828125, 20.0]),
            ],
        ),
    ]
}

/// A tab moves its line on to the next stop, or to the one after it when less
/// than half a space is left: "mZ9" is 34 px wide, which leaves 2 px of the 36
/// px to the first stop, and "xsGI" is 33.41 px wide, which leaves 2.59 px. A
/// letter spacing that cancels the width of a space leaves no tab stops at all.
pub fn tabs_near_a_stop() -> Vec<Case> {
    vec![rounded_case(
        "tabs after \"mZ9\" (34 px), \"xsGI\" (33.41 px) and \"a\" with a letter spacing of -4.5 px",
        board(
            "brd00001",
            fixed(1000.0),
            fixed(300.0),
            flex(json!({"alignItems": "start", "direction": "column"})),
            vec![
                text("txt00001", "mZ9\t", 16.0, 1.5),
                text("txt00002", "xsGI\t", 16.0, 1.5),
                with(
                    &text("txt00003", "a\tb", 16.0, 1.5),
                    "letterSpacing",
                    json!(-4.5),
                ),
            ],
        ),
        &[
            ("brd00001", [0.0, 0.0, 1000.0, 300.0]),
            ("txt00001", [0.0, 0.0, 72.0, 24.0]),
            ("txt00002", [0.0, 24.0, 36.0, 24.0]),
            ("txt00003", [0.0, 48.0, 9.78125, 24.0]),
        ],
    )]
}

/// An empty text has no line, so no baseline: its bottom edge stands in for one,
/// as the bottom edge of a box does.
pub fn empty_texts_have_no_baseline() -> Vec<Case> {
    vec![
        rounded_case(
            "an empty text and a 32 px text on their baselines",
            baseline_row(vec![
                text("txt00001", "", 16.0, 1.5),
                text("txt00002", "Hello", 32.0, 1.5),
            ]),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 100.0]),
                ("txt00001", [0.0, 35.5, 0.0, 0.0]),
                ("txt00002", [0.0, 0.0, 77.125, 48.0]),
            ],
        ),
        rounded_case(
            "a 32 px text and an empty text on their baselines",
            baseline_row(vec![
                text("txt00002", "Hello", 32.0, 1.5),
                text("txt00001", "", 16.0, 1.5),
            ]),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 100.0]),
                ("txt00002", [0.0, 0.0, 77.125, 48.0]),
                ("txt00001", [77.125, 35.5, 0.0, 0.0]),
            ],
        ),
        rounded_case(
            "an empty text 30 px tall and a 16 px text on their baselines",
            baseline_row(vec![
                with(&text("txt00001", "", 16.0, 1.5), "height", fixed(30.0)),
                text("txt00002", "Hello", 16.0, 1.5),
            ]),
            &[
                ("brd00001", [0.0, 0.0, 400.0, 100.0]),
                ("txt00001", [0.0, 0.0, 0.0, 30.0]),
                ("txt00002", [0.0, 12.5, 38.5625, 24.0]),
            ],
        ),
    ]
}

/// A column that hugs both axes around a text that fills its height: the text
/// has no minHeight, so the automatic minimum of a flex item, its content, is
/// what the fill item gets.
fn filling_text_column(direction: &str, wrap: &str) -> Value {
    hug_column(
        "frm00001",
        json!({"direction": direction, "wrap": wrap}),
        vec![with(
            &text("txt00001", "iii", 16.0, 1.5),
            "height",
            json!("fill"),
        )],
    )
}

/// A column whose content decides its height is as tall as its content, whatever
/// the space around it: a fill item takes its content's height, the automatic
/// minimum of a flex item, and the maxHeight of a text bounds it.
pub fn hug_columns_take_the_height_their_items_need() -> Vec<Case> {
    let column = || filling_text_column("column", "nowrap");
    vec![
        rounded_case(
            "a column around a fill-height text, in a board that hugs it",
            hug_board(flex(json!({})), vec![column()]),
            &[
                ("brd00001", [0.0, 0.0, 11.625, 24.0]),
                ("frm00001", [0.0, 0.0, 11.625, 24.0]),
                ("txt00001", [0.0, 0.0, 11.625, 24.0]),
            ],
        ),
        rounded_case(
            "the same column in a grid area",
            hug_board(grid(json!({})), vec![column()]),
            &[
                ("brd00001", [0.0, 0.0, 11.625, 24.0]),
                ("frm00001", [0.0, 0.0, 11.625, 24.0]),
                ("txt00001", [0.0, 0.0, 11.625, 24.0]),
            ],
        ),
        rounded_case(
            "the same column absolutely positioned in a flex parent",
            board(
                "brd00001",
                fixed(200.0),
                fixed(200.0),
                flex(json!({})),
                vec![absolute(&column(), 10.0, 10.0)],
            ),
            &[
                ("brd00001", [0.0, 0.0, 200.0, 200.0]),
                ("frm00001", [10.0, 10.0, 11.625, 24.0]),
                ("txt00001", [10.0, 10.0, 11.625, 24.0]),
            ],
        ),
        rounded_case(
            "the same column in a `none` layout",
            board(
                "brd00001",
                fixed(200.0),
                fixed(200.0),
                none(),
                vec![at(&column(), 10.0, 10.0)],
            ),
            &[
                ("brd00001", [0.0, 0.0, 200.0, 200.0]),
                ("frm00001", [10.0, 10.0, 11.625, 24.0]),
                ("txt00001", [10.0, 10.0, 11.625, 24.0]),
            ],
        ),
        rounded_case(
            "a column that wraps, in a row of fixed height 60 that aligns on baselines",
            board(
                "brd00001",
                json!("hug"),
                fixed(60.0),
                flex(json!({"alignItems": "baseline"})),
                vec![filling_text_column("column", "wrap")],
            ),
            &[
                ("brd00001", [0.0, 0.0, 11.625, 60.0]),
                ("frm00001", [0.0, 0.0, 11.625, 24.0]),
                ("txt00001", [0.0, 0.0, 11.625, 24.0]),
            ],
        ),
        rounded_case(
            "a column in reverse inside a frame with a `none` layout",
            board(
                "brd00001",
                json!("hug"),
                json!("hug"),
                flex(json!({"direction": "column-reverse"})),
                vec![frame(
                    "frm00002",
                    json!("hug"),
                    json!("hug"),
                    none(),
                    vec![filling_text_column("column-reverse", "nowrap")],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 0.0, 0.0]),
                ("frm00002", [0.0, 0.0, 0.0, 0.0]),
                ("frm00001", [0.0, 0.0, 11.625, 24.0]),
                ("txt00001", [0.0, 0.0, 11.625, 24.0]),
            ],
        ),
        rounded_case(
            "a column around a fill-height text and a 20 x 20 box",
            hug_board(
                flex(json!({})),
                vec![hug_column(
                    "frm00001",
                    json!({"direction": "column"}),
                    vec![
                        with(&text("txt00001", "iii", 16.0, 1.5), "height", json!("fill")),
                        rect("a0000001", 20.0, 20.0),
                    ],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 20.0, 44.0]),
                ("frm00001", [0.0, 0.0, 20.0, 44.0]),
                ("txt00001", [0.0, 0.0, 20.0, 24.0]),
                ("a0000001", [0.0, 24.0, 20.0, 20.0]),
            ],
        ),
        rounded_case(
            "a column around a text with maxHeight 10, in a row that centers it",
            hug_board(
                flex(json!({"alignItems": "center"})),
                vec![hug_column(
                    "frm00001",
                    json!({"direction": "column"}),
                    vec![with(
                        &text("txt00001", "a", 16.0, 1.5),
                        "maxHeight",
                        json!(10),
                    )],
                )],
            ),
            &[
                ("brd00001", [0.0, 0.0, 8.984375, 10.0]),
                ("frm00001", [0.0, 0.0, 8.984375, 10.0]),
                ("txt00001", [0.0, 0.0, 8.984375, 10.0]),
            ],
        ),
    ]
}

/// A text in a column whose width is narrower than the text: a column that
/// stretches it wraps it there, and a column that wraps sizes its line by it.
/// A board that is a wrapping column of fixed height hugs the lines it wraps
/// into.
pub fn narrow_columns_wrap_their_texts() -> Vec<Case> {
    vec![
        rounded_case(
            "a text that hugs its width in a 40 px wide column that stretches it",
            board(
                "brd00001",
                fixed(40.0),
                json!("hug"),
                flex(json!({"direction": "column"})),
                vec![text("txt00001", "Billing & invoices", 16.0, 1.5)],
            ),
            &[
                ("brd00001", [0.0, 0.0, 40.0, 72.0]),
                ("txt00001", [0.0, 0.0, 40.0, 72.0]),
            ],
        ),
        rounded_case(
            "a text that hugs its width in a 40 px wide column that wraps",
            board(
                "brd00001",
                fixed(40.0),
                json!("hug"),
                flex(json!({"direction": "column", "wrap": "wrap"})),
                vec![text("txt00001", "Dashboard e.g. this", 16.0, 1.5)],
            ),
            &[
                ("brd00001", [0.0, 0.0, 40.0, 48.0]),
                ("txt00001", [0.0, 0.0, 82.5234375, 48.0]),
            ],
        ),
        rounded_case(
            "a board that is a column of fixed height 50 that wraps, around a 24 px wide text and a 150 x 24 box",
            board(
                "brd00001",
                json!("hug"),
                fixed(50.0),
                flex(json!({"direction": "column", "wrap": "wrap"})),
                vec![
                    with(
                        &text("txt00001", "Long-running", 16.0, 1.5),
                        "width",
                        fixed(24.0),
                    ),
                    rect("a0000001", 150.0, 24.0),
                ],
            ),
            &[
                ("brd00001", [0.0, 0.0, 174.0, 50.0]),
                ("txt00001", [0.0, 0.0, 24.0, 48.0]),
                ("a0000001", [24.0, 0.0, 150.0, 24.0]),
            ],
        ),
    ]
}
