//! US-003: absolute children that fill an axis stretch to their parent's far
//! edge, not its padding, as CSS absolute positioning places them: the parent
//! is their containing block.

mod design;

use design::{absolute, assert_box, board, fixed, flex, frame, grid, lay_out, rect, sized, with};
use serde_json::json;

#[test]
fn absolute_children_that_fill_stretch_to_their_parents_far_edges_not_its_padding() {
    let boxes = lay_out(&board(
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
            absolute(
                &with(
                    &with(
                        &sized("e0000001", json!("fill"), json!("fill")),
                        "maxHeight",
                        json!(60),
                    ),
                    "minWidth",
                    json!(250),
                ),
                100.0,
                50.0,
            ),
        ],
    ));
    let case = "absolute children that fill an axis of a 300 x 200 row padded by 20 px";
    assert_box(&boxes, "a0000001", [20.0, 20.0, 50.0, 50.0], case);
    assert_box(&boxes, "b0000001", [5.0, 7.0, 295.0, 10.0], case);
    assert_box(&boxes, "c0000001", [100.0, 50.0, 200.0, 150.0], case);
    assert_box(&boxes, "d0000001", [0.0, 150.0, 40.0, 50.0], case);
    assert_box(&boxes, "e0000001", [100.0, 50.0, 250.0, 60.0], case);

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
            absolute(
                &sized("b0000001", json!("fill"), json!("fill")),
                150.0,
                120.0,
            ),
        ],
    ));
    let case = "an absolute child that fills both axes of a padded 300 x 200 grid";
    assert_box(&boxes, "a0000001", [20.0, 20.0, 100.0, 100.0], case);
    assert_box(&boxes, "b0000001", [150.0, 120.0, 150.0, 80.0], case);

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
                absolute(&sized("g0000001", json!("fill"), json!("fill")), 30.0, 40.0),
            ],
        )],
    ));
    let case =
        "an absolute child that fills both axes of a padded 200 x 100 frame inside a padded column";
    assert_box(&boxes, "f0000001", [10.0, 10.0, 200.0, 100.0], case);
    assert_box(&boxes, "g0000001", [40.0, 50.0, 170.0, 60.0], case);
}
