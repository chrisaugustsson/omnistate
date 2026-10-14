//! US-003: flex layouts place and size their children by CSS flexbox: row and
//! column directions, gap, padding, wrapping, alignment, `fixed`, `hug` and
//! `fill` sizing on each axis, and min and max sizes. Rows that hug their
//! width take CSS's fit-content width where CSS gives them one.

mod design;

use design::{
    EPSILON, SIX_WORDS, absolute, assert_box, assert_box_within, assert_boxes, assert_cases, at,
    board, cases, fixed, flex, frame, grid, hugging, lay_out, none, rect, sized, text, with,
};
use serde_json::{Value, json};

/// How far a box that text sizes may be from the box Chrome gives it, which
/// keeps text widths to 1/64 px.
const TEXT_TOLERANCE: f32 = 0.05;

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

/// A board that is a `width` px wide column, aligning its children by
/// `align`, and hugging its height.
fn column(width: f64, align: &str, children: Vec<Value>) -> Value {
    board(
        "brd00001",
        fixed(width),
        json!("hug"),
        flex(json!({"direction": "column", "alignItems": align})),
        children,
    )
}

/// A row frame that hugs both axes and holds one text of six words of 16 px
/// Inter: about 250 px wide on one line, and 24 px lines.
fn row_hugging_six_words() -> Value {
    frame(
        "frm00001",
        json!("hug"),
        json!("hug"),
        flex(json!({})),
        vec![text("txt00001", SIX_WORDS, 16.0, 1.5)],
    )
}

/// A chip: a frame that hugs a label of 16 px Inter on 24 px lines, padded
/// 4 px above and below it and 8 px beside it.
fn chip(id: &str, label_id: &str, label: &str) -> Value {
    frame(
        id,
        json!("hug"),
        json!("hug"),
        flex(json!({"padding": {"top": 4, "right": 8, "bottom": 4, "left": 8}})),
        vec![text(label_id, label, 16.0, 1.5)],
    )
}

/// A row frame that hugs both axes and wraps three chips onto lines, 8 px
/// apart both ways: "Design" (68.46 px wide in Chrome), "Research"
/// (86.66 px) and a long label whose chip is 287.59 px wide on one line.
fn wrapping_row_of_chips() -> Value {
    frame(
        "frm00001",
        json!("hug"),
        json!("hug"),
        flex(json!({"wrap": "wrap", "gap": {"row": 8, "column": 8}})),
        vec![
            chip("c0000001", "t0000001", "Design"),
            chip("c0000002", "t0000002", "Research"),
            chip(
                "c0000003",
                "t0000003",
                "A much longer chip label that wraps",
            ),
        ],
    )
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
    // share (`align-content: normal`): Chrome puts the second and the third at
    // x 106.66 and 213.34, to its 1/128 px.
    let expected = [
        ("a0000001", [0.0, 0.0, 80.0, 30.0]),
        ("b0000001", [0.0, 40.0, 80.0, 30.0]),
        ("c0000001", [106.6640625, 0.0, 80.0, 30.0]),
        ("d0000001", [106.6640625, 40.0, 80.0, 30.0]),
        ("e0000001", [213.3359375, 0.0, 80.0, 30.0]),
    ];
    assert_boxes(&boxes, &expected, EPSILON, case);
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
    // three lines share equally, to Chrome's 1/128 px: each grows by 36.66 px.
    let case = "five children wrapping onto three lines in a 200 x 200 row";
    let expected = [
        ("a0000001", [0.0, 0.0, 80.0, 30.0]),
        ("b0000001", [100.0, 0.0, 80.0, 30.0]),
        ("c0000001", [0.0, 76.6640625, 80.0, 30.0]),
        ("d0000001", [100.0, 76.6640625, 80.0, 30.0]),
        ("e0000001", [0.0, 153.3359375, 80.0, 46.6640625]),
    ];
    assert_boxes(&boxes, &expected, EPSILON, case);
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
    // Lines of Inter are lineHeight x fontSize tall, with half the leading
    // above the glyphs (CSS half-leading). Chrome rounds the font's ascent and
    // descent to whole device pixels, and that half of the leading down to a
    // whole device pixel. At the device pixel ratio of 2 of the renderer gate,
    // a device pixel is half a CSS pixel. A box without text has no baseline,
    // so its bottom edge stands in for one. The row lines every first
    // baseline up with the deepest one. These are Chrome's boxes at that
    // ratio: baselines 35.5 px below the top of the 32 px line, and 17.5 px
    // below that of the 16 px line.
    let case = "a 32 px text, a 16 px text and a 20 x 20 box aligned on their baselines in a 400 x 100 row";
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
    let expected = [
        ("txt00001", [0.0, 0.0, 77.125, 48.0]),
        ("txt00002", [77.125, 18.0, 38.5625, 24.0]),
        ("c0000001", [115.6875, 15.5, 20.0, 20.0]),
    ];
    assert_boxes(&boxes, &expected, TEXT_TOLERANCE, case);
}

#[test]
fn align_items_baseline_lines_up_texts_without_a_line_height_by_their_normal_line_height() {
    // The line of a text without a lineHeight is its font's normal line height
    // at full precision (decision Q44), which is shorter than Chrome's rounded
    // ascent and descent at 16 px and below: half the leading is then negative,
    // and Chrome rounds it down to a whole device pixel.
    assert_cases(cases::normal_line_heights_on_baselines());
}

#[test]
fn a_container_aligned_on_its_first_baseline_takes_it_from_the_item_chrome_takes_it_from() {
    // A row that runs in reverse takes its first baseline from its last item,
    // the first along its main axis, a row that wraps in reverse from the line
    // at its top, its last line, and a grid from its first item's own first
    // baseline, not from that item's bottom edge. So a row-reverse of a 16 px
    // and a 24 px text sits 9 px below the top of a row that lines it up with
    // a 32 px text, on the baseline of the 24 px text, and a grid of a 16 px
    // text 18 px below it.
    assert_cases(cases::first_baselines_of_containers());
}

#[test]
fn lines_and_items_that_overflow_a_container_in_reverse_go_where_chrome_puts_them() {
    // A row that wraps in reverse packs its lines against its far edge, the
    // bottom, so lines that need more than its height overflow its top: two
    // 60 x 30 boxes in a 100 x 40 row are at y 10 and -20. A row that runs in
    // reverse with justifyContent space-between packs a line that overflows it
    // against its main start, the right edge: two 60 px boxes in a 100 px row
    // are at x 40 and -20. Columns do the same along their axes. Items that
    // alignItems baseline aligns in a column that wraps in reverse start
    // together at the start of their line.
    assert_cases(cases::lines_that_overflow());
}

#[test]
fn align_items_start_and_end_follow_the_page_when_lines_wrap_in_reverse() {
    // `wrap-reverse` stacks the lines of a row from the bottom up, which
    // moves `flex-start` to the bottom of each line. `start` and `end` are the
    // CSS keywords, so they still align each child to the top and the bottom
    // of its line. Here lines of 40 and 30 px and a 10 px gap leave 120 of
    // the 200 px, which stretch each line by 60 px: the first line runs from
    // y 100 to 200, and the second from y 0 to 90.
    for (align, [a, b, c, d]) in [
        (
            "start",
            [
                [0.0, 100.0, 80.0, 20.0],
                [100.0, 100.0, 80.0, 40.0],
                [0.0, 0.0, 80.0, 30.0],
                [100.0, 0.0, 80.0, 10.0],
            ],
        ),
        (
            "end",
            [
                [0.0, 180.0, 80.0, 20.0],
                [100.0, 160.0, 80.0, 40.0],
                [0.0, 60.0, 80.0, 30.0],
                [100.0, 80.0, 80.0, 10.0],
            ],
        ),
    ] {
        let case =
            format!("four boxes wrapping in reverse in a 200 x 200 row with alignItems {align}");
        let boxes = lay_out(&board(
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
        ));
        assert_box(&boxes, "a0000001", a, &case);
        assert_box(&boxes, "b0000001", b, &case);
        assert_box(&boxes, "c0000001", c, &case);
        assert_box(&boxes, "d0000001", d, &case);
    }

    // A column that wraps in reverse stacks its lines from the right, but
    // `start` still aligns to the left of a line and `end` to its right.
    for (align, a_x) in [("start", 0.0), ("end", 30.0)] {
        let case = format!(
            "a 20 x 20 and a 50 x 40 box in a column that wraps in reverse, with alignItems {align}"
        );
        let boxes = lay_out(&board(
            "brd00001",
            json!("hug"),
            json!("hug"),
            flex(json!({
                "direction": "column",
                "wrap": "wrap-reverse",
                "alignItems": align
            })),
            vec![rect("a0000001", 20.0, 20.0), rect("b0000001", 50.0, 40.0)],
        ));
        assert_box(&boxes, "a0000001", [a_x, 0.0, 20.0, 20.0], &case);
        assert_box(&boxes, "b0000001", [0.0, 20.0, 50.0, 40.0], &case);
    }
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
fn justify_content_start_and_end_follow_the_page_in_reversed_directions() {
    // `start` and `end` are the CSS keywords, not `flex-start` and
    // `flex-end`: they pack the children against the left or top edge and the
    // right or bottom edge, whatever `direction` says, in reversed order.
    for (direction, justify, height, [a, b, c]) in [
        (
            "row-reverse",
            "start",
            100.0,
            [
                [110.0, 0.0, 50.0, 20.0],
                [30.0, 0.0, 80.0, 30.0],
                [0.0, 0.0, 30.0, 40.0],
            ],
        ),
        (
            "row-reverse",
            "end",
            100.0,
            [
                [350.0, 0.0, 50.0, 20.0],
                [270.0, 0.0, 80.0, 30.0],
                [240.0, 0.0, 30.0, 40.0],
            ],
        ),
        (
            "column-reverse",
            "start",
            300.0,
            [
                [0.0, 70.0, 50.0, 20.0],
                [0.0, 40.0, 80.0, 30.0],
                [0.0, 0.0, 30.0, 40.0],
            ],
        ),
        (
            "column-reverse",
            "end",
            300.0,
            [
                [0.0, 280.0, 50.0, 20.0],
                [0.0, 250.0, 80.0, 30.0],
                [0.0, 210.0, 30.0, 40.0],
            ],
        ),
    ] {
        let case = format!("a 400 x {height} {direction} with justifyContent {justify}");
        let boxes = lay_out(&board(
            "brd00001",
            fixed(400.0),
            fixed(height),
            flex(json!({"direction": direction, "justifyContent": justify})),
            three_rects(),
        ));
        assert_box(&boxes, "a0000001", a, &case);
        assert_box(&boxes, "b0000001", b, &case);
        assert_box(&boxes, "c0000001", c, &case);
    }
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
fn a_hug_row_fits_into_the_space_where_css_gives_it_a_fit_content_width() {
    // A row that hugs its width is a flex container with `width: auto`. Where
    // CSS makes that width fit-content, the row is as wide as its content on
    // one line, its max-content width, but no wider than the space unless its
    // min-content width is wider still. Six words need about 250 px on one
    // line, so in 150 px the row is 150 px wide, and its text shrinks with it
    // and wraps onto two lines of three words.
    for align in ["start", "center", "end"] {
        let case = format!("a row hugging six words in a 150 px column with alignItems {align}");
        let boxes = lay_out(&column(150.0, align, vec![row_hugging_six_words()]));
        assert_box(&boxes, "frm00001", [0.0, 0.0, 150.0, 48.0], &case);
        assert_box(&boxes, "txt00001", [0.0, 0.0, 150.0, 48.0], &case);
    }

    let case = "a row hugging six words in a grid area 150 px wide";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        json!("hug"),
        grid(json!({"columns": [{"fixed": 150}, {"fr": 1}]})),
        vec![row_hugging_six_words()],
    ));
    assert_box(&boxes, "frm00001", [0.0, 0.0, 150.0, 48.0], case);
    assert_box(&boxes, "txt00001", [0.0, 0.0, 150.0, 48.0], case);

    // A positioned row fits into the space between its x and its parent's
    // right edge: CSS's shrink-to-fit width.
    let case = "a row hugging six words at x 50 in a 200 px board with no layout";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(200.0),
        fixed(200.0),
        none(),
        vec![at(&row_hugging_six_words(), 50.0, 10.0)],
    ));
    assert_box(&boxes, "frm00001", [50.0, 10.0, 150.0, 48.0], case);
    assert_box(&boxes, "txt00001", [50.0, 10.0, 150.0, 48.0], case);

    let case = "an absolute row hugging six words at x 50 in a padded 200 px row";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(200.0),
        fixed(200.0),
        flex(json!({"padding": {"top": 10, "right": 10, "bottom": 10, "left": 10}})),
        vec![absolute(&row_hugging_six_words(), 50.0, 10.0)],
    ));
    assert_box(&boxes, "frm00001", [50.0, 10.0, 150.0, 48.0], case);
    assert_box(&boxes, "txt00001", [50.0, 10.0, 150.0, 48.0], case);

    // Two texts of three words, about 125 px each on one line, shrink by the
    // same amount to share the 150 px: 75 px each, where every word takes a
    // line of its own.
    let case =
        "a row hugging two texts of three words in a 150 px column that aligns it to the start";
    let boxes = lay_out(&column(
        150.0,
        "start",
        vec![frame(
            "frm00001",
            json!("hug"),
            json!("hug"),
            flex(json!({})),
            vec![
                text("txt00001", "word word word", 16.0, 1.5),
                text("txt00002", "word word word", 16.0, 1.5),
            ],
        )],
    ));
    assert_box(&boxes, "frm00001", [0.0, 0.0, 150.0, 72.0], case);
    assert_box(&boxes, "txt00001", [0.0, 0.0, 75.0, 72.0], case);
    assert_box(&boxes, "txt00002", [75.0, 0.0, 75.0, 72.0], case);
}

#[test]
fn a_hug_row_that_wraps_fits_into_the_space_where_css_gives_it_a_fit_content_width() {
    // On one line the chips need 458.72 px, so where CSS makes the row's width
    // fit-content the row is as wide as the 200 px of space. "Design" and
    // "Research" share its first line. The long chip, wider than the row,
    // gets the second line to itself, shrinks to the row's width and wraps
    // its label onto two lines, 56 px tall with its padding, and the row is
    // 96 px tall. Chrome lays the chips out so in each of these places.
    let at_the_left = [
        ("frm00001", [0.0, 0.0, 200.0, 96.0]),
        ("c0000001", [0.0, 0.0, 68.4609375, 32.0]),
        ("c0000002", [76.4609375, 0.0, 86.6640625, 32.0]),
        ("c0000003", [0.0, 40.0, 200.0, 56.0]),
        ("t0000003", [8.0, 44.0, 184.0, 48.0]),
    ];
    let at_x_50 = [
        ("frm00001", [50.0, 0.0, 200.0, 96.0]),
        ("c0000001", [50.0, 0.0, 68.4609375, 32.0]),
        ("c0000002", [126.4609375, 0.0, 86.6640625, 32.0]),
        ("c0000003", [50.0, 40.0, 200.0, 56.0]),
        ("t0000003", [58.0, 44.0, 184.0, 48.0]),
    ];
    let cases = [
        (
            "a wrapping row of chips in a 200 px column that aligns it to the start",
            at_the_left,
            column(200.0, "start", vec![wrapping_row_of_chips()]),
        ),
        (
            "a wrapping row of chips in a 200 px column that centers it",
            at_the_left,
            column(200.0, "center", vec![wrapping_row_of_chips()]),
        ),
        (
            "a wrapping row of chips in a grid area 200 px wide",
            at_the_left,
            board(
                "brd00001",
                fixed(200.0),
                json!("hug"),
                grid(json!({"columns": [{"fr": 1}]})),
                vec![wrapping_row_of_chips()],
            ),
        ),
        (
            "a wrapping row of chips at x 50 in a 250 px board with no layout",
            at_x_50,
            board(
                "brd00001",
                fixed(250.0),
                fixed(300.0),
                none(),
                vec![at(&wrapping_row_of_chips(), 50.0, 0.0)],
            ),
        ),
    ];
    for (case, expected, design) in cases {
        assert_boxes(&lay_out(&design), &expected, TEXT_TOLERANCE, case);
    }
}

#[test]
fn a_hug_row_is_bounded_by_its_min_and_max_width_before_its_text_wraps() {
    // Where CSS makes the width of a row that hugs it fit-content, the row's
    // own min and max widths bound that width before its content is laid out
    // in it. A row hugging six words with maxWidth 150, in a 300 px grid area,
    // is 150 px wide, and its text wraps there onto two lines. One with
    // minWidth 170, in a 100 px grid area, is 170 px wide, and its text needs
    // two lines there too, where 100 px would take three.
    for (area, bound, value, width) in [
        (300.0, "maxWidth", 150, 150.0),
        (100.0, "minWidth", 170, 170.0),
    ] {
        let case =
            format!("a row hugging six words with {bound} {value} in a grid area {area} px wide");
        let boxes = lay_out(&board(
            "brd00001",
            fixed(400.0),
            json!("hug"),
            grid(json!({"columns": [{"fixed": area}, {"fr": 1}]})),
            vec![with(&row_hugging_six_words(), bound, json!(value))],
        ));
        assert_box(&boxes, "frm00001", [0.0, 0.0, width, 48.0], &case);
        assert_box(&boxes, "txt00001", [0.0, 0.0, width, 48.0], &case);
    }
}

#[test]
fn a_wrapping_row_with_a_max_width_wraps_its_items_at_that_width() {
    // CSS measures the content of a row without the row's own min and max
    // widths: on one line the chips need 458.72 px. In a 500 px column that
    // aligns it to the start, a wrapping row of chips with maxWidth 300 is
    // 300 px wide: "Design" and "Research" share its first line, and the
    // long chip (287.59 px) fits on the second line whole.
    let case =
        "a wrapping row of chips with maxWidth 300 in a 500 px column that aligns it to the start";
    let boxes = lay_out(&column(
        500.0,
        "start",
        vec![with(&wrapping_row_of_chips(), "maxWidth", json!(300))],
    ));
    assert_box(&boxes, "frm00001", [0.0, 0.0, 300.0, 72.0], case);
    let expected = [
        ("c0000001", [0.0, 0.0, 68.4609375, 32.0]),
        ("c0000002", [76.4609375, 0.0, 86.6640625, 32.0]),
        ("c0000003", [0.0, 40.0, 287.59375, 32.0]),
    ];
    assert_boxes(&boxes, &expected, TEXT_TOLERANCE, case);

    // Nothing limits the width of a row that hugs it, so a wrapping row of
    // chips with maxWidth 200 inside it is 200 px wide, and wraps as it does
    // in 200 px of space: the long chip takes the second line, shrinks to the
    // row's width and wraps its label onto two lines.
    let case = "a wrapping row of chips with maxWidth 200 in a board that hugs its row";
    let boxes = lay_out(&board(
        "brd00001",
        json!("hug"),
        json!("hug"),
        flex(json!({"alignItems": "start"})),
        vec![with(&wrapping_row_of_chips(), "maxWidth", json!(200))],
    ));
    assert_box(&boxes, "brd00001", [0.0, 0.0, 200.0, 96.0], case);
    assert_box(&boxes, "frm00001", [0.0, 0.0, 200.0, 96.0], case);
    assert_boxes(
        &boxes,
        &[("c0000002", [76.4609375, 0.0, 86.6640625, 32.0])],
        TEXT_TOLERANCE,
        case,
    );
    assert_box(&boxes, "c0000003", [0.0, 40.0, 200.0, 56.0], case);
    assert_box(&boxes, "t0000003", [8.0, 44.0, 184.0, 48.0], case);
}

#[test]
fn a_hug_row_that_wraps_counts_each_item_at_its_bounded_width() {
    // An item's max width bounds what it adds to the width of a row that
    // hugs it, whether the row wraps or not.
    let case = "a wrapping row hugging a 100 px box with maxWidth 60 and a 30 px box, 8 px apart";
    let boxes = lay_out(&board(
        "brd00001",
        json!("hug"),
        json!("hug"),
        flex(json!({"wrap": "wrap", "gap": {"column": 8}})),
        vec![
            with(&rect("a0000001", 100.0, 20.0), "maxWidth", json!(60)),
            rect("b0000001", 30.0, 20.0),
        ],
    ));
    assert_box(&boxes, "brd00001", [0.0, 0.0, 98.0, 20.0], case);
    assert_box(&boxes, "a0000001", [0.0, 0.0, 60.0, 20.0], case);
    assert_box(&boxes, "b0000001", [68.0, 0.0, 30.0, 20.0], case);

    // "input/output" never breaks, and is 80.82 px wide in 14 px Inter, but
    // with maxWidth 40 its text is 40 px wide and overflows, and so is the
    // wrapping row that hugs it.
    let case = "a wrapping row hugging \"input/output\" with maxWidth 40, in a board that hugs it";
    let boxes = lay_out(&board(
        "brd00001",
        json!("hug"),
        json!("hug"),
        flex(json!({"alignItems": "start"})),
        vec![frame(
            "frm00001",
            json!("hug"),
            json!("hug"),
            flex(json!({"wrap": "wrap"})),
            vec![with(
                &text("txt00001", "input/output", 14.0, 1.5),
                "maxWidth",
                json!(40),
            )],
        )],
    ));
    assert_box(&boxes, "brd00001", [0.0, 0.0, 40.0, 21.0], case);
    assert_box(&boxes, "frm00001", [0.0, 0.0, 40.0, 21.0], case);
    assert_box(&boxes, "txt00001", [0.0, 0.0, 40.0, 21.0], case);
}

#[test]
fn a_hug_row_that_wraps_is_as_wide_as_its_fill_items_can_shrink() {
    // A fill item grows from a flex basis of 0, but it never shrinks below
    // its min-content width, like any flex item. Where CSS makes the width of
    // a wrapping row fit-content, the row is never narrower than its widest
    // item can shrink: a row holding a fill text of "Supercalifragilistic"
    // is as wide as the word, 139.70 px, in a 50 px column as in a 100 px
    // one, where a 30 px box after the text wraps onto a second line.
    let super_text = || {
        with(
            &text("txt00001", "Supercalifragilistic", 16.0, 1.5),
            "width",
            json!("fill"),
        )
    };
    let case = "a wrapping row hugging a fill text in a 50 px column that aligns it to the start";
    let boxes = lay_out(&column(
        50.0,
        "start",
        vec![frame(
            "frm00001",
            json!("hug"),
            json!("hug"),
            flex(json!({"wrap": "wrap"})),
            vec![super_text()],
        )],
    ));
    assert_box_within(
        &boxes,
        "frm00001",
        [0.0, 0.0, 139.703, 24.0],
        TEXT_TOLERANCE,
        case,
    );
    assert_box_within(
        &boxes,
        "txt00001",
        [0.0, 0.0, 139.703, 24.0],
        TEXT_TOLERANCE,
        case,
    );

    let case = "a wrapping row hugging a fill text and a 30 px box, 8 px apart, in a 100 px column that aligns it to the start";
    let boxes = lay_out(&column(
        100.0,
        "start",
        vec![frame(
            "frm00001",
            json!("hug"),
            json!("hug"),
            flex(json!({"wrap": "wrap", "gap": {"row": 8, "column": 8}})),
            vec![super_text(), rect("a0000001", 30.0, 20.0)],
        )],
    ));
    assert_box_within(
        &boxes,
        "frm00001",
        [0.0, 0.0, 139.703, 52.0],
        TEXT_TOLERANCE,
        case,
    );
    assert_box_within(
        &boxes,
        "txt00001",
        [0.0, 0.0, 139.703, 24.0],
        TEXT_TOLERANCE,
        case,
    );
    assert_box(&boxes, "a0000001", [0.0, 32.0, 30.0, 20.0], case);
}

#[test]
fn a_hug_row_is_as_wide_as_a_column_that_wraps_inside_it() {
    // A column that wraps is as wide as the lines it wraps into, and the
    // height those lines break at can come from the row around it. Here a
    // row 100 px tall stretches a column of five 80 x 30 boxes to its height,
    // so the boxes wrap into three lines 20 px apart, and the row, which
    // hugs its width in a 600 px column that aligns it to the start, is as
    // wide as those lines: 280 px.
    let wrapping_column = || {
        frame(
            "frm00001",
            json!("hug"),
            json!("hug"),
            flex(json!({
                "direction": "column",
                "wrap": "wrap",
                "gap": {"row": 10, "column": 20}
            })),
            five_rects(),
        )
    };
    let wrapped = [
        ("a0000001", [0.0, 0.0, 80.0, 30.0]),
        ("b0000001", [0.0, 40.0, 80.0, 30.0]),
        ("c0000001", [100.0, 0.0, 80.0, 30.0]),
        ("d0000001", [100.0, 40.0, 80.0, 30.0]),
        ("e0000001", [200.0, 0.0, 80.0, 30.0]),
    ];
    let case = "a row 100 px tall, hugging its width in a 600 px column, around a column of five boxes that wraps";
    let boxes = lay_out(&column(
        600.0,
        "start",
        vec![frame(
            "frm00002",
            json!("hug"),
            fixed(100.0),
            flex(json!({})),
            vec![wrapping_column()],
        )],
    ));
    assert_box(&boxes, "frm00002", [0.0, 0.0, 280.0, 100.0], case);
    assert_box(&boxes, "frm00001", [0.0, 0.0, 280.0, 100.0], case);
    for (id, expected) in wrapped {
        assert_box(&boxes, id, expected, case);
    }

    // A column that hugs its height wraps at its maxHeight instead, and is
    // as tall as its longest line: 30 + 10 + 30 = 70 px.
    let case = "a column of five boxes that wraps at maxHeight 100, in a board that hugs it";
    let boxes = lay_out(&board(
        "brd00001",
        json!("hug"),
        json!("hug"),
        flex(json!({"alignItems": "start"})),
        vec![with(&wrapping_column(), "maxHeight", json!(100))],
    ));
    assert_box(&boxes, "brd00001", [0.0, 0.0, 280.0, 70.0], case);
    assert_box(&boxes, "frm00001", [0.0, 0.0, 280.0, 70.0], case);
    for (id, expected) in wrapped {
        assert_box(&boxes, id, expected, case);
    }
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

#[test]
fn a_child_is_never_narrower_than_its_padding() {
    // CSS never makes a box smaller than its padding: its content box is 0 px
    // at least. A frame padded 15 px on each side, holding a 4 x 4 box, is
    // 30 px wide even where a narrower column stretches it or gives it a fill
    // width, and its box sits inside its padding.
    let padded = |width: Value| {
        frame(
            "frm00001",
            width,
            json!("hug"),
            flex(json!({"padding": {"top": 10, "right": 15, "bottom": 10, "left": 15}})),
            vec![rect("a0000001", 4.0, 4.0)],
        )
    };
    for (case, column_width, align, width) in [
        (
            "a frame that a 20 px column stretches",
            20.0,
            None,
            json!("hug"),
        ),
        (
            "a frame that a 0 px column stretches",
            0.0,
            None,
            json!("hug"),
        ),
        (
            "a frame that fills the width of a 20 px column that aligns it to the start",
            20.0,
            Some("start"),
            json!("fill"),
        ),
    ] {
        let layout = match align {
            Some(align) => flex(json!({"direction": "column", "alignItems": align})),
            None => flex(json!({"direction": "column"})),
        };
        let boxes = lay_out(&board(
            "brd00001",
            fixed(column_width),
            json!("hug"),
            layout,
            vec![padded(width)],
        ));
        let case = format!("{case}, padded 15 px on each side");
        assert_box(&boxes, "frm00001", [0.0, 0.0, 30.0, 24.0], &case);
        assert_box(&boxes, "a0000001", [15.0, 10.0, 4.0, 4.0], &case);
    }
}

#[test]
fn a_fill_height_item_with_a_min_height_below_its_content_stays_at_its_min_height() {
    // A hug-height column holds a text (width fixed 60, height fill, minHeight
    // 30, "Quarterly report Supercalifragilistic Lorem ipsum dolor sit amet" in
    // 16 px Inter, lineHeight 1.5) and a 20 x 20 box. The text would need
    // 168 px at that width, but its explicit minHeight turns off the automatic
    // minimum of a flex item, and the column's height is the sum of the
    // hypothetical sizes of its items. Chrome gives the text 60 x 30, the box
    // y 30 and the column a height of 50, in a grid area, absolutely
    // positioned in a flex parent, in a `none` layout and in a fixed-height
    // row.
    assert_cases(cases::fill_height_with_min_height());
}

#[test]
fn a_max_width_clamps_the_hypothetical_size_of_a_shrinking_item_not_its_flex_basis() {
    // CSS clamps an item's min and max width in its hypothetical size, not in
    // its flex basis, which stays the width of its content. A 160 px row
    // holds a hug frame with maxWidth 100 around "alpha beta gamma delta
    // epsilon" and the text "zeta eta theta". The row shrinks both items
    // from the widths of their content, and the frame, which its max width
    // stops at 100 px, leaves 60 px to the text.
    assert_cases(cases::max_width_while_shrinking());
}

#[test]
fn hug_items_of_a_wrapping_column_are_sized_against_their_flex_lines_cross_size() {
    // In a wrapping column a hug item's width is fit into the cross size of its
    // flex line, which a wider item in the line sets, not into the inner width
    // of the container: with alignItems start, center or end a text is as wide
    // as it needs up to its line, where the container is narrower.
    assert_cases(cases::wrapping_columns());
}

#[test]
fn a_hug_row_that_wraps_sizes_items_with_a_max_width_and_fill_items_that_shrink_as_chrome_does() {
    assert_cases(cases::hug_rows_that_wrap());
}

#[test]
fn a_row_that_does_not_wrap_is_as_wide_at_its_narrowest_as_its_items_and_gaps() {
    // The min-content width of a row that does not wrap is the sum of the
    // min-content widths of its items plus the gaps: in a 50 px column that
    // aligns it to the start, a row of "alpha beta" and "gamma delta", 8 px
    // apart, is 104.914 px wide, and its texts are 41.102 and 55.812 px.
    assert_cases(cases::nowrap_hug_row());
}

#[test]
fn a_columns_content_width_includes_its_padding() {
    assert_cases(cases::column_padding());
}

#[test]
fn item_contributions_to_the_width_of_a_row_respect_their_min_width() {
    assert_cases(cases::minimum_widths());
}

#[test]
fn a_column_that_wraps_is_as_wide_at_its_narrowest_as_its_widest_item() {
    assert_cases(cases::wrapping_column_overflow());
}

#[test]
fn a_grid_counts_in_the_content_widths_of_the_row_that_holds_it() {
    assert_cases(cases::grids_in_rows());
}

#[test]
fn a_column_that_hugs_its_width_takes_its_min_width_in_a_grid_area() {
    assert_cases(cases::auto_widths_for_columns());
}

#[test]
fn fixed_sizes_are_bounded_by_min_and_max_sizes_where_a_hug_parent_counts_them() {
    assert_cases(cases::clamped_fixed_sizes());
}

#[test]
fn fill_items_never_shrink_below_their_min_content_and_count_at_their_content_in_a_hug_row() {
    // A fill item grows from a flex basis of 0, but like any flex item it
    // never shrinks below its min-content width, the width of its widest word
    // for a text. In a row that hugs its width it counts at the width of its
    // content.
    assert_cases(cases::fill_items());
}

#[test]
fn a_box_is_never_smaller_than_its_padding_on_either_axis_or_in_the_width_it_adds_to_a_row() {
    assert_cases(cases::boxes_are_never_smaller_than_their_padding());
}

#[test]
fn a_column_wraps_its_children_at_its_fixed_height_or_at_the_height_a_row_stretches_it_to() {
    // A column wraps its children at its height when that is fixed, bounded by
    // its min and max heights and never below its padding, or when a row that
    // does not wrap stretches it to the row's own fixed height, less the row's
    // padding. A row that wraps does not stretch it. A column that hugs its
    // height wraps at its maxHeight less its padding: with maxHeight 70 and 6
    // px of padding above and below, three 30 px boxes take a line each. The
    // lines it wraps into set its width, where nothing else does.
    assert_cases(cases::wrapping_columns_wrap_at_their_height());
}

#[test]
fn a_row_that_wraps_keeps_its_items_on_one_line_at_their_max_content_widths() {
    // The widths of the items and of the row's padding add up to numbers a
    // float cannot hold exactly. Chrome keeps the items on one line anyway.
    assert_cases(cases::wrapping_rows_at_their_max_content_width());
}

#[test]
fn an_empty_text_has_no_baseline_so_its_bottom_edge_stands_in_for_one() {
    assert_cases(cases::empty_texts_have_no_baseline());
}

#[test]
fn a_column_whose_content_decides_its_height_is_as_tall_as_its_items_need() {
    // Whatever the space around it, a column that hugs its height is as tall as
    // its content: a fill item gets the height of its content, the automatic
    // minimum of a flex item, where it would get nothing from a flex basis of
    // 0, and a text's maxHeight bounds it.
    assert_cases(cases::hug_columns_take_the_height_their_items_need());
}

#[test]
fn a_column_narrower_than_its_text_wraps_the_text_at_its_width_or_lets_its_line_grow() {
    // A column that stretches a text that hugs its width wraps it at the
    // column's width. A column that wraps sizes its line by the text, and its
    // flex basis, the text's height, was measured at the column's width. A
    // board that is a column of fixed height that wraps is as wide as the lines
    // it wraps into.
    assert_cases(cases::narrow_columns_wrap_their_texts());
}
