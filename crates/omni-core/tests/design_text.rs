//! US-002: text properties: plain-text content, font family, size, weight,
//! style, line height, letter spacing, alignment, decoration and color, with
//! one style per text node.

mod common;

use common::{
    assert_distinct_values, assert_rejected, assert_round_trips, flex_frame, parse, strings, text,
    with, without,
};
use serde_json::{Value, json};

/// A text node with every text property set.
fn styled_text() -> Value {
    json!({
        "id": "text0001",
        "type": "text",
        "name": "Order total",
        "width": {"fixed": 280},
        "height": "hug",
        "content": "Totalt: 1 299 kr",
        "fontFamily": "IBM Plex Sans",
        "fontSize": 18,
        "fontWeight": 650,
        "fontStyle": "italic",
        "lineHeight": 1.4,
        "letterSpacing": -0.2,
        "textAlign": "center",
        "textDecoration": "underline",
        "color": "#0f172a"
    })
}

#[test]
fn text_nodes_round_trip_every_text_property() {
    assert_round_trips(&flex_frame("frame001", vec![styled_text()]));
}

#[test]
fn text_content_is_plain_text_kept_exactly() {
    for content in [
        "Line one\nLine två — åäö ✓ 🛒",
        "<b>not bold</b> & <script>alert(1)</script>",
        "  leading and trailing spaces  ",
        "Tab\tseparated, \"quoted\" and back\\slashed",
        "**not markdown** either",
    ] {
        let output = assert_round_trips(&with(&styled_text(), "/content", json!(content)));
        assert_eq!(
            output["content"], content,
            "text content is plain text, written back exactly"
        );
    }
    assert_rejected(
        &without(&styled_text(), "/content"),
        "a text node without content",
    );
}

#[test]
fn font_values_change_the_node() {
    let base = styled_text();
    assert_distinct_values(
        &base,
        "/fontFamily",
        &strings(["IBM Plex Sans", "Inter", "Georgia"]),
    );
    assert_distinct_values(&base, "/fontSize", &[json!(12), json!(18), json!(28.5)]);
    assert_distinct_values(
        &base,
        "/fontWeight",
        &[json!(100), json!(400), json!(650), json!(900)],
    );
    assert_distinct_values(&base, "/lineHeight", &[json!(1), json!(1.4), json!(2)]);
    assert_distinct_values(
        &base,
        "/letterSpacing",
        &[json!(-0.2), json!(0), json!(0.8)],
    );
    assert_distinct_values(
        &base,
        "/color",
        &strings(["#0f172a", "#16a34a", "#16a34a80"]),
    );
}

#[test]
fn text_keywords_are_the_css_keywords() {
    let base = styled_text();
    assert_distinct_values(&base, "/fontStyle", &strings(["normal", "italic"]));
    assert_distinct_values(
        &base,
        "/textAlign",
        &strings(["left", "center", "right", "justify"]),
    );
    assert_distinct_values(
        &base,
        "/textDecoration",
        &strings(["none", "underline", "line-through"]),
    );
    for (pointer, bad) in [
        ("/fontStyle", "slanted"),
        ("/textAlign", "middle"),
        ("/textDecoration", "blink"),
    ] {
        assert_rejected(
            &with(&base, pointer, json!(bad)),
            &format!("{pointer} = `{bad}` is not a text value of the model"),
        );
    }
}

#[test]
fn a_text_node_has_exactly_one_style() {
    let base = styled_text();
    parse(&base);
    assert_rejected(
        &with(
            &base,
            "/content",
            json!([
                {"text": "Totalt: ", "fontWeight": 400},
                {"text": "1 299 kr", "fontWeight": 700}
            ]),
        ),
        "one style per text node, so content split into styled runs",
    );
    for key in ["runs", "spans", "styles"] {
        let message = assert_rejected(
            &with(
                &base,
                &format!("/{key}"),
                json!([{"start": 0, "end": 7, "fontWeight": 700}]),
            ),
            &format!("one style per text node, so `{key}`"),
        );
        assert!(
            message.contains(key),
            "the error must name `{key}`; it says: {message}"
        );
    }
    assert_rejected(
        &with(&base, "/fontWeight", json!([400, 700])),
        "one style per text node, so two weights",
    );
    assert_rejected(
        &with(&base, "/color", json!(["#0f172a", "#16a34a"])),
        "one style per text node, so two colors",
    );
}

#[test]
fn a_text_node_names_its_font_family_and_size() {
    let base = text("text0001");
    parse(&base);
    for field in ["fontFamily", "fontSize"] {
        assert_rejected(
            &without(&base, &format!("/{field}")),
            &format!("a text node without `{field}`"),
        );
    }
}

#[test]
fn the_other_text_style_properties_may_be_left_out() {
    let plain = text("text0001");
    let output = assert_round_trips(&plain);
    assert_eq!(output["content"], "Hello");
    assert_ne!(
        parse(&plain),
        parse(&with(&plain, "/textDecoration", json!("underline"))),
        "a text style property changes the node"
    );
}
