//! US-053: the node model holds the layout and stroke features that Penpot
//! designs use (decision Q48), so that importing them loses nothing.
//!
//! - Every node below a page may set `margin`, with an optional length per
//!   side (`top`, `right`, `bottom`, `left`), as CSS `margin`. A side is a
//!   number of px, which may be negative as in CSS, or a token reference.
//! - A child of a grid layout may set `justifySelf` and `alignSelf`, as CSS
//!   `justify-self` and `align-self`: `start`, `center`, `end` or `stretch`.
//!   Like `gridColumn` and `gridRow`, only the children of grids set them.
//! - Any stroke may be dashed: `"dash": {"length": <px>, "gap": <px>}`, both
//!   lengths required, 0 or more, or token references.
//! - A stroke of a frame or a board may set `align`: `inside`, `center` or
//!   `outside`.
//!
//! Each new property round-trips through JSON without loss, as every other
//! property does.

mod common;

use common::{
    assert_distinct_values, assert_rejected, assert_round_trips, board, containing, flex_frame,
    frame, image, instance, page, parse, strings, text, vector, with,
};
use serde_json::{Value, json};

/// A margin on every side: a positive and a negative length, zero and a token.
fn every_side() -> Value {
    json!({"top": 5, "right": -2.5, "bottom": 0, "left": "$space.sm"})
}

/// A grid frame holding `children`.
fn grid_frame(id: &str, children: Vec<Value>) -> Value {
    containing(
        &with(
            &frame(id),
            "/layout",
            json!({"mode": "grid", "columns": [{"fixed": 40}, {"fr": 1}], "rows": [{"fixed": 40}]}),
        ),
        children,
    )
}

/// A frame with the strokes `strokes`.
fn stroked(strokes: Value) -> Value {
    with(&frame("frame001"), "/strokes", strokes)
}

#[test]
fn every_node_below_a_page_may_have_a_margin_on_each_side() {
    for node in [
        frame("frame001"),
        text("text0001"),
        image("jpeg0001"),
        vector("vec00001"),
        instance("btn00001"),
    ] {
        let output = assert_round_trips(&with(&node, "/margin", every_side()));
        assert_eq!(
            output["margin"],
            every_side(),
            "a {} keeps its margin through a round trip",
            node["type"]
        );
    }
    let board = with(&board("brd00001"), "/margin", every_side());
    assert_round_trips(&page("page0001", vec![board]));
}

#[test]
fn margins_hold_only_the_four_sides_as_lengths() {
    let node = frame("frame001");
    for margin in [
        json!({"vertical": 5}),
        json!({"top": "5px"}),
        json!({"top": null}),
        json!(5),
        json!([5, 5, 5, 5]),
    ] {
        assert_rejected(
            &with(&node, "/margin", margin.clone()),
            &format!(
                "a margin is an object of lengths for `top`, `right`, `bottom` and `left`, so {margin} must be rejected"
            ),
        );
    }
}

#[test]
fn self_alignment_keywords_are_start_center_end_and_stretch() {
    let base = grid_frame("grd00001", vec![image("jpeg0001")]);
    let keywords = strings(["start", "center", "end", "stretch"]);
    assert_distinct_values(&base, "/children/0/justifySelf", &keywords);
    assert_distinct_values(&base, "/children/0/alignSelf", &keywords);
    for keyword in ["flex-start", "middle", "left"] {
        for key in ["justifySelf", "alignSelf"] {
            assert_rejected(
                &with(&base, &format!("/children/0/{key}"), json!(keyword)),
                &format!(
                    "`{key}` is one of start, center, end and stretch, so {keyword:?} must be rejected"
                ),
            );
        }
    }
}

#[test]
fn only_the_children_of_grid_layouts_align_themselves() {
    for key in ["justifySelf", "alignSelf"] {
        let child = with(&text("text0001"), &format!("/{key}"), json!("center"));
        assert_rejected(
            &flex_frame("frame001", vec![child.clone()]),
            &format!(
                "`{key}` aligns a child in its grid area, so the child of a flex layout must not set it"
            ),
        );
        assert_rejected(
            &containing(&frame("frame001"), vec![child]),
            &format!(
                "`{key}` aligns a child in its grid area, so the child of a none layout must not set it"
            ),
        );
    }
}

#[test]
fn strokes_may_be_dashed_with_a_dash_length_and_a_gap() {
    let dashed = json!({"color": "#0f172a", "width": 2, "dash": {"length": 4, "gap": 2}});
    let tokens = json!({"color": "#0f172a", "width": 1, "dash": {"length": "$dash.long", "gap": "$dash.gap"}});
    let output = assert_round_trips(&stroked(json!([dashed, tokens])));
    assert_eq!(output["strokes"][0]["dash"], json!({"length": 4, "gap": 2}));
    let strokes = json!([dashed]);
    assert_round_trips(&with(&image("jpeg0001"), "/strokes", strokes.clone()));
    assert_round_trips(&with(&vector("vec00001"), "/strokes", strokes));
    assert_distinct_values(
        &stroked(json!([dashed])),
        "/strokes/0/dash",
        &[
            json!({"length": 4, "gap": 2}),
            json!({"length": 2, "gap": 4}),
            json!({"length": 0.5, "gap": 0.5}),
        ],
    );
}

#[test]
fn a_dash_needs_a_length_and_a_gap_of_0_or_more() {
    for dash in [
        json!({"length": 4}),
        json!({"gap": 2}),
        json!({"length": -4, "gap": 2}),
        json!({"length": 4, "gap": -2}),
        json!({"length": 4, "gap": 2, "offset": 1}),
        json!([4, 2]),
    ] {
        assert_rejected(
            &stroked(json!([{"color": "#0f172a", "width": 2, "dash": dash.clone()}])),
            &format!("a dash is a length and a gap, each 0 or more, so {dash} must be rejected"),
        );
    }
}

#[test]
fn frame_strokes_sit_inside_centered_on_or_outside_the_edge() {
    let base = stroked(json!([{"color": "#0f172a", "width": 4, "align": "inside"}]));
    assert_distinct_values(
        &base,
        "/strokes/0/align",
        &strings(["inside", "center", "outside"]),
    );
    let both = json!([
        {"color": "#0f172a", "width": 4, "align": "outside", "dash": {"length": 6, "gap": 3}},
        {"color": "#ffffff", "width": 1}
    ]);
    assert_round_trips(&stroked(both.clone()));
    let board = with(&board("brd00001"), "/strokes", both);
    assert_round_trips(&page("page0001", vec![board]));
    for align in ["middle", "inner", "border"] {
        assert_rejected(
            &with(&base, "/strokes/0/align", json!(align)),
            &format!(
                "a stroke sits inside, centered on or outside the edge, so {align:?} must be rejected"
            ),
        );
    }
}

#[test]
fn a_stroke_without_an_align_or_a_dash_is_written_back_without_them() {
    let node = parse(&stroked(json!([{"color": "#0f172a", "width": 4}])));
    let output = serde_json::to_value(&node).expect("a design node serializes to JSON");
    assert_eq!(
        output["strokes"],
        json!([{"color": "#0f172a", "width": 4}]),
        "a solid stroke inside the edge, as before US-053, keeps its JSON"
    );
}
