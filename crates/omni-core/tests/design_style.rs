//! US-002: style properties: fills (solid color, linear gradient, image),
//! strokes, corner radius per corner, drop and inner shadows, opacity and
//! clipping of children.

mod common;

use common::{
    PHOTO, assert_distinct_values, assert_rejected, assert_round_trips, flex_frame, frame, image,
    parse, strings, text, vector, with, without,
};
use serde_json::{Value, json};

/// A solid fill, a linear gradient and an image fill, in paint order.
fn three_fills() -> Value {
    json!([
        {"type": "solid", "color": "#2563eb"},
        {
            "type": "linear-gradient",
            "angle": 135,
            "stops": [
                {"position": 0, "color": "#0f172a"},
                {"position": 0.35, "color": "#1e293b80"},
                {"position": 1, "color": "#334155"}
            ]
        },
        {"type": "image", "image": PHOTO, "fit": "cover"}
    ])
}

/// A frame with every style property set.
fn styled_frame() -> Value {
    json!({
        "id": "frame001",
        "type": "frame",
        "name": "Card",
        "width": {"fixed": 320},
        "height": {"fixed": 200},
        "layout": {"mode": "flex"},
        "fills": three_fills(),
        "strokes": [{"color": "#e2e8f0", "width": 1}, {"color": "#0f172a1a", "width": 2.5}],
        "radius": {"topLeft": 16, "topRight": 8, "bottomRight": 4, "bottomLeft": 2},
        "shadows": [
            {"type": "drop", "color": "#0f172a33", "offsetX": 2, "offsetY": 8, "blur": 24, "spread": -4},
            {"type": "inner", "color": "#ffffff80", "offsetX": 1, "offsetY": 1, "blur": 2, "spread": 1}
        ],
        "opacity": 0.85,
        "clip": true,
        "children": [text("text0001")]
    })
}

/// Pointers to every color in `styled_frame()`.
const FRAME_COLORS: [&str; 8] = [
    "/fills/0/color",
    "/fills/1/stops/0/color",
    "/fills/1/stops/1/color",
    "/fills/1/stops/2/color",
    "/strokes/0/color",
    "/strokes/1/color",
    "/shadows/0/color",
    "/shadows/1/color",
];

#[test]
fn a_frame_round_trips_every_style_property() {
    assert_round_trips(&styled_frame());
}

#[test]
fn fills_are_solid_colors_linear_gradients_or_images_in_paint_order() {
    let json = with(&frame("frame001"), "/fills", three_fills());
    assert_round_trips(&json);

    let mut reversed = three_fills().as_array().expect("fills are a list").clone();
    reversed.reverse();
    assert_ne!(
        parse(&with(&frame("frame001"), "/fills", Value::Array(reversed))),
        parse(&json),
        "fills are painted in order, so their order is part of the node"
    );
    assert_ne!(
        parse(&json),
        parse(&frame("frame001")),
        "fills change the node"
    );
}

#[test]
fn only_solid_linear_gradient_and_image_fills_exist() {
    let base = with(&frame("frame001"), "/fills", three_fills());
    parse(&base);
    for bad in [
        json!({"type": "radial-gradient", "stops": [{"position": 0, "color": "#000000"}, {"position": 1, "color": "#ffffff"}]}),
        json!({"type": "pattern", "image": PHOTO}),
        json!({"type": "Solid", "color": "#2563eb"}),
        json!({"color": "#2563eb"}),
    ] {
        assert_rejected(
            &with(&base, "/fills/0", bad.clone()),
            &format!("{bad} is not a solid, linear-gradient or image fill"),
        );
    }
}

#[test]
fn gradients_have_an_angle_and_color_stops() {
    let base = with(&frame("frame001"), "/fills", three_fills());
    assert_distinct_values(
        &base,
        "/fills/1/angle",
        &[json!(135), json!(90), json!(-45.5)],
    );
    assert_distinct_values(
        &base,
        "/fills/1/stops/1/position",
        &[json!(0.35), json!(0.5), json!(0.9)],
    );
    assert_distinct_values(
        &base,
        "/fills/1/stops",
        &[
            json!([{"position": 0, "color": "#000000"}, {"position": 1, "color": "#ffffff"}]),
            json!([{"position": 0, "color": "#ffffff"}, {"position": 1, "color": "#000000"}]),
        ],
    );
    assert_rejected(
        &without(&base, "/fills/1/angle"),
        "a linear gradient states its angle",
    );
    assert_rejected(
        &without(&base, "/fills/1/stops/1/position"),
        "a gradient stop states its position",
    );
}

#[test]
fn colors_are_hex_rgb_or_rgba() {
    let base = styled_frame();
    assert_distinct_values(
        &base,
        "/fills/0/color",
        &strings(["#2563eb", "#2563eb80", "#000000", "#ffffff", "#ffffff00"]),
    );
    for bad in [
        "blue",
        "#12345",
        "#1234567",
        "#gggggg",
        "rgb(37, 99, 235)",
        "2563eb",
        "#2563eb ",
        "",
    ] {
        assert_rejected(
            &with(&base, "/fills/0/color", json!(bad)),
            &format!("`{bad}` is not a #rrggbb or #rrggbbaa color"),
        );
    }
    for pointer in FRAME_COLORS {
        assert_rejected(
            &with(&base, pointer, json!("blue")),
            &format!("{pointer} is a color, so `blue`"),
        );
    }
}

#[test]
fn images_come_from_workspace_attachments_only() {
    let fill = with(
        &frame("frame001"),
        "/fills",
        json!([{"type": "image", "image": PHOTO, "fit": "cover"}]),
    );
    let node = image("jpeg0001");
    parse(&fill);
    parse(&node);
    for bad in [
        "https://example.com/photo.png",
        "http://example.com/photo.png",
        "data:image/png;base64,iVBORw0KGgo=",
        "file:///tmp/photo.png",
        "photos/cart.png",
        "attachment:",
    ] {
        assert_rejected(
            &with(&fill, "/fills/0/image", json!(bad)),
            &format!("an image fill shows an `attachment:<id>`, so `{bad}`"),
        );
        assert_rejected(
            &with(&node, "/image", json!(bad)),
            &format!("an image node shows an `attachment:<id>`, so `{bad}`"),
        );
    }
}

#[test]
fn images_fit_their_box_by_cover_or_contain() {
    let fill = with(&frame("frame001"), "/fills", three_fills());
    assert_distinct_values(&fill, "/fills/2/fit", &strings(["cover", "contain"]));
    assert_rejected(
        &with(&fill, "/fills/2/fit", json!("zoom")),
        "`zoom` is not an image fit",
    );
    assert_rejected(
        &without(&fill, "/fills/2/fit"),
        "an image fill states how it fits",
    );
    let node = image("jpeg0001");
    assert_distinct_values(&node, "/fit", &strings(["cover", "contain"]));
    assert_rejected(
        &with(&node, "/fit", json!("zoom")),
        "`zoom` is not an image fit",
    );
    assert_rejected(&without(&node, "/fit"), "an image node states how it fits");
}

#[test]
fn image_nodes_show_an_attachment_and_take_box_styles() {
    let json = json!({
        "id": "jpeg0001",
        "type": "image",
        "name": "Hero photo",
        "width": "fill",
        "height": {"fixed": 240},
        "image": PHOTO,
        "fit": "contain",
        "radius": {"topLeft": 16, "topRight": 16, "bottomRight": 16, "bottomLeft": 16},
        "strokes": [{"color": "#cbd5e1", "width": 0.5}],
        "shadows": [{"type": "drop", "color": "#00000033", "offsetX": 1, "offsetY": 2, "blur": 6, "spread": 1}],
        "opacity": 0.9
    });
    assert_round_trips(&flex_frame("frame001", vec![json.clone()]));
    assert_rejected(
        &without(&json, "/image"),
        "an image node without the image it shows",
    );
}

#[test]
fn strokes_have_a_color_and_a_width() {
    let base = styled_frame();
    assert_distinct_values(
        &base,
        "/strokes/0",
        &[
            json!({"color": "#e2e8f0", "width": 1}),
            json!({"color": "#e2e8f0", "width": 3}),
            json!({"color": "#0f172a", "width": 1}),
        ],
    );
    assert_rejected(
        &without(&base, "/strokes/0/width"),
        "a stroke without a width",
    );
    assert_rejected(
        &without(&base, "/strokes/0/color"),
        "a stroke without a color",
    );
}

#[test]
fn corner_radius_is_set_per_corner() {
    let base = styled_frame();
    assert_distinct_values(
        &base,
        "/radius",
        &[
            json!({"topLeft": 24, "topRight": 2, "bottomRight": 2, "bottomLeft": 2}),
            json!({"topLeft": 2, "topRight": 24, "bottomRight": 2, "bottomLeft": 2}),
            json!({"topLeft": 2, "topRight": 2, "bottomRight": 24, "bottomLeft": 2}),
            json!({"topLeft": 2, "topRight": 2, "bottomRight": 2, "bottomLeft": 24}),
        ],
    );
}

#[test]
fn shadows_are_drop_or_inner_and_keep_their_order() {
    let base = styled_frame();
    assert_distinct_values(&base, "/shadows/0/type", &strings(["drop", "inner"]));
    for (field, values) in [
        ("offsetX", [json!(2), json!(-6)]),
        ("offsetY", [json!(8), json!(0.5)]),
        ("blur", [json!(24), json!(40)]),
        ("spread", [json!(-4), json!(3)]),
    ] {
        assert_distinct_values(&base, &format!("/shadows/0/{field}"), &values);
    }
    assert_rejected(
        &with(&base, "/shadows/0/type", json!("outer")),
        "`outer` is not a shadow type",
    );

    let mut reversed = base["shadows"]
        .as_array()
        .expect("shadows are a list")
        .clone();
    reversed.reverse();
    assert_ne!(
        parse(&with(&base, "/shadows", Value::Array(reversed))),
        parse(&base),
        "shadows are drawn in order, so their order is part of the node"
    );
}

#[test]
fn opacity_and_clipping_change_the_node() {
    let base = styled_frame();
    assert_distinct_values(&base, "/opacity", &[json!(0.85), json!(0.5), json!(0.05)]);
    let unclipped = without(&base, "/clip");
    assert_round_trips(&unclipped);
    assert_ne!(
        parse(&unclipped),
        parse(&base),
        "`clip: true` clips the children"
    );
}

#[test]
fn vectors_and_text_take_opacity_too() {
    let faded_vector = with(&vector("vec00001"), "/opacity", json!(0.4));
    let faded_text = with(&text("text0001"), "/opacity", json!(0.6));
    assert_round_trips(&flex_frame(
        "frame001",
        vec![faded_vector.clone(), faded_text.clone()],
    ));
    assert_ne!(parse(&faded_vector), parse(&vector("vec00001")));
    assert_ne!(parse(&faded_text), parse(&text("text0001")));
}
