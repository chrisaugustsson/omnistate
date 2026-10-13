//! US-003: flex layouts place and size their children by CSS flexbox: row and
//! column directions, gap, padding, wrapping, alignment, `fixed`, `hug` and
//! `fill` sizing on each axis, and min and max sizes.

mod design;

use design::{
    assert_box, assert_box_within, board, box_of, fixed, flex, frame, hugging, lay_out, rect,
    sized, text, with,
};
use serde_json::{Value, json};

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

#[test]
fn a_row_places_children_side_by_side_from_the_left() {
    for layout in [flex(json!({})), flex(json!({"direction": "row"}))] {
        let case = format!("a 400 x 100 board with the flex layout {layout}");
        let boxes = lay_out(&board(
            "brd00001",
            fixed(400.0),
            fixed(100.0),
            layout,
            three_rects(),
        ));
        assert_box(&boxes, "brd00001", [0.0, 0.0, 400.0, 100.0], &case);
        assert_box(&boxes, "a0000001", [0.0, 0.0, 50.0, 20.0], &case);
        assert_box(&boxes, "b0000001", [50.0, 0.0, 80.0, 30.0], &case);
        assert_box(&boxes, "c0000001", [130.0, 0.0, 30.0, 40.0], &case);
    }
}

#[test]
fn a_column_stacks_children_from_the_top() {
    let case = "a 400 x 300 column";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(300.0),
        flex(json!({"direction": "column"})),
        three_rects(),
    ));
    assert_box(&boxes, "a0000001", [0.0, 0.0, 50.0, 20.0], case);
    assert_box(&boxes, "b0000001", [0.0, 20.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [0.0, 50.0, 30.0, 40.0], case);
}

#[test]
fn reversed_directions_place_children_from_the_far_edge() {
    let case = "a 400 x 100 row-reverse";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(100.0),
        flex(json!({"direction": "row-reverse"})),
        three_rects(),
    ));
    assert_box(&boxes, "a0000001", [350.0, 0.0, 50.0, 20.0], case);
    assert_box(&boxes, "b0000001", [270.0, 0.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [240.0, 0.0, 30.0, 40.0], case);

    let case = "a 400 x 300 column-reverse";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(300.0),
        flex(json!({"direction": "column-reverse"})),
        three_rects(),
    ));
    assert_box(&boxes, "a0000001", [0.0, 280.0, 50.0, 20.0], case);
    assert_box(&boxes, "b0000001", [0.0, 250.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [0.0, 210.0, 30.0, 40.0], case);
}

#[test]
fn the_column_gap_separates_the_children_of_a_row_and_the_row_gap_those_of_a_column() {
    let case = "a row with a column gap of 12 (and a row gap of 100, which one line never uses)";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(100.0),
        flex(json!({"gap": {"row": 100, "column": 12}})),
        three_rects(),
    ));
    assert_box(&boxes, "a0000001", [0.0, 0.0, 50.0, 20.0], case);
    assert_box(&boxes, "b0000001", [62.0, 0.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [154.0, 0.0, 30.0, 40.0], case);

    let case = "a column with a row gap of 8 (and a column gap of 100, which one line never uses)";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(300.0),
        flex(json!({"direction": "column", "gap": {"row": 8, "column": 100}})),
        three_rects(),
    ));
    assert_box(&boxes, "a0000001", [0.0, 0.0, 50.0, 20.0], case);
    assert_box(&boxes, "b0000001", [0.0, 28.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [0.0, 66.0, 30.0, 40.0], case);
}

#[test]
fn padding_insets_the_children_from_each_edge() {
    let padding = json!({"top": 10, "right": 20, "bottom": 30, "left": 40});

    let case = "a 400 x 300 row with padding 10 20 30 40";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(300.0),
        flex(json!({"padding": padding})),
        three_rects(),
    ));
    assert_box(&boxes, "a0000001", [40.0, 10.0, 50.0, 20.0], case);
    assert_box(&boxes, "b0000001", [90.0, 10.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [170.0, 10.0, 30.0, 40.0], case);

    let case = "the same row, packed to the end of both axes, inside the right and bottom padding";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(300.0),
        flex(json!({"padding": padding, "justifyContent": "end", "alignItems": "end"})),
        three_rects(),
    ));
    assert_box(&boxes, "a0000001", [220.0, 250.0, 50.0, 20.0], case);
    assert_box(&boxes, "b0000001", [270.0, 240.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [350.0, 230.0, 30.0, 40.0], case);

    let case = "a board that hugs its row adds its padding around the children";
    let boxes = lay_out(&board(
        "brd00001",
        json!("hug"),
        json!("hug"),
        flex(json!({"padding": padding})),
        three_rects(),
    ));
    assert_box(&boxes, "brd00001", [0.0, 0.0, 220.0, 80.0], case);
    assert_box(&boxes, "a0000001", [40.0, 10.0, 50.0, 20.0], case);
}

#[test]
fn wrap_moves_children_that_do_not_fit_onto_new_lines() {
    let gap = json!({"row": 10, "column": 20});

    let case = "five 80 x 30 boxes wrapping in a 200 px wide row that hugs its height";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(200.0),
        json!("hug"),
        flex(json!({"wrap": "wrap", "gap": gap})),
        five_rects(),
    ));
    assert_box(&boxes, "brd00001", [0.0, 0.0, 200.0, 110.0], case);
    assert_box(&boxes, "a0000001", [0.0, 0.0, 80.0, 30.0], case);
    assert_box(&boxes, "b0000001", [100.0, 0.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [0.0, 40.0, 80.0, 30.0], case);
    assert_box(&boxes, "d0000001", [100.0, 40.0, 80.0, 30.0], case);
    assert_box(&boxes, "e0000001", [0.0, 80.0, 80.0, 30.0], case);

    let case = "the same row with wrap-reverse stacks its lines from the bottom";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(200.0),
        json!("hug"),
        flex(json!({"wrap": "wrap-reverse", "gap": gap})),
        five_rects(),
    ));
    assert_box(&boxes, "brd00001", [0.0, 0.0, 200.0, 110.0], case);
    assert_box(&boxes, "a0000001", [0.0, 80.0, 80.0, 30.0], case);
    assert_box(&boxes, "b0000001", [100.0, 80.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [0.0, 40.0, 80.0, 30.0], case);
    assert_box(&boxes, "d0000001", [100.0, 40.0, 80.0, 30.0], case);
    assert_box(&boxes, "e0000001", [0.0, 0.0, 80.0, 30.0], case);

    for layout in [
        flex(json!({"gap": gap})),
        flex(json!({"wrap": "nowrap", "gap": gap})),
    ] {
        let case = format!("the same boxes in a row that does not wrap ({layout}) overflow it");
        let boxes = lay_out(&board(
            "brd00001",
            fixed(200.0),
            json!("hug"),
            layout,
            five_rects(),
        ));
        assert_box(&boxes, "brd00001", [0.0, 0.0, 200.0, 30.0], &case);
        assert_box(&boxes, "c0000001", [200.0, 0.0, 80.0, 30.0], &case);
        assert_box(&boxes, "e0000001", [400.0, 0.0, 80.0, 30.0], &case);
    }

    let case = "five 80 x 30 boxes wrapping in a 300 x 100 column";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(300.0),
        fixed(100.0),
        flex(json!({"direction": "column", "wrap": "wrap", "gap": gap})),
        five_rects(),
    ));
    // Three 80 px columns and two 20 px gaps leave 20 px, which the columns
    // share: each grows to 86.67 px (`align-content: normal`).
    let third = 20.0 / 3.0;
    assert_box(&boxes, "a0000001", [0.0, 0.0, 80.0, 30.0], case);
    assert_box(&boxes, "b0000001", [0.0, 40.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [100.0 + third, 0.0, 80.0, 30.0], case);
    assert_box(&boxes, "d0000001", [100.0 + third, 40.0, 80.0, 30.0], case);
    assert_box(
        &boxes,
        "e0000001",
        [200.0 + 2.0 * third, 0.0, 80.0, 30.0],
        case,
    );
}

#[test]
fn wrapped_lines_share_the_free_cross_space() {
    // `align-content` keeps its CSS initial value, `normal`, which stretches
    // the lines of a wrapping flex container that has room to spare. The fifth
    // child hugs its height, so it stretches to its line.
    let mut children = five_rects();
    children[4] = with(
        &hugging("e0000001", "e1000001", 10.0, 10.0),
        "width",
        fixed(80.0),
    );
    let boxes = lay_out(&board(
        "brd00001",
        fixed(200.0),
        fixed(200.0),
        flex(json!({"wrap": "wrap", "gap": {"row": 10, "column": 20}})),
        children,
    ));
    // Lines of 30, 30 and 10 px with two 10 px gaps leave 110 px, which the
    // three lines share equally.
    let share = 110.0 / 3.0;
    let case = "five children wrapping onto three lines in a 200 x 200 row";
    assert_box(&boxes, "a0000001", [0.0, 0.0, 80.0, 30.0], case);
    assert_box(&boxes, "b0000001", [100.0, 0.0, 80.0, 30.0], case);
    assert_box(&boxes, "c0000001", [0.0, 40.0 + share, 80.0, 30.0], case);
    assert_box(&boxes, "d0000001", [100.0, 40.0 + share, 80.0, 30.0], case);
    assert_box(
        &boxes,
        "e0000001",
        [0.0, 80.0 + 2.0 * share, 80.0, 10.0 + share],
        case,
    );
}

#[test]
fn align_items_places_children_on_the_cross_axis_of_a_row() {
    // `b` is 40 px wide and hugs a 10 x 10 box, so its own height is 10 px.
    let children = || {
        vec![
            rect("a0000001", 50.0, 20.0),
            with(
                &hugging("b0000001", "b1000001", 10.0, 10.0),
                "width",
                fixed(40.0),
            ),
        ]
    };
    for (align, a_y, b_y, b_height) in [
        (Some("start"), 0.0, 0.0, 10.0),
        (Some("center"), 40.0, 45.0, 10.0),
        (Some("end"), 80.0, 90.0, 10.0),
        (Some("stretch"), 0.0, 0.0, 100.0),
        (None, 0.0, 0.0, 100.0),
    ] {
        let layout = match align {
            Some(align) => flex(json!({"alignItems": align})),
            None => flex(json!({})),
        };
        let case = format!("a 300 x 100 row with alignItems {align:?}");
        let boxes = lay_out(&board(
            "brd00001",
            fixed(300.0),
            fixed(100.0),
            layout,
            children(),
        ));
        assert_box(&boxes, "a0000001", [0.0, a_y, 50.0, 20.0], &case);
        assert_box(&boxes, "b0000001", [50.0, b_y, 40.0, b_height], &case);
    }
}

#[test]
fn align_items_places_children_on_the_cross_axis_of_a_column() {
    // `b` is 30 px tall and hugs a 10 x 10 box, so its own width is 10 px.
    let children = || {
        vec![
            rect("a0000001", 50.0, 20.0),
            with(
                &hugging("b0000001", "b1000001", 10.0, 10.0),
                "height",
                fixed(30.0),
            ),
        ]
    };
    for (align, a_x, b_x, b_width) in [
        ("start", 0.0, 0.0, 10.0),
        ("center", 125.0, 145.0, 10.0),
        ("end", 250.0, 290.0, 10.0),
        ("stretch", 0.0, 0.0, 300.0),
    ] {
        let case = format!("a 300 x 200 column with alignItems {align}");
        let boxes = lay_out(&board(
            "brd00001",
            fixed(300.0),
            fixed(200.0),
            flex(json!({"direction": "column", "alignItems": align})),
            children(),
        ));
        assert_box(&boxes, "a0000001", [a_x, 0.0, 50.0, 20.0], &case);
        assert_box(&boxes, "b0000001", [b_x, 20.0, b_width, 30.0], &case);
    }
}

#[test]
fn align_items_baseline_lines_up_the_first_baselines_of_a_row() {
    // Lines of Inter are lineHeight x fontSize tall, with the leading split
    // evenly above and below the glyphs (CSS half-leading), so the baseline
    // of a 32 px line with lineHeight 1.5 is 35 to 35.6 px below its top, and
    // that of a 16 px line 17.8 to 18 px: Chrome rounds the font's ascent and
    // descent to whole pixels first. A box without text has no baseline, so
    // its bottom edge stands in for one. The 16 px text then sits 17 to 17.8
    // px down, and the box 15 to 15.6 px down.
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(100.0),
        flex(json!({"alignItems": "baseline"})),
        vec![
            text("txt00001", "Hello", 32.0, 1.5),
            text("txt00002", "Hello", 16.0, 1.5),
            rect("c0000001", 20.0, 20.0),
        ],
    ));
    let case = "a 32 px text, a 16 px text and a 20 x 20 box aligned on their baselines";
    let first = box_of(&boxes, "txt00001");
    assert!(
        first.y.abs() <= 0.01 && (first.height - 48.0).abs() <= 0.01,
        "{case}: the 32 px text has the deepest baseline, so it sits at the top of the row as a 48 px line; layout gave {first:?}"
    );
    let second = box_of(&boxes, "txt00002");
    assert_box_within(
        &boxes,
        "txt00002",
        [first.x + first.width, 17.4, second.width, 24.0],
        0.5,
        case,
    );
    assert_box_within(
        &boxes,
        "c0000001",
        [second.x + second.width, 15.3, 20.0, 20.0],
        0.5,
        case,
    );
}

#[test]
fn justify_content_spreads_the_free_space_along_the_main_axis() {
    // Three 50 px boxes in a 300 px row leave 150 px free.
    for (justify, xs) in [
        (Some("start"), [0.0, 50.0, 100.0]),
        (None, [0.0, 50.0, 100.0]),
        (Some("center"), [75.0, 125.0, 175.0]),
        (Some("end"), [150.0, 200.0, 250.0]),
        (Some("space-between"), [0.0, 125.0, 250.0]),
        (Some("space-around"), [25.0, 125.0, 225.0]),
        (Some("space-evenly"), [37.5, 125.0, 212.5]),
    ] {
        let layout = match justify {
            Some(justify) => flex(json!({"justifyContent": justify})),
            None => flex(json!({})),
        };
        let case = format!("three 50 px boxes in a 300 px row with justifyContent {justify:?}");
        let boxes = lay_out(&board(
            "brd00001",
            fixed(300.0),
            fixed(50.0),
            layout,
            vec![
                rect("a0000001", 50.0, 20.0),
                rect("b0000001", 50.0, 20.0),
                rect("c0000001", 50.0, 20.0),
            ],
        ));
        for (id, x) in ["a0000001", "b0000001", "c0000001"].iter().zip(xs) {
            assert_box(&boxes, id, [x, 0.0, 50.0, 20.0], &case);
        }
    }

    // With 10 px of padding on the left, 20 px on the right and 15 px gaps,
    // the same boxes leave 90 px free.
    for (justify, xs) in [
        ("center", [55.0, 120.0, 185.0]),
        ("end", [100.0, 165.0, 230.0]),
        ("space-between", [10.0, 120.0, 230.0]),
        ("space-around", [25.0, 120.0, 215.0]),
        ("space-evenly", [32.5, 120.0, 207.5]),
    ] {
        let case = format!(
            "three 50 px boxes in a padded 300 px row with gaps and justifyContent {justify}"
        );
        let boxes = lay_out(&board(
            "brd00001",
            fixed(300.0),
            fixed(50.0),
            flex(json!({
                "justifyContent": justify,
                "gap": {"column": 15},
                "padding": {"left": 10, "right": 20}
            })),
            vec![
                rect("a0000001", 50.0, 20.0),
                rect("b0000001", 50.0, 20.0),
                rect("c0000001", 50.0, 20.0),
            ],
        ));
        for (id, x) in ["a0000001", "b0000001", "c0000001"].iter().zip(xs) {
            assert_box(&boxes, id, [x, 0.0, 50.0, 20.0], &case);
        }
    }

    let case = "three 50 px boxes centered in a 300 px column";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(50.0),
        fixed(300.0),
        flex(json!({"direction": "column", "justifyContent": "center"})),
        vec![
            rect("a0000001", 20.0, 50.0),
            rect("b0000001", 20.0, 50.0),
            rect("c0000001", 20.0, 50.0),
        ],
    ));
    assert_box(&boxes, "a0000001", [0.0, 75.0, 20.0, 50.0], case);
    assert_box(&boxes, "b0000001", [0.0, 125.0, 20.0, 50.0], case);
    assert_box(&boxes, "c0000001", [0.0, 175.0, 20.0, 50.0], case);
}

#[test]
fn fixed_children_keep_their_size_when_the_line_overflows() {
    let case = "three 100 px boxes in a 200 px row";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(200.0),
        fixed(100.0),
        flex(json!({})),
        vec![
            rect("a0000001", 100.0, 20.0),
            rect("b0000001", 100.0, 20.0),
            rect("c0000001", 100.0, 20.0),
        ],
    ));
    assert_box(&boxes, "a0000001", [0.0, 0.0, 100.0, 20.0], case);
    assert_box(&boxes, "b0000001", [100.0, 0.0, 100.0, 20.0], case);
    assert_box(&boxes, "c0000001", [200.0, 0.0, 100.0, 20.0], case);

    let case = "three 100 px boxes in a 200 px column";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(100.0),
        fixed(200.0),
        flex(json!({"direction": "column"})),
        vec![
            rect("a0000001", 20.0, 100.0),
            rect("b0000001", 20.0, 100.0),
            rect("c0000001", 20.0, 100.0),
        ],
    ));
    assert_box(&boxes, "a0000001", [0.0, 0.0, 20.0, 100.0], case);
    assert_box(&boxes, "b0000001", [0.0, 100.0, 20.0, 100.0], case);
    assert_box(&boxes, "c0000001", [0.0, 200.0, 20.0, 100.0], case);
}

#[test]
fn hug_children_take_the_size_of_their_content() {
    let row = frame(
        "frm00001",
        json!("hug"),
        json!("hug"),
        flex(json!({
            "gap": {"column": 5},
            "padding": {"top": 10, "right": 20, "bottom": 30, "left": 40}
        })),
        vec![rect("a0000001", 50.0, 20.0), rect("b0000001", 30.0, 40.0)],
    );
    let column = frame(
        "frm00002",
        json!("hug"),
        json!("hug"),
        flex(json!({"direction": "column", "gap": {"row": 5}})),
        vec![rect("c0000001", 50.0, 20.0), rect("d0000001", 30.0, 40.0)],
    );
    let boxes = lay_out(&board(
        "brd00001",
        fixed(500.0),
        fixed(200.0),
        flex(json!({"alignItems": "start"})),
        vec![row, column],
    ));
    let case = "a padded row frame that hugs a 50 x 20 and a 30 x 40 box";
    assert_box(&boxes, "frm00001", [0.0, 0.0, 145.0, 80.0], case);
    assert_box(&boxes, "a0000001", [40.0, 10.0, 50.0, 20.0], case);
    assert_box(&boxes, "b0000001", [95.0, 10.0, 30.0, 40.0], case);
    let case = "a column frame that hugs a 50 x 20 and a 30 x 40 box";
    assert_box(&boxes, "frm00002", [145.0, 0.0, 50.0, 65.0], case);
    assert_box(&boxes, "c0000001", [145.0, 0.0, 50.0, 20.0], case);
    assert_box(&boxes, "d0000001", [145.0, 25.0, 30.0, 40.0], case);
}

#[test]
fn fill_children_share_the_free_space_of_the_main_axis_equally() {
    let case = "a 100 px box and two fill children in a 500 px row with 20 px gaps";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(500.0),
        fixed(100.0),
        flex(json!({"alignItems": "start", "gap": {"column": 20}})),
        vec![
            rect("a0000001", 100.0, 20.0),
            sized("b0000001", json!("fill"), fixed(20.0)),
            sized("c0000001", json!("fill"), fixed(20.0)),
        ],
    ));
    assert_box(&boxes, "b0000001", [120.0, 0.0, 180.0, 20.0], case);
    assert_box(&boxes, "c0000001", [320.0, 0.0, 180.0, 20.0], case);

    let case =
        "a fill text and an empty fill frame share a 300 px row equally, whatever their content";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(300.0),
        fixed(100.0),
        flex(json!({"alignItems": "start"})),
        vec![
            with(&text("txt00001", "Hi", 16.0, 1.5), "width", json!("fill")),
            sized("d0000001", json!("fill"), fixed(20.0)),
        ],
    ));
    assert_box(&boxes, "txt00001", [0.0, 0.0, 150.0, 24.0], case);
    assert_box(&boxes, "d0000001", [150.0, 0.0, 150.0, 20.0], case);

    let case = "a 100 px box and two fill children in a 300 px column";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(100.0),
        fixed(300.0),
        flex(json!({"direction": "column", "alignItems": "start"})),
        vec![
            rect("a0000001", 20.0, 100.0),
            sized("b0000001", fixed(20.0), json!("fill")),
            sized("c0000001", fixed(20.0), json!("fill")),
        ],
    ));
    assert_box(&boxes, "b0000001", [0.0, 100.0, 20.0, 100.0], case);
    assert_box(&boxes, "c0000001", [0.0, 200.0, 20.0, 100.0], case);
}

#[test]
fn fill_on_the_cross_axis_stretches_whatever_the_alignment() {
    let case =
        "a fill height in a centered 300 x 100 row with 10 px of padding on top and 20 px below";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(300.0),
        fixed(100.0),
        flex(json!({"alignItems": "center", "padding": {"top": 10, "bottom": 20}})),
        vec![
            sized("a0000001", fixed(50.0), json!("fill")),
            rect("b0000001", 50.0, 20.0),
        ],
    ));
    assert_box(&boxes, "a0000001", [0.0, 10.0, 50.0, 70.0], case);
    assert_box(&boxes, "b0000001", [50.0, 35.0, 50.0, 20.0], case);

    let case = "a fill width in an end-aligned 300 x 200 column with 10 px of padding left and 30 px right";
    let boxes = lay_out(&board(
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
    ));
    assert_box(&boxes, "c0000001", [10.0, 0.0, 260.0, 20.0], case);
    assert_box(&boxes, "d0000001", [220.0, 20.0, 50.0, 20.0], case);
}

#[test]
fn min_and_max_sizes_bound_fill_children() {
    let case = "a fill child with maxWidth 150 next to another fill child in a 400 px row";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(100.0),
        flex(json!({"alignItems": "start"})),
        vec![
            with(
                &sized("a0000001", json!("fill"), fixed(20.0)),
                "maxWidth",
                json!(150),
            ),
            sized("b0000001", json!("fill"), fixed(20.0)),
        ],
    ));
    assert_box(&boxes, "a0000001", [0.0, 0.0, 150.0, 20.0], case);
    assert_box(&boxes, "b0000001", [150.0, 0.0, 250.0, 20.0], case);

    let case = "a fill child with minWidth 200 next to another fill child in a 300 px row";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(300.0),
        fixed(100.0),
        flex(json!({"alignItems": "start"})),
        vec![
            with(
                &sized("c0000001", json!("fill"), fixed(20.0)),
                "minWidth",
                json!(200),
            ),
            sized("d0000001", json!("fill"), fixed(20.0)),
        ],
    ));
    assert_box(&boxes, "c0000001", [0.0, 0.0, 200.0, 20.0], case);
    assert_box(&boxes, "d0000001", [200.0, 0.0, 100.0, 20.0], case);

    let case = "fill heights in a 400 px column, the first with maxHeight 100";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(100.0),
        fixed(400.0),
        flex(json!({"direction": "column", "alignItems": "start"})),
        vec![
            with(
                &sized("e0000001", fixed(20.0), json!("fill")),
                "maxHeight",
                json!(100),
            ),
            sized("f0000001", fixed(20.0), json!("fill")),
        ],
    ));
    assert_box(&boxes, "e0000001", [0.0, 0.0, 20.0, 100.0], case);
    assert_box(&boxes, "f0000001", [0.0, 100.0, 20.0, 300.0], case);
}

#[test]
fn min_and_max_sizes_bound_fixed_children_and_a_min_beats_a_max() {
    let fixed_rect = |id: &str, width: f64, bounds: Value| {
        let mut json = rect(id, width, 20.0);
        for (key, value) in bounds.as_object().expect("bounds are an object") {
            json = with(&json, key, value.clone());
        }
        json
    };
    let boxes = lay_out(&board(
        "brd00001",
        fixed(700.0),
        fixed(100.0),
        flex(json!({"alignItems": "start"})),
        vec![
            fixed_rect("a0000001", 300.0, json!({"maxWidth": 200})),
            fixed_rect("b0000001", 100.0, json!({"minWidth": 150})),
            fixed_rect("c0000001", 100.0, json!({"minWidth": 250, "maxWidth": 200})),
        ],
    ));
    let case = "fixed widths of 300 with maxWidth 200, 100 with minWidth 150, and 100 with minWidth 250 and maxWidth 200";
    assert_box(&boxes, "a0000001", [0.0, 0.0, 200.0, 20.0], case);
    assert_box(&boxes, "b0000001", [200.0, 0.0, 150.0, 20.0], case);
    assert_box(&boxes, "c0000001", [350.0, 0.0, 250.0, 20.0], case);
}

#[test]
fn min_and_max_sizes_bound_hug_children() {
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        fixed(200.0),
        flex(json!({"alignItems": "start"})),
        vec![
            with(
                &with(
                    &hugging("a0000001", "a1000001", 10.0, 10.0),
                    "minWidth",
                    json!(60),
                ),
                "minHeight",
                json!(50),
            ),
            with(
                &with(
                    &hugging("b0000001", "b1000001", 80.0, 30.0),
                    "maxWidth",
                    json!(60),
                ),
                "maxHeight",
                json!(20),
            ),
        ],
    ));
    let case = "a frame hugging a 10 x 10 box with minWidth 60 and minHeight 50";
    assert_box(&boxes, "a0000001", [0.0, 0.0, 60.0, 50.0], case);
    let case = "a frame hugging an 80 x 30 box with maxWidth 60 and maxHeight 20";
    assert_box(&boxes, "b0000001", [60.0, 0.0, 60.0, 20.0], case);
}

#[test]
fn min_and_max_sizes_bound_stretched_children() {
    let boxes = lay_out(&board(
        "brd00001",
        fixed(300.0),
        fixed(100.0),
        flex(json!({})),
        vec![
            with(
                &with(
                    &hugging("a0000001", "a1000001", 10.0, 10.0),
                    "width",
                    fixed(50.0),
                ),
                "maxHeight",
                json!(40),
            ),
            with(
                &sized("b0000001", fixed(50.0), json!("fill")),
                "minHeight",
                json!(150),
            ),
        ],
    ));
    let case = "a hug height that the row stretches, with maxHeight 40";
    assert_box(&boxes, "a0000001", [0.0, 0.0, 50.0, 40.0], case);
    let case = "a fill height in a 100 px row, with minHeight 150";
    assert_box(&boxes, "b0000001", [50.0, 0.0, 50.0, 150.0], case);
}
