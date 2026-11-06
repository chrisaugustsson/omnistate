//! US-002: style values (fills, strokes, radii, shadows, opacity, typography)
//! and layout values (gap, padding, sizes) accept either a literal or a token
//! reference such as `$color.surface`.

mod common;

use common::{assert_rejected, assert_round_trips, parse, with};
use serde_json::{Value, json};

/// A frame with a token reference in every style and layout value.
fn tokenized_frame() -> Value {
    json!({
        "id": "frame001",
        "type": "frame",
        "name": "Card",
        "width": {"fixed": "$size.card"},
        "height": {"fixed": "$size.card-tall"},
        "minWidth": "$size.min",
        "maxWidth": "$size.max",
        "minHeight": "$size.min-tall",
        "maxHeight": "$size.max-tall",
        "layout": {
            "mode": "flex",
            "direction": "column",
            "gap": {"row": "$space.md", "column": "$space.lg"},
            "padding": {
                "top": "$space.xs",
                "right": "$space.sm",
                "bottom": "$space.md",
                "left": "$space.lg"
            }
        },
        "fills": [
            {"type": "solid", "color": "$color.surface"},
            {"type": "linear-gradient", "angle": 180, "stops": [
                {"position": 0, "color": "$color.brand-500"},
                {"position": 1, "color": "$color.brand-700"}
            ]}
        ],
        "strokes": [{"color": "$color.border", "width": "$border.thin"}],
        "radius": {
            "topLeft": "$radius.lg",
            "topRight": "$radius.md",
            "bottomRight": "$radius.sm",
            "bottomLeft": "$radius.xs"
        },
        "shadows": [
            "$shadow.card",
            {"type": "drop", "color": "$color.shadow", "offsetX": 0.5, "offsetY": 4, "blur": 12, "spread": -2}
        ],
        "opacity": "$opacity.muted",
        "visible": "$state.isOpen",
        "children": [tokenized_text()]
    })
}

/// A text node with a token reference in every typography value.
fn tokenized_text() -> Value {
    json!({
        "id": "text0001",
        "type": "text",
        "name": "Label",
        "width": "hug",
        "height": "hug",
        "content": "$state.cta",
        "fontFamily": "$font.family.sans",
        "fontSize": "$font.size.lg",
        "fontWeight": "$font.weight.semibold",
        "lineHeight": "$font.line-height.tight",
        "letterSpacing": "$font.tracking.wide",
        "color": "$color.text.strong",
        "opacity": "$opacity.subtle"
    })
}

/// Every token reference in `tokenized_frame()`: where it is, the token, and
/// a literal that may stand in its place.
fn token_slots() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("/width/fixed", "$size.card", json!(320)),
        ("/height/fixed", "$size.card-tall", json!(480)),
        ("/minWidth", "$size.min", json!(160)),
        ("/maxWidth", "$size.max", json!(640)),
        ("/minHeight", "$size.min-tall", json!(240)),
        ("/maxHeight", "$size.max-tall", json!(960)),
        ("/layout/gap/row", "$space.md", json!(16)),
        ("/layout/gap/column", "$space.lg", json!(24)),
        ("/layout/padding/top", "$space.xs", json!(4)),
        ("/layout/padding/right", "$space.sm", json!(8)),
        ("/layout/padding/bottom", "$space.md", json!(16)),
        ("/layout/padding/left", "$space.lg", json!(24)),
        ("/fills/0/color", "$color.surface", json!("#ffffff")),
        (
            "/fills/1/stops/0/color",
            "$color.brand-500",
            json!("#3b82f6"),
        ),
        (
            "/fills/1/stops/1/color",
            "$color.brand-700",
            json!("#1d4ed8"),
        ),
        ("/strokes/0/color", "$color.border", json!("#e2e8f0")),
        ("/strokes/0/width", "$border.thin", json!(1)),
        ("/radius/topLeft", "$radius.lg", json!(16)),
        ("/radius/topRight", "$radius.md", json!(12)),
        ("/radius/bottomRight", "$radius.sm", json!(8)),
        ("/radius/bottomLeft", "$radius.xs", json!(4)),
        (
            "/shadows/0",
            "$shadow.card",
            json!({"type": "drop", "color": "#0f172a33", "offsetX": 2, "offsetY": 8, "blur": 24, "spread": -4}),
        ),
        ("/shadows/1/color", "$color.shadow", json!("#0f172a33")),
        ("/opacity", "$opacity.muted", json!(0.7)),
        ("/visible", "$state.isOpen", json!(false)),
        ("/children/0/content", "$state.cta", json!("Fri frakt")),
        (
            "/children/0/fontFamily",
            "$font.family.sans",
            json!("Inter"),
        ),
        ("/children/0/fontSize", "$font.size.lg", json!(18)),
        (
            "/children/0/fontWeight",
            "$font.weight.semibold",
            json!(600),
        ),
        (
            "/children/0/lineHeight",
            "$font.line-height.tight",
            json!(1.25),
        ),
        (
            "/children/0/letterSpacing",
            "$font.tracking.wide",
            json!(0.5),
        ),
        ("/children/0/color", "$color.text.strong", json!("#0f172a")),
        ("/children/0/opacity", "$opacity.subtle", json!(0.9)),
    ]
}

#[test]
fn style_and_layout_values_accept_token_references() {
    let output = assert_round_trips(&tokenized_frame());
    for (pointer, token, _) in token_slots() {
        assert_eq!(
            output.pointer(pointer),
            Some(&json!(token)),
            "{pointer} keeps the token reference `{token}`"
        );
    }
}

#[test]
fn token_references_are_dollar_and_dot_separated_names() {
    let base = tokenized_frame();
    for token in [
        "$color.surface",
        "$space.md",
        "$color.brand-500",
        "$space.2xl",
        "$type.heading.h1",
        "$color.text_muted",
    ] {
        for pointer in ["/fills/0/color", "/layout/gap/row", "/children/0/fontSize"] {
            let output = assert_round_trips(&with(&base, pointer, json!(token)));
            assert_eq!(output.pointer(pointer), Some(&json!(token)));
        }
    }
}

#[test]
fn malformed_token_references_are_rejected() {
    let base = tokenized_frame();
    parse(&base);
    for bad in [
        "$",
        "$.",
        "$color.",
        "$.surface",
        "$color..surface",
        "$color surface",
        "$color/surface",
        "${color.surface}",
        "$color.surface!",
        "$ color.surface",
        "$color.surface ",
    ] {
        for pointer in [
            "/fills/0/color",
            "/layout/gap/row",
            "/opacity",
            "/shadows/0",
            "/children/0/fontFamily",
        ] {
            assert_rejected(
                &with(&base, pointer, json!(bad)),
                &format!("{pointer} = `{bad}` is not a token reference like `$color.surface`"),
            );
        }
    }
}

#[test]
fn token_file_references_are_not_design_values() {
    let base = tokenized_frame();
    parse(&base);
    for (pointer, bad) in [
        ("/fills/0/color", "{color.surface}"),
        ("/layout/padding/top", "{space.xs}"),
        ("/children/0/fontSize", "{font.size.lg}"),
        ("/children/0/fontFamily", "{font.family.sans}"),
    ] {
        assert_rejected(
            &with(&base, pointer, json!(bad)),
            &format!("designs write tokens as `$name`; `{bad}` is token file syntax"),
        );
    }
}

#[test]
fn strings_that_are_neither_a_literal_nor_a_token_are_rejected() {
    let base = tokenized_frame();
    parse(&base);
    for (pointer, bad) in [
        ("/width/fixed", "320px"),
        ("/minWidth", "auto"),
        ("/layout/gap/row", "16"),
        ("/layout/padding/top", "16px"),
        ("/radius/topLeft", "8px"),
        ("/strokes/0/width", "thin"),
        ("/fills/0/color", "surface"),
        ("/shadows/0", "0 4px 12px #0f172a33"),
        ("/opacity", "50%"),
        ("/opacity", "0.5"),
        ("/children/0/fontSize", "16px"),
        ("/children/0/fontWeight", "bold"),
        ("/children/0/lineHeight", "150%"),
        ("/children/0/letterSpacing", "0.02em"),
        ("/children/0/color", "black"),
    ] {
        assert_rejected(
            &with(&base, pointer, json!(bad)),
            &format!("{pointer} = `{bad}` is neither a literal nor a token reference"),
        );
    }
}
