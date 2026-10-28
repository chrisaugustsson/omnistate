//! US-002: style properties: fills (solid color, linear gradient, image),
//! strokes, corner radius per corner, drop and inner shadows, opacity and
//! clipping of children.

mod common;

use common::{
    PHOTO, assert_distinct_values, assert_rejected, assert_round_trips, frame, image, parse,
    strings, text, try_parse, vector, with, without,
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
    assert_rejected(
        &without(&node, "/image"),
        "an image node without the image it shows",
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
fn shadows_are_drop_or_inner() {
    let base = styled_frame();
    assert_distinct_values(&base, "/shadows/0/type", &strings(["drop", "inner"]));
    assert_rejected(
        &with(&base, "/shadows/0/type", json!("outer")),
        "`outer` is not a shadow type",
    );
}

/// `styled_frame()`, and an image and a vector with its strokes, corner radii
/// (images only) and shadows.
fn styled_boxes() -> [Value; 3] {
    let card = styled_frame();
    let styled = |json: Value, keys: &[&str]| {
        keys.iter().fold(json, |json, &key| {
            with(&json, &format!("/{key}"), card[key].clone())
        })
    };
    let photo = styled(image("jpeg0001"), &["strokes", "radius", "shadows"]);
    let icon = styled(vector("vec00001"), &["strokes", "shadows"]);
    [card, photo, icon]
}

/// Style lengths that CSS never lets go below 0: stroke widths, corner radii
/// and shadow blurs.
const NON_NEGATIVE_STYLE_LENGTHS: [&str; 8] = [
    "/strokes/0/width",
    "/strokes/1/width",
    "/radius/topLeft",
    "/radius/topRight",
    "/radius/bottomRight",
    "/radius/bottomLeft",
    "/shadows/0/blur",
    "/shadows/1/blur",
];

#[test]
fn style_values_stay_in_their_ranges() {
    for json in styled_boxes() {
        let kind = json["type"]
            .as_str()
            .expect("a node has a type")
            .to_string();
        parse(&json);
        for pointer in NON_NEGATIVE_STYLE_LENGTHS {
            if json.pointer(pointer).is_none() {
                continue;
            }
            let zero = with(&json, pointer, json!(0));
            if let Err(error) = try_parse(&zero) {
                panic!("CSS allows {pointer} = 0 on the {kind} node, but it was rejected: {error}");
            }
            assert_round_trips(&zero);
            for negative in [json!(-1), json!(-0.5)] {
                assert_rejected(
                    &with(&json, pointer, negative.clone()),
                    &format!(
                        "CSS has no negative stroke widths, corner radii or blurs, so {pointer} = {negative} on the {kind} node"
                    ),
                );
            }
        }
        // Shadows may be offset up and to the left, and a negative spread
        // shrinks them, as in CSS `box-shadow`.
        for pointer in [
            "/shadows/0/offsetX",
            "/shadows/0/offsetY",
            "/shadows/0/spread",
            "/shadows/1/spread",
        ] {
            assert_round_trips(&with(&json, pointer, json!(-12.5)));
        }
    }

    let base = styled_frame();
    for angle in [json!(-90), json!(-405.5), json!(720)] {
        assert_round_trips(&with(&base, "/fills/1/angle", angle));
    }
    for opacity in [json!(0), json!(1)] {
        assert_round_trips(&with(&base, "/opacity", opacity));
    }
    for opacity in [json!(-0.1), json!(1.5)] {
        assert_rejected(
            &with(&base, "/opacity", opacity.clone()),
            &format!("opacity goes from 0 to 1, so {opacity}"),
        );
    }
}
