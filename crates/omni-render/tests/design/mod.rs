//! Design fixtures and box assertions shared by the layout tests (US-003).
//!
//! The tests lay designs out with `omni_render::layout::compute` and the
//! bundled fonts, and expect the boxes a browser gives the same design written
//! as HTML and CSS, by the mapping in `docs/formats/layout.md`:
//!
//! - Boxes are in page coordinates and keep their fractions, as a browser's
//!   do (it lays out in 1/64 px), so nothing is rounded to whole pixels.
//!   Sizes include padding (`box-sizing: border-box`).
//! - Flex layouts are CSS flexbox: `direction` is `flex-direction`,
//!   `gap.row` is `row-gap` and `gap.column` is `column-gap`, and a property
//!   left out keeps its CSS initial value: `row`, `nowrap`, and `normal`
//!   alignment, which stretches children that hug their cross axis and
//!   stretches wrapped lines into free cross space (`align-content`).
//! - On the main axis of a flex layout, `fixed` never grows or shrinks
//!   (`flex-shrink: 0`), and `fill` children share the free space equally
//!   (`flex-grow: 1` from `flex-basis: 0`). On the cross axis, `hug` is
//!   `auto`, so `alignItems` places it, and `fill` stretches
//!   (`align-self: stretch`).
//! - On a grid, tracks are `px`, `fr` and `auto`, `gridColumn` and `gridRow`
//!   are `grid-column` and `grid-row`, `fixed` and `hug` children sit at the
//!   start of their area (`justify-self: start`, `align-self: start`), and
//!   `fill` children fill it (`stretch`).
//! - Children of `none` layouts and `absolute` children of flex and grid
//!   layouts sit at `x` and `y` from their parent's top-left corner
//!   (`position: absolute` in a `position: relative` parent) and take no space.
//! - Hidden nodes (`"visible": false`) take no space (`display: none`).
//! - `minWidth`, `maxWidth`, `minHeight` and `maxHeight` bound every sizing,
//!   and a min beats a max, as in CSS.
//! - Text is shaped and measured with Skia's paragraph layout, in the bundled
//!   Inter when its family is missing. A `hug` width is the text's own width,
//!   which wraps only where CSS makes it `fit-content`; a `fixed` or `fill`
//!   width wraps lines at that width, and so does a line break. Each line is
//!   `lineHeight` × `fontSize` tall, fractions included, or as tall as Chrome
//!   makes `line-height: normal` when `lineHeight` is left out: the font's
//!   ascent and descent, each rounded to whole pixels. `letterSpacing` is CSS
//!   `letter-spacing`, which follows every character.
#![allow(dead_code)]

use omni_core::design::{Node, NodeId};
use omni_render::layout::{self, LayoutBox};
use omni_render::text::Fonts;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

/// The boxes of a laid-out design, by node ID.
pub type Boxes = HashMap<NodeId, LayoutBox>;

/// How far an edge or a size may be from the expected one. Browsers lay out
/// in 1/64 px, so a box rounded to whole pixels is off by more than this.
pub const EPSILON: f32 = 0.01;

/// Parses `json` as a design node. Panics if the design model rejects it.
pub fn node(json: &Value) -> Node {
    serde_json::from_str(&json.to_string()).unwrap_or_else(|error| {
        panic!("the fixture must be a valid design node, but it was rejected: {error}\n{json:#}")
    })
}

/// Lays out the design `json` with the bundled fonts.
pub fn lay_out(json: &Value) -> Boxes {
    layout::compute(&node(json), &Fonts::bundled())
}

/// The node ID written as `id`.
pub fn id(id: &str) -> NodeId {
    id.parse()
        .unwrap_or_else(|error| panic!("the fixture uses an invalid node ID: {error}"))
}

/// The box of node `id`. Panics if layout gave it none.
pub fn box_of(boxes: &Boxes, node: &str) -> LayoutBox {
    *boxes.get(&id(node)).unwrap_or_else(|| {
        panic!("layout must return a box for every node, but node {node} has none")
    })
}

/// Asserts that node `node` has the box `[x, y, width, height]`, give or take
/// [`EPSILON`]. `case` names the situation, for the failure message.
pub fn assert_box(boxes: &Boxes, node: &str, expected: [f32; 4], case: &str) {
    assert_box_within(boxes, node, expected, EPSILON, case);
}

/// Like [`assert_box`], give or take `tolerance` pixels.
pub fn assert_box_within(
    boxes: &Boxes,
    node: &str,
    [x, y, width, height]: [f32; 4],
    tolerance: f32,
    case: &str,
) {
    let actual = box_of(boxes, node);
    let near = |actual: f32, expected: f32| (actual - expected).abs() <= tolerance;
    assert!(
        near(actual.x, x)
            && near(actual.y, y)
            && near(actual.width, width)
            && near(actual.height, height),
        "{case}: node {node} must have the box x {x}, y {y}, {width} x {height} (give or take {tolerance} px), but layout gave x {}, y {}, {} x {}",
        actual.x,
        actual.y,
        actual.width,
        actual.height
    );
}

/// The IDs of `json` and of every node below it.
pub fn ids_in(json: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let mut pending = vec![json];
    while let Some(node) = pending.pop() {
        let id = node["id"].as_str().expect("every fixture node has an id");
        ids.insert(id.to_string());
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            pending.extend(children);
        }
    }
    ids
}

/// A fixed size: `{"fixed": length}`.
pub fn fixed(length: f64) -> Value {
    json!({"fixed": length})
}

/// A layout with no layout of its own: children sit at their `x` and `y`.
pub fn none() -> Value {
    json!({"mode": "none"})
}

/// A flex layout with `properties`, such as `{"direction": "column"}`.
pub fn flex(properties: Value) -> Value {
    with_mode("flex", properties)
}

/// A grid layout with `properties`, such as `{"columns": [{"fr": 1}]}`.
pub fn grid(properties: Value) -> Value {
    with_mode("grid", properties)
}

fn with_mode(mode: &str, properties: Value) -> Value {
    let mut layout = properties;
    layout
        .as_object_mut()
        .expect("layout properties are an object")
        .insert("mode".to_string(), json!(mode));
    layout
}

/// A board with `layout` and `children`, at the page origin unless it is
/// placed with [`at`].
pub fn board(id: &str, width: Value, height: Value, layout: Value, children: Vec<Value>) -> Value {
    json!({
        "id": id,
        "type": "board",
        "name": format!("Board {id}"),
        "width": width,
        "height": height,
        "layout": layout,
        "children": children
    })
}

/// A frame with `layout` and `children`.
pub fn frame(id: &str, width: Value, height: Value, layout: Value, children: Vec<Value>) -> Value {
    json!({
        "id": id,
        "type": "frame",
        "name": format!("Frame {id}"),
        "width": width,
        "height": height,
        "layout": layout,
        "children": children
    })
}

/// An empty frame sized `width` x `height`.
pub fn sized(id: &str, width: Value, height: Value) -> Value {
    frame(id, width, height, none(), Vec::new())
}

/// An empty frame of a fixed `width` x `height`.
pub fn rect(id: &str, width: f64, height: f64) -> Value {
    sized(id, fixed(width), fixed(height))
}

/// A frame that hugs its one child, a `width` x `height` rect named
/// `child`: its content is exactly that size.
pub fn hugging(id: &str, child: &str, width: f64, height: f64) -> Value {
    frame(
        id,
        json!("hug"),
        json!("hug"),
        flex(json!({})),
        vec![rect(child, width, height)],
    )
}

/// A text node in the bundled Inter that hugs its content: `font_size` px,
/// with lines `line_height` times as tall.
pub fn text(id: &str, content: &str, font_size: f64, line_height: f64) -> Value {
    json!({
        "id": id,
        "type": "text",
        "name": format!("Text {id}"),
        "width": "hug",
        "height": "hug",
        "content": content,
        "fontFamily": "Inter",
        "fontSize": font_size,
        "lineHeight": line_height
    })
}

/// A copy of `json` with `value` set at `key`.
pub fn with(json: &Value, key: &str, value: Value) -> Value {
    let mut copy = json.clone();
    copy.as_object_mut()
        .expect("fixture nodes are objects")
        .insert(key.to_string(), value);
    copy
}

/// A copy of `json` placed at `x` and `y` in its parent.
pub fn at(json: &Value, x: f64, y: f64) -> Value {
    with(&with(json, "x", json!(x)), "y", json!(y))
}

/// A copy of `json` taken out of its flex or grid parent's flow and placed at
/// `x` and `y`.
pub fn absolute(json: &Value, x: f64, y: f64) -> Value {
    at(&with(json, "absolute", json!(true)), x, y)
}
