//! Components and instances (US-029, Q27, FR-30): the properties a component
//! defines, the layers that take them (`bind`), and what an instance stores:
//! its property values and its overrides.

use super::node::NodeKind;
use super::style::Fill;
use super::{InvalidNodeId, NodeId};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The four kinds of component property, as written in `type`.
pub const PROP_KINDS: &[&str] = &["variant", "boolean", "text", "slot"];

/// The fields of an override: what an instance may change on a node of its
/// component.
pub const OVERRIDE_FIELDS: &[&str] = &["content", "fills", "visible", "component"];

/// The fields of a bind: what a layer of a component takes from a property.
pub const BIND_FIELDS: &[&str] = &["visible", "content", "component"];

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

fn quoted(words: &[&str]) -> String {
    let words: Vec<String> = words.iter().map(|word| format!("`{word}`")).collect();
    match words.as_slice() {
        [first @ .., last] if !first.is_empty() => format!("{} or {last}", first.join(", ")),
        _ => words.join(""),
    }
}

/// A property that a component defines.
#[derive(Clone, Debug, PartialEq)]
pub enum PropDef {
    /// One of a fixed set of values.
    Variant {
        values: Vec<String>,
        default: String,
    },
    /// Shows or hides a layer.
    Boolean { default: bool },
    /// Replaces a label.
    Text { default: String },
    /// Swaps in another component: the default is a component's node ID.
    Slot { default: NodeId },
}

impl PropDef {
    /// The kind as written in `type`.
    pub fn kind(&self) -> &'static str {
        match self {
            PropDef::Variant { .. } => "variant",
            PropDef::Boolean { .. } => "boolean",
            PropDef::Text { .. } => "text",
            PropDef::Slot { .. } => "slot",
        }
    }

    fn from_value(name: &str, value: &Value) -> Result<Self, String> {
        let Some(map) = value.as_object() else {
            return Err(format!(
                "`{name}` is {}, but a property is an object with a `type` and a `default`",
                describe(value)
            ));
        };
        let kind = match map.get("type") {
            Some(Value::String(kind)) => kind.as_str(),
            Some(other) => {
                return Err(format!(
                    "the `type` of `{name}` is {}: write one of {}",
                    describe(other),
                    quoted(PROP_KINDS)
                ));
            }
            None => {
                return Err(format!(
                    "`{name}` needs a `type`: write one of {}",
                    quoted(PROP_KINDS)
                ));
            }
        };
        if !PROP_KINDS.contains(&kind) {
            return Err(format!(
                "`{kind}` is not a kind of property, but the type of `{name}`: use {}",
                quoted(PROP_KINDS)
            ));
        }
        let fields: &[&str] = if kind == "variant" {
            &["type", "values", "default"]
        } else {
            &["type", "default"]
        };
        if let Some(extra) = map.keys().find(|key| !fields.contains(&key.as_str())) {
            return Err(format!(
                "`{extra}` is not a field of the {kind} property `{name}`: it has {}",
                quoted(fields)
            ));
        }
        let default = map.get("default").ok_or_else(|| {
            format!("the {kind} property `{name}` needs a `default`: write its value")
        })?;
        match kind {
            "variant" => {
                let values = match map.get("values") {
                    Some(Value::Array(items)) if !items.is_empty() => items
                        .iter()
                        .map(|item| match item {
                            Value::String(text) => Ok(text.clone()),
                            other => Err(format!(
                                "a value of the variant `{name}` is {}: write text, such as `values: [\"primary\", \"secondary\"]`",
                                describe(other)
                            )),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => {
                        return Err(format!(
                            "the variant `{name}` needs `values`: write a list of text, such as `values: [\"primary\", \"secondary\"]`"
                        ));
                    }
                };
                for (index, text) in values.iter().enumerate() {
                    if values[..index].contains(text) {
                        return Err(format!(
                            "`{text}` is a value of the variant `{name}` twice: write each value once"
                        ));
                    }
                }
                match default {
                    Value::String(text) if values.contains(text) => Ok(PropDef::Variant {
                        values,
                        default: text.clone(),
                    }),
                    Value::String(text) => Err(format!(
                        "`{text}` is not a value of the variant `{name}`, so it cannot be its default: use one of {}",
                        quoted(&values.iter().map(String::as_str).collect::<Vec<_>>())
                    )),
                    other => Err(format!(
                        "the default of the variant `{name}` is {}: use one of its values",
                        describe(other)
                    )),
                }
            }
            "boolean" => match default {
                Value::Bool(flag) => Ok(PropDef::Boolean { default: *flag }),
                other => Err(format!(
                    "the default of the boolean `{name}` is {}: write `true` or `false`",
                    describe(other)
                )),
            },
            "text" => match default {
                Value::String(text) => Ok(PropDef::Text {
                    default: text.clone(),
                }),
                other => Err(format!(
                    "the default of the text property `{name}` is {}: write text",
                    describe(other)
                )),
            },
            _ => match default {
                Value::String(text) => text
                    .parse::<NodeId>()
                    .map(|default| PropDef::Slot { default })
                    .map_err(|InvalidNodeId { .. }| {
                        format!(
                            "`{text}` is not the ID of a component, but the default of the slot `{name}`: write the node ID of a component, such as `k3f9x2qa`"
                        )
                    }),
                other => Err(format!(
                    "the default of the slot `{name}` is {}: write the node ID of a component",
                    describe(other)
                )),
            },
        }
    }
}

impl Serialize for PropDef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("type", self.kind())?;
        match self {
            PropDef::Variant { values, default } => {
                map.serialize_entry("values", values)?;
                map.serialize_entry("default", default)?;
            }
            PropDef::Boolean { default } => map.serialize_entry("default", default)?,
            PropDef::Text { default } => map.serialize_entry("default", default)?,
            PropDef::Slot { default } => map.serialize_entry("default", default)?,
        }
        map.end()
    }
}

/// The properties of a component, by name.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PropDefs(BTreeMap<String, PropDef>);

impl PropDefs {
    /// Reads the `props` of a component from its JSON form.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let Some(map) = value.as_object() else {
            return Err(format!(
                "`props` is {}, but it is an object of properties, such as `{{label: {{type: \"text\", default: \"Buy\"}}}}`",
                describe(value)
            ));
        };
        let mut props = BTreeMap::new();
        for (name, def) in map {
            if name.is_empty() {
                return Err("a property needs a name".to_string());
            }
            props.insert(name.clone(), PropDef::from_value(name, def)?);
        }
        Ok(Self(props))
    }

    pub fn get(&self, name: &str) -> Option<&PropDef> {
        self.0.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &PropDef)> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for PropDefs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

/// The value an instance gives a property: text for a variant, a text
/// property or a slot (a component's node ID), `true` or `false` for a
/// boolean.
#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    Text(String),
    Flag(bool),
}

impl Serialize for PropValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PropValue::Text(text) => serializer.serialize_str(text),
            PropValue::Flag(flag) => serializer.serialize_bool(*flag),
        }
    }
}

/// The property values of an instance, by property name.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PropValues(BTreeMap<String, PropValue>);

impl PropValues {
    /// Reads the `props` of an instance from its JSON form.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let Some(map) = value.as_object() else {
            return Err(format!(
                "`props` is {}, but it is an object of property values, such as `{{label: \"Pay\"}}`",
                describe(value)
            ));
        };
        let mut values = BTreeMap::new();
        for (name, value) in map {
            let value = match value {
                Value::String(text) => PropValue::Text(text.clone()),
                Value::Bool(flag) => PropValue::Flag(*flag),
                other => {
                    return Err(format!(
                        "`{name}` is {}, but the value of a property is text, `true` or `false`",
                        describe(other)
                    ));
                }
            };
            values.insert(name.clone(), value);
        }
        Ok(Self(values))
    }

    pub fn get(&self, name: &str) -> Option<&PropValue> {
        self.0.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &PropValue)> {
        self.0.iter()
    }
}

impl<'de> Deserialize<'de> for PropValues {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

/// What an instance changes on one node of its component.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Override {
    /// The text of a text node.
    pub content: Option<String>,
    pub fills: Option<Vec<Fill>>,
    pub visible: Option<bool>,
    /// The contents of a slot: the component an instance node places.
    pub component: Option<NodeId>,
}

impl Override {
    fn from_value(key: &NodeId, value: &Value) -> Result<Self, String> {
        let Some(map) = value.as_object() else {
            return Err(format!(
                "the override of `{key}` is {}, but it is an object, such as `{{content: \"Pay now\"}}`",
                describe(value)
            ));
        };
        let mut set = Override::default();
        for (field, value) in map {
            let wrong = |wanted: &str| {
                format!(
                    "`{field}` of the override of `{key}` is {}, but it is {wanted}",
                    describe(value)
                )
            };
            match field.as_str() {
                "content" => match value {
                    Value::String(text) => set.content = Some(text.clone()),
                    _ => return Err(wrong("text")),
                },
                "fills" => {
                    let Value::Array(fills) = value else {
                        return Err(wrong("a list of fills"));
                    };
                    let mut parsed = Vec::with_capacity(fills.len());
                    for fill in fills {
                        parsed.push(
                            serde_json::from_value::<Fill>(fill.clone())
                                .map_err(|error| error.to_string())?,
                        );
                    }
                    set.fills = Some(parsed);
                }
                "visible" => match value {
                    Value::Bool(flag) => set.visible = Some(*flag),
                    _ => return Err(wrong("`true` or `false`")),
                },
                "component" => match value {
                    Value::String(text) => {
                        set.component =
                            Some(text.parse().map_err(|e: InvalidNodeId| e.to_string())?)
                    }
                    _ => return Err(wrong("the node ID of a component")),
                },
                other => {
                    return Err(format!(
                        "`{other}` cannot be overridden: an override changes {}",
                        quoted(OVERRIDE_FIELDS)
                    ));
                }
            }
        }
        Ok(set)
    }
}

impl Serialize for Override {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if let Some(content) = &self.content {
            map.serialize_entry("content", content)?;
        }
        if let Some(fills) = &self.fills {
            map.serialize_entry("fills", fills)?;
        }
        if let Some(visible) = &self.visible {
            map.serialize_entry("visible", visible)?;
        }
        if let Some(component) = &self.component {
            map.serialize_entry("component", component)?;
        }
        map.end()
    }
}

/// The overrides of an instance, by the ID of the component node they change.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Overrides(BTreeMap<NodeId, Override>);

impl Overrides {
    /// Reads the `overrides` of an instance from its JSON form.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let Some(map) = value.as_object() else {
            return Err(format!(
                "`overrides` is {}, but it is an object of overrides by node ID, such as `{{k3f9x2qa: {{content: \"Pay now\"}}}}`",
                describe(value)
            ));
        };
        let mut overrides = BTreeMap::new();
        for (key, value) in map {
            let id: NodeId = key.parse().map_err(|e: InvalidNodeId| e.to_string())?;
            overrides.insert(id, Override::from_value(&id, value)?);
        }
        Ok(Self(overrides))
    }

    pub fn get(&self, node: &NodeId) -> Option<&Override> {
        self.0.get(node)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&NodeId, &Override)> {
        self.0.iter()
    }
}

impl<'de> Deserialize<'de> for Overrides {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

/// What a layer of a component takes from the component's properties: a
/// boolean property shows or hides it, a text property replaces the text of a
/// text layer, and a slot property swaps the component of an instance layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bind {
    pub visible: Option<String>,
    pub content: Option<String>,
    pub component: Option<String>,
}

impl Bind {
    /// Reads the `bind` of a layer from its JSON form.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let Some(map) = value.as_object() else {
            return Err(format!(
                "`bind` is {}, but it is an object, such as `{{visible: \"showIcon\"}}`",
                describe(value)
            ));
        };
        let mut bind = Bind::default();
        for (field, value) in map {
            let Value::String(property) = value else {
                return Err(format!(
                    "`{field}` of `bind` is {}, but it names a property of the component, as text",
                    describe(value)
                ));
            };
            let slot = match field.as_str() {
                "visible" => &mut bind.visible,
                "content" => &mut bind.content,
                "component" => &mut bind.component,
                other => {
                    return Err(format!(
                        "`{other}` is not something a layer binds: bind {}",
                        quoted(BIND_FIELDS)
                    ));
                }
            };
            *slot = Some(property.clone());
        }
        if bind == Bind::default() {
            return Err(format!(
                "`bind` binds nothing: bind {}",
                quoted(BIND_FIELDS)
            ));
        }
        Ok(bind)
    }

    /// The bound fields with the properties they take, in the order of
    /// [`BIND_FIELDS`].
    pub fn fields(&self) -> Vec<(&'static str, &str)> {
        [
            ("visible", self.visible.as_deref()),
            ("content", self.content.as_deref()),
            ("component", self.component.as_deref()),
        ]
        .into_iter()
        .filter_map(|(field, property)| property.map(|property| (field, property)))
        .collect()
    }

    /// What is wrong with the bind of a layer of `kind` in a component with
    /// `props`, if anything: a property that the component does not define or
    /// that is of the wrong kind, or a field that the layer's type cannot take.
    pub fn problem(&self, kind: NodeKind, props: &PropDefs) -> Option<String> {
        for (field, name) in self.fields() {
            let (layer, needs) = match field {
                "visible" => (None, "boolean"),
                "content" => (Some(NodeKind::Text), "text"),
                _ => (Some(NodeKind::Instance), "slot"),
            };
            if let Some(layer) = layer.filter(|layer| *layer != kind) {
                return Some(format!(
                    "`{field}` binds a {layer} layer, but this layer is a {kind}"
                ));
            }
            match props.get(name) {
                None => {
                    let defined: Vec<&str> = props.iter().map(|(name, _)| name.as_str()).collect();
                    let known = if defined.is_empty() {
                        "it defines no properties".to_string()
                    } else {
                        format!("it defines {}", quoted(&defined))
                    };
                    return Some(format!(
                        "`{name}` is not a property of this component, but `{field}` is bound to it: {known}"
                    ));
                }
                Some(def) if def.kind() != needs => {
                    return Some(format!(
                        "`{name}` is a {} property, but `{field}` needs a {needs} property",
                        def.kind()
                    ));
                }
                Some(_) => {}
            }
        }
        None
    }

    fn to_map(&self) -> Map<String, Value> {
        self.fields()
            .into_iter()
            .map(|(field, property)| (field.to_string(), Value::from(property)))
            .collect()
    }
}

impl Serialize for Bind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_map().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Bind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}
