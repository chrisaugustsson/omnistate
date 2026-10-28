//! US-002: layout properties follow CSS (none, flex and grid layouts, sizing
//! per axis, min and max sizes, absolute positioning), and no computed
//! positions or transform matrices are stored.

mod common;

use common::{
    assert_distinct_values, assert_rejected, assert_round_trips, board, containing, flex_frame,
    frame, image, instance, page, parse, placed, strings, text, try_parse, vector, with, without,
};
use serde_json::{Value, json};

/// A flex frame with every flex property set.
fn flex_card() -> Value {
    json!({
        "id": "frame001",
        "type": "frame",
        "name": "Card",
        "width": {"fixed": 320},
        "height": "hug",
        "layout": {
            "mode": "flex",
            "direction": "column",
            "wrap": "wrap",
            "gap": {"row": 12, "column": 8},
            "padding": {"top": 16, "right": 24, "bottom": 20, "left": 28},
            "alignItems": "center",
            "justifyContent": "space-between"
        },
        "children": [text("text0001"), frame("frame002")]
    })
}

/// A grid frame with tracks, gaps, padding and placed children.
fn product_grid() -> Value {
    let hero = with(
        &with(
            &image("jpeg0001"),
            "/gridColumn",
            json!({"start": 1, "span": 2}),
        ),
        "/gridRow",
        json!({"start": 1, "span": 2}),
    );
    let price = with(
        &with(
            &text("text0001"),
            "/gridColumn",
            json!({"start": 3, "span": 2}),
        ),
        "/gridRow",
        json!({"start": 2, "span": 3}),
    );
    json!({
        "id": "frame002",
        "type": "frame",
        "name": "Products",
        "width": {"fixed": 960},
        "height": "hug",
        "layout": {
            "mode": "grid",
            "columns": [{"fixed": 240}, {"fr": 1}, {"fr": 2.5}, "auto"],
            "rows": ["auto", {"fixed": 180}],
            "gap": {"row": 16, "column": 24},
            "padding": {"top": 8, "right": 12, "bottom": 16, "left": 20}
        },
        "children": [hero, price, text("text0002")]
    })
}

#[test]
fn children_of_a_none_layout_are_placed_at_x_and_y() {
    let parent = containing(
        &frame("frame001"),
        vec![
            placed(&text("text0001"), 12.0, 8.0),
            placed(&vector("vec00001"), -4.5, 36.25),
            placed(&instance("btn00001"), 0.5, 120.0),
        ],
    );
    let output = assert_round_trips(&parent);
    assert_eq!(output["layout"]["mode"], "none");
}

#[test]
fn flex_keywords_are_the_css_keywords() {
    let base = flex_card();
    assert_distinct_values(
        &base,
        "/layout/direction",
        &strings(["row", "column", "row-reverse", "column-reverse"]),
    );
    assert_distinct_values(
        &base,
        "/layout/wrap",
        &strings(["nowrap", "wrap", "wrap-reverse"]),
    );
    assert_distinct_values(
        &base,
        "/layout/alignItems",
        &strings(["start", "center", "end", "stretch", "baseline"]),
    );
    assert_distinct_values(
        &base,
        "/layout/justifyContent",
        &strings([
            "start",
            "center",
            "end",
            "space-between",
            "space-around",
            "space-evenly",
        ]),
    );
}

#[test]
fn boards_and_frames_state_their_layout_and_boxes_state_their_size() {
    for json in [board("brd00001"), frame("frame001")] {
        parse(&json);
        assert_rejected(
            &without(&json, "/layout"),
            &format!(
                "a {} states its layout mode, so one without `layout`",
                json["type"]
            ),
        );
    }
    for json in [
        board("brd00001"),
        frame("frame001"),
        text("text0001"),
        image("jpeg0001"),
        vector("vec00001"),
    ] {
        parse(&json);
        for axis in ["width", "height"] {
            assert_rejected(
                &without(&json, &format!("/{axis}")),
                &format!(
                    "a {} states how its {axis} is sized, so one without `{axis}`",
                    json["type"]
                ),
            );
        }
    }
}

/// Every layout length that CSS never lets go below 0, set to `length`: the
/// fixed sizes and the min and max sizes of a frame and of an instance, the
/// gaps and padding of a flex and a grid layout, and fixed grid tracks. Each
/// comes with a name for messages.
fn non_negative_layout_lengths(length: &Value) -> Vec<(String, Value)> {
    let card = flex_card();
    let grid = product_grid();
    let button = instance("btn00001");
    let mut lengths = Vec::new();
    for (name, base) in [("a frame", &card), ("an instance", &button)] {
        for axis in ["/width", "/height"] {
            let json = with(base, axis, json!({"fixed": length}));
            lengths.push((format!("{axis} of {name}"), json));
        }
        for bound in ["/minWidth", "/maxWidth", "/minHeight", "/maxHeight"] {
            let json = with(base, bound, length.clone());
            lengths.push((format!("{bound} of {name}"), json));
        }
    }
    for (name, base) in [("flex", &card), ("grid", &grid)] {
        for spacing in [
            "/layout/gap/row",
            "/layout/gap/column",
            "/layout/padding/top",
            "/layout/padding/right",
            "/layout/padding/bottom",
            "/layout/padding/left",
        ] {
            let json = with(base, spacing, length.clone());
            lengths.push((format!("{spacing} of a {name} layout"), json));
        }
    }
    for track in ["/layout/columns/0", "/layout/rows/1"] {
        let json = with(&grid, track, json!({"fixed": length}));
        lengths.push((format!("the fixed track {track}"), json));
    }
    lengths
}

#[test]
fn layout_values_stay_in_the_ranges_css_allows() {
    for (slot, json) in non_negative_layout_lengths(&json!(0)) {
        if let Err(error) = try_parse(&json) {
            panic!("CSS allows {slot} = 0, but it was rejected: {error}");
        }
        assert_round_trips(&json);
    }
    for negative in [json!(-1), json!(-0.5), json!(-320)] {
        for (slot, json) in non_negative_layout_lengths(&negative) {
            assert_rejected(
                &json,
                &format!(
                    "CSS has no negative sizes, gaps, padding or tracks, so {slot} = {negative}"
                ),
            );
        }
    }

    let grid = product_grid();
    assert_round_trips(&with(&grid, "/layout/columns/1", json!({"fr": 0})));
    assert_rejected(
        &with(&grid, "/layout/columns/1", json!({"fr": -1})),
        "CSS has no negative `fr` tracks, so {\"fr\": -1}",
    );
    assert_round_trips(&with(&grid, "/children/0/gridColumn/start", json!(-1)));
    for (field, bad) in [
        ("start", json!(0)),
        ("start", json!(1.5)),
        ("span", json!(0)),
        ("span", json!(-2)),
    ] {
        assert_rejected(
            &with(
                &grid,
                &format!("/children/0/gridColumn/{field}"),
                bad.clone(),
            ),
            &format!(
                "grid lines are whole numbers other than 0 and spans are 1 or more, so {field} = {bad}"
            ),
        );
    }

    // x and y may be negative, as CSS `left` and `top` may: a node can start
    // left of or above its parent's edge.
    let pinned = placed(
        &with(&text("text0002"), "/absolute", json!(true)),
        -24.0,
        -0.5,
    );
    assert_round_trips(&flex_frame("frame001", vec![pinned]));
    assert_round_trips(&containing(
        &frame("frame001"),
        vec![placed(&vector("vec00001"), -12.0, -36.5)],
    ));
    assert_round_trips(&page(
        "page0001",
        vec![placed(&board("brd00001"), -1440.0, -900.5)],
    ));
}

#[test]
fn unknown_layout_values_are_rejected() {
    let base = flex_card();
    parse(&base);
    for (pointer, bad) in [
        ("/layout/mode", "block"),
        ("/layout/mode", "table"),
        ("/layout/mode", "Flex"),
        ("/layout/direction", "vertical"),
        ("/layout/wrap", "sometimes"),
        ("/layout/alignItems", "middle"),
        ("/layout/justifyContent", "between"),
        ("/width", "auto"),
        ("/width", "stretch"),
        ("/width", "fixed"),
        ("/height", "fit"),
    ] {
        assert_rejected(
            &with(&base, pointer, json!(bad)),
            &format!("{pointer} = `{bad}` is not a layout value of the model"),
        );
    }
}

#[test]
fn in_flow_children_of_flex_and_grid_layouts_store_no_position() {
    for layout in [
        json!({"mode": "flex", "direction": "row"}),
        json!({"mode": "grid", "columns": [{"fr": 1}, {"fr": 1}]}),
    ] {
        let mode = layout["mode"].clone();
        let base = with(
            &flex_frame("frame001", vec![text("text0001")]),
            "/layout",
            layout,
        );
        parse(&base);
        for key in ["x", "y"] {
            assert_rejected(
                &with(&base, &format!("/children/0/{key}"), json!(24)),
                &format!(
                    "layout computes the {key} of a child in a {mode} layout, so a stored {key}"
                ),
            );
        }
        let absolute = with(&base, "/children/0/absolute", json!(true));
        let pinned = with(
            &with(&absolute, "/children/0/x", json!(24)),
            "/children/0/y",
            json!(16),
        );
        assert_round_trips(&pinned);
    }
}

#[test]
fn no_transform_matrix_or_computed_box_is_stored() {
    let base = flex_frame("frame001", vec![text("text0001")]);
    parse(&base);
    for (key, value) in [
        ("transform", json!([1, 0, 0, 1, 40, 60])),
        (
            "matrix",
            json!({"a": 1, "b": 0, "c": 0, "d": 1, "e": 40, "f": 60}),
        ),
        (
            "absoluteBoundingBox",
            json!({"x": 40, "y": 60, "width": 120, "height": 80}),
        ),
        (
            "bounds",
            json!({"x": 40, "y": 60, "width": 120, "height": 80}),
        ),
    ] {
        for pointer in [format!("/{key}"), format!("/children/0/{key}")] {
            let message = assert_rejected(
                &with(&base, &pointer, value.clone()),
                &format!("layout is computed when rendering, so {pointer}"),
            );
            assert!(
                message.contains(key),
                "the error must name `{key}`; it says: {message}"
            );
        }
    }
}

#[test]
fn hug_and_fill_axes_store_no_size() {
    let base = flex_frame("frame001", vec![text("text0001")]);
    parse(&base);
    for (pointer, bad) in [
        ("/width", json!({"hug": 120})),
        ("/height", json!({"fill": 300})),
        ("/children/0/width", json!({"hug": 87.5})),
    ] {
        assert_rejected(
            &with(&base, pointer, bad),
            &format!("layout computes a hug or fill size, so {pointer} stores none"),
        );
    }
}
