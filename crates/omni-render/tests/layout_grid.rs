//! US-003: grid layouts place and size their children by CSS grid: fixed,
//! fractional and auto tracks, gap and padding, placement by start line and
//! span, automatic placement, and `fixed`, `hug` and `fill` sizing inside a
//! grid area.

mod design;

use design::{
    assert_box, assert_cases, board, cases, fixed, grid, hugging, lay_out, rect, sized, with,
};
use serde_json::{Value, json};

/// An empty child that fills its grid area.
fn filling(id: &str) -> Value {
    sized(id, json!("fill"), json!("fill"))
}

/// A copy of `json` placed on grid lines: `column` and `row` are
/// `{"start": <line>, "span": <tracks>}`, or `null` to leave an axis out.
fn placed(json: &Value, column: Value, row: Value) -> Value {
    let mut placed = json.clone();
    if !column.is_null() {
        placed = with(&placed, "gridColumn", column);
    }
    if !row.is_null() {
        placed = with(&placed, "gridRow", row);
    }
    placed
}

#[test]
fn tracks_are_fixed_fractions_of_the_free_space_or_sized_by_their_content() {
    let boxes = lay_out(&board(
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
    ));
    // 440 px inside the padding, less 100 px and two 20 px gaps, leaves
    // 300 px for 1fr + 2fr. The auto row is as tall as its 35 px box.
    let case = "a 460 px grid of 100 px, 1fr and 2fr columns and a 50 px and an auto row";
    assert_box(&boxes, "brd00001", [0.0, 0.0, 460.0, 115.0], case);
    assert_box(&boxes, "a0000001", [10.0, 10.0, 100.0, 50.0], case);
    assert_box(&boxes, "b0000001", [130.0, 10.0, 100.0, 50.0], case);
    assert_box(&boxes, "c0000001", [250.0, 10.0, 200.0, 50.0], case);
    assert_box(&boxes, "d0000001", [10.0, 70.0, 60.0, 35.0], case);
    assert_box(&boxes, "e0000001", [130.0, 70.0, 100.0, 35.0], case);
    assert_box(&boxes, "f0000001", [250.0, 70.0, 200.0, 35.0], case);
}

#[test]
fn auto_tracks_fit_their_content_and_fractions_take_the_rest() {
    let case = "a 300 px grid of an auto and a 1fr column, with a 70 x 20 box in the auto column";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(300.0),
        json!("hug"),
        grid(json!({"columns": ["auto", {"fr": 1}]})),
        vec![rect("a0000001", 70.0, 20.0), filling("b0000001")],
    ));
    assert_box(&boxes, "brd00001", [0.0, 0.0, 300.0, 20.0], case);
    assert_box(&boxes, "a0000001", [0.0, 0.0, 70.0, 20.0], case);
    assert_box(&boxes, "b0000001", [70.0, 0.0, 230.0, 20.0], case);
}

#[test]
fn auto_tracks_stretch_into_free_space_that_no_fraction_takes() {
    // `justify-content` and `align-content` keep their CSS initial value,
    // `normal`, which stretches auto tracks over the free space of a grid.
    let case = "a 300 x 100 grid of an auto and a 100 px column and one auto row";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(300.0),
        fixed(100.0),
        grid(json!({"columns": ["auto", {"fixed": 100}], "rows": ["auto"]})),
        vec![rect("a0000001", 70.0, 20.0), filling("b0000001")],
    ));
    assert_box(&boxes, "a0000001", [0.0, 0.0, 70.0, 20.0], case);
    assert_box(&boxes, "b0000001", [200.0, 0.0, 100.0, 100.0], case);
}

#[test]
fn children_are_placed_on_grid_lines_by_start_and_span() {
    let boxes = lay_out(&board(
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
    ));
    let case = "a 3 x 3 grid of 100 px tracks with 10 px gaps";
    assert_box(&boxes, "a0000001", [110.0, 0.0, 210.0, 320.0], case);
    assert_box(&boxes, "b0000001", [0.0, 110.0, 100.0, 100.0], case);
    assert_box(&boxes, "c0000001", [0.0, 220.0, 100.0, 100.0], case);
    assert_box(&boxes, "d0000001", [0.0, 0.0, 100.0, 100.0], case);
}

#[test]
fn negative_lines_count_from_the_end_of_the_grid() {
    let boxes = lay_out(&board(
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
    ));
    let case = "three columns, so line -2 is line 3, -3 is line 2 and -4 is line 1";
    assert_box(&boxes, "a0000001", [200.0, 0.0, 100.0, 100.0], case);
    assert_box(&boxes, "b0000001", [100.0, 0.0, 100.0, 100.0], case);
    assert_box(&boxes, "c0000001", [0.0, 0.0, 100.0, 100.0], case);
}

#[test]
fn children_without_a_start_line_flow_into_the_next_free_cells_and_new_rows() {
    let boxes = lay_out(&board(
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
    ));
    let case = "a 2 x 2 grid whose second cell is taken, then a child that spans both columns";
    assert_box(&boxes, "a0000001", [100.0, 0.0, 100.0, 100.0], case);
    assert_box(&boxes, "b0000001", [0.0, 0.0, 100.0, 100.0], case);
    assert_box(&boxes, "c0000001", [0.0, 100.0, 100.0, 100.0], case);
    assert_box(&boxes, "d0000001", [100.0, 100.0, 100.0, 100.0], case);
    // A third, implicit row is added for it, sized by its content.
    assert_box(&boxes, "e0000001", [0.0, 200.0, 200.0, 30.0], case);
    assert_box(&boxes, "brd00001", [0.0, 0.0, 200.0, 230.0], case);
}

#[test]
fn fixed_and_hug_children_sit_at_the_start_of_their_area_and_fill_children_fill_it() {
    let two_cells = || {
        grid(json!({
            "columns": [{"fixed": 200}, {"fixed": 200}],
            "rows": [{"fixed": 100}]
        }))
    };

    let case = "a 50 x 30 box and a frame hugging a 40 x 20 box, in 200 x 100 areas";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(100.0),
        two_cells(),
        vec![
            rect("a0000001", 50.0, 30.0),
            hugging("b0000001", "b1000001", 40.0, 20.0),
        ],
    ));
    assert_box(&boxes, "a0000001", [0.0, 0.0, 50.0, 30.0], case);
    assert_box(&boxes, "b0000001", [200.0, 0.0, 40.0, 20.0], case);

    let case = "a child that fills both axes, and one that fills its width and hugs a 40 x 20 box";
    let boxes = lay_out(&board(
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
    ));
    assert_box(&boxes, "c0000001", [0.0, 0.0, 200.0, 100.0], case);
    assert_box(&boxes, "d0000001", [200.0, 0.0, 200.0, 20.0], case);

    let case = "a 60 px wide child that fills its height, and a fill child with maxWidth 120 and maxHeight 50";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(100.0),
        two_cells(),
        vec![
            sized("e0000001", fixed(60.0), json!("fill")),
            with(
                &with(&filling("f0000001"), "maxWidth", json!(120)),
                "maxHeight",
                json!(50),
            ),
        ],
    ));
    assert_box(&boxes, "e0000001", [0.0, 0.0, 60.0, 100.0], case);
    assert_box(&boxes, "f0000001", [200.0, 0.0, 120.0, 50.0], case);
}

#[test]
fn an_absolute_child_ignores_its_grid_lines_and_a_placement_without_a_start_or_span_is_automatic() {
    // An absolute child is placed by its `x` and `y` only, so it is not placed
    // in the grid area its `gridColumn` and `gridRow` name; and an empty
    // placement, `{}`, is `auto / auto` like one that is left out. A frame in
    // the second column of a grid places a child at x 10 from its own left
    // edge, at x 110 on the page.
    assert_cases(cases::absolute_children_and_empty_grid_lines());
}

#[test]
fn an_empty_grid_keeps_the_tracks_it_declares() {
    // A grid without children still has its `grid-template-columns` and
    // `grid-template-rows`: one that hugs its content is as large as its fixed
    // tracks, its gaps and its padding. Columns of 50 and 30 px make it 80 px
    // wide.
    assert_cases(cases::empty_grids());
}

#[test]
fn a_texts_max_height_bounds_the_grid_row_that_holds_it() {
    // A grid row is as tall as its items, and a text's maxHeight bounds the
    // text there as everywhere else: "alpha beta gamma" needs three lines, 72
    // px, at 50 px wide, but with a maxHeight of 30 px the text, its row and a
    // grid that hugs them are 30 px tall, whether the text is 50 px wide, hugs
    // its width or fills a column of 50 px. A taller item of the row, a 20 x 40
    // box, makes it 40 px tall, and a box in the next row starts at y 30.
    assert_cases(cases::max_heights_of_texts_in_grid_rows());
}
