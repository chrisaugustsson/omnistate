//! US-002: vector nodes hold SVG path data only, never raw SVG.

mod common;

use common::{
    assert_distinct_values, assert_rejected, assert_round_trips, flex_frame, parse, strings,
    vector, with, without,
};
use serde_json::json;

/// Valid SVG path data: every command, absolute and relative, with commas,
/// exponents, implicit repeats and the compact arc flags of minified icons.
const PATHS: [&str; 7] = [
    "M0 0 L24 0 L24 24 L0 24 Z",
    "M12 2a10 10 0 1 0 0.001 0z",
    "M4 12h16M12 4v16",
    "M2,12 C2,6.5 6.5,2 12,2 S22,6.5 22,12 Q22,17 17,20 T12,22 Z",
    "M10 18a8 8 0 100-16 8 8 0 000 16zm3.7-9.3a1 1 0 00-1.4-1.4L9 10.6 7.7 9.3a1 1 0 00-1.4 1.4l2 2a1 1 0 001.4 0l4-4z",
    "m-1.5e-1 3.25E1l.5-.5",
    "M 3 3 H 21 V 21 H 3 Z m 4 4 h 10 v 10 h -10 z",
];

/// Raw SVG markup, which a vector never stores.
const MARKUP: &str =
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0 L24 24"/></svg>"#;

#[test]
fn vector_path_data_is_kept_exactly_as_written() {
    for path in PATHS {
        let output = assert_round_trips(&with(&vector("vec00001"), "/path", json!(path)));
        assert_eq!(output["path"], path, "path data is written back as written");
    }
    assert_distinct_values(&vector("vec00001"), "/path", &strings(PATHS));
}

#[test]
fn a_vector_is_its_path_data() {
    let base = vector("vec00001");
    parse(&base);
    assert_rejected(&without(&base, "/path"), "a vector without path data");
}

#[test]
fn raw_svg_is_never_stored() {
    let base = vector("vec00001");
    parse(&base);
    let message = assert_rejected(
        &with(&base, "/svg", json!(MARKUP)),
        "vectors hold path data only, so an `svg` property",
    );
    assert!(
        message.contains("svg"),
        "the error must name `svg`; it says: {message}"
    );
    for path in [
        MARKUP,
        r#"<path d="M0 0 L24 24"/>"#,
        r#"M0 0 L24 24"/><script>alert(1)</script>"#,
    ] {
        assert_rejected(
            &with(&base, "/path", json!(path)),
            &format!("`{path}` is SVG markup, not path data"),
        );
    }
}

#[test]
fn path_data_follows_the_svg_path_grammar() {
    let base = vector("vec00001");
    parse(&base);
    for (bad, why) in [
        ("hello", "it is not path data"),
        ("M0 0 X10 10", "`X` is not a path command"),
        ("M0 0 L10", "the line has no y"),
        ("L10 10", "path data starts with a moveto"),
        ("M0 0 L10 10 url(#a)", "`url(#a)` is not path data"),
        ("M 0,0 L 10;10", "`;` is not a separator"),
    ] {
        assert_rejected(
            &with(&base, "/path", json!(bad)),
            &format!("`{bad}` is invalid: {why}"),
        );
    }
}

#[test]
fn vectors_are_drawn_with_fills_strokes_and_opacity() {
    let json = json!({
        "id": "vec00001",
        "type": "vector",
        "name": "Cart icon",
        "width": {"fixed": 24},
        "height": {"fixed": 24},
        "path": PATHS[4],
        "fills": [
            {"type": "solid", "color": "#2563eb"},
            {"type": "linear-gradient", "angle": 90, "stops": [
                {"position": 0, "color": "#ffffff33"},
                {"position": 1, "color": "#ffffff00"}
            ]}
        ],
        "strokes": [{"color": "#1e3a8a", "width": 1.5}],
        "opacity": 0.95
    });
    assert_round_trips(&flex_frame("frame001", vec![json.clone()]));
    assert_ne!(
        parse(&json),
        parse(&without(&json, "/strokes")),
        "strokes change a vector"
    );
}
