//! The tag format: designs as short, JSX-like tags that agents read and write
//! (Q26, FR-33). Storage stays JSON, and [`print`] and [`parse`] convert
//! between the two without loss.
//!
//! - One tag is one node, named after its type: `Page`, `Board`, `Frame`,
//!   `Text`, `Image`, `Vector`, `Instance` and `Component`. A tag's children
//!   are the node's children, in order.
//! - Attributes are the node's properties, written `name="text"` or
//!   `name={expression}`, such as
//!   `<Frame id="k3f9x2qa" name="Card" dir="col" gap="$space.md" pad={16} fill="$color.surface">`.
//!   A tag without children may close itself: `<Frame name="Card"/>`.
//! - `id` refers to an existing node. A tag written without an `id` is a new
//!   node: the server assigns its ID (US-031), never the agent.
//! - [`print`] writes every property a node sets, and every number as the same
//!   double, so parsing what it prints gives back the same node.
//! - [`parse`] rejects unknown tags, unknown attributes and invalid values with
//!   a [`TagError`] naming the line, the column and how to fix it. Lines and
//!   columns count from 1, and columns count characters. An error points at
//!   the first character of the tag name, the attribute name or the value
//!   (its opening `"` or `{`) that has the problem.
//!
//! `docs/formats/design-tags.md` documents every tag and attribute.

use super::json::Object;
use super::proto::Interaction;
use super::value::non_negative;
use super::{
    AttachmentRef, Bind, Color, Fill, FontFamily, Gap, GridPlacement, LayoutMode, Length, Margin,
    Node, NodeData, NodeId, NodeKind, Overrides, PROP_KINDS, Padding, PathData, PropDefs,
    PropValues, Radius, Shadow, Sizing, Stroke, TokenOr, Track,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Number, Value, json};
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

/// A tag of the format and the attributes it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    /// The tag's name, such as `Frame`.
    pub name: &'static str,
    /// Every attribute the tag takes, such as `id`, `name` and `dir`.
    pub attributes: &'static [&'static str],
}

const BOARD: &[&str] = &[
    "id",
    "name",
    "visible",
    "absolute",
    "x",
    "y",
    "gridColumn",
    "gridRow",
    "margin",
    "justifySelf",
    "alignSelf",
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
    "layout",
    "dir",
    "wrap",
    "gap",
    "pad",
    "align",
    "justify",
    "cols",
    "rows",
    "fill",
    "strokes",
    "radius",
    "shadows",
    "opacity",
    "clip",
    "interactions",
];

const FRAME: &[&str] = &[
    "id",
    "name",
    "visible",
    "absolute",
    "x",
    "y",
    "gridColumn",
    "gridRow",
    "margin",
    "justifySelf",
    "alignSelf",
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
    "layout",
    "dir",
    "wrap",
    "gap",
    "pad",
    "align",
    "justify",
    "cols",
    "rows",
    "fill",
    "strokes",
    "radius",
    "shadows",
    "opacity",
    "clip",
    "bind",
    "interactions",
];

const COMPONENT: &[&str] = &[
    "id",
    "name",
    "visible",
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
    "layout",
    "dir",
    "wrap",
    "gap",
    "pad",
    "align",
    "justify",
    "cols",
    "rows",
    "fill",
    "strokes",
    "radius",
    "shadows",
    "opacity",
    "clip",
    "props",
    "interactions",
];

const TEXT: &[&str] = &[
    "id",
    "name",
    "visible",
    "absolute",
    "x",
    "y",
    "gridColumn",
    "gridRow",
    "margin",
    "justifySelf",
    "alignSelf",
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
    "content",
    "fontFamily",
    "fontSize",
    "fontWeight",
    "fontStyle",
    "lineHeight",
    "letterSpacing",
    "textAlign",
    "textDecoration",
    "color",
    "opacity",
    "bind",
    "interactions",
];

const IMAGE: &[&str] = &[
    "id",
    "name",
    "visible",
    "absolute",
    "x",
    "y",
    "gridColumn",
    "gridRow",
    "margin",
    "justifySelf",
    "alignSelf",
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
    "image",
    "fit",
    "strokes",
    "radius",
    "shadows",
    "opacity",
    "bind",
    "interactions",
];

const VECTOR: &[&str] = &[
    "id",
    "name",
    "visible",
    "absolute",
    "x",
    "y",
    "gridColumn",
    "gridRow",
    "margin",
    "justifySelf",
    "alignSelf",
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
    "path",
    "fill",
    "strokes",
    "shadows",
    "opacity",
    "bind",
    "interactions",
];

const INSTANCE: &[&str] = &[
    "id",
    "name",
    "visible",
    "absolute",
    "x",
    "y",
    "gridColumn",
    "gridRow",
    "margin",
    "justifySelf",
    "alignSelf",
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
    "component",
    "props",
    "overrides",
    "bind",
    "interactions",
];

/// The tags, in the order of [`KINDS`].
static TAGS: [Tag; 8] = [
    Tag {
        name: "Page",
        attributes: &["id", "name", "visible"],
    },
    Tag {
        name: "Board",
        attributes: BOARD,
    },
    Tag {
        name: "Frame",
        attributes: FRAME,
    },
    Tag {
        name: "Text",
        attributes: TEXT,
    },
    Tag {
        name: "Image",
        attributes: IMAGE,
    },
    Tag {
        name: "Vector",
        attributes: VECTOR,
    },
    Tag {
        name: "Instance",
        attributes: INSTANCE,
    },
    Tag {
        name: "Component",
        attributes: COMPONENT,
    },
];

/// The node type of each tag of [`TAGS`].
const KINDS: [NodeKind; 8] = [
    NodeKind::Page,
    NodeKind::Board,
    NodeKind::Frame,
    NodeKind::Text,
    NodeKind::Image,
    NodeKind::Vector,
    NodeKind::Instance,
    NodeKind::Component,
];

/// Every tag of the format, one per node type.
pub fn all() -> &'static [Tag] {
    &TAGS
}

fn tag_of(kind: NodeKind) -> &'static Tag {
    let index = KINDS.iter().position(|each| *each == kind);
    &TAGS[index.expect("every node type has a tag")]
}

fn kind_of(tag: &str) -> Option<NodeKind> {
    let index = TAGS.iter().position(|each| each.name == tag)?;
    Some(KINDS[index])
}

/// The tags, quoted for a message: `` `Page`, `Board`, ... or `Instance` ``.
fn tag_names() -> String {
    let names: Vec<String> = TAGS.iter().map(|tag| format!("`{}`", tag.name)).collect();
    list_of(&names)
}

fn list_of(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [first @ .., last] => format!("{} or {last}", first.join(", ")),
    }
}

/// Whether nodes of `kind` have children.
fn has_children(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Page | NodeKind::Board | NodeKind::Frame | NodeKind::Component
    )
}

/// How deep tags nest at most, which keeps the parser and the printer off
/// deep recursion.
const MAX_DEPTH: usize = 100;

// How each attribute is written, and where it goes in the node's JSON.

type Keywords = &'static [(&'static str, &'static str)];

const LAYOUT: Keywords = &[("none", "none"), ("flex", "flex"), ("grid", "grid")];
const DIR: Keywords = &[
    ("row", "row"),
    ("col", "column"),
    ("row-reverse", "row-reverse"),
    ("col-reverse", "column-reverse"),
];
const WRAP: Keywords = &[
    ("nowrap", "nowrap"),
    ("wrap", "wrap"),
    ("wrap-reverse", "wrap-reverse"),
];
const ALIGN: Keywords = &[
    ("start", "start"),
    ("center", "center"),
    ("end", "end"),
    ("stretch", "stretch"),
    ("baseline", "baseline"),
];
const JUSTIFY: Keywords = &[
    ("start", "start"),
    ("center", "center"),
    ("end", "end"),
    ("space-between", "space-between"),
    ("space-around", "space-around"),
    ("space-evenly", "space-evenly"),
];
const SELF: Keywords = &[
    ("start", "start"),
    ("center", "center"),
    ("end", "end"),
    ("stretch", "stretch"),
];
const FONT_STYLE: Keywords = &[("normal", "normal"), ("italic", "italic")];
const TEXT_ALIGN: Keywords = &[
    ("left", "left"),
    ("center", "center"),
    ("right", "right"),
    ("justify", "justify"),
];
const TEXT_DECORATION: Keywords = &[
    ("none", "none"),
    ("underline", "underline"),
    ("line-through", "line-through"),
];
const FIT: Keywords = &[("cover", "cover"), ("contain", "contain")];

/// Where an attribute goes in a node's JSON.
#[derive(Clone, Copy)]
enum Home {
    /// A property of the node.
    Node,
    /// A property of the node's `layout`.
    Layout,
}

/// The attributes that give one value to every side (or corner) of a box, or
/// one value per side in an object.
#[derive(Clone, Copy)]
enum Sides {
    Gap,
    Padding,
    Margin,
    Radius,
}

impl Sides {
    fn fields(self) -> &'static [&'static str] {
        match self {
            Sides::Gap => &["row", "column"],
            Sides::Padding | Sides::Margin => &["top", "right", "bottom", "left"],
            Sides::Radius => &["topLeft", "topRight", "bottomRight", "bottomLeft"],
        }
    }
}

/// What kind of value an attribute takes.
#[derive(Clone, Copy)]
enum Shape {
    Text,
    Path,
    Flag,
    Number,
    /// A length of 0 or more, or a token reference.
    Length,
    /// A number or a token reference.
    Any,
    /// A number between two limits, both included, or a token reference.
    Within(f64, f64),
    Keyword(Keywords),
    Size,
    Sides(Sides),
    Fills,
    Strokes,
    Shadows,
    Tracks,
    Grid,
    Color,
    Family,
    Image,
    Component,
    /// The properties a component defines.
    Definitions,
    /// The values an instance gives to properties.
    Values,
    Overrides,
    Bind,
    Interactions,
}

struct Spec {
    attr: &'static str,
    key: &'static str,
    shape: Shape,
    home: Home,
}

const fn node(attr: &'static str, key: &'static str, shape: Shape) -> Spec {
    Spec {
        attr,
        key,
        shape,
        home: Home::Node,
    }
}

const fn layout(attr: &'static str, key: &'static str, shape: Shape) -> Spec {
    Spec {
        attr,
        key,
        shape,
        home: Home::Layout,
    }
}

static SPECS: &[Spec] = &[
    node("name", "name", Shape::Text),
    node("visible", "visible", Shape::Flag),
    node("absolute", "absolute", Shape::Flag),
    node("x", "x", Shape::Number),
    node("y", "y", Shape::Number),
    node("gridColumn", "gridColumn", Shape::Grid),
    node("gridRow", "gridRow", Shape::Grid),
    node("margin", "margin", Shape::Sides(Sides::Margin)),
    node("justifySelf", "justifySelf", Shape::Keyword(SELF)),
    node("alignSelf", "alignSelf", Shape::Keyword(SELF)),
    node("width", "width", Shape::Size),
    node("height", "height", Shape::Size),
    node("minWidth", "minWidth", Shape::Length),
    node("maxWidth", "maxWidth", Shape::Length),
    node("minHeight", "minHeight", Shape::Length),
    node("maxHeight", "maxHeight", Shape::Length),
    layout("layout", "mode", Shape::Keyword(LAYOUT)),
    layout("dir", "direction", Shape::Keyword(DIR)),
    layout("wrap", "wrap", Shape::Keyword(WRAP)),
    layout("gap", "gap", Shape::Sides(Sides::Gap)),
    layout("pad", "padding", Shape::Sides(Sides::Padding)),
    layout("align", "alignItems", Shape::Keyword(ALIGN)),
    layout("justify", "justifyContent", Shape::Keyword(JUSTIFY)),
    layout("cols", "columns", Shape::Tracks),
    layout("rows", "rows", Shape::Tracks),
    node("fill", "fills", Shape::Fills),
    node("strokes", "strokes", Shape::Strokes),
    node("radius", "radius", Shape::Sides(Sides::Radius)),
    node("shadows", "shadows", Shape::Shadows),
    node("opacity", "opacity", Shape::Within(0.0, 1.0)),
    node("clip", "clip", Shape::Flag),
    node("content", "content", Shape::Text),
    node("fontFamily", "fontFamily", Shape::Family),
    node("fontSize", "fontSize", Shape::Length),
    node("fontWeight", "fontWeight", Shape::Within(1.0, 1000.0)),
    node("fontStyle", "fontStyle", Shape::Keyword(FONT_STYLE)),
    node("lineHeight", "lineHeight", Shape::Length),
    node("letterSpacing", "letterSpacing", Shape::Any),
    node("textAlign", "textAlign", Shape::Keyword(TEXT_ALIGN)),
    node(
        "textDecoration",
        "textDecoration",
        Shape::Keyword(TEXT_DECORATION),
    ),
    node("color", "color", Shape::Color),
    node("image", "image", Shape::Image),
    node("fit", "fit", Shape::Keyword(FIT)),
    node("path", "path", Shape::Path),
    node("component", "component", Shape::Component),
    node("props", "props", Shape::Values),
    node("overrides", "overrides", Shape::Overrides),
    node("bind", "bind", Shape::Bind),
    node("interactions", "interactions", Shape::Interactions),
];

/// The `props` of a component are its property definitions, and those of an
/// instance the values it gives them.
static DEFINITIONS: Spec = node("props", "props", Shape::Definitions);

fn spec_of(kind: NodeKind, attr: &str) -> &'static Spec {
    if kind == NodeKind::Component && attr == "props" {
        return &DEFINITIONS;
    }
    SPECS
        .iter()
        .find(|spec| spec.attr == attr)
        .expect("every attribute of a tag has a spec")
}

/// A value that does not fit its attribute: what is wrong and how to fix it.
struct Problem {
    message: String,
    fix: String,
}

fn problem(message: impl Into<String>, fix: impl Into<String>) -> Problem {
    Problem {
        message: message.into(),
        fix: fix.into(),
    }
}

/// How to write the attribute, as the fix for a value that does not fit it.
fn how(spec: &Spec) -> String {
    let attr = spec.attr;
    match spec.shape {
        Shape::Text => format!("write `{attr}=\"text\"`"),
        Shape::Path => format!("write SVG path data, such as `{attr}=\"M0 0 L24 24\"`"),
        Shape::Flag => format!("write `{attr}`, `{attr}={{true}}` or `{attr}={{false}}`"),
        Shape::Number => format!("write a number, such as `{attr}={{10}}`"),
        Shape::Length => format!(
            "write a number of pixels, 0 or more, such as `{attr}={{16}}`, or a token reference such as `{attr}=\"$space.md\"`"
        ),
        Shape::Any => format!(
            "write a number, such as `{attr}={{16}}`, or a token reference such as `{attr}=\"$token.name\"`"
        ),
        Shape::Within(low, high) => format!(
            "write a number from {low} to {high}, such as `{attr}={{{high}}}`, or a token reference such as `{attr}=\"$token.name\"`"
        ),
        Shape::Keyword(table) => {
            let words: Vec<String> = table.iter().map(|(word, _)| format!("`{word}`")).collect();
            format!("use one of {}", list_of(&words))
        }
        Shape::Size => format!(
            "write `\"hug\"`, `\"fill\"`, a number of pixels such as `{attr}={{320}}`, or a token reference such as `{attr}=\"$size.card\"`"
        ),
        Shape::Sides(sides) => {
            let [first, second, ..] = sides.fields() else {
                unreachable!("every box has more than one side")
            };
            format!(
                "write one value for all of them, such as `{attr}={{8}}`, or an object with some of {}, such as `{attr}={{{{{first}: 8, {second}: 16}}}}`",
                list_of(
                    &sides
                        .fields()
                        .iter()
                        .map(|field| format!("`{field}`"))
                        .collect::<Vec<_>>()
                )
            )
        }
        Shape::Fills => "write a color such as `fill=\"#2563eb\"`, a token reference such as `fill=\"$color.surface\"`, or a list of fills such as `fill={[{type: \"solid\", color: \"#2563eb\"}]}`".to_string(),
        Shape::Strokes => "write a list of strokes, such as `strokes={[{color: \"#e2e8f0\", width: 1}]}`".to_string(),
        Shape::Shadows => "write a list of shadows or token references, such as `shadows={[{type: \"drop\", color: \"#0f172a33\", offsetX: 0, offsetY: 4, blur: 8}]}`".to_string(),
        Shape::Tracks => format!("write a list of tracks, such as `{attr}={{[{{fixed: 240}}, {{fr: 1}}, \"auto\"]}}`"),
        Shape::Grid => format!("write an object, such as `{attr}={{{{start: 1, span: 2}}}}`"),
        Shape::Color => format!("write a color such as `{attr}=\"#2563eb\"`, or a token reference such as `{attr}=\"$color.text\"`"),
        Shape::Family => format!("write the family's name, such as `{attr}=\"Inter\"`, or a token reference such as `{attr}=\"$font.family.sans\"`"),
        Shape::Image => format!("write `{attr}=\"attachment:<id>\"`"),
        Shape::Component => format!("write the ID of a component, such as `{attr}=\"k3f9x2qa\"`"),
        Shape::Definitions => format!(
            "write an object with a property of one of the kinds {}, such as `{attr}={{{{label: {{type: \"text\", default: \"Buy\"}}}}}}`",
            list_of(
                &PROP_KINDS
                    .iter()
                    .map(|kind| format!("`{kind}`"))
                    .collect::<Vec<_>>()
            )
        ),
        Shape::Values => format!(
            "write an object of property values, text or `true` or `false`, such as `{attr}={{{{label: \"Pay\", showIcon: false}}}}`"
        ),
        Shape::Overrides => format!(
            "write an object by the ID of a component node, such as `{attr}={{{{k3f9x2qa: {{content: \"Pay now\"}}}}}}` with `content`, `fills`, `visible` or `component`"
        ),
        Shape::Interactions => format!(
            "write a list of interactions, such as `{attr}={{[{{trigger: {{type: \"onClick\"}}, actions: [{{type: \"navigate\", dest: \"k3f9x2qa\"}}]}}]}}`"
        ),
        Shape::Bind => format!(
            "bind a layer of a component to its properties, such as `{attr}={{{{visible: \"showIcon\"}}}}` with `visible`, `content` or `component`"
        ),
    }
}

/// A problem from the text of a deserialization error. The errors of the
/// design values are written as "`value` is not a color: write ...", which
/// splits into what is wrong and how to fix it.
fn from_text(spec: &Spec, text: &str) -> Problem {
    if text.starts_with('`')
        && let Some((message, fix)) = text.split_once(": ")
    {
        return problem(message, fix);
    }
    problem(
        format!("`{}` is not valid here: {text}", spec.attr),
        how(spec),
    )
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "empty",
        Value::Bool(_) => "true or false",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

fn expected(spec: &Spec, value: &Value) -> Problem {
    problem(
        format!(
            "`{}` is {}, which it does not take",
            spec.attr,
            describe(value)
        ),
        how(spec),
    )
}

/// Checks that `value` is a `T`.
fn check<T: DeserializeOwned>(spec: &Spec, value: &Value) -> Result<(), Problem> {
    serde_json::from_value::<T>(value.clone())
        .map(|_| ())
        .map_err(|error| from_text(spec, &error.to_string()))
}

/// The JSON value that a tag's value stands for, checked to be a value of the
/// attribute.
fn convert(spec: &Spec, value: Value) -> Result<Value, Problem> {
    match spec.shape {
        Shape::Text => match value {
            Value::String(_) => Ok(value),
            other => Err(expected(spec, &other)),
        },
        Shape::Path => {
            let Value::String(text) = &value else {
                return Err(expected(spec, &value));
            };
            PathData::from_str(text)
                .map_err(|error| from_text(spec, &error.to_string()))
                .map(|_| value)
        }
        Shape::Flag => match value {
            Value::Bool(_) => Ok(value),
            other => Err(expected(spec, &other)),
        },
        Shape::Number => match value {
            Value::Number(_) => Ok(value),
            other => Err(expected(spec, &other)),
        },
        Shape::Length => non_negative(value.clone())
            .map(|_| value)
            .map_err(|error| from_text(spec, &error.to_string())),
        Shape::Any => Length::deserialize(value.clone())
            .map(|_| value)
            .map_err(|error| from_text(spec, &error.to_string())),
        Shape::Within(low, high) => {
            Length::deserialize(value.clone())
                .map_err(|error| from_text(spec, &error.to_string()))?;
            match value.as_f64() {
                Some(number) if !(low..=high).contains(&number) => Err(problem(
                    format!("`{}` is {number}, which is outside its range", spec.attr),
                    how(spec),
                )),
                _ => Ok(value),
            }
        }
        Shape::Keyword(table) => {
            let Value::String(word) = &value else {
                return Err(expected(spec, &value));
            };
            match table.iter().find(|(written, _)| written == word) {
                Some((_, keyword)) => Ok(Value::String((*keyword).to_string())),
                None => Err(problem(
                    format!("`{word}` is not a value of `{}`", spec.attr),
                    how(spec),
                )),
            }
        }
        Shape::Size => {
            let candidate = match &value {
                Value::Number(_) => json!({"fixed": value}),
                Value::String(text) if text.starts_with('$') => json!({"fixed": value}),
                Value::String(text) if text == "hug" || text == "fill" => value.clone(),
                Value::Object(_) => value.clone(),
                Value::String(text) => {
                    return Err(problem(format!("`{text}` is not a size"), how(spec)));
                }
                other => return Err(expected(spec, other)),
            };
            check::<Sizing>(spec, &candidate)?;
            Ok(candidate)
        }
        Shape::Sides(sides) => {
            let object = match &value {
                Value::Number(_) | Value::String(_) => Value::Object(
                    sides
                        .fields()
                        .iter()
                        .map(|field| ((*field).to_string(), value.clone()))
                        .collect(),
                ),
                Value::Object(_) => value,
                other => return Err(expected(spec, other)),
            };
            match sides {
                Sides::Gap => check::<Object<Gap>>(spec, &object)?,
                Sides::Padding => check::<Object<Padding>>(spec, &object)?,
                Sides::Margin => check::<Object<Margin>>(spec, &object)?,
                Sides::Radius => check::<Object<Radius>>(spec, &object)?,
            }
            Ok(object)
        }
        Shape::Fills => {
            let fills = match value {
                Value::String(_) => vec![json!({"type": "solid", "color": value})],
                Value::Array(fills) => fills,
                other => return Err(expected(spec, &other)),
            };
            for fill in &fills {
                check::<Fill>(spec, fill)?;
            }
            Ok(Value::Array(fills))
        }
        Shape::Strokes => each::<Object<Stroke>>(spec, value),
        Shape::Shadows => each::<TokenOr<Shadow>>(spec, value),
        Shape::Tracks => each::<Track>(spec, value),
        Shape::Grid => {
            if !value.is_object() {
                return Err(expected(spec, &value));
            }
            check::<Object<GridPlacement>>(spec, &value)?;
            Ok(value)
        }
        Shape::Color => check::<TokenOr<Color>>(spec, &value).map(|()| value),
        Shape::Family => check::<TokenOr<FontFamily>>(spec, &value).map(|()| value),
        Shape::Image => check::<AttachmentRef>(spec, &value).map(|()| value),
        Shape::Component => check::<NodeId>(spec, &value).map(|()| value),
        Shape::Definitions => check::<PropDefs>(spec, &value).map(|()| value),
        Shape::Values => check::<PropValues>(spec, &value).map(|()| value),
        Shape::Overrides => check::<Overrides>(spec, &value).map(|()| value),
        Shape::Bind => check::<Bind>(spec, &value).map(|()| value),
        Shape::Interactions => Interaction::list_from_value(&value)
            .map_err(|error| from_text(spec, &error))
            .and_then(|interactions| {
                serde_json::to_value(interactions)
                    .map_err(|error| from_text(spec, &error.to_string()))
            }),
    }
}

/// Checks that `value` is a list of `T`.
fn each<T: DeserializeOwned>(spec: &Spec, value: Value) -> Result<Value, Problem> {
    let Value::Array(items) = &value else {
        return Err(expected(spec, &value));
    };
    for item in items {
        check::<T>(spec, item)?;
    }
    Ok(value)
}

/// A node read from the tag format, and its subtree. A node written without
/// an `id` is new until the server assigns it one.
#[derive(Clone, Debug, PartialEq)]
pub struct TagNode {
    id: Option<NodeId>,
    kind: NodeKind,
    name: String,
    /// The node's other properties, as in its JSON form.
    props: Map<String, Value>,
    children: Vec<TagNode>,
}

impl TagNode {
    /// The node's ID, or `None` for a new node.
    pub fn id(&self) -> Option<&NodeId> {
        self.id.as_ref()
    }

    /// The node with the ID `id`, whatever ID it had.
    pub fn with_id(mut self, id: NodeId) -> TagNode {
        self.id = Some(id);
        self
    }

    /// Whether the node was written without an `id`.
    pub fn is_new(&self) -> bool {
        self.id.is_none()
    }

    pub fn kind(&self) -> NodeKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The node's children, in order.
    pub fn children(&self) -> &[TagNode] {
        &self.children
    }

    /// The `id` of every node of the subtree that was written with one, in
    /// document order.
    pub fn ids(&self) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self.id.into_iter().collect();
        for child in &self.children {
            ids.extend(child.ids());
        }
        ids
    }

    /// The node with an ID on every node: each new node, in document order,
    /// takes the next ID from `next`, and every other node keeps its own.
    ///
    /// # Panics
    ///
    /// If `next` gives an ID that another node of the tree has, because node
    /// IDs are unique.
    pub fn assign_ids(self, next: impl FnMut() -> NodeId) -> Node {
        self.try_assign_ids(next)
            .expect("the parser checked every node, and `next` gives IDs no other node has")
    }

    /// Like [`assign_ids`](Self::assign_ids), but the error of a tree that the
    /// node model refuses is returned instead of a panic.
    pub fn try_assign_ids(self, mut next: impl FnMut() -> NodeId) -> Result<Node, String> {
        let json = self.with_ids(&mut next);
        serde_json::from_value(json).map_err(|error| error.to_string())
    }

    fn with_ids(self, next: &mut dyn FnMut() -> NodeId) -> Value {
        let id = self.id.unwrap_or_else(&mut *next);
        let children = self
            .children
            .into_iter()
            .map(|child| child.with_ids(next))
            .collect();
        object(&id.to_string(), self.kind, &self.name, self.props, children)
    }
}

/// The node's JSON form.
fn object(
    id: &str,
    kind: NodeKind,
    name: &str,
    props: Map<String, Value>,
    children: Vec<Value>,
) -> Value {
    let mut map = Map::new();
    map.insert("id".to_string(), Value::from(id));
    map.insert("type".to_string(), Value::from(kind.as_str()));
    map.insert("name".to_string(), Value::from(name));
    map.extend(props);
    if has_children(kind) {
        map.insert("children".to_string(), Value::Array(children));
    }
    Value::Object(map)
}

/// Why text is not valid in the tag format: where the problem is, what it is
/// and how to fix it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagError {
    line: usize,
    column: usize,
    message: String,
    fix: String,
}

impl TagError {
    /// The line of the problem, counting from 1.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The column of the problem, counting characters from 1.
    pub fn column(&self) -> usize {
        self.column
    }

    /// The problem, such as "`Box` is not a tag".
    pub fn message(&self) -> &str {
        &self.message
    }

    /// How to fix it, such as "use one of `Page`, `Board`, `Frame`, ...".
    pub fn fix(&self) -> &str {
        &self.fix
    }
}

impl fmt::Display for TagError {
    /// `line <line>, column <column>: <message>`, then how to fix it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: {}\nfix: {}",
            self.line, self.column, self.message, self.fix
        )
    }
}

impl std::error::Error for TagError {}

/// Parses one tag and its subtree.
pub fn parse(source: &str) -> Result<TagNode, TagError> {
    parse_under(source, None)
}

/// The node that a tag goes inside, for a tag read on its own: what the
/// node's type and layout allow its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Host {
    kind: NodeKind,
    mode: LayoutMode,
}

impl Host {
    /// The host that `node` is.
    pub fn of(node: &Node) -> Host {
        let mode = match node.data() {
            NodeData::Board(frame) | NodeData::Frame(frame) => frame.layout.mode(),
            NodeData::Component(component) => component.frame.layout.mode(),
            _ => LayoutMode::None,
        };
        Host {
            kind: node.kind(),
            mode,
        }
    }

    fn parent(self) -> Parent {
        Parent {
            kind: self.kind,
            mode: self.mode,
            defs: None,
        }
    }
}

/// Parses one tag and its subtree that goes inside `host`, so that a tag that
/// does not fit there (a board inside a frame, a frame inside a page, `x` on a
/// child that flows) is refused. Without a host it goes at the top.
pub fn parse_in(source: &str, host: Option<&Host>) -> Result<TagNode, TagError> {
    parse_under(source, host.map(|host| host.parent()).as_ref())
}

fn parse_under(source: &str, parent: Option<&Parent>) -> Result<TagNode, TagError> {
    let mut parser = Parser {
        source,
        pos: 0,
        ids: HashSet::new(),
        expression: 0,
    };
    parser.skip_whitespace();
    match parser.peek() {
        None => {
            return Err(parser.error(
                parser.pos,
                "there is no tag",
                "write one tag, such as `<Frame name=\"Card\"/>`",
            ));
        }
        Some('<') => {}
        Some(_) => {
            return Err(parser.error(
                parser.pos,
                "expected a tag, but found text",
                "start with `<`, such as `<Frame name=\"Card\"/>`",
            ));
        }
    }
    let root = parser.element(parent, 0)?;
    parser.skip_whitespace();
    if parser.peek().is_some() {
        return Err(parser.error(
            parser.pos,
            "there is more than one tag at the top",
            "put the tags inside one parent tag, or write one tag at a time",
        ));
    }
    Ok(root)
}

/// Parses the tags written one after the other at the top, none or more, each
/// with its subtree. An `id` is written once in all of them.
pub fn parse_all(source: &str) -> Result<Vec<TagNode>, TagError> {
    let mut parser = Parser {
        source,
        pos: 0,
        ids: HashSet::new(),
        expression: 0,
    };
    let mut roots = Vec::new();
    loop {
        parser.skip_whitespace();
        match parser.peek() {
            None => return Ok(roots),
            Some('<') => roots.push(parser.element(None, 0)?),
            Some(_) => {
                return Err(parser.error(
                    parser.pos,
                    "expected a tag, but found text",
                    "start each tag with `<`, such as `<Frame name=\"Card\"/>`",
                ));
            }
        }
    }
}

/// What a parent tells its children: its type and how it lays them out.
struct Parent {
    kind: NodeKind,
    mode: LayoutMode,
    /// The properties of the component that the tag is inside, if it is inside
    /// one.
    defs: Option<PropDefs>,
}

struct Parser<'a> {
    source: &'a str,
    /// The byte offset of the next character.
    pos: usize,
    /// The IDs written so far.
    ids: HashSet<NodeId>,
    /// Where the `{` of the expression being read is.
    expression: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.source[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn skip_whitespace(&mut self) {
        let rest = self.rest();
        self.pos += rest.len() - rest.trim_start().len();
    }

    /// Reads letters, digits and underscores.
    fn word(&mut self) -> &'a str {
        let rest = self.rest();
        let length = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        self.pos += length;
        &rest[..length]
    }

    /// The error at the byte offset `at`.
    fn error(&self, at: usize, message: impl Into<String>, fix: impl Into<String>) -> TagError {
        let before = &self.source[..at.min(self.source.len())];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |index| index + 1);
        TagError {
            line,
            column: before[line_start..].chars().count() + 1,
            message: message.into(),
            fix: fix.into(),
        }
    }

    fn problem(&self, at: usize, problem: Problem) -> TagError {
        self.error(at, problem.message, problem.fix)
    }

    /// The error for an expression that stops before it is complete.
    fn unfinished(&self) -> TagError {
        self.error(
            self.expression,
            "this expression is never finished",
            "close every `[`, `{` and `\"` of the expression, and end it with `}`",
        )
    }

    /// Reads a tag and its subtree, from its `<`.
    fn element(&mut self, parent: Option<&Parent>, depth: usize) -> Result<TagNode, TagError> {
        if depth > MAX_DEPTH {
            return Err(self.error(
                self.pos,
                "the tags are nested too deeply",
                format!("nest tags at most {MAX_DEPTH} levels deep"),
            ));
        }
        self.pos += 1;
        if self.peek() == Some('/') {
            self.pos += 1;
            let at = self.pos;
            let name = self.word();
            return Err(self.error(
                at,
                format!("`</{name}>` closes a tag that is not open"),
                "remove it, or open the tag with `<` before it",
            ));
        }
        let name_pos = self.pos;
        let tag_name = self.word();
        if tag_name.is_empty() {
            return Err(self.error(
                name_pos,
                "`<` must be followed by a tag name",
                format!(
                    "write one of {}, such as `<Frame name=\"Card\"/>`",
                    tag_names()
                ),
            ));
        }
        let Some(kind) = kind_of(tag_name) else {
            return Err(self.error(
                name_pos,
                format!("`{tag_name}` is not a tag"),
                format!("use one of {}", tag_names()),
            ));
        };
        if let Some(problem) = misplaced(kind, parent) {
            return Err(self.problem(name_pos, problem));
        }
        let tag = tag_of(kind);

        let mut id = None;
        let mut name = None;
        let mut props = Map::new();
        let mut layout = Map::new();
        let mut explicit_mode = None;
        let mut bind = None;
        let mut layout_attrs = Vec::new();
        let mut attrs: Vec<(&str, usize)> = Vec::new();
        let self_closing = loop {
            self.skip_whitespace();
            let at = self.pos;
            match self.peek() {
                None => {
                    return Err(self.error(
                        name_pos,
                        format!("`<{tag_name}` is never finished"),
                        "end the tag with `>`, or with `/>` when it has no children",
                    ));
                }
                Some('/') => {
                    self.pos += 1;
                    if self.peek() != Some('>') {
                        return Err(self.error(
                            at,
                            "`/` must be followed by `>`",
                            "close a tag without children with `/>`",
                        ));
                    }
                    self.pos += 1;
                    break true;
                }
                Some('>') => {
                    self.pos += 1;
                    break false;
                }
                Some(c) if c.is_ascii_alphabetic() => {}
                Some(c) => {
                    return Err(self.error(
                        at,
                        format!("unexpected `{c}` in the tag `{tag_name}`"),
                        "write attributes as `name=\"text\"` or `name={expression}`, and end the tag with `>` or `/>`",
                    ));
                }
            }

            let attr = self.word();
            if !tag.attributes.contains(&attr) {
                return Err(self.error(
                    at,
                    format!("`{attr}` is not an attribute of `<{tag_name}>`"),
                    format!(
                        "`<{tag_name}>` takes {}",
                        list_of(
                            &tag.attributes
                                .iter()
                                .map(|attribute| format!("`{attribute}`"))
                                .collect::<Vec<_>>()
                        )
                    ),
                ));
            }
            if attrs.iter().any(|(seen, _)| *seen == attr) {
                return Err(self.error(
                    at,
                    format!("`{attr}` is written twice"),
                    format!("write `{attr}` once"),
                ));
            }
            attrs.push((attr, at));
            self.skip_whitespace();
            let (value_pos, value) = if self.peek() == Some('=') {
                self.pos += 1;
                self.skip_whitespace();
                self.value()?
            } else {
                (at, Value::Bool(true))
            };

            if attr == "id" {
                let Value::String(text) = &value else {
                    return Err(self.error(
                        value_pos,
                        format!("`id` is {}, which it does not take", describe(&value)),
                        "write `id=\"k3f9x2qa\"`, or leave `id` out for a new node",
                    ));
                };
                let node_id = NodeId::from_str(text).map_err(|error| {
                    let text = error.to_string();
                    let (message, fix) = text.split_once(": ").unwrap_or((&text, ""));
                    self.error(
                        value_pos,
                        message,
                        format!("{fix}, or leave `id` out for a new node"),
                    )
                })?;
                if !self.ids.insert(node_id) {
                    return Err(self.error(
                        value_pos,
                        format!("`{node_id}` is the `id` of more than one node"),
                        "give every node its own `id`, or leave `id` out for a new node",
                    ));
                }
                id = Some(node_id);
                continue;
            }

            let spec = spec_of(kind, attr);
            let json = convert(spec, value).map_err(|problem| self.problem(value_pos, problem))?;
            if attr == "bind" {
                self.check_bind(kind, &json, parent, value_pos)?;
                bind = Some(json);
                continue;
            }
            match (attr, spec.home) {
                ("name", _) => {
                    name = json.as_str().map(str::to_string);
                }
                ("layout", _) => {
                    let mode = match json.as_str() {
                        Some("flex") => LayoutMode::Flex,
                        Some("grid") => LayoutMode::Grid,
                        _ => LayoutMode::None,
                    };
                    explicit_mode = Some(mode);
                }
                (_, Home::Node) => {
                    props.insert(spec.key.to_string(), json);
                }
                (_, Home::Layout) => {
                    layout.insert(spec.key.to_string(), json);
                    layout_attrs.push((attr, at));
                }
            }
        };

        let mode = self.layout_mode(kind, explicit_mode, &layout_attrs)?;
        if matches!(
            kind,
            NodeKind::Board | NodeKind::Frame | NodeKind::Component
        ) {
            layout.insert("mode".to_string(), Value::from(mode.as_str()));
            props.insert("layout".to_string(), Value::Object(layout));
        }

        let Some(name) = name else {
            return Err(needs(self, name_pos, tag_name, "name"));
        };
        let required: &[&str] = match kind {
            NodeKind::Text => &["content", "fontFamily", "fontSize"],
            NodeKind::Image => &["image"],
            NodeKind::Vector => &["path"],
            NodeKind::Instance => &["component"],
            _ => &[],
        };
        if let Some(missing) = required.iter().find(|key| !props.contains_key(**key)) {
            return Err(needs(self, name_pos, tag_name, missing));
        }
        if !matches!(kind, NodeKind::Page | NodeKind::Instance) {
            for key in ["width", "height"] {
                props.entry(key).or_insert_with(|| Value::from("hug"));
            }
        }
        if kind == NodeKind::Image {
            props.entry("fit").or_insert_with(|| Value::from("cover"));
        }

        if let Some(parent) = parent {
            self.check_placement(parent, &props, &attrs)?;
        }
        if let Err(error) = serde_json::from_value::<Node>(object(
            "00000000",
            kind,
            &name,
            props.clone(),
            Vec::new(),
        )) {
            return Err(self.error(
                name_pos,
                format!("`<{tag_name}>` is not valid: {error}"),
                "docs/formats/design-tags.md says what each attribute takes",
            ));
        }

        // A component's own tag has already checked its properties, and its
        // layers check their binds against them.
        let defs = match (kind, props.get("props")) {
            (NodeKind::Component, Some(defs)) => PropDefs::from_value(defs).ok(),
            (NodeKind::Component, None) => Some(PropDefs::default()),
            _ => parent.and_then(|parent| parent.defs.clone()),
        };
        if let Some(bind) = bind {
            props.insert("bind".to_string(), bind);
        }
        let mut node = TagNode {
            id,
            kind,
            name,
            props,
            children: Vec::new(),
        };
        if self_closing {
            return Ok(node);
        }
        let parent = Parent { kind, mode, defs };
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => {
                    return Err(self.error(
                        name_pos,
                        format!("`<{tag_name}>` is never closed"),
                        format!("close it with `</{tag_name}>`"),
                    ));
                }
                Some('<') if self.rest().starts_with("</") => {
                    self.pos += 2;
                    let close_pos = self.pos;
                    let close = self.word();
                    if close != tag_name {
                        return Err(self.error(
                            close_pos,
                            format!("`</{close}>` does not close `<{tag_name}>`"),
                            format!("write `</{tag_name}>`"),
                        ));
                    }
                    self.skip_whitespace();
                    if self.peek() != Some('>') {
                        return Err(self.error(
                            self.pos,
                            format!("`</{tag_name}` must end with `>`"),
                            format!("write `</{tag_name}>`"),
                        ));
                    }
                    self.pos += 1;
                    return Ok(node);
                }
                Some('<') => {
                    let child = self.element(Some(&parent), depth + 1)?;
                    node.children.push(child);
                }
                Some(_) => {
                    return Err(self.error(
                        self.pos,
                        "expected a tag, but found text",
                        format!(
                            "put text in the `content` attribute of a `<Text>` tag, or close `<{tag_name}>` with `</{tag_name}>`"
                        ),
                    ));
                }
            }
        }
    }

    /// How a board or frame lays out its children, from its `layout` and the
    /// layout attributes it sets.
    fn layout_mode(
        &self,
        kind: NodeKind,
        explicit: Option<LayoutMode>,
        attrs: &[(&str, usize)],
    ) -> Result<LayoutMode, TagError> {
        let class = |attr: &str| match attr {
            "dir" | "wrap" | "align" | "justify" => Some(LayoutMode::Flex),
            "cols" | "rows" => Some(LayoutMode::Grid),
            _ => None,
        };
        let inferred = if attrs
            .iter()
            .any(|(attr, _)| class(attr) == Some(LayoutMode::Grid))
        {
            LayoutMode::Grid
        } else if attrs
            .iter()
            .any(|(attr, _)| class(attr) == Some(LayoutMode::Flex))
        {
            LayoutMode::Flex
        } else if kind == NodeKind::Component && !attrs.is_empty() {
            // A component that sets only `gap` or `pad` is a flex layout.
            LayoutMode::Flex
        } else {
            LayoutMode::None
        };
        let mode = explicit.unwrap_or(inferred);
        for (attr, at) in attrs {
            let fits = match (mode, class(attr)) {
                (LayoutMode::None, _) => false,
                (_, None) => true,
                (mode, Some(class)) => mode == class,
            };
            if fits {
                continue;
            }
            let (message, fix) = match (mode, class(attr)) {
                (LayoutMode::None, _) if explicit.is_some() => (
                    format!("`{attr}` is not allowed in a `none` layout"),
                    format!("remove `{attr}`, or set `layout=\"flex\"` or `layout=\"grid\"`"),
                ),
                (LayoutMode::None, _) => (
                    format!("`{attr}` needs a layout to apply to"),
                    "add `dir=\"row\"` or `dir=\"col\"` for a flex layout, `cols` or `rows` for a grid, or `layout=\"flex\"`".to_string(),
                ),
                (mode, class) => {
                    let class = class.unwrap_or(mode);
                    (
                        format!("`{attr}` is for {class} layouts, but this layout is {mode}"),
                        format!("remove `{attr}`, or lay the children out as {class}"),
                    )
                }
            };
            return Err(self.error(*at, message, fix));
        }
        Ok(mode)
    }

    /// Checks the `bind` of a layer: it is for the layers of a component, and
    /// it names properties that the component defines, of the kinds the bound
    /// fields need.
    fn check_bind(
        &self,
        kind: NodeKind,
        json: &Value,
        parent: Option<&Parent>,
        at: usize,
    ) -> Result<(), TagError> {
        let Some(defs) = parent.and_then(|parent| parent.defs.as_ref()) else {
            return Err(self.error(
                at,
                "`bind` is for the layers of a component, but this tag is not inside a `<Component>`",
                "remove `bind`, or put the tag inside the `<Component>` that defines the property",
            ));
        };
        let bind = Bind::from_value(json).expect("the value was checked as a bind");
        match bind.problem(kind, defs) {
            Some(problem) => Err(self.error(
                at,
                problem,
                "bind `visible` to a boolean property, `content` of a `<Text>` to a text property and `component` of an `<Instance>` to a slot property of the component",
            )),
            None => Ok(()),
        }
    }

    /// Checks the attributes that only fit some parents: layout computes the
    /// position of children that flow, and grid placement is for the children
    /// of grids.
    fn check_placement(
        &self,
        parent: &Parent,
        props: &Map<String, Value>,
        attrs: &[(&str, usize)],
    ) -> Result<(), TagError> {
        let flows = matches!(parent.mode, LayoutMode::Flex | LayoutMode::Grid)
            && props.get("absolute") != Some(&Value::Bool(true));
        if flows && let Some((attr, at)) = attrs.iter().find(|(attr, _)| matches!(*attr, "x" | "y"))
        {
            return Err(self.error(
                *at,
                format!(
                    "`{attr}` has no effect: the {} layout of the parent computes the position of a child that flows",
                    parent.mode
                ),
                format!("remove `{attr}`, or add `absolute` to take the child out of the flow"),
            ));
        }
        if parent.mode != LayoutMode::Grid
            && let Some((attr, at)) = attrs.iter().find(|(attr, _)| {
                matches!(
                    *attr,
                    "gridColumn" | "gridRow" | "justifySelf" | "alignSelf"
                )
            })
        {
            return Err(self.error(
                *at,
                format!("`{attr}` is for the children of grids, but the parent is not a grid"),
                format!("remove `{attr}`, or lay the parent out as a grid with `cols` or `rows`"),
            ));
        }
        Ok(())
    }

    /// Reads the value of an attribute: text in quotes, or an expression in
    /// braces. Returns where it starts.
    fn value(&mut self) -> Result<(usize, Value), TagError> {
        let at = self.pos;
        match self.peek() {
            Some('"') => Ok((at, Value::String(self.string()?))),
            Some('{') => {
                self.pos += 1;
                self.expression = at;
                self.skip_whitespace();
                if self.peek() == Some('}') {
                    return Err(self.error(
                        at,
                        "there is nothing between these braces",
                        "write a value between the braces, such as `pad={16}`",
                    ));
                }
                let value = self.expression(0)?;
                self.skip_whitespace();
                if self.peek() != Some('}') {
                    return Err(self.error(
                        at,
                        "this `{` is never closed with `}`",
                        "end the expression with `}`",
                    ));
                }
                self.pos += 1;
                Ok((at, value))
            }
            _ => Err(self.error(
                at,
                "an attribute with `=` needs a value",
                "write the value as `\"text\"` or `{expression}`",
            )),
        }
    }

    /// Reads a value of an expression: a number, text, `true`, `false`, a list
    /// such as `[1, 2]` or an object such as `{row: 8, column: 16}`.
    fn expression(&mut self, depth: usize) -> Result<Value, TagError> {
        self.skip_whitespace();
        let at = self.pos;
        if depth > MAX_DEPTH {
            return Err(self.error(
                at,
                "this expression is nested too deeply",
                format!("nest lists and objects at most {MAX_DEPTH} levels deep"),
            ));
        }
        match self.peek() {
            None => Err(self.unfinished()),
            Some('"') => Ok(Value::String(self.string()?)),
            Some('[') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_whitespace();
                    match self.peek() {
                        None => return Err(self.unfinished()),
                        Some(']') => {
                            self.pos += 1;
                            return Ok(Value::Array(items));
                        }
                        Some(_) => {}
                    }
                    items.push(self.expression(depth + 1)?);
                    self.skip_whitespace();
                    match self.peek() {
                        Some(',') => self.pos += 1,
                        Some(']') => {}
                        None => return Err(self.unfinished()),
                        Some(c) => {
                            return Err(self.error(
                                self.pos,
                                format!("expected `,` or `]`, but found `{c}`"),
                                "separate the items of a list with commas, and end it with `]`",
                            ));
                        }
                    }
                }
            }
            Some('{') => {
                self.pos += 1;
                let mut map = Map::new();
                loop {
                    self.skip_whitespace();
                    let key_pos = self.pos;
                    let key = match self.peek() {
                        None => return Err(self.unfinished()),
                        Some('}') => {
                            self.pos += 1;
                            return Ok(Value::Object(map));
                        }
                        Some('"') => self.string()?,
                        Some(c) if c.is_ascii_alphabetic() || c == '_' => self.word().to_string(),
                        Some(c) => {
                            return Err(self.error(
                                key_pos,
                                format!("expected a property name, but found `{c}`"),
                                "write the properties of an object as `name: value`, separated by commas",
                            ));
                        }
                    };
                    self.skip_whitespace();
                    if self.peek() != Some(':') {
                        if self.peek().is_none() {
                            return Err(self.unfinished());
                        }
                        return Err(self.error(
                            self.pos,
                            format!("expected `:` after `{key}`"),
                            format!("write `{key}: value`"),
                        ));
                    }
                    self.pos += 1;
                    let value = self.expression(depth + 1)?;
                    if map.insert(key.clone(), value).is_some() {
                        return Err(self.error(
                            key_pos,
                            format!("`{key}` is written twice"),
                            format!("write `{key}` once"),
                        ));
                    }
                    self.skip_whitespace();
                    match self.peek() {
                        Some(',') => self.pos += 1,
                        Some('}') => {}
                        None => return Err(self.unfinished()),
                        Some(c) => {
                            return Err(self.error(
                                self.pos,
                                format!("expected `,` or `}}`, but found `{c}`"),
                                "separate the properties of an object with commas, and end it with `}`",
                            ));
                        }
                    }
                }
            }
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            Some(c) if c.is_ascii_alphabetic() => match self.word() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                word => Err(self.error(
                    at,
                    format!("`{word}` is not a value"),
                    "write text in quotes, such as `\"col\"`, or a number, `true`, `false`, a list or an object",
                )),
            },
            Some(c) => Err(self.error(
                at,
                format!("unexpected `{c}` in an expression"),
                "write a number, text in quotes, `true`, `false`, a list or an object",
            )),
        }
    }

    /// Reads a number, such as `16`, `-0.5` or `1e-7`, as the exact double.
    fn number(&mut self) -> Result<Value, TagError> {
        let start = self.pos;
        let bytes = self.source.as_bytes();
        let digits = |mut at: usize| {
            let from = at;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            (at, at > from)
        };
        let mut end = start + usize::from(bytes.get(start) == Some(&b'-'));
        let (after, mut valid) = digits(end);
        end = after;
        if valid && bytes.get(end) == Some(&b'.') {
            (end, valid) = digits(end + 1);
        }
        if valid && matches!(bytes.get(end), Some(b'e' | b'E')) {
            let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
            (end, valid) = digits(end + 1 + sign);
        }
        self.pos = end.max(start + 1);
        let text = &self.source[start..self.pos];
        let number = text.parse::<f64>().ok().filter(|number| number.is_finite());
        match number {
            Some(number) if valid => Ok(number_value(number)),
            _ => Err(self.error(
                start,
                format!("`{text}` is not a number"),
                "write numbers as `16`, `-0.5` or `1e-7`",
            )),
        }
    }

    /// Reads text in double quotes, from its opening quote. A backslash
    /// writes the characters that cannot stand for themselves: `\"`, `\\`,
    /// `\n`, `\r`, `\t` and `\u{...}` for the character with that hexadecimal
    /// code point.
    fn string(&mut self) -> Result<String, TagError> {
        let start = self.pos;
        self.pos += 1;
        let mut text = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.error(
                    start,
                    "this text is never closed",
                    "end the text with a closing `\"`, and write a quote inside it as `\\\"`",
                ));
            };
            let at = self.pos;
            self.pos += c.len_utf8();
            match c {
                '"' => return Ok(text),
                '\\' => text.push(self.escape(start, at)?),
                other => text.push(other),
            }
        }
    }

    /// Reads the escape after a backslash at `at` in the text starting at
    /// `start`.
    fn escape(&mut self, start: usize, at: usize) -> Result<char, TagError> {
        let fix = "write `\\\\` for a backslash, `\\\"` for a quote, `\\n` for a new line, `\\r`, `\\t` or `\\u{...}` with a hexadecimal code point";
        let Some(c) = self.peek() else {
            return Err(self.error(start, "this text is never closed", fix));
        };
        self.pos += c.len_utf8();
        match c {
            '"' | '\\' => Ok(c),
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            'u' if self.peek() == Some('{') => {
                self.pos += 1;
                let rest = self.rest();
                let length = rest.find('}').unwrap_or(0);
                let code = u32::from_str_radix(&rest[..length], 16)
                    .ok()
                    .filter(|_| (1..=6).contains(&length))
                    .and_then(char::from_u32);
                match code {
                    Some(code) if rest[length..].starts_with('}') => {
                        self.pos += length + 1;
                        Ok(code)
                    }
                    _ => Err(self.error(
                        at,
                        "`\\u{...}` needs the hexadecimal code point of a character",
                        "write it like `\\u{1f6d2}`",
                    )),
                }
            }
            other => Err(self.error(at, format!("`\\{other}` is not an escape"), fix)),
        }
    }
}

/// A number as JSON: an integer when it is one, as designs write numbers.
fn number_value(number: f64) -> Value {
    // Integers below 2^53 convert to i64 and back without loss.
    if number.fract() == 0.0 && number.abs() < 9_007_199_254_740_992.0 {
        Value::from(number as i64)
    } else {
        Number::from_f64(number).map_or(Value::Null, Value::Number)
    }
}

/// The problem with a tag of `kind` inside `parent`, if there is one.
fn misplaced(kind: NodeKind, parent: Option<&Parent>) -> Option<Problem> {
    let parent = parent?;
    let tag = tag_of(kind).name;
    let parent_tag = tag_of(parent.kind).name;
    if !has_children(parent.kind) {
        return Some(problem(
            format!("`<{parent_tag}>` has no children, but `<{tag}>` is inside it"),
            format!("move `<{tag}>` into a `<Frame>`"),
        ));
    }
    match (parent.kind, kind) {
        (_, NodeKind::Component) => Some(problem(
            format!(
                "a `<Component>` sits only at the top of the design system doc, but this one is inside a `<{parent_tag}>`"
            ),
            "write `<Component>` tags at the top, one after the other, and place a component with `<Instance>`",
        )),
        (NodeKind::Page, NodeKind::Board) => None,
        (NodeKind::Page, _) => Some(problem(
            format!("pages hold only boards, but `<{tag}>` is inside a `<Page>`"),
            format!("put `<{tag}>` inside a `<Board>`"),
        )),
        (_, NodeKind::Board) => Some(problem(
            format!("boards sit only on pages, but this board is inside a `<{parent_tag}>`"),
            "use `<Frame>` for a box inside another box, and put each `<Board>` in a `<Page>`",
        )),
        (_, NodeKind::Page) => Some(problem(
            format!(
                "pages are never inside other nodes, but this page is inside a `<{parent_tag}>`"
            ),
            "write `<Page>` at the top",
        )),
        _ => None,
    }
}

fn needs(parser: &Parser<'_>, at: usize, tag: &str, attr: &str) -> TagError {
    parser.error(
        at,
        format!("`<{tag}>` needs `{attr}`"),
        format!("add `{attr}` to the tag; docs/formats/design-tags.md shows how it is written"),
    )
}

/// Prints `node` and its subtree in the tag format, one tag per node, with
/// each node's `id`.
pub fn print(node: &Node) -> String {
    print_with(node, None)
}

/// The revision of a node, for [`print_with`].
pub type Revision<'a> = &'a dyn Fn(&NodeId) -> Option<u64>;

/// Prints `node` and its subtree like [`print`]. With `revision`, a tag also
/// carries `rev={n}` after its `id`, the revision that `revision` gives the
/// node: what an agent reads before it edits a doc.
pub fn print_with(node: &Node, revision: Option<Revision>) -> String {
    let json = serde_json::to_value(node).expect("a node serializes to JSON");
    let mut out = String::new();
    write_node(&json, 0, revision, &mut out);
    out.truncate(out.trim_end().len());
    out
}

fn write_node(node: &Value, depth: usize, revision: Option<Revision>, out: &mut String) {
    let Some(map) = node.as_object() else {
        return;
    };
    let type_name = map.get("type").and_then(Value::as_str).unwrap_or_default();
    let Some(index) = KINDS.iter().position(|kind| kind.as_str() == type_name) else {
        return;
    };
    let (tag, kind) = (&TAGS[index], KINDS[index]);
    let indent = "  ".repeat(depth);
    out.push_str(&indent);
    out.push('<');
    out.push_str(tag.name);
    for attr in tag.attributes {
        let text = match *attr {
            "id" | "name" => map.get(*attr).map(|value| written(attr, value)),
            _ => attribute(spec_of(kind, attr), map, kind),
        };
        if let Some(text) = text {
            out.push(' ');
            out.push_str(&text);
        }
        if *attr == "id"
            && let Some(rev) = revision.and_then(|revision| {
                let id = map.get("id")?.as_str()?.parse::<NodeId>().ok()?;
                revision(&id)
            })
        {
            out.push_str(&format!(" rev={{{rev}}}"));
        }
    }
    let children = map
        .get("children")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if children.is_empty() {
        out.push_str("/>\n");
        return;
    }
    out.push_str(">\n");
    for child in children {
        write_node(child, depth + 1, revision, out);
    }
    out.push_str(&indent);
    out.push_str("</");
    out.push_str(tag.name);
    out.push_str(">\n");
}

/// Changes attributes of a node, named as the attributes of its tag, with
/// their values in JSON, such as `{"content": "Hej", "fontSize": 18}`; a
/// `null` takes the attribute away. The node's own tag is written with the
/// changes and read again, so every rule of the tag format holds for the result
/// and the node keeps its ID and its children. `host` is the node it sits in,
/// `None` for a page. With no `properties` it only checks that the node fits
/// there.
///
/// The node is left as it was when the changes are refused; the error says
/// which attribute and how to fix it.
pub fn set_properties(
    node: &mut Node,
    host: Option<&Host>,
    properties: &Map<String, Value>,
) -> Result<(), String> {
    let kind = node.kind();
    let tag = tag_of(kind);
    for name in properties.keys() {
        if name == "id" {
            return Err("`id` cannot be set: a node keeps its ID".to_string());
        }
        if !tag.attributes.contains(&name.as_str()) {
            return Err(format!(
                "`{name}` is not an attribute of `<{}>`: it takes {}",
                tag.name,
                list_of(
                    &tag.attributes
                        .iter()
                        .map(|attribute| format!("`{attribute}`"))
                        .collect::<Vec<_>>()
                )
            ));
        }
    }
    // The node's own tag, without the nodes inside it.
    let children = node.take_children();
    let json = serde_json::to_value(&*node).expect("a node serializes to JSON");
    let map = json.as_object().expect("a node is a JSON object");
    let mut source = format!("<{}", tag.name);
    for attr in tag.attributes {
        let text = match properties.get(*attr) {
            Some(Value::Null) => None,
            Some(value) => Some(written(attr, value)),
            None if matches!(*attr, "id" | "name") => {
                map.get(*attr).map(|value| written(attr, value))
            }
            None => attribute(spec_of(kind, attr), map, kind),
        };
        if let Some(text) = text {
            source.push(' ');
            source.push_str(&text);
        }
    }
    source.push_str("/>");
    let parsed = parse_in(&source, host)
        .map_err(|error| format!("{}: {}", error.message(), error.fix()))
        .and_then(|parsed| parsed.try_assign_ids(|| unreachable!("the node keeps its ID")));
    match parsed {
        Ok(mut changed) => {
            changed.set_children(children);
            *node = changed;
            Ok(())
        }
        Err(message) => {
            node.set_children(children);
            Err(message)
        }
    }
}

/// The attribute that writes the property `spec` stands for, or `None` when
/// the node leaves it out or it has the value that leaving it out means.
fn attribute(spec: &Spec, map: &Map<String, Value>, kind: NodeKind) -> Option<String> {
    let layout = map.get("layout").and_then(Value::as_object);
    let value = match spec.home {
        Home::Node => map.get(spec.key)?,
        Home::Layout => layout?.get(spec.key)?,
    };
    if spec.attr == "layout" {
        return layout_attribute(layout?).map(|mode| written("layout", &Value::from(mode)));
    }
    let shown = match spec.shape {
        Shape::Flag if spec.attr == "visible" => {
            if value == &Value::Bool(true) {
                return None;
            }
            value.clone()
        }
        Shape::Size => {
            let hugs = value == "hug" && kind != NodeKind::Instance;
            if hugs {
                return None;
            }
            match value.as_object().and_then(|object| object.get("fixed")) {
                Some(fixed) => fixed.clone(),
                None => value.clone(),
            }
        }
        Shape::Keyword(table) => {
            let word = value.as_str()?;
            if spec.attr == "fit" && word == "cover" {
                return None;
            }
            match table.iter().find(|(_, keyword)| *keyword == word) {
                Some((written, _)) => Value::from(*written),
                None => value.clone(),
            }
        }
        Shape::Sides(sides) => match value.as_object().and_then(|object| uniform(object, sides)) {
            Some(same) => same.clone(),
            None => value.clone(),
        },
        Shape::Fills => match value.as_array().map(Vec::as_slice) {
            Some([fill]) => match solid_color(fill) {
                Some(color) => color.clone(),
                None => value.clone(),
            },
            _ => value.clone(),
        },
        _ => value.clone(),
    };
    Some(written(spec.attr, &shown))
}

/// The `layout` attribute that a flex or grid layout needs, because no other
/// attribute says what mode it is.
fn layout_attribute(layout: &Map<String, Value>) -> Option<&'static str> {
    let sets = |keys: &[&str]| keys.iter().any(|key| layout.contains_key(*key));
    match layout.get("mode")?.as_str()? {
        "flex" if !sets(&["direction", "wrap", "alignItems", "justifyContent"]) => Some("flex"),
        "grid" if !sets(&["columns", "rows"]) => Some("grid"),
        _ => None,
    }
}

/// The value that every side of `object` has, when it sets them all alike.
fn uniform(object: &Map<String, Value>, sides: Sides) -> Option<&Value> {
    let mut values = sides.fields().iter().map(|field| object.get(*field));
    let first = values.next()??;
    values
        .all(|value| value == Some(first))
        .then_some(first)
        .filter(|value| value.is_number() || value.is_string())
}

/// The color of a fill that is one solid color.
fn solid_color(fill: &Value) -> Option<&Value> {
    let fill = fill.as_object()?;
    (fill.len() == 2 && fill.get("type")? == "solid")
        .then(|| fill.get("color"))?
        .filter(|color| color.is_string())
}

/// An attribute: `name="text"`, `name` for `true`, and `name={expression}`
/// for everything else.
fn written(name: &str, value: &Value) -> String {
    match value {
        Value::String(text) => {
            let mut out = format!("{name}=");
            quote(text, &mut out);
            out
        }
        Value::Bool(true) => name.to_string(),
        other => {
            let mut out = format!("{name}={{");
            expression(other, &mut out);
            out.push('}');
            out
        }
    }
}

fn expression(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => quote(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                expression(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (index, (key, item)) in map.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                let is_word = key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
                if is_word {
                    out.push_str(key);
                } else {
                    quote(key, out);
                }
                out.push_str(": ");
                expression(item, out);
            }
            out.push('}');
        }
    }
}

/// Writes `text` in double quotes, with a backslash before the characters
/// that cannot stand for themselves.
fn quote(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    //! `print_with` writes `rev={n}` after the `id` of every tag, and a tag
    //! that sets `rev` is not part of the format.

    use super::{Revision, parse, print_with};
    use crate::design::{Node, NodeId};

    const TAGS: &str = r#"<Board id="brd00001" name="Cart" x={0} y={0} width={360} height={640} dir="col">
  <Text id="txt00001" name="Total" content="Summa 12 kr" fontFamily="Inter" fontSize={16}/>
</Board>"#;

    fn board() -> Node {
        let root = parse(TAGS).expect("the tags parse");
        root.assign_ids(|| unreachable!("every node has an ID"))
    }

    #[test]
    fn print_with_writes_the_rev_after_the_id_of_every_tag() {
        let revision = |id: &NodeId| match id.to_string().as_str() {
            "brd00001" => Some(3),
            "txt00001" => Some(2),
            _ => None,
        };
        let revision: Revision = &revision;
        let printed = print_with(&board(), Some(revision));
        assert_eq!(
            printed,
            TAGS.replace(r#"id="brd00001""#, r#"id="brd00001" rev={3}"#)
                .replace(r#"id="txt00001""#, r#"id="txt00001" rev={2}"#)
        );
    }

    #[test]
    fn a_tag_that_sets_rev_is_refused() {
        let tags_with_rev = TAGS.replace(r#"id="brd00001""#, r#"id="brd00001" rev={3}"#);
        let error = parse(&tags_with_rev).expect_err("rev is only read, never written");
        assert_eq!(
            (error.line(), error.column()),
            (1, 22),
            "the error points at `rev`: {error}"
        );
        assert!(
            error.message().contains("rev"),
            "the message names `rev`: {}",
            error.message()
        );
    }
}
