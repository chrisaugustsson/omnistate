//! Helpers shared by the design node model tests (US-002).
//!
//! The tests pin the JSON form of design nodes, which the renderer, the API
//! and the CLI share:
//!
//! - Every node has an `id` (8 lowercase Crockford base32 characters: `0-9`
//!   and `a-z` without `i`, `l`, `o` and `u`), a `type` (`page`, `board`,
//!   `frame`, `text`, `image`, `vector` or `instance`), a `name`, and
//!   `visible`, which may be left out when it is `true`.
//! - Pages, boards and frames have ordered `children`. A page's children are
//!   boards, and boards sit only on pages.
//! - Boards, frames, text, images and vectors size each axis with `width` and
//!   `height`: `{"fixed": <length>}`, `"hug"` or `"fill"`. Instances may leave
//!   both out. `minWidth`, `maxWidth`, `minHeight` and `maxHeight` are
//!   optional lengths.
//! - `x` and `y` place the children of pages and of `none` layouts, and
//!   children marked `"absolute": true`. In-flow children of flex and grid
//!   layouts have no `x` or `y`: layout computes them when rendering.
//! - Boards and frames have a `layout`: `{"mode": "none"}`; `{"mode": "flex"}`
//!   with optional `direction`, `wrap`, `gap` (`row`, `column`), `padding`
//!   (`top`, `right`, `bottom`, `left`), `alignItems` and `justifyContent`; or
//!   `{"mode": "grid"}` with `columns`, `rows`, `gap` and `padding`. Grid
//!   tracks are `{"fixed": <px>}`, `{"fr": <n>}` or `"auto"`, and grid
//!   children may set `gridColumn` and `gridRow` to `{"start": <line>,
//!   "span": <tracks>}`. Keywords are the CSS ones.
//! - Style: `fills` (`solid` with a `color`, `linear-gradient` with an `angle`
//!   in CSS degrees and `stops` of `position` and `color`, `image` with an
//!   `attachment:<id>` and a `fit`), `strokes` (`color`, `width`), `radius`
//!   (`topLeft`, `topRight`, `bottomRight`, `bottomLeft`), `shadows` (`drop`
//!   or `inner`, with `color`, `offsetX`, `offsetY`, `blur` and `spread`),
//!   `opacity` (0 to 1) and `clip`.
//! - Text: `content` (plain text), `fontFamily`, `fontSize`, `fontWeight`,
//!   `fontStyle`, `lineHeight` (a multiple of the font size, like unitless CSS
//!   `line-height`), `letterSpacing`, `textAlign`, `textDecoration` and
//!   `color`. Images: `image` and `fit`. Vectors: `path`, SVG path data kept
//!   exactly as written. Instances: `component`.
//! - A length is a number of CSS pixels. A color is `#rrggbb` or `#rrggbbaa`.
//!   Every style and layout value used here also accepts a token reference
//!   such as `$color.surface`.
//! - Required: `id`, `type` and `name`; `layout` on boards and frames; `width`
//!   and `height` on every node except pages and instances; `content`,
//!   `fontFamily` and `fontSize` on text; `image` and `fit` on images and image
//!   fills; `angle` and stop `position`s on gradients; `color` and `width` on
//!   strokes; `path` on vectors; `component` on instances. Unknown properties
//!   are rejected.
//! - Serializing writes back every property a node sets, with its value. What
//!   else it writes must parse back to the same node.
#![allow(dead_code)]

use omni_core::design::Node;
use serde_json::{Value, json};

/// The attachment the image fixtures show.
pub const PHOTO: &str = "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd";

/// Keys that would hold geometry computed by layout.
const COMPUTED_GEOMETRY: [&str; 9] = [
    "transform",
    "matrix",
    "bounds",
    "boundingBox",
    "absoluteBoundingBox",
    "absoluteTransform",
    "relativeTransform",
    "selrect",
    "points",
];

/// Parses `json` as a design node, from its JSON text. Panics if it is rejected.
pub fn parse(json: &Value) -> Node {
    try_parse(json).unwrap_or_else(|error| {
        panic!("this design node must be accepted, but it was rejected: {error}\n{json:#}")
    })
}

/// Parses `json` as a design node, from its JSON text.
pub fn try_parse(json: &Value) -> Result<Node, serde_json::Error> {
    serde_json::from_str(&json.to_string())
}

/// `node` serialized to JSON text, read back as a JSON value.
pub fn to_json(node: &Node) -> Value {
    let text = serde_json::to_string(node).expect("a design node serializes to JSON");
    serde_json::from_str(&text).expect("a serialized design node is valid JSON")
}

/// Asserts that `json` is accepted and round-trips without loss, and returns
/// the serialized node. Every property of `json` must be written back with
/// the same value, the serialized JSON must parse back to the same node, and
/// it must hold no computed geometry.
pub fn assert_round_trips(json: &Value) -> Value {
    let node = parse(json);
    let output = to_json(&node);
    if let Some(difference) = first_difference(json, &output, "") {
        panic!(
            "serializing the node lost or changed a property: {difference}\n--- input ---\n{json:#}\n--- serialized ---\n{output:#}"
        );
    }
    let reparsed = try_parse(&output).unwrap_or_else(|error| {
        panic!("the serialized node must parse again, but it was rejected: {error}\n{output:#}")
    });
    assert_eq!(
        reparsed, node,
        "the serialized node must parse back to the same node\n{output:#}"
    );
    assert_no_stored_geometry(&output);
    output
}

/// Asserts that `json` is rejected, and returns the error message.
pub fn assert_rejected(json: &Value, why: &str) -> String {
    match try_parse(json) {
        Ok(node) => panic!("{why}, but this JSON was accepted as {node:?}:\n{json:#}"),
        Err(error) => error.to_string(),
    }
}

/// Asserts that each of `values`, set at `pointer` in `base`, is accepted,
/// survives a round trip, and gives a different node than the others.
pub fn assert_distinct_values(base: &Value, pointer: &str, values: &[Value]) {
    let nodes: Vec<Node> = values
        .iter()
        .map(|value| {
            let node = parse(&with(base, pointer, value.clone()));
            let again = try_parse(&to_json(&node)).unwrap_or_else(|error| {
                panic!("{pointer} = {value} must survive a round trip, but: {error}")
            });
            assert_eq!(again, node, "{pointer} = {value} must survive a round trip");
            node
        })
        .collect();
    for (i, first) in nodes.iter().enumerate() {
        for (j, second) in nodes.iter().enumerate().skip(i + 1) {
            assert_ne!(
                first, second,
                "{pointer} = {} and {pointer} = {} must give different nodes",
                values[i], values[j]
            );
        }
    }
}

/// The first property of `expected` that `actual` lacks or changes. Numbers
/// compare by value, so `320` and `320.0` are equal; arrays must match in length.
pub fn first_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            expected.iter().find_map(|(key, expected)| {
                let path = format!("{path}/{key}");
                match actual.get(key) {
                    None => Some(format!("{path} is missing (it was {expected})")),
                    Some(actual) => first_difference(expected, actual, &path),
                }
            })
        }
        (Value::Array(expected), Value::Array(actual)) if expected.len() != actual.len() => {
            Some(format!(
                "{path} has {} items instead of {}",
                actual.len(),
                expected.len()
            ))
        }
        (Value::Array(expected), Value::Array(actual)) => expected
            .iter()
            .zip(actual)
            .enumerate()
            .find_map(|(index, (expected, actual))| {
                first_difference(expected, actual, &format!("{path}/{index}"))
            }),
        (Value::Number(expected), Value::Number(actual))
            if expected.as_f64() == actual.as_f64() =>
        {
            None
        }
        _ if expected == actual => None,
        _ => Some(format!("{path} is {actual} instead of {expected}")),
    }
}

/// Asserts that serialized nodes hold no geometry computed by layout: no
/// transform matrices or computed boxes anywhere, and no `x` or `y` on the
/// in-flow children of flex and grid layouts.
pub fn assert_no_stored_geometry(node: &Value) {
    assert_no_computed_keys(node, "");
    assert_flow_children_unplaced(node);
}

fn assert_no_computed_keys(value: &Value, path: &str) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                assert!(
                    !COMPUTED_GEOMETRY.contains(&key.as_str()) && !key.starts_with("computed"),
                    "{path}/{key}: layout is computed when rendering, so no computed geometry is stored"
                );
                assert_no_computed_keys(value, &format!("{path}/{key}"));
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                assert_no_computed_keys(item, &format!("{path}/{index}"));
            }
        }
        _ => {}
    }
}

fn assert_flow_children_unplaced(node: &Value) {
    let flow = matches!(
        node.pointer("/layout/mode").and_then(Value::as_str),
        Some("flex" | "grid")
    );
    let children = node
        .get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for child in children {
        let absolute = child.get("absolute") == Some(&Value::Bool(true));
        if flow && !absolute {
            for key in ["x", "y"] {
                assert!(
                    child.get(key).is_none(),
                    "node {} flows in a {} layout, so its `{key}` is computed and must not be stored",
                    child["id"],
                    node["layout"]["mode"]
                );
            }
        }
        assert_flow_children_unplaced(child);
    }
}

/// A copy of `json` with `value` set at `pointer`, such as `/layout/gap` or
/// `/children/0/x`. The last step may add a new key.
pub fn with(json: &Value, pointer: &str, value: Value) -> Value {
    let mut copy = json.clone();
    let (parent, key) = pointer
        .rsplit_once('/')
        .unwrap_or_else(|| panic!("`{pointer}` is not a JSON pointer"));
    match copy.pointer_mut(parent) {
        Some(Value::Object(object)) => {
            object.insert(key.to_string(), value);
        }
        Some(Value::Array(items)) => {
            let index: usize = key.parse().expect("an array index");
            items[index] = value;
        }
        _ => panic!("the fixture has no object or array at `{parent}`"),
    }
    copy
}

/// A copy of `json` without the property at `pointer`.
pub fn without(json: &Value, pointer: &str) -> Value {
    let mut copy = json.clone();
    let (parent, key) = pointer
        .rsplit_once('/')
        .unwrap_or_else(|| panic!("`{pointer}` is not a JSON pointer"));
    let removed = copy
        .pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .and_then(|object| object.remove(key));
    assert!(removed.is_some(), "the fixture has no `{pointer}`");
    copy
}

/// A copy of `json` with `children`.
pub fn containing(json: &Value, children: Vec<Value>) -> Value {
    with(json, "/children", Value::Array(children))
}

/// A copy of `json` placed at `x` and `y` in its parent.
pub fn placed(json: &Value, x: f64, y: f64) -> Value {
    with(&with(json, "/x", json!(x)), "/y", json!(y))
}

/// A page holding `children`.
pub fn page(id: &str, children: Vec<Value>) -> Value {
    let page = json!({"id": id, "type": "page", "name": format!("Page {id}")});
    if children.is_empty() {
        page
    } else {
        containing(&page, children)
    }
}

/// A board placed on its page, with no layout of its own.
pub fn board(id: &str) -> Value {
    json!({
        "id": id,
        "type": "board",
        "name": format!("Board {id}"),
        "x": 100,
        "y": 200,
        "width": {"fixed": 1440},
        "height": {"fixed": 900},
        "layout": {"mode": "none"}
    })
}

/// A fixed-size frame with no layout of its own.
pub fn frame(id: &str) -> Value {
    json!({
        "id": id,
        "type": "frame",
        "name": format!("Frame {id}"),
        "width": {"fixed": 120},
        "height": {"fixed": 80},
        "layout": {"mode": "none"}
    })
}

/// A frame whose flex layout places `children` in flow.
pub fn flex_frame(id: &str, children: Vec<Value>) -> Value {
    containing(
        &with(&frame(id), "/layout", json!({"mode": "flex"})),
        children,
    )
}

/// A text node that hugs its content.
pub fn text(id: &str) -> Value {
    json!({
        "id": id,
        "type": "text",
        "name": format!("Text {id}"),
        "width": "hug",
        "height": "hug",
        "content": "Hello",
        "fontFamily": "Inter",
        "fontSize": 16
    })
}

/// A fixed-size image node.
pub fn image(id: &str) -> Value {
    json!({
        "id": id,
        "type": "image",
        "name": format!("Image {id}"),
        "width": {"fixed": 160},
        "height": {"fixed": 90},
        "image": PHOTO,
        "fit": "cover"
    })
}

/// A fixed-size vector node.
pub fn vector(id: &str) -> Value {
    json!({
        "id": id,
        "type": "vector",
        "name": format!("Vector {id}"),
        "width": {"fixed": 24},
        "height": {"fixed": 24},
        "path": "M0 0 L24 24"
    })
}

/// An instance of a component, sized by the component.
pub fn instance(id: &str) -> Value {
    json!({
        "id": id,
        "type": "instance",
        "name": format!("Instance {id}"),
        "component": "cmp00001"
    })
}

/// Strings as JSON values.
pub fn strings<const N: usize>(values: [&str; N]) -> [Value; N] {
    values.map(Value::from)
}
