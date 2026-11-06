//! What the design viewer's inspect panel shows of the selected node (US-044):
//! its layout, size, style and text properties as the scene draws them, with
//! the tokens they use resolved in the scene's mode, and for an instance its
//! component, property values and overrides.
//!
//! These are the types of `Scene::inspect` (`view.rs`).

use crate::layout::LayoutBox;
use omni_core::design::{NodeAddress, NodeId};
use serde_json::{Map, Value};

/// The keys of a node in the `layout` group of an inspection.
pub const LAYOUT_KEYS: &[&str] = &[
    "layout",
    "absolute",
    "x",
    "y",
    "gridColumn",
    "gridRow",
    "margin",
    "justifySelf",
    "alignSelf",
];

/// The keys of a node in the `size` group of an inspection.
pub const SIZE_KEYS: &[&str] = &[
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
];

/// The keys of a node in the `style` group of an inspection.
pub const STYLE_KEYS: &[&str] = &[
    "visible", "opacity", "fills", "strokes", "radius", "shadows", "clip", "image", "fit",
];

/// The keys of a node in the `text` group of an inspection.
pub const TEXT_KEYS: &[&str] = &[
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
];

/// A node of the scene as the inspect panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspection {
    /// The node's ID, or `<instanceId>/<componentNodeId>` for a node inside
    /// an instance.
    pub address: NodeAddress,
    pub name: String,
    /// The node's type as the design has it, so an instance is an `instance`.
    pub kind: String,
    /// The mode the scene resolves tokens in; `None` for a scene without a
    /// design system.
    pub mode: Option<String>,
    /// The node's laid-out box in page coordinates (`Scene::bounds_of`).
    pub bounds: LayoutBox,
    /// The properties of the node with a key of [`LAYOUT_KEYS`], in that order.
    pub layout: Vec<Property>,
    /// The properties of the node with a key of [`SIZE_KEYS`], in that order.
    pub size: Vec<Property>,
    /// The properties of the node with a key of [`STYLE_KEYS`], in that order.
    pub style: Vec<Property>,
    /// The properties of the node with a key of [`TEXT_KEYS`], in that order.
    pub text: Vec<Property>,
    /// For an instance: its component, property values and overrides.
    pub instance: Option<InstanceInspection>,
    /// The node's own interactions, as `Scene::inspect` reads them: the
    /// trigger, actions, transition, duration, easing and delay with their
    /// defaults filled in, each action with the name and kind of the node it
    /// points at, and `dangling` set when a target is not in the doc.
    pub interactions: Vec<Value>,
    /// For a board or frame: how it scrolls its content, `none` when left out.
    pub overflow: Option<String>,
    /// For a frame: `scrolls`, `fixed` or `sticky`, `scrolls` when left out.
    pub scroll_position: Option<String>,
}

/// One property of a node, as the scene draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    /// The node's key, such as `fills` or `fontSize`.
    pub name: String,
    /// The value with every token reference resolved in the scene's mode.
    pub value: Value,
    /// The tokens the node's setting refers to, each once, in the order they
    /// first appear in it, with the value it takes in the scene's mode.
    pub tokens: Vec<TokenUse>,
}

/// A token that a property refers to.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenUse {
    /// The reference as the node writes it, such as `$color.brand`.
    pub token: String,
    /// The token's value in the scene's mode, as the property takes it (a
    /// dimension such as `10px` is the number 10).
    pub value: Value,
}

/// What an instance stores, for the inspect panel.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceInspection {
    /// The component the instance names.
    pub component: NodeId,
    /// That component's name.
    pub component_name: String,
    /// Every property the component defines, by name in alphabetical order.
    pub props: Vec<PropValue>,
    /// The instance's own overrides, one per component node it changes, in
    /// the order the instance lists them.
    pub overrides: Vec<OverrideInspection>,
}

/// The value an instance gives a property of its component.
#[derive(Clone, Debug, PartialEq)]
pub struct PropValue {
    pub name: String,
    /// `variant`, `boolean`, `text` or `slot`.
    pub kind: String,
    /// The instance's value, or the property's default where it sets none.
    pub value: Value,
    /// Whether the instance sets the value itself.
    pub set: bool,
}

/// An override of an instance.
#[derive(Clone, Debug, PartialEq)]
pub struct OverrideInspection {
    /// The component node it changes.
    pub node: NodeId,
    /// That node's name in the component.
    pub name: String,
    /// The fields it sets, as the instance stores them.
    pub fields: Map<String, Value>,
}
