//! Prototype interactions: what a layer does when the person using a
//! prototype touches it. A layer lists its interactions, each a trigger, the
//! actions it runs and the transition they play with.
//!
//! The model and its JSON names are FigCraft's (`crates/doc/src/proto.rs`,
//! MIT OR Apache-2.0; see `NOTICE`). Interactions are plain data here: the
//! destinations of actions are checked where they are written, not by this
//! module; the variables and expressions they name are checked by
//! [`Action::check_variables`].
//!
//! An interaction leaves out every field that holds its default, in JSON and
//! in the tag format: trigger `onClick`, transition `instant`, `durationMs`
//! 300, easing `easeOut`, `delayMs` 0, and the defaults of each action's
//! fields.

mod expr;

pub use expr::{Expr, ExprError};

use super::devices::{self, CUSTOM, NONE};
use super::id::NodeId;
use super::json::quoted;
use super::value::Color;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value, json};
use std::str::FromStr;

const TRIGGERS: &[&str] = &[
    "onClick",
    "onHover",
    "onPress",
    "onDrag",
    "mouseEnter",
    "mouseLeave",
    "mouseDown",
    "mouseUp",
    "afterDelay",
    "keyPress",
];

const ACTIONS: &[&str] = &[
    "navigate",
    "back",
    "scrollTo",
    "openOverlay",
    "swapOverlay",
    "closeOverlay",
    "openUrl",
    "changeTo",
    "setVariable",
    "setVariableMode",
    "conditional",
];

const TRANSITIONS: &[&str] = &[
    "instant",
    "dissolve",
    "smartAnimate",
    "moveIn",
    "moveOut",
    "push",
    "slideIn",
    "slideOut",
];

const DIRECTIONS: &[&str] = &["left", "right", "top", "bottom"];

const EASINGS: &[&str] = &[
    "linear",
    "easeIn",
    "easeOut",
    "easeInOut",
    "easeInBack",
    "easeOutBack",
    "easeInOutBack",
    "gentle",
    "quick",
    "bouncy",
    "slow",
    "customBezier",
    "customSpring",
];

const POSITIONS: &[&str] = &[
    "center",
    "topLeft",
    "topCenter",
    "topRight",
    "bottomLeft",
    "bottomCenter",
    "bottomRight",
    "manual",
];

const ANCHORS: &[&str] = &["screen", "triggerInside", "triggerOutside"];

const INTERACTION_FIELDS: &[&str] = &[
    "trigger",
    "actions",
    "transition",
    "durationMs",
    "easing",
    "delayMs",
];

const DEFAULT_DURATION_MS: u32 = 300;

keywords! {
    /// How a board or frame scrolls its content.
    #[derive(Default)]
    pub enum Overflow {
        #[default]
        None = "none",
        Horizontal = "horizontal",
        Vertical = "vertical",
        Both = "both",
    }
}

keywords! {
    /// Whether a layer scrolls with its board, stays where it is, or sticks
    /// to the edge it scrolls to.
    #[derive(Default)]
    pub enum ScrollPosition {
        #[default]
        Scrolls = "scrolls",
        Fixed = "fixed",
        Sticky = "sticky",
    }
}

/// What starts an interaction.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Trigger {
    #[default]
    OnClick,
    OnHover,
    OnPress,
    OnDrag,
    MouseEnter,
    MouseLeave,
    MouseDown,
    MouseUp,
    /// After `ms` milliseconds on the board.
    AfterDelay {
        ms: u32,
    },
    /// A key, a combination such as `Shift+K`, or a controller button.
    KeyPress {
        key: String,
    },
}

/// The side a transition moves from or to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

/// How the board changes when the actions run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Transition {
    #[default]
    Instant,
    Dissolve,
    SmartAnimate,
    MoveIn {
        direction: Direction,
    },
    MoveOut {
        direction: Direction,
    },
    Push {
        direction: Direction,
    },
    SlideIn {
        direction: Direction,
    },
    SlideOut {
        direction: Direction,
    },
}

/// How a transition moves between its start and end.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Easing {
    Linear,
    EaseIn,
    #[default]
    EaseOut,
    EaseInOut,
    EaseInBack,
    EaseOutBack,
    EaseInOutBack,
    Gentle,
    Quick,
    Bouncy,
    Slow,
    CustomBezier {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
    CustomSpring {
        mass: f64,
        stiffness: f64,
        damping: f64,
    },
}

impl Easing {
    const PLAIN: [(&'static str, Easing); 11] = [
        ("linear", Easing::Linear),
        ("easeIn", Easing::EaseIn),
        ("easeOut", Easing::EaseOut),
        ("easeInOut", Easing::EaseInOut),
        ("easeInBack", Easing::EaseInBack),
        ("easeOutBack", Easing::EaseOutBack),
        ("easeInOutBack", Easing::EaseInOutBack),
        ("gentle", Easing::Gentle),
        ("quick", Easing::Quick),
        ("bouncy", Easing::Bouncy),
        ("slow", Easing::Slow),
    ];
}

impl Serialize for Easing {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match *self {
            Easing::CustomBezier { x1, y1, x2, y2 } => {
                json!({"type": "customBezier", "x1": x1, "y1": y1, "x2": x2, "y2": y2})
                    .serialize(serializer)
            }
            Easing::CustomSpring {
                mass,
                stiffness,
                damping,
            } => json!({"type": "customSpring", "mass": mass, "stiffness": stiffness, "damping": damping})
                .serialize(serializer),
            plain => {
                let (name, _) = Easing::PLAIN
                    .iter()
                    .find(|(_, easing)| *easing == plain)
                    .expect("every plain easing is listed");
                serializer.serialize_str(name)
            }
        }
    }
}

/// Where an overlay opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayPosition {
    #[default]
    Center,
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    /// At `offset` from the layer that opened it.
    Manual,
}

/// What an overlay's position is relative to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayAnchor {
    #[default]
    Screen,
    TriggerInside,
    TriggerOutside,
}

/// One branch of a `conditional` action: its actions run when its condition
/// holds, and a block without a condition always does.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConditionBlock {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Value>,
    pub actions: Vec<Action>,
}

/// What an interaction does. A `dest` is the ID of a node; this module does
/// not check that it exists.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    #[serde(rename_all = "camelCase")]
    Navigate {
        dest: String,
        #[serde(skip_serializing_if = "is_false")]
        preserve_scroll: bool,
    },
    Back,
    ScrollTo {
        dest: String,
        #[serde(skip_serializing_if = "is_origin")]
        offset: [f64; 2],
    },
    #[serde(rename_all = "camelCase")]
    OpenOverlay {
        dest: String,
        #[serde(skip_serializing_if = "is_center")]
        position: OverlayPosition,
        #[serde(skip_serializing_if = "is_true")]
        close_on_outside: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        background: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        offset: Option<[f64; 2]>,
        #[serde(skip_serializing_if = "is_screen")]
        anchor: OverlayAnchor,
    },
    SwapOverlay {
        dest: String,
    },
    CloseOverlay,
    #[serde(rename_all = "camelCase")]
    OpenUrl {
        url: String,
        #[serde(skip_serializing_if = "is_true")]
        new_tab: bool,
    },
    ChangeTo {
        dest: String,
    },
    /// Sets a variable to a `value` or to what the expression `expr` gives.
    SetVariable {
        variable: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        expr: Option<Value>,
    },
    SetVariableMode {
        collection: String,
        mode: String,
    },
    Conditional {
        blocks: Vec<ConditionBlock>,
    },
}

impl Action {
    /// The node an action points at, `None` for an action with no `dest` or
    /// one that is not a node ID.
    pub fn dest(&self) -> Option<NodeId> {
        self.dest_text()?.parse().ok()
    }

    /// The `dest` as written.
    pub fn dest_text(&self) -> Option<&str> {
        match self {
            Action::Navigate { dest, .. }
            | Action::ScrollTo { dest, .. }
            | Action::OpenOverlay { dest, .. }
            | Action::SwapOverlay { dest }
            | Action::ChangeTo { dest } => Some(dest),
            _ => None,
        }
    }
}

fn is_false(flag: &bool) -> bool {
    !*flag
}

fn is_true(flag: &bool) -> bool {
    *flag
}

fn is_origin(offset: &[f64; 2]) -> bool {
    *offset == [0.0, 0.0]
}

fn is_center(position: &OverlayPosition) -> bool {
    *position == OverlayPosition::Center
}

fn is_screen(anchor: &OverlayAnchor) -> bool {
    *anchor == OverlayAnchor::Screen
}

fn is_default_trigger(trigger: &Trigger) -> bool {
    *trigger == Trigger::OnClick
}

fn is_default_transition(transition: &Transition) -> bool {
    *transition == Transition::Instant
}

fn is_default_duration(ms: &u32) -> bool {
    *ms == DEFAULT_DURATION_MS
}

fn is_default_easing(easing: &Easing) -> bool {
    *easing == Easing::EaseOut
}

fn is_zero(ms: &u32) -> bool {
    *ms == 0
}

/// A trigger, the actions it runs and how the change plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Interaction {
    pub trigger: Trigger,
    pub actions: Vec<Action>,
    pub transition: Transition,
    pub duration_ms: u32,
    pub easing: Easing,
    pub delay_ms: u32,
}

impl Default for Interaction {
    fn default() -> Self {
        Interaction {
            trigger: Trigger::default(),
            actions: Vec::new(),
            transition: Transition::default(),
            duration_ms: DEFAULT_DURATION_MS,
            easing: Easing::default(),
            delay_ms: 0,
        }
    }
}

impl Serialize for Interaction {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if !is_default_trigger(&self.trigger) {
            map.serialize_entry("trigger", &self.trigger)?;
        }
        map.serialize_entry("actions", &self.actions)?;
        if !is_default_transition(&self.transition) {
            map.serialize_entry("transition", &self.transition)?;
        }
        if !is_default_duration(&self.duration_ms) {
            map.serialize_entry("durationMs", &self.duration_ms)?;
        }
        if !is_default_easing(&self.easing) {
            map.serialize_entry("easing", &self.easing)?;
        }
        if !is_zero(&self.delay_ms) {
            map.serialize_entry("delayMs", &self.delay_ms)?;
        }
        map.end()
    }
}

impl Interaction {
    /// Reads the `interactions` of a layer from its JSON form.
    pub fn list_from_value(value: &Value) -> Result<Vec<Interaction>, String> {
        let Some(items) = value.as_array() else {
            return Err(format!(
                "`interactions` is {}: write a list, such as `interactions={{[{{actions: [{{type: \"navigate\", dest: \"k3f9x2qa\"}}]}}]}}`",
                describe(value)
            ));
        };
        items.iter().map(Interaction::from_value).collect()
    }

    /// Reads one interaction from its JSON form.
    pub fn from_value(value: &Value) -> Result<Interaction, String> {
        let map = object(value, "an interaction")?;
        fields(map, INTERACTION_FIELDS, "an interaction")?;
        let actions = match map.get("actions") {
            Some(actions) => actions_from(actions)?,
            None => {
                return Err(
                    "`actions` is required for an interaction: write a list of actions, such as `actions: [{type: \"back\"}]`"
                        .to_string(),
                );
            }
        };
        Ok(Interaction {
            trigger: map
                .get("trigger")
                .map(trigger_from)
                .transpose()?
                .unwrap_or_default(),
            actions,
            transition: map
                .get("transition")
                .map(transition_from)
                .transpose()?
                .unwrap_or_default(),
            duration_ms: match map.get("durationMs") {
                Some(ms) => milliseconds("durationMs", ms)?,
                None => DEFAULT_DURATION_MS,
            },
            easing: map
                .get("easing")
                .map(easing_from)
                .transpose()?
                .unwrap_or_default(),
            delay_ms: match map.get("delayMs") {
                Some(ms) => milliseconds("delayMs", ms)?,
                None => 0,
            },
        })
    }
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

fn names(names: &[&str]) -> String {
    names.join(", ")
}

fn object<'a>(value: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    value.as_object().ok_or_else(|| {
        format!(
            "`{}` is {}, but {what} is an object: write it between braces",
            shown(value),
            describe(value)
        )
    })
}

fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Checks that `map` sets only `allowed` fields.
fn fields(map: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<(), String> {
    match map.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(extra) => Err(format!(
            "`{extra}` is not a field of {what}: write one of {}",
            names(allowed)
        )),
        None => Ok(()),
    }
}

/// The `type` of `map`, which must be one of `known`.
fn kind<'a>(map: &'a Map<String, Value>, known: &[&str], what: &str) -> Result<&'a str, String> {
    match map.get("type") {
        Some(Value::String(kind)) if known.contains(&kind.as_str()) => Ok(kind),
        Some(Value::String(kind)) => Err(format!(
            "`{kind}` is not {what}: write one of {}",
            names(known)
        )),
        Some(other) => Err(format!(
            "`type` is {}, but it names {what}: write one of {}",
            describe(other),
            names(known)
        )),
        None => Err(format!(
            "`type` is required for {what}: write one of {}",
            names(known)
        )),
    }
}

fn required<'a>(
    map: &'a Map<String, Value>,
    field: &str,
    kind: &str,
    hint: &str,
) -> Result<&'a Value, String> {
    map.get(field)
        .ok_or_else(|| format!("`{field}` is required for {kind}: {hint}"))
}

fn text(map: &Map<String, Value>, field: &str, kind: &str, hint: &str) -> Result<String, String> {
    match required(map, field, kind, hint)? {
        Value::String(text) => Ok(text.clone()),
        other => Err(format!(
            "`{field}` is {}, but it is text for {kind}: {hint}",
            describe(other)
        )),
    }
}

fn dest(map: &Map<String, Value>, kind: &str) -> Result<String, String> {
    text(
        map,
        "dest",
        kind,
        "write the ID of a node, such as `dest: \"k3f9x2qa\"`",
    )
}

fn flag(map: &Map<String, Value>, field: &str, default: bool) -> Result<bool, String> {
    match map.get(field) {
        None => Ok(default),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(other) => Err(format!(
            "`{field}` is {}, but it is `true` or `false`: write `{field}: true`",
            describe(other)
        )),
    }
}

fn number(field: &str, value: &Value) -> Result<f64, String> {
    value.as_f64().ok_or_else(|| {
        format!(
            "`{field}` is {}, but it is a number: write `{field}: 0`",
            describe(value)
        )
    })
}

fn milliseconds(field: &str, value: &Value) -> Result<u32, String> {
    value
        .as_u64()
        .and_then(|ms| u32::try_from(ms).ok())
        .ok_or_else(|| {
            format!(
                "`{field}` is a whole number of milliseconds: write 0 or more, such as `{field}: 300`"
            )
        })
}

fn offset(field: &str, value: &Value) -> Result<[f64; 2], String> {
    match value.as_array().map(Vec::as_slice) {
        Some([x, y]) => Ok([number(field, x)?, number(field, y)?]),
        _ => Err(format!(
            "`{field}` is not a pair of numbers: write `{field}: [12, 40]`"
        )),
    }
}

fn keyword<T: for<'de> Deserialize<'de>>(
    value: &Value,
    known: &[&str],
    what: &str,
    field: &str,
) -> Result<T, String> {
    match value {
        Value::String(word) if known.contains(&word.as_str()) => {
            Ok(serde_json::from_value(value.clone()).expect("a listed keyword deserializes"))
        }
        Value::String(word) => Err(format!(
            "`{word}` is not {what}: write one of {}",
            names(known)
        )),
        other => Err(format!(
            "`{field}` is {}, but it is {what}: write one of {}",
            describe(other),
            names(known)
        )),
    }
}

fn trigger_from(value: &Value) -> Result<Trigger, String> {
    let (kind, map) = match value {
        Value::String(word) => (word.as_str(), Map::new()),
        Value::Object(map) => (kind(map, TRIGGERS, "a trigger")?, map.clone()),
        other => {
            return Err(format!(
                "`trigger` is {}, but it is an object with a `type`: write one of {}",
                describe(other),
                names(TRIGGERS)
            ));
        }
    };
    let allowed: &[&str] = match kind {
        "afterDelay" => &["type", "ms"],
        "keyPress" => &["type", "key"],
        _ => &["type"],
    };
    fields(&map, allowed, &format!("the {kind} trigger"))?;
    Ok(match kind {
        "onClick" => Trigger::OnClick,
        "onHover" => Trigger::OnHover,
        "onPress" => Trigger::OnPress,
        "onDrag" => Trigger::OnDrag,
        "mouseEnter" => Trigger::MouseEnter,
        "mouseLeave" => Trigger::MouseLeave,
        "mouseDown" => Trigger::MouseDown,
        "mouseUp" => Trigger::MouseUp,
        "afterDelay" => Trigger::AfterDelay {
            ms: milliseconds(
                "ms",
                required(
                    &map,
                    "ms",
                    "afterDelay",
                    "write the delay in milliseconds, such as `ms: 600`",
                )?,
            )?,
        },
        "keyPress" => Trigger::KeyPress {
            key: text(
                &map,
                "key",
                "keyPress",
                "write a key, such as `key: \"Shift+K\"`",
            )?,
        },
        other => {
            return Err(format!(
                "`{other}` is not a trigger: write one of {}",
                names(TRIGGERS)
            ));
        }
    })
}

fn transition_from(value: &Value) -> Result<Transition, String> {
    let map = object(value, "a transition")?;
    let kind = kind(map, TRANSITIONS, "a transition")?;
    let directional = matches!(kind, "moveIn" | "moveOut" | "push" | "slideIn" | "slideOut");
    fields(
        map,
        if directional {
            &["type", "direction"]
        } else {
            &["type"]
        },
        &format!("the {kind} transition"),
    )?;
    if !directional {
        return Ok(match kind {
            "dissolve" => Transition::Dissolve,
            "smartAnimate" => Transition::SmartAnimate,
            _ => Transition::Instant,
        });
    }
    let direction = keyword::<Direction>(
        required(
            map,
            "direction",
            kind,
            "write one of left, right, top, bottom, such as `direction: \"left\"`",
        )?,
        DIRECTIONS,
        "a direction",
        "direction",
    )?;
    Ok(match kind {
        "moveIn" => Transition::MoveIn { direction },
        "moveOut" => Transition::MoveOut { direction },
        "push" => Transition::Push { direction },
        "slideIn" => Transition::SlideIn { direction },
        _ => Transition::SlideOut { direction },
    })
}

fn easing_from(value: &Value) -> Result<Easing, String> {
    let (kind, map) = match value {
        Value::String(word) => (word.as_str(), Map::new()),
        Value::Object(map) => (kind(map, EASINGS, "an easing")?, map.clone()),
        other => {
            return Err(format!(
                "`easing` is {}, but it is a name or an object with a `type`: write one of {}",
                describe(other),
                names(EASINGS)
            ));
        }
    };
    let allowed: &[&str] = match kind {
        "customBezier" => &["type", "x1", "y1", "x2", "y2"],
        "customSpring" => &["type", "mass", "stiffness", "damping"],
        _ => &["type"],
    };
    fields(&map, allowed, &format!("the {kind} easing"))?;
    let field = |name: &str, hint: &str| -> Result<f64, String> {
        number(name, required(&map, name, kind, hint)?)
    };
    match kind {
        "customBezier" => {
            let hint = "write the four control points, such as `{type: \"customBezier\", x1: 0.2, y1: 0, x2: 0, y2: 1}`";
            Ok(Easing::CustomBezier {
                x1: field("x1", hint)?,
                y1: field("y1", hint)?,
                x2: field("x2", hint)?,
                y2: field("y2", hint)?,
            })
        }
        "customSpring" => {
            let hint = "write `{type: \"customSpring\", mass: 1, stiffness: 100, damping: 15}`";
            Ok(Easing::CustomSpring {
                mass: field("mass", hint)?,
                stiffness: field("stiffness", hint)?,
                damping: field("damping", hint)?,
            })
        }
        word => Easing::PLAIN
            .iter()
            .find(|(name, _)| *name == word)
            .map(|(_, easing)| *easing)
            .ok_or_else(|| format!("`{word}` is not an easing: write one of {}", names(EASINGS))),
    }
}

fn actions_from(value: &Value) -> Result<Vec<Action>, String> {
    let Some(items) = value.as_array() else {
        return Err(format!(
            "`actions` is {}, but it is a list: write `actions: [{{type: \"back\"}}]`",
            describe(value)
        ));
    };
    items.iter().map(action_from).collect()
}

fn action_from(value: &Value) -> Result<Action, String> {
    let map = object(value, "an action")?;
    let kind = kind(map, ACTIONS, "an action")?;
    let allowed: &[&str] = match kind {
        "navigate" => &["type", "dest", "preserveScroll"],
        "scrollTo" => &["type", "dest", "offset"],
        "openOverlay" => &[
            "type",
            "dest",
            "position",
            "closeOnOutside",
            "background",
            "offset",
            "anchor",
        ],
        "swapOverlay" | "changeTo" => &["type", "dest"],
        "openUrl" => &["type", "url", "newTab"],
        "setVariable" => &["type", "variable", "value", "expr"],
        "setVariableMode" => &["type", "collection", "mode"],
        "conditional" => &["type", "blocks"],
        _ => &["type"],
    };
    fields(map, allowed, &format!("the {kind} action"))?;
    Ok(match kind {
        "navigate" => Action::Navigate {
            dest: dest(map, kind)?,
            preserve_scroll: flag(map, "preserveScroll", false)?,
        },
        "scrollTo" => Action::ScrollTo {
            dest: dest(map, kind)?,
            offset: map
                .get("offset")
                .map(|value| offset("offset", value))
                .transpose()?
                .unwrap_or([0.0, 0.0]),
        },
        "openOverlay" => Action::OpenOverlay {
            dest: dest(map, kind)?,
            position: map
                .get("position")
                .map(|value| keyword(value, POSITIONS, "an overlay position", "position"))
                .transpose()?
                .unwrap_or_default(),
            close_on_outside: flag(map, "closeOnOutside", true)?,
            background: match map.get("background") {
                None => None,
                Some(Value::String(color)) => {
                    Color::from_str(color).map_err(|error| error.to_string())?;
                    Some(color.clone())
                }
                Some(other) => {
                    return Err(format!(
                        "`background` is {}, but it is a color: write `background: \"#00000033\"`",
                        describe(other)
                    ));
                }
            },
            offset: map
                .get("offset")
                .map(|value| offset("offset", value))
                .transpose()?,
            anchor: map
                .get("anchor")
                .map(|value| keyword(value, ANCHORS, "an overlay anchor", "anchor"))
                .transpose()?
                .unwrap_or_default(),
        },
        "swapOverlay" => Action::SwapOverlay {
            dest: dest(map, kind)?,
        },
        "changeTo" => Action::ChangeTo {
            dest: dest(map, kind)?,
        },
        "back" => Action::Back,
        "closeOverlay" => Action::CloseOverlay,
        "openUrl" => Action::OpenUrl {
            url: text(
                map,
                "url",
                kind,
                "write a link, such as `url: \"https://example.com\"`",
            )?,
            new_tab: flag(map, "newTab", true)?,
        },
        "setVariable" => {
            let variable = text(
                map,
                "variable",
                kind,
                "write the name of a variable, such as `variable: \"count\"`",
            )?;
            let value = map.get("value").cloned();
            let expr = map.get("expr").cloned();
            if value.is_none() && expr.is_none() {
                return Err(
                    "setVariable needs `value` or `expr`: write `value: 1`, or an expression as `expr: {...}`"
                        .to_string(),
                );
            }
            if let Some(expr) = &expr {
                object(expr, "an expression")?;
            }
            Action::SetVariable {
                variable,
                value,
                expr,
            }
        }
        "setVariableMode" => Action::SetVariableMode {
            collection: text(
                map,
                "collection",
                kind,
                "write the name of a collection, such as `collection: \"color\"`",
            )?,
            mode: text(
                map,
                "mode",
                kind,
                "write the name of a mode, such as `mode: \"dark\"`",
            )?,
        },
        _ => Action::Conditional {
            blocks: blocks_from(required(
                map,
                "blocks",
                kind,
                "write a list of blocks, such as `blocks: [{condition: {...}, actions: [...]}]`",
            )?)?,
        },
    })
}

fn blocks_from(value: &Value) -> Result<Vec<ConditionBlock>, String> {
    let Some(items) = value.as_array() else {
        return Err(format!(
            "`blocks` is {}, but it is a list: write `blocks: [{{actions: [...]}}]`",
            describe(value)
        ));
    };
    items
        .iter()
        .map(|item| {
            let map = object(item, "a block")?;
            fields(map, &["condition", "actions"], "a conditional block")?;
            let condition = map.get("condition").cloned();
            if let Some(condition) = &condition {
                object(condition, "a condition")?;
            }
            Ok(ConditionBlock {
                condition,
                actions: actions_from(required(
                    map,
                    "actions",
                    "a conditional block",
                    "write a list of actions, such as `actions: [{type: \"back\"}]`",
                )?)?,
            })
        })
        .collect()
}

/// A flow of a page: a name and the board it starts at.
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    pub name: String,
    pub start: NodeId,
    /// Empty when the flow has none, and then not written.
    pub description: String,
}

const FLOW_FIELDS: &[&str] = &["name", "start", "description"];

impl Serialize for Flow {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("name", &self.name)?;
        map.serialize_entry("start", &self.start)?;
        if !self.description.is_empty() {
            map.serialize_entry("description", &self.description)?;
        }
        map.end()
    }
}

impl Flow {
    /// Reads the `flows` of a page from their JSON form. Two flows may not
    /// share a name.
    pub fn list_from_value(value: &Value) -> Result<Vec<Flow>, String> {
        let Some(items) = value.as_array() else {
            return Err(format!(
                "`flows` is {}: write a list, such as `flows={{[{{name: \"Checkout\", start: \"b1a2c3d4\"}}]}}`",
                describe(value)
            ));
        };
        let flows = items
            .iter()
            .map(Flow::from_value)
            .collect::<Result<Vec<_>, _>>()?;
        for (at, flow) in flows.iter().enumerate() {
            if flows[..at].iter().any(|other| other.name == flow.name) {
                return Err(format!("two flows are named `{}`", flow.name));
            }
        }
        Ok(flows)
    }

    /// Reads one flow from its JSON form.
    pub fn from_value(value: &Value) -> Result<Flow, String> {
        let map = object(value, "a flow")?;
        fields(map, FLOW_FIELDS, "a flow")?;
        let name = text(
            map,
            "name",
            "a flow",
            "write its name, such as `name: \"Checkout\"`",
        )?;
        let start = text(
            map,
            "start",
            "a flow",
            "write the ID of the board it starts at, such as `start: \"b1a2c3d4\"`",
        )?;
        let start = start.parse::<NodeId>().map_err(|error| error.to_string())?;
        let description = match map.get("description") {
            None => String::new(),
            Some(Value::String(description)) => description.clone(),
            Some(other) => {
                return Err(format!(
                    "`description` is {}, but it is text: write `description: \"From the cart to the receipt\"`",
                    describe(other)
                ));
            }
        };
        Ok(Flow {
            name,
            start,
            description,
        })
    }
}

/// How the device is held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rotation {
    #[default]
    Portrait,
    /// Turned a quarter turn, so the screen is wider than it is tall.
    Landscape,
}

const ROTATIONS: &[&str] = &["portrait", "landscape"];

const PROTOTYPE_FIELDS: &[&str] = &[
    "device",
    "color",
    "rotation",
    "background",
    "responsive",
    "customSize",
];

const DEFAULT_BACKGROUND: &str = "#1e1e1e";

/// How a page is shown in play mode. A field that holds its default is left
/// out of the JSON and of the tag format.
#[derive(Clone, Debug, PartialEq)]
pub struct PrototypeSettings {
    /// A name of [`devices::DEVICES`].
    pub device: String,
    /// A colour of the device; `None` for the device's first.
    pub color: Option<String>,
    pub rotation: Rotation,
    pub background: Color,
    /// Whether the board is resized to the screen and its layout responds.
    pub responsive: bool,
    /// The screen of the device `Custom`, in points, width then height.
    pub custom_size: Option<[f64; 2]>,
}

impl Default for PrototypeSettings {
    fn default() -> Self {
        PrototypeSettings {
            device: NONE.to_string(),
            color: None,
            rotation: Rotation::Portrait,
            background: DEFAULT_BACKGROUND
                .parse()
                .expect("the default background is a color"),
            responsive: false,
            custom_size: None,
        }
    }
}

impl PrototypeSettings {
    fn holds_background_default(&self) -> bool {
        self.background
            .as_str()
            .eq_ignore_ascii_case(DEFAULT_BACKGROUND)
    }

    /// Whether every field holds its default, so that nothing is written.
    pub fn is_default(&self) -> bool {
        self.device == NONE
            && self.color.is_none()
            && self.rotation == Rotation::Portrait
            && self.holds_background_default()
            && !self.responsive
            && self.custom_size.is_none()
    }

    /// The settings with every field, as the doc outline carries them: a
    /// field left out holds its default, and `color` and `customSize` are
    /// `null` when unset.
    pub fn outline(&self) -> Value {
        json!({
            "device": self.device,
            "color": self.color,
            "rotation": self.rotation,
            "background": self.background,
            "responsive": self.responsive,
            "customSize": self.custom_size.map(sides),
        })
    }

    /// Reads the `prototype` of a page from its JSON form.
    pub fn from_value(value: &Value) -> Result<PrototypeSettings, String> {
        let map = object(value, "the prototype settings")?;
        fields(map, PROTOTYPE_FIELDS, "the prototype settings")?;
        let mut settings = PrototypeSettings::default();
        if let Some(device) = map.get("device") {
            let Some(name) = device.as_str() else {
                return Err(format!(
                    "`device` is {}, but it names a device: write one of {}",
                    describe(device),
                    device_names()
                ));
            };
            if devices::device(name).is_none() {
                return Err(format!(
                    "`{name}` is not a device: write one of {}",
                    device_names()
                ));
            }
            settings.device = name.to_string();
        }
        let device = devices::device(&settings.device).expect("the device is in the table");
        if let Some(size) = map.get("customSize") {
            if device.name != CUSTOM {
                return Err(format!("`customSize` is for the device {CUSTOM}"));
            }
            settings.custom_size = Some(custom_size(size)?);
        } else if device.name == CUSTOM {
            return Err(format!(
                "the device {CUSTOM} needs a `customSize`: write `customSize: [390, 844]`"
            ));
        }
        if let Some(rotation) = map.get("rotation") {
            settings.rotation = keyword(rotation, ROTATIONS, "a rotation", "rotation")?;
            if settings.rotation == Rotation::Landscape && !device.rotates {
                return Err(format!("`{}` does not rotate", device.name));
            }
        }
        if let Some(color) = map.get("color") {
            let Some(name) = color.as_str() else {
                return Err(format!(
                    "`color` is {}, but it names a color of the device: write `color: \"Black\"`",
                    describe(color)
                ));
            };
            if !device.colors.iter().any(|color| color.name == name) {
                return Err(if device.colors.is_empty() {
                    format!(
                        "`{name}` is not a color of `{}`: it comes in none",
                        device.name
                    )
                } else {
                    let colors: Vec<&str> = device.colors.iter().map(|color| color.name).collect();
                    format!(
                        "`{name}` is not a color of `{}`: write one of {}",
                        device.name,
                        quoted(&colors)
                    )
                });
            }
            settings.color = Some(name.to_string());
        }
        if let Some(background) = map.get("background") {
            settings.background = background
                .as_str()
                .ok_or_else(|| {
                    format!(
                        "`background` is {}, but it is a color: write `background: \"#000000\"`",
                        describe(background)
                    )
                })?
                .parse::<Color>()
                .map_err(|error| error.to_string())?;
        }
        settings.responsive = flag(map, "responsive", false)?;
        Ok(settings)
    }
}

impl Serialize for PrototypeSettings {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if self.device != NONE {
            map.serialize_entry("device", &self.device)?;
        }
        if let Some(color) = &self.color {
            map.serialize_entry("color", color)?;
        }
        if self.rotation != Rotation::Portrait {
            map.serialize_entry("rotation", &self.rotation)?;
        }
        if !self.holds_background_default() {
            map.serialize_entry("background", &self.background)?;
        }
        if self.responsive {
            map.serialize_entry("responsive", &true)?;
        }
        if let Some(size) = &self.custom_size {
            map.serialize_entry("customSize", &sides(*size))?;
        }
        map.end()
    }
}

/// The sides of a custom size as JSON, a whole number without a fraction.
fn sides(size: [f64; 2]) -> [Value; 2] {
    size.map(|side| {
        if side.fract() == 0.0 && side.abs() < 9_007_199_254_740_992.0 {
            Value::from(side as i64)
        } else {
            Value::from(side)
        }
    })
}

fn device_names() -> String {
    let all: Vec<&str> = devices::DEVICES.iter().map(|device| device.name).collect();
    quoted(&all)
}

fn custom_size(value: &Value) -> Result<[f64; 2], String> {
    let side = |side: &Value| {
        side.as_f64()
            .filter(|side| side.is_finite() && *side >= 1.0)
    };
    match value.as_array().map(Vec::as_slice) {
        Some([width, height]) => match (side(width), side(height)) {
            (Some(width), Some(height)) => Ok([width, height]),
            _ => Err(
                "`customSize` has a side below 1: write the width and height in points, such as `customSize: [390, 844]`"
                    .to_string(),
            ),
        },
        _ => Err(
            "`customSize` is not a pair of numbers: write `customSize: [390, 844]`".to_string(),
        ),
    }
}
