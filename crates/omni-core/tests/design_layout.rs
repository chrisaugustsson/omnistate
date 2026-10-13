//! US-002: layout properties follow CSS (none, flex and grid layouts, sizing
//! per axis, min and max sizes, absolute positioning), and no computed
//! positions or transform matrices are stored.

mod common;

use common::{
    assert_distinct_values, assert_rejected, assert_round_trips, board, containing, flex_frame,
    frame, image, instance, parse, placed, strings, text, vector, with, without,
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
fn flex_layouts_have_direction_wrap_gap_padding_and_alignment() {
    assert_round_trips(&flex_card());
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
fn gap_and_padding_are_set_per_axis_and_per_side() {
    let base = flex_card();
    assert_distinct_values(
        &base,
        "/layout/gap",
        &[
            json!({"row": 12, "column": 8}),
            json!({"row": 8, "column": 12}),
            json!({"row": 12, "column": 12}),
        ],
    );
    assert_distinct_values(
        &base,
        "/layout/padding",
        &[
            json!({"top": 16, "right": 4, "bottom": 4, "left": 4}),
            json!({"top": 4, "right": 16, "bottom": 4, "left": 4}),
            json!({"top": 4, "right": 4, "bottom": 16, "left": 4}),
            json!({"top": 4, "right": 4, "bottom": 4, "left": 16}),
        ],
    );
}

#[test]
fn a_flex_layout_may_leave_its_css_properties_at_their_initial_values() {
    let output = assert_round_trips(&flex_frame("frame001", vec![text("text0001")]));
    assert_eq!(output["layout"]["mode"], "flex");
}

#[test]
fn grid_layouts_have_templates_gap_padding_and_child_placement() {
    assert_round_trips(&product_grid());
}

#[test]
fn grid_tracks_are_fixed_fractional_or_auto() {
    let base = product_grid();
    for pointer in ["/layout/columns", "/layout/rows"] {
        assert_distinct_values(
            &base,
            pointer,
            &[
                json!([{"fixed": 120}]),
                json!([{"fr": 1}]),
                json!(["auto"]),
                json!([{"fixed": 120}, {"fr": 1}]),
                json!([{"fr": 1}, {"fixed": 120}]),
            ],
        );
    }
}

#[test]
fn grid_children_are_placed_by_start_line_and_span() {
    let base = product_grid();
    for pointer in ["/children/0/gridColumn", "/children/0/gridRow"] {
        assert_distinct_values(
            &base,
            pointer,
            &[
                json!({"start": 1, "span": 2}),
                json!({"start": 2, "span": 2}),
                json!({"start": 1, "span": 3}),
            ],
        );
    }
    let auto_placed = without(
        &without(&base, "/children/0/gridColumn"),
        "/children/0/gridRow",
    );
    assert_ne!(
        parse(&auto_placed),
        parse(&base),
        "a child without grid placement is placed automatically"
    );
}

#[test]
fn each_axis_is_sized_fixed_hug_or_fill() {
    let base = flex_card();
    for axis in ["width", "height"] {
        let pointer = format!("/{axis}");
        let sizings = [
            json!({"fixed": 320}),
            json!({"fixed": 48.5}),
            json!("hug"),
            json!("fill"),
        ];
        assert_distinct_values(&base, &pointer, &sizings);
        for sizing in sizings {
            assert_round_trips(&with(&base, &pointer, sizing));
        }
    }
    let width_hugs = with(
        &with(&base, "/width", json!("hug")),
        "/height",
        json!("fill"),
    );
    let height_hugs = with(
        &with(&base, "/width", json!("fill")),
        "/height",
        json!("hug"),
    );
    assert_ne!(
        parse(&width_hugs),
        parse(&height_hugs),
        "width and height are sized separately"
    );
}

#[test]
fn every_node_below_a_page_sizes_itself_per_axis() {
    let children = vec![
        with(&text("text0001"), "/width", json!({"fixed": 280})),
        with(&image("jpeg0001"), "/width", json!("fill")),
        with(&vector("vec00001"), "/height", json!("fill")),
        with(
            &with(&instance("btn00001"), "/width", json!("fill")),
            "/height",
            json!({"fixed": 44}),
        ),
        with(
            &flex_frame("frame002", vec![text("text0002")]),
            "/height",
            json!("hug"),
        ),
    ];
    assert_round_trips(&flex_frame("frame001", children));
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

#[test]
fn an_instance_without_a_size_keeps_its_components_size() {
    let sized = with(&instance("btn00001"), "/width", json!("fill"));
    assert_round_trips(&sized);
    assert_ne!(
        parse(&sized),
        parse(&instance("btn00001")),
        "an instance may override its component's size"
    );
}

#[test]
fn min_and_max_sizes_bound_each_axis() {
    let bounded = with(
        &with(
            &with(
                &with(&flex_card(), "/minWidth", json!(120)),
                "/maxWidth",
                json!(480),
            ),
            "/minHeight",
            json!(40),
        ),
        "/maxHeight",
        json!(960.5),
    );
    assert_round_trips(&bounded);

    let base = flex_card();
    let nodes: Vec<_> = ["/minWidth", "/maxWidth", "/minHeight", "/maxHeight"]
        .iter()
        .map(|pointer| parse(&with(&base, pointer, json!(120))))
        .collect();
    for (i, first) in nodes.iter().enumerate() {
        assert_ne!(first, &parse(&base), "a min or max size changes the node");
        for second in &nodes[i + 1..] {
            assert_ne!(first, second, "min and max sizes are separate per axis");
        }
    }
}

#[test]
fn absolute_children_of_flex_and_grid_layouts_keep_x_and_y() {
    let pinned = placed(
        &with(&text("text0002"), "/absolute", json!(true)),
        12.0,
        -8.0,
    );
    let flex = flex_frame("frame001", vec![text("text0001"), pinned]);
    assert_round_trips(&flex);
    let grid = with(
        &flex,
        "/layout",
        json!({"mode": "grid", "columns": [{"fr": 1}]}),
    );
    assert_round_trips(&grid);

    let in_flow = flex_frame("frame001", vec![text("text0001"), text("text0002")]);
    let taken_out = with(&in_flow, "/children/1/absolute", json!(true));
    assert_ne!(
        parse(&taken_out),
        parse(&in_flow),
        "`absolute` takes a child out of its parent's flow"
    );
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
