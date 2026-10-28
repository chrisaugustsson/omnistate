//! US-002: JSON serialization round-trips every property without loss.

mod common;

use common::{assert_rejected, assert_round_trips, parse, with};
use omni_core::design::{Node, NodeKind};
use serde_json::{Value, json};

/// A page with two boards that uses every node type, every layout mode and
/// every property, with literals and token references.
fn complete_design() -> Value {
    json!({
        "id": "page0001",
        "type": "page",
        "name": "Checkout — kassa 🛒",
        "children": [desktop_board(), mobile_board()]
    })
}

fn desktop_board() -> Value {
    json!({
        "id": "brd00001",
        "type": "board",
        "name": "Cart / desktop",
        "x": 40,
        "y": -120.5,
        "width": {"fixed": 1440},
        "height": "hug",
        "minHeight": 900,
        "layout": {
            "mode": "flex",
            "direction": "column",
            "gap": {"row": 32, "column": "$space.lg"},
            "padding": {"top": 48, "right": 64, "bottom": 56, "left": "$space.xl"},
            "alignItems": "center",
            "justifyContent": "space-between"
        },
        "fills": [
            {"type": "solid", "color": "#f8fafc"},
            {"type": "linear-gradient", "angle": 180, "stops": [
                {"position": 0, "color": "#ffffff00"},
                {"position": 1, "color": "$color.backdrop"}
            ]}
        ],
        "radius": {"topLeft": 24, "topRight": 24, "bottomRight": 8, "bottomLeft": 8},
        "clip": true,
        "children": [header(), product_grid()]
    })
}

fn header() -> Value {
    json!({
        "id": "frame001",
        "type": "frame",
        "name": "Header",
        "width": "fill",
        "height": {"fixed": 72},
        "maxWidth": 1200,
        "layout": {
            "mode": "flex",
            "direction": "row-reverse",
            "wrap": "wrap",
            "gap": {"row": 8, "column": 16},
            "alignItems": "baseline",
            "justifyContent": "space-evenly"
        },
        "strokes": [{"color": "#e2e8f0", "width": 1}],
        "shadows": [
            {"type": "drop", "color": "#0f172a33", "offsetX": 2, "offsetY": 8, "blur": 24, "spread": -4},
            "$shadow.raised"
        ],
        "children": [title(), account_button(), notification_dot()]
    })
}

fn title() -> Value {
    json!({
        "id": "text0001",
        "type": "text",
        "name": "Title",
        "width": "hug",
        "height": "hug",
        "content": "Din varukorg — 3 artiklar",
        "fontFamily": "IBM Plex Sans",
        "fontSize": 28,
        "fontWeight": 650,
        "fontStyle": "italic",
        "lineHeight": 1.25,
        "letterSpacing": -0.4,
        "textAlign": "right",
        "textDecoration": "underline",
        "color": "$color.text.strong"
    })
}

fn account_button() -> Value {
    json!({
        "id": "btn00001",
        "type": "instance",
        "name": "Account button",
        "component": "cmp00001",
        "width": "fill",
        "minWidth": 44,
        "maxWidth": 160
    })
}

fn notification_dot() -> Value {
    json!({
        "id": "vec00001",
        "type": "vector",
        "name": "Notification dot",
        "absolute": true,
        "x": 1180,
        "y": -6,
        "width": {"fixed": 12},
        "height": {"fixed": 12},
        "path": "M6 0a6 6 0 1 0 0.001 0Z",
        "fills": [{"type": "solid", "color": "#ef4444"}],
        "strokes": [{"color": "#ffffff", "width": 2}],
        "opacity": 0.95
    })
}

fn product_grid() -> Value {
    json!({
        "id": "frame002",
        "type": "frame",
        "name": "Products",
        "width": "fill",
        "height": "hug",
        "minWidth": 640,
        "layout": {
            "mode": "grid",
            "columns": [{"fixed": 240}, {"fr": 1}, {"fr": 2.5}, "auto"],
            "rows": ["auto", {"fixed": 320}],
            "gap": {"row": 24, "column": 16},
            "padding": {"top": 8, "right": 12, "bottom": 16, "left": 20}
        },
        "children": [hero_photo(), price_card(), caption()]
    })
}

fn hero_photo() -> Value {
    json!({
        "id": "jpeg0001",
        "type": "image",
        "name": "Hero product",
        "gridColumn": {"start": 1, "span": 2},
        "gridRow": {"start": 1, "span": 2},
        "width": "fill",
        "height": "fill",
        "image": "attachment:0192f1c4-5b6a-7e11-8a22-b33cc44dd55e",
        "fit": "cover",
        "radius": {"topLeft": 16, "topRight": 16, "bottomRight": 16, "bottomLeft": 16},
        "strokes": [{"color": "#cbd5e1", "width": 0.5}],
        "shadows": [
            {"type": "inner", "color": "#00000014", "offsetX": 1, "offsetY": 1, "blur": 2, "spread": 1}
        ],
        "opacity": 0.9
    })
}

fn price_card() -> Value {
    json!({
        "id": "frame003",
        "type": "frame",
        "name": "Price card",
        "visible": false,
        "gridColumn": {"start": 3, "span": 2},
        "gridRow": {"start": 2, "span": 2},
        "width": {"fixed": "$size.card"},
        "height": {"fixed": 200},
        "layout": {"mode": "none"},
        "fills": [
            {"type": "image", "image": "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd", "fit": "contain"}
        ],
        "opacity": "$opacity.muted",
        "children": [price(), buy_button()]
    })
}

fn price() -> Value {
    json!({
        "id": "text0002",
        "type": "text",
        "name": "Price",
        "x": 16,
        "y": 24,
        "width": {"fixed": 200},
        "height": {"fixed": 40},
        "content": "1 299 kr\n<b>inkl. moms</b> & frakt",
        "fontFamily": "$font.family.mono",
        "fontSize": "$font.size.lg",
        "fontWeight": "$font.weight.bold",
        "lineHeight": "$font.line-height.tight",
        "letterSpacing": "$font.tracking.wide",
        "textAlign": "justify",
        "textDecoration": "line-through",
        "color": "#16a34a",
        "opacity": 0.75
    })
}

fn buy_button() -> Value {
    json!({
        "id": "btn00002",
        "type": "instance",
        "name": "Buy button",
        "visible": false,
        "component": "cmp00002",
        "x": 16,
        "y": 140
    })
}

fn caption() -> Value {
    json!({
        "id": "text0003",
        "type": "text",
        "name": "Caption",
        "width": "fill",
        "height": "hug",
        "content": "Fri frakt över 499 kr ✓",
        "fontFamily": "Inter",
        "fontSize": 14
    })
}

fn mobile_board() -> Value {
    json!({
        "id": "brd00002",
        "type": "board",
        "name": "Cart / mobile (hidden)",
        "visible": false,
        "x": 1600,
        "y": 40,
        "width": {"fixed": "$size.mobile"},
        "height": {"fixed": 844},
        "maxHeight": 2400,
        "layout": {"mode": "none"},
        "fills": [{"type": "solid", "color": "$color.surface"}],
        "strokes": [{"color": "$color.border", "width": "$border.thin"}],
        "radius": {
            "topLeft": "$radius.device",
            "topRight": "$radius.device",
            "bottomRight": "$radius.device",
            "bottomLeft": "$radius.device"
        },
        "shadows": [
            {"type": "drop", "color": "$color.shadow", "offsetX": 4, "offsetY": 12, "blur": 32, "spread": 2}
        ],
        "opacity": 0.98,
        "children": [sheet()]
    })
}

fn sheet() -> Value {
    json!({
        "id": "frame004",
        "type": "frame",
        "name": "Sheet",
        "x": 12,
        "y": 480,
        "width": {"fixed": 366},
        "height": "hug",
        "minWidth": "$size.sheet.min",
        "maxWidth": "$size.sheet.max",
        "minHeight": 120,
        "maxHeight": "$size.sheet.tall",
        "layout": {
            "mode": "flex",
            "direction": "column-reverse",
            "wrap": "wrap-reverse",
            "gap": {"row": "$space.sm", "column": 4},
            "padding": {"top": "$space.md", "right": 20, "bottom": "$space.lg", "left": 20},
            "alignItems": "end",
            "justifyContent": "space-around"
        },
        "clip": true,
        "children": [grabber(), chevron(), thumbnail()]
    })
}

fn grabber() -> Value {
    json!({
        "id": "text0004",
        "type": "text",
        "name": "Grabber",
        "absolute": true,
        "x": 167,
        "y": -12,
        "width": {"fixed": 32},
        "height": {"fixed": 4},
        "content": "—",
        "fontFamily": "Inter",
        "fontSize": 12,
        "fontWeight": 300,
        "color": "#94a3b8cc"
    })
}

fn chevron() -> Value {
    json!({
        "id": "vec00002",
        "type": "vector",
        "name": "Chevron",
        "width": {"fixed": 16},
        "height": {"fixed": 16},
        "path": "M4 6l4 4 4-4",
        "strokes": [{"color": "$color.icon", "width": "$border.icon"}]
    })
}

fn thumbnail() -> Value {
    json!({
        "id": "jpeg0002",
        "type": "image",
        "name": "Thumbnail",
        "width": "fill",
        "height": {"fixed": "$size.thumb"},
        "maxHeight": 96,
        "image": "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd",
        "fit": "contain"
    })
}

/// The IDs of `node` and every node below it that `include` accepts, depth first.
fn ids(node: &Node, include: fn(&Node) -> bool) -> Vec<String> {
    let mut found = Vec::new();
    if include(node) {
        found.push(node.id().as_str().to_string());
    }
    for child in node.children() {
        found.extend(ids(child, include));
    }
    found
}

#[test]
fn a_complete_design_round_trips_without_loss() {
    let design = complete_design();
    assert_round_trips(&design);

    let node = parse(&design);
    assert_eq!(node.kind(), NodeKind::Page);
    assert_eq!(
        ids(&node, |_| true),
        [
            "page0001", "brd00001", "frame001", "text0001", "btn00001", "vec00001", "frame002",
            "jpeg0001", "frame003", "text0002", "btn00002", "text0003", "brd00002", "frame004",
            "text0004", "vec00002", "jpeg0002",
        ],
        "every node of the design is parsed, in order"
    );
    assert_eq!(
        ids(&node, |node| !node.visible()),
        ["frame003", "btn00002", "brd00002"],
        "the hidden nodes stay hidden"
    );
}

#[test]
fn serializing_a_parsed_design_again_gives_the_same_text() {
    let first = serde_json::to_string(&parse(&complete_design())).expect("the design serializes");
    let again: Node = serde_json::from_str(&first).expect("the serialized design parses");
    let second = serde_json::to_string(&again).expect("the design serializes again");
    assert_eq!(
        first, second,
        "serialization is stable: the same design gives the same JSON text"
    );
}

/// Doubles that need all 17 significant digits: 37.035000000000004,
/// 1439.1764831232663, 216.66666666666666 and 1.5384615384615385. A JSON
/// parser that is not exact, such as serde_json without its `float_roundtrip`
/// feature, reads each of them back as a neighboring double.
const FULL_PRECISION: [f64; 4] = [12.345 * 3.0, 1439.1764831232663, 650.0 / 3.0, 20.0 / 13.0];

#[test]
fn every_number_in_a_design_keeps_all_its_digits() {
    let [length, wide, weight, ratio] = FULL_PRECISION;
    let fraction = 10.0 / 11.0;
    let design = complete_design();
    for (pointer, value) in [
        ("/children/0/x", -wide),
        ("/children/0/y", length),
        ("/children/0/width/fixed", wide),
        ("/children/0/minHeight", weight),
        ("/children/0/layout/gap/row", length),
        ("/children/0/layout/padding/top", length),
        ("/children/0/fills/1/angle", -length),
        ("/children/0/fills/1/stops/1/position", fraction),
        ("/children/0/radius/topLeft", length),
        ("/children/0/children/0/maxWidth", wide),
        ("/children/0/children/0/strokes/0/width", ratio),
        ("/children/0/children/0/shadows/0/offsetX", -length),
        ("/children/0/children/0/shadows/0/offsetY", length),
        ("/children/0/children/0/shadows/0/blur", length),
        ("/children/0/children/0/shadows/0/spread", -ratio),
        ("/children/0/children/0/children/0/fontSize", length),
        ("/children/0/children/0/children/0/fontWeight", weight),
        ("/children/0/children/0/children/0/lineHeight", ratio),
        ("/children/0/children/0/children/0/letterSpacing", -ratio),
        ("/children/0/children/0/children/2/opacity", fraction),
        ("/children/0/children/1/layout/columns/0/fixed", weight),
        ("/children/0/children/1/layout/columns/1/fr", ratio),
    ] {
        let output = assert_round_trips(&with(&design, pointer, json!(value)));
        assert_eq!(
            output.pointer(pointer).and_then(Value::as_f64),
            Some(value),
            "{pointer} = {value} is written back exactly"
        );
    }
}

#[test]
fn the_format_is_closed_so_unknown_properties_are_rejected() {
    let design = complete_design();
    parse(&design);
    for (pointer, key) in [
        ("", "background"),
        ("/children/0", "opactiy"),
        ("/children/0/layout", "justify"),
        ("/children/0/layout/padding", "horizontal"),
        ("/children/0/layout/gap", "both"),
        ("/children/0/fills/0", "colour"),
        ("/children/0/fills/1/stops/0", "offset"),
        ("/children/0/radius", "all"),
        ("/children/0/children/0/strokes/0", "weight"),
        ("/children/0/children/0/shadows/0", "blurRadius"),
        ("/children/0/children/0/children/0", "fontColor"),
        ("/children/0/children/0/children/2", "d"),
        ("/children/0/children/1/layout", "template"),
        ("/children/0/children/1/children/0", "src"),
        ("/children/1/children/0/children/0", "textColor"),
    ] {
        let target = format!("{pointer}/{key}");
        let message = assert_rejected(
            &with(&design, &target, json!(1)),
            &format!("`{key}` is not a property of the model, so {target}"),
        );
        assert!(
            message.contains(key),
            "the error must name `{key}`; it says: {message}"
        );
    }
}
