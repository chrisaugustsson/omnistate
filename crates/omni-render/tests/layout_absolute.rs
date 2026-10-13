//! US-003: children of `none` layouts and absolute children of flex and grid
//! layouts sit at their `x` and `y` from their parent's top-left corner, as
//! CSS absolute positioning places them, and take no space in their parent.

mod design;

use design::{
    absolute, assert_box, at, board, fixed, flex, frame, grid, hugging, lay_out, none, rect, sized,
};
use serde_json::json;

#[test]
fn children_of_a_none_layout_sit_at_their_x_and_y() {
    let boxes = lay_out(&board(
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
    ));
    let case = "children of a 400 x 300 board with no layout";
    assert_box(&boxes, "a0000001", [10.0, 20.0, 50.0, 40.0], case);
    assert_box(&boxes, "b0000001", [-5.0, 30.5, 20.0, 20.0], case);
    assert_box(&boxes, "c0000001", [0.0, 0.0, 10.0, 10.0], case);
    assert_box(&boxes, "d0000001", [100.0, 100.0, 30.0, 20.0], case);
    assert_box(&boxes, "f0000001", [300.0, 200.0, 80.0, 80.0], case);
    assert_box(&boxes, "g0000001", [310.0, 210.0, 5.0, 5.0], case);
}

#[test]
fn absolute_children_of_a_flex_layout_sit_at_x_and_y_from_its_edge_not_its_padding() {
    let boxes = lay_out(&board(
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
    ));
    let case = "a padded 300 x 200 row with two absolute children";
    assert_box(&boxes, "a0000001", [20.0, 20.0, 50.0, 50.0], case);
    assert_box(&boxes, "b0000001", [5.0, 7.0, 40.0, 40.0], case);
    assert_box(&boxes, "c0000001", [80.0, 20.0, 60.0, 50.0], case);
    assert_box(&boxes, "d0000001", [200.0, 150.0, 30.0, 20.0], case);
}

#[test]
fn absolute_children_of_a_grid_layout_sit_at_x_and_y_and_take_no_cell() {
    let boxes = lay_out(&board(
        "brd00001",
        fixed(300.0),
        fixed(200.0),
        grid(json!({
            "columns": [{"fixed": 100}, {"fixed": 100}],
            "rows": [{"fixed": 100}],
            "padding": {"top": 20, "right": 20, "bottom": 20, "left": 20}
        })),
        vec![
            sized("a0000001", json!("fill"), json!("fill")),
            absolute(&rect("b0000001", 30.0, 30.0), 250.0, 150.0),
            sized("c0000001", json!("fill"), json!("fill")),
        ],
    ));
    let case = "a padded grid of two 100 px columns with an absolute child between its cells";
    assert_box(&boxes, "a0000001", [20.0, 20.0, 100.0, 100.0], case);
    assert_box(&boxes, "b0000001", [250.0, 150.0, 30.0, 30.0], case);
    assert_box(&boxes, "c0000001", [120.0, 20.0, 100.0, 100.0], case);
}

#[test]
fn absolute_children_take_no_space_in_a_parent_that_hugs_its_content() {
    let boxes = lay_out(&board(
        "brd00001",
        json!("hug"),
        json!("hug"),
        flex(json!({})),
        vec![
            rect("a0000001", 50.0, 20.0),
            absolute(&rect("b0000001", 500.0, 500.0), 0.0, 0.0),
            rect("c0000001", 50.0, 20.0),
        ],
    ));
    let case = "a row that hugs two 50 x 20 boxes and an absolute 500 x 500 box";
    assert_box(&boxes, "brd00001", [0.0, 0.0, 100.0, 20.0], case);
    assert_box(&boxes, "b0000001", [0.0, 0.0, 500.0, 500.0], case);
    assert_box(&boxes, "c0000001", [50.0, 0.0, 50.0, 20.0], case);
}

#[test]
fn absolute_children_are_placed_from_their_own_parent_inside_nested_frames() {
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(300.0),
        flex(json!({
            "direction": "column",
            "padding": {"top": 10, "right": 10, "bottom": 10, "left": 10}
        })),
        vec![frame(
            "f0000001",
            fixed(200.0),
            fixed(100.0),
            flex(json!({"padding": {"top": 5, "right": 5, "bottom": 5, "left": 5}})),
            vec![
                rect("a0000001", 20.0, 20.0),
                absolute(&rect("g0000001", 10.0, 10.0), 30.0, 40.0),
            ],
        )],
    ));
    let case = "an absolute child of a padded frame inside a padded column";
    assert_box(&boxes, "f0000001", [10.0, 10.0, 200.0, 100.0], case);
    assert_box(&boxes, "a0000001", [15.0, 15.0, 20.0, 20.0], case);
    assert_box(&boxes, "g0000001", [40.0, 50.0, 10.0, 10.0], case);
}
