//! US-044: what the design viewer needs from the renderer for its inspect
//! panel, which shows every property of the selected node with the tokens it
//! uses resolved in the current mode, and for an instance its component,
//! property values and overrides.
//!
//! `omni_render::view::Scene::inspect(&address)` answers it with the types of
//! `omni_render::inspect`, for the node at `address` (a node ID, or
//! `<instanceId>/<componentNodeId>` for a node inside an instance), or `None`
//! when no node of the scene has that address:
//!
//! - `address`, `name`, and `kind`: the node's type as the design has it, so
//!   an instance is an `instance` although it is drawn as a frame.
//! - `mode`: the mode the scene resolves tokens in (`None` without a design
//!   system), and `bounds`: the node's laid-out box (`Scene::bounds_of`).
//! - `layout`, `size`, `style` and `text`: the node's properties as the scene
//!   draws them, grouped by key (`LAYOUT_KEYS`, `SIZE_KEYS`, `STYLE_KEYS`,
//!   `TEXT_KEYS`). A node inside an instance has the properties it is drawn
//!   with: its component layer's, with the instance's property values bound
//!   and its overrides applied; an instance has those of the frame it is drawn
//!   as. Each `Property` has the node's key as `name`, its `value` with every
//!   token resolved in the mode, and `tokens`: each token the setting refers
//!   to, once, with the value it takes in the mode.
//! - `instance`, for an instance only: the `component` it names and that
//!   component's name, every property of the component (alphabetical) with
//!   the instance's value or the default and whether the instance sets it,
//!   and the instance's overrides, each with the component node it changes,
//!   that node's name and the fields as the instance stores them.
//!
//! The WebAssembly module forwards it to the web app (the last test):
//!
//! - `omni_render_inspect(address, len) -> *const c_char`: `Scene::inspect`
//!   of the loaded scene for an address as text, as a NUL-ended JSON object
//!   that stays until the next call, or null when the address names no node:
//!   `{"address": "<address>", "name": "Buy", "type": "instance", "mode":
//!   "light", "box": {"x": 20, "y": 20, "width": 120, "height": 40},
//!   "layout": [{"name": "x", "value": 20, "tokens": []}], "size": [...],
//!   "style": [{"name": "fills", "value": [{"type": "solid", "color":
//!   "#ff0000"}], "tokens": [{"token": "$color.brand", "value": "#ff0000"}]}],
//!   "text": [], "instance": {"component": "<id>", "componentName": "Button",
//!   "props": [{"name": "label", "kind": "text", "value": "Pay now", "set":
//!   true}], "overrides": [{"node": "<id>", "name": "Label", "fields":
//!   {...}}]}}`, with `"instance": null` for any other node.
//!
//! The mode switch itself is `Scene::set_mode` and `omni_render_scene_set_mode`
//! (US-032, crates/omni-render/tests/instances.rs). That the panel matches the
//! selected node in both modes in a browser is the human check.

mod common;
mod design;

use design::{fixed, node, with};
use omni_core::design::NodeAddress;
use omni_render::inspect::{Inspection, Property, TokenUse};
use omni_render::paint::Assets;
use omni_render::system::DesignSystem;
use omni_render::view::Scene;
use serde_json::{Value, json};
use std::fs;

// ---------------------------------------------------------------------------
// The design system and the page
// ---------------------------------------------------------------------------
//
// Tokens, in the modes light and dark:
//
//   color.surface   #ffffff / #000000
//   color.brand     #ff0000 / #0000ff
//   color.ink       #000000 / #ffffff
//   color.accent    #00a000 / #80ff80
//   spacing.pad     10px    / 30px
//
// Components:
//
//   Cart (20 x 20), with one layer, Wheel (a 6 x 6 frame).
//   Button (120 x 40, filled $color.brand), with the properties label (text,
//   "Buy") and showIcon (boolean, true), and two layers: Label (a text in
//   $color.ink whose content is bound to label) and Icon (an instance of
//   Cart, shown by showIcon).
//
// The page has one board, Checkout (0, 0, 400 x 300, filled $color.surface):
//
//   Buy, an instance of Button at (20, 20) with label "Pay now", which
//   overrides Label's fill with $color.accent;
//   Summary, a frame at (20, 100) that hugs its content, filled
//   $color.surface, with a flex layout padded $spacing.pad on every side,
//   holding the text Total (14 px Inter, line height 1.25, #000000).

const CART: &str = "cc000001";
const WHEEL: &str = "cw000001";
const BUTTON: &str = "cb000001";
const LABEL: &str = "ct000001";
const ICON: &str = "cn000001";

const CHECKOUT: &str = "a0000001";
const BUY: &str = "e0000001";
const SUMMARY: &str = "f0000001";
const TOTAL: &str = "f0000002";

fn solid(color: &str) -> Value {
    json!([{"type": "solid", "color": color}])
}

fn tokens() -> Value {
    let per_mode = |light: &str, dark: &str| {
        json!({
            "$value": light,
            "$extensions": {"omnistate": {"modes": {"light": light, "dark": dark}}}
        })
    };
    json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "$type": "color",
            "surface": per_mode("#ffffff", "#000000"),
            "brand": per_mode("#ff0000", "#0000ff"),
            "ink": per_mode("#000000", "#ffffff"),
            "accent": per_mode("#00a000", "#80ff80")
        },
        "spacing": {
            "$type": "dimension",
            "pad": per_mode("10px", "30px")
        }
    })
}

fn components() -> Value {
    let wheel = json!({
        "id": WHEEL,
        "type": "frame",
        "name": "Wheel",
        "x": 2,
        "y": 12,
        "width": fixed(6.0),
        "height": fixed(6.0),
        "layout": {"mode": "none"},
        "fills": solid("#00a000")
    });
    let cart = json!({
        "id": CART,
        "type": "component",
        "name": "Cart",
        "width": fixed(20.0),
        "height": fixed(20.0),
        "layout": {"mode": "none"},
        "fills": solid("#a0a0a0"),
        "children": [wheel]
    });
    let label = json!({
        "id": LABEL,
        "type": "text",
        "name": "Label",
        "x": 40,
        "y": 10,
        "width": "hug",
        "height": "hug",
        "content": "Buy",
        "fontFamily": "Inter",
        "fontSize": 16,
        "lineHeight": 1.25,
        "color": "$color.ink",
        "bind": {"content": "label"}
    });
    let icon = json!({
        "id": ICON,
        "type": "instance",
        "name": "Icon",
        "component": CART,
        "x": 10,
        "y": 10,
        "bind": {"visible": "showIcon"}
    });
    let button = json!({
        "id": BUTTON,
        "type": "component",
        "name": "Button",
        "width": fixed(120.0),
        "height": fixed(40.0),
        "layout": {"mode": "none"},
        "fills": solid("$color.brand"),
        "props": {
            "label": {"type": "text", "default": "Buy"},
            "showIcon": {"type": "boolean", "default": true}
        },
        "children": [label, icon]
    });
    json!([cart, button])
}

fn system() -> DesignSystem {
    DesignSystem::from_json(&with(&tokens(), "$components", components()))
        .unwrap_or_else(|error| panic!("the design system fixture must be read, but: {error}"))
}

fn page_json() -> Value {
    let buy = json!({
        "id": BUY,
        "type": "instance",
        "name": "Buy",
        "component": BUTTON,
        "x": 20,
        "y": 20,
        "props": {"label": "Pay now"},
        "overrides": {LABEL: {"fills": solid("$color.accent")}}
    });
    let summary = json!({
        "id": SUMMARY,
        "type": "frame",
        "name": "Summary",
        "x": 20,
        "y": 100,
        "width": "hug",
        "height": "hug",
        "fills": solid("$color.surface"),
        "layout": {
            "mode": "flex",
            "padding": {
                "top": "$spacing.pad",
                "right": "$spacing.pad",
                "bottom": "$spacing.pad",
                "left": "$spacing.pad"
            }
        },
        "children": [{
            "id": TOTAL,
            "type": "text",
            "name": "Total",
            "width": "hug",
            "height": "hug",
            "content": "Total 96 kr",
            "fontFamily": "Inter",
            "fontSize": 14,
            "lineHeight": 1.25,
            "color": "#000000"
        }]
    });
    json!({
        "id": "p0000001",
        "type": "page",
        "name": "Mobile",
        "children": [{
            "id": CHECKOUT,
            "type": "board",
            "name": "Checkout",
            "x": 0,
            "y": 0,
            "width": fixed(400.0),
            "height": fixed(300.0),
            "layout": {"mode": "none"},
            "fills": solid("$color.surface"),
            "children": [buy, summary]
        }]
    })
}

fn scene() -> Scene {
    Scene::with_system(node(&page_json()), Assets::new(), system())
}

fn address(text: &str) -> NodeAddress {
    text.parse()
        .unwrap_or_else(|error| panic!("the test uses an invalid address `{text}`: {error}"))
}

fn inside(instance: &str, node: &str) -> String {
    format!("{instance}/{node}")
}

fn inspect(scene: &Scene, text: &str) -> Inspection {
    scene
        .inspect(&address(text))
        .unwrap_or_else(|| panic!("{text} is a node of the scene, so it can be inspected"))
}

/// The property `name` of `group`, which is called `group_name` in messages.
fn property<'a>(group: &'a [Property], group_name: &str, name: &str) -> &'a Property {
    group
        .iter()
        .find(|property| property.name == name)
        .unwrap_or_else(|| {
            let names: Vec<&str> = group
                .iter()
                .map(|property| property.name.as_str())
                .collect();
            panic!("the {group_name} properties must include `{name}`, but they are {names:?}")
        })
}

fn names(group: &[Property]) -> Vec<&str> {
    group
        .iter()
        .map(|property| property.name.as_str())
        .collect()
}

fn number(value: &Value, what: &str) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("{what} must be a number, but it is {value}"))
}

fn token(name: &str, value: Value) -> TokenUse {
    TokenUse {
        token: name.to_string(),
        value,
    }
}

/// The token uses of `property` with numbers read as `f64`, so that `10` and
/// `10.0` compare equal.
fn uses(property: &Property) -> Vec<(String, Value)> {
    property
        .tokens
        .iter()
        .map(|used| {
            let value = match used.value.as_f64() {
                Some(number) => json!(number),
                None => used.value.clone(),
            };
            (used.token.clone(), value)
        })
        .collect()
}

fn expected(tokens: Vec<TokenUse>) -> Vec<(String, Value)> {
    tokens
        .into_iter()
        .map(|used| {
            let value = match used.value.as_f64() {
                Some(number) => json!(number),
                None => used.value,
            };
            (used.token, value)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Layout, size, style and text
// ---------------------------------------------------------------------------

#[test]
fn a_frame_shows_its_layout_size_and_style_with_its_tokens_in_the_mode() {
    let scene = scene();
    let summary = inspect(&scene, SUMMARY);
    assert_eq!(summary.address, address(SUMMARY));
    assert_eq!(summary.name, "Summary");
    assert_eq!(summary.kind, "frame");
    assert_eq!(summary.mode.as_deref(), Some("light"), "the first mode");
    assert_eq!(
        Some(summary.bounds),
        scene.bounds_of(&address(SUMMARY)),
        "the box the node is laid out in"
    );

    assert_eq!(
        number(&property(&summary.layout, "layout", "x").value, "x"),
        20.0
    );
    assert_eq!(
        number(&property(&summary.layout, "layout", "y").value, "y"),
        100.0
    );
    let layout = property(&summary.layout, "layout", "layout");
    assert_eq!(layout.value["mode"], "flex");
    assert_eq!(
        number(&layout.value["padding"]["left"], "the padding"),
        10.0,
        "the padding is $spacing.pad resolved in light mode: {}",
        layout.value
    );
    assert_eq!(
        uses(layout),
        expected(vec![token("$spacing.pad", json!(10))]),
        "the layout uses $spacing.pad, listed once although four sides use it"
    );

    let width = property(&summary.size, "size", "width");
    assert_eq!(width.value, json!("hug"));
    assert!(width.tokens.is_empty());
    property(&summary.size, "size", "height");

    let fills = property(&summary.style, "style", "fills");
    assert_eq!(
        fills.value,
        solid("#ffffff"),
        "the fill resolved in light mode"
    );
    assert_eq!(
        uses(fills),
        expected(vec![token("$color.surface", json!("#ffffff"))])
    );

    assert!(
        summary.text.is_empty(),
        "a frame has no text properties: {:?}",
        names(&summary.text)
    );
    assert_eq!(summary.instance, None, "a frame is no instance");
}

// ---------------------------------------------------------------------------
// Instances
// ---------------------------------------------------------------------------

#[test]
fn an_instance_shows_its_component_property_values_and_overrides() {
    let scene = scene();
    let buy = inspect(&scene, BUY);
    assert_eq!(buy.kind, "instance", "listed as the instance it is");
    assert_eq!(buy.name, "Buy");
    let instance = buy
        .instance
        .as_ref()
        .expect("an instance shows what it stores");
    assert_eq!(instance.component.as_str(), BUTTON);
    assert_eq!(instance.component_name, "Button");

    let props: Vec<(&str, &str, &Value, bool)> = instance
        .props
        .iter()
        .map(|prop| {
            (
                prop.name.as_str(),
                prop.kind.as_str(),
                &prop.value,
                prop.set,
            )
        })
        .collect();
    assert_eq!(
        props,
        vec![
            ("label", "text", &json!("Pay now"), true),
            ("showIcon", "boolean", &json!(true), false),
        ],
        "every property of Button: the instance's label, and showIcon's default"
    );

    assert_eq!(instance.overrides.len(), 1, "{:?}", instance.overrides);
    let label = &instance.overrides[0];
    assert_eq!(label.node.as_str(), LABEL);
    assert_eq!(label.name, "Label", "the overridden layer's name");
    assert_eq!(
        label.fields.keys().collect::<Vec<_>>(),
        vec!["fills"],
        "the fields the instance sets"
    );
    assert_eq!(
        label.fields["fills"][0]["color"],
        json!("$color.accent"),
        "an override is shown as the instance stores it"
    );

    // The instance is drawn as a frame with its component's style, placed
    // and sized as the instance says.
    assert_eq!(
        number(&property(&buy.layout, "layout", "x").value, "x"),
        20.0
    );
    assert_eq!(
        number(&property(&buy.layout, "layout", "y").value, "y"),
        20.0
    );
    property(&buy.size, "size", "width");
    assert_eq!((buy.bounds.width, buy.bounds.height), (120.0, 40.0));
    let fills = property(&buy.style, "style", "fills");
    assert_eq!(fills.value, solid("#ff0000"), "Button's fill in light mode");
    assert_eq!(
        uses(fills),
        expected(vec![token("$color.brand", json!("#ff0000"))])
    );
}

#[test]
fn a_node_inside_an_instance_shows_what_the_instance_gave_it() {
    let scene = scene();
    let label = inspect(&scene, &inside(BUY, LABEL));
    assert_eq!(label.address, address(&inside(BUY, LABEL)));
    assert_eq!(label.kind, "text");
    assert_eq!(label.name, "Label");
    assert_eq!(
        property(&label.text, "text", "content").value,
        json!("Pay now"),
        "the content bound to the instance's label"
    );
    let color = property(&label.text, "text", "color");
    assert_eq!(
        color.value,
        json!("#00a000"),
        "the override's color in light mode"
    );
    assert_eq!(
        uses(color),
        expected(vec![token("$color.accent", json!("#00a000"))]),
        "the token of the override, not the component's $color.ink"
    );
    assert_eq!(
        label.instance, None,
        "a text inside an instance is no instance"
    );
    assert_eq!(
        Some(label.bounds),
        scene.bounds_of(&address(&inside(BUY, LABEL)))
    );

    let icon = inspect(&scene, &inside(BUY, ICON));
    assert_eq!(icon.kind, "instance");
    let instance = icon.instance.expect("Icon is an instance of Cart");
    assert_eq!(instance.component.as_str(), CART);
    assert_eq!(instance.component_name, "Cart");
    assert!(instance.props.is_empty(), "Cart has no properties");
    assert!(instance.overrides.is_empty(), "Icon overrides nothing");
}

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

#[test]
fn after_a_mode_switch_the_properties_have_that_modes_values() {
    let mut scene = scene();
    let light = inspect(&scene, SUMMARY);
    scene
        .set_mode("dark")
        .unwrap_or_else(|error| panic!("dark is a mode of the design system, but: {error}"));

    let summary = inspect(&scene, SUMMARY);
    assert_eq!(summary.mode.as_deref(), Some("dark"));
    let layout = property(&summary.layout, "layout", "layout");
    assert_eq!(number(&layout.value["padding"]["top"], "the padding"), 30.0);
    assert_eq!(
        uses(layout),
        expected(vec![token("$spacing.pad", json!(30))])
    );
    let fills = property(&summary.style, "style", "fills");
    assert_eq!(fills.value, solid("#000000"));
    assert_eq!(
        uses(fills),
        expected(vec![token("$color.surface", json!("#000000"))])
    );
    assert!(
        (summary.bounds.width - (light.bounds.width + 40.0)).abs() < 0.01,
        "a padding of 30 px on both sides makes Summary 40 px wider: {} in light, {} in dark",
        light.bounds.width,
        summary.bounds.width
    );

    let buy = inspect(&scene, BUY);
    assert_eq!(
        uses(property(&buy.style, "style", "fills")),
        expected(vec![token("$color.brand", json!("#0000ff"))])
    );
    let label = inspect(&scene, &inside(BUY, LABEL));
    let color = property(&label.text, "text", "color");
    assert_eq!(color.value, json!("#80ff80"));
    assert_eq!(
        uses(color),
        expected(vec![token("$color.accent", json!("#80ff80"))])
    );

    scene.set_mode("light").expect("light is a mode too");
    assert_eq!(inspect(&scene, SUMMARY), light, "back in light mode");
}

// A mode per collection (US-24): `color` inherits the root list (light,
// dark), `density` lists its own (comfortable, compact):
//
//   color.surface     #ffffff / #000000
//   density.space.md  16px (comfortable) / 12px (compact)
//
// The page's board Checkout holds Summary, filled $color.surface with a
// padding of $density.space.md. Its own scene, so the shared fixture keeps a
// single mode.

fn density_scene() -> Scene {
    let tokens = json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "$type": "color",
            "surface": {
                "$value": "#ffffff",
                "$extensions": {"omnistate": {"modes": {"light": "#ffffff", "dark": "#000000"}}}
            }
        },
        "density": {
            "$extensions": {"omnistate": {"modes": ["comfortable", "compact"]}},
            "space": {
                "md": {
                    "$type": "dimension",
                    "$value": "16px",
                    "$extensions": {"omnistate": {"modes": {"comfortable": "16px", "compact": "12px"}}}
                }
            }
        }
    });
    let system = DesignSystem::from_json(&tokens)
        .unwrap_or_else(|error| panic!("the density design system must be read, but: {error}"));
    let summary = json!({
        "id": SUMMARY,
        "type": "frame",
        "name": "Summary",
        "x": 20,
        "y": 20,
        "width": fixed(100.0),
        "height": fixed(40.0),
        "fills": solid("$color.surface"),
        "layout": {"mode": "flex", "padding": {"top": "$density.space.md"}}
    });
    let page = json!({
        "id": "p0000001",
        "type": "page",
        "name": "Mobile",
        "children": [{
            "id": CHECKOUT,
            "type": "board",
            "name": "Checkout",
            "x": 0,
            "y": 0,
            "width": fixed(400.0),
            "height": fixed(300.0),
            "layout": {"mode": "none"},
            "children": [summary]
        }]
    });
    Scene::with_system(node(&page), Assets::new(), system)
}

/// The token uses of a property, as `uses` reads them.
type Uses = Vec<(String, Value)>;

/// The padding token and the fill token of Summary, as the inspection shows them.
fn density_and_surface(scene: &Scene) -> (Uses, Uses) {
    let summary = inspect(scene, SUMMARY);
    (
        uses(property(&summary.layout, "layout", "layout")),
        uses(property(&summary.style, "style", "fills")),
    )
}

#[test]
fn a_choice_of_one_collection_resolves_it_and_leaves_the_others_in_their_modes() {
    let mut scene = density_scene();
    scene
        .set_mode(r#"{"density": "compact"}"#)
        .unwrap_or_else(|error| panic!("compact is a mode of density, but: {error}"));
    assert_eq!(
        density_and_surface(&scene),
        (
            expected(vec![token("$density.space.md", json!(12))]),
            expected(vec![token("$color.surface", json!("#ffffff"))]),
        ),
        "density is compact, color stays light"
    );
    assert_eq!(
        inspect(&scene, SUMMARY).mode.as_deref(),
        Some("color:light,density:compact"),
        "collections in different modes are each named with their mode"
    );

    scene
        .set_mode(r#"{"color": "dark"}"#)
        .unwrap_or_else(|error| panic!("dark is a mode of color, but: {error}"));
    assert_eq!(
        density_and_surface(&scene),
        (
            expected(vec![token("$density.space.md", json!(12))]),
            expected(vec![token("$color.surface", json!("#000000"))]),
        ),
        "density stays compact when the choice names only color"
    );
}

#[test]
fn a_bare_mode_name_sets_only_the_collections_that_inherit_the_root_list() {
    let mut scene = density_scene();
    scene
        .set_mode("dark")
        .unwrap_or_else(|error| panic!("dark is in the root list, but: {error}"));
    assert_eq!(
        inspect(&scene, SUMMARY).mode.as_deref(),
        Some("color:dark,density:comfortable"),
        "dark sets color and leaves density, which lists its own modes"
    );
    assert_eq!(
        density_and_surface(&scene),
        (
            expected(vec![token("$density.space.md", json!(16))]),
            expected(vec![token("$color.surface", json!("#000000"))]),
        )
    );

    assert_eq!(
        scene.set_mode("compact"),
        Err("`compact` is not a mode of the design system: it has `light`, `dark`".to_string()),
        "a bare name is looked up in the root list only"
    );
    assert_eq!(
        inspect(&scene, SUMMARY).mode.as_deref(),
        Some("color:dark,density:comfortable"),
        "the refused name keeps every mode"
    );
}

#[test]
fn a_choice_naming_a_mode_or_collection_the_system_lacks_is_refused_as_a_whole() {
    let mut scene = density_scene();
    assert_eq!(
        scene.set_mode(r#"{"color": "dusk"}"#),
        Err("`dusk` is not a mode of `color` (light, dark)".to_string())
    );
    assert_eq!(
        scene.set_mode(r#"{"color": "dark", "shade": "deep"}"#),
        Err("`shade` is not a collection of the design system (color, density)".to_string())
    );
    assert_eq!(
        density_and_surface(&scene),
        (
            expected(vec![token("$density.space.md", json!(16))]),
            expected(vec![token("$color.surface", json!("#ffffff"))]),
        ),
        "the valid color entry of the refused choice is not applied"
    );
}

// ---------------------------------------------------------------------------
// Interactions and scrolling (US-41)
// ---------------------------------------------------------------------------
//
// `Inspection::interactions` lists the node's own interactions, each with the
// trigger, transition, duration, easing and delay filled in, each action with
// `destName` and `destKind` of the node it points at (a board or layer of the
// page, or a component of the design system), and `dangling` set when an
// action, a branch's included, points at a node that is not in the doc.
// `overflow` is set for a board or frame and `scrollPosition` for a frame.

#[test]
fn a_layer_lists_its_interactions_with_their_targets_and_a_frame_its_scrolling() {
    let hotspot = "h0000001";
    let confirmation = "b0000002";
    let gone = "d0000001";
    let interactions = json!([
        {
            "actions": [{"type": "navigate", "dest": confirmation, "preserveScroll": true}],
            "transition": {"type": "slideIn", "direction": "right"}
        },
        {
            "trigger": "onHover",
            "actions": [{"type": "changeTo", "dest": BUTTON}],
            "transition": {"type": "smartAnimate"},
            "durationMs": 150,
            "easing": "easeInOut"
        },
        {
            "trigger": {"type": "afterDelay", "ms": 2000},
            "actions": [{"type": "conditional", "blocks": [
                {"actions": [{"type": "openOverlay", "dest": gone}]}
            ]}]
        }
    ]);
    let pay_frame = json!({
        "id": hotspot,
        "type": "frame",
        "name": "Pay",
        "x": 0,
        "y": 0,
        "width": fixed(100.0),
        "height": fixed(40.0),
        "layout": {"mode": "none"},
        "scrollPosition": "fixed",
        "interactions": interactions
    });
    let checkout_board = json!({
        "id": CHECKOUT,
        "type": "board",
        "name": "Checkout",
        "x": 0,
        "y": 0,
        "width": fixed(400.0),
        "height": fixed(300.0),
        "layout": {"mode": "none"},
        "overflow": "vertical",
        "children": [pay_frame]
    });
    let confirmation_board = json!({
        "id": confirmation,
        "type": "board",
        "name": "Confirmation",
        "x": 500,
        "y": 0,
        "width": fixed(400.0),
        "height": fixed(300.0),
        "layout": {"mode": "none"}
    });
    let page = json!({
        "id": "p0000001",
        "type": "page",
        "name": "Mobile",
        "children": [checkout_board, confirmation_board]
    });
    let scene = Scene::with_system(node(&page), Assets::new(), system());

    let pay = inspect(&scene, hotspot);
    assert_eq!(pay.interactions.len(), 3);
    assert_eq!(
        pay.interactions[0],
        json!({
            "trigger": {"type": "onClick"},
            "actions": [{
                "type": "navigate",
                "dest": confirmation,
                "preserveScroll": true,
                "destName": "Confirmation",
                "destKind": "board"
            }],
            "transition": {"type": "slideIn", "direction": "right"},
            "durationMs": 300,
            "easing": "easeOut",
            "delayMs": 0,
            "dangling": false
        })
    );
    assert_eq!(pay.interactions[1]["trigger"], json!({"type": "onHover"}));
    assert_eq!(pay.interactions[1]["durationMs"], json!(150));
    assert_eq!(pay.interactions[1]["easing"], json!("easeInOut"));
    assert_eq!(
        pay.interactions[1]["actions"][0]["destName"],
        json!("Button"),
        "a component of the design system is named too"
    );
    assert_eq!(
        pay.interactions[1]["actions"][0]["destKind"],
        json!("component")
    );
    assert_eq!(pay.interactions[1]["dangling"], json!(false));
    assert_eq!(pay.interactions[2]["dangling"], json!(true));
    assert_eq!(
        pay.interactions[2]["actions"][0]["blocks"][0]["actions"][0].get("destName"),
        None,
        "a target that is gone has no name"
    );
    assert_eq!(pay.overflow.as_deref(), Some("none"));
    assert_eq!(pay.scroll_position.as_deref(), Some("fixed"));

    let checkout = inspect(&scene, CHECKOUT);
    assert!(checkout.interactions.is_empty());
    assert_eq!(checkout.overflow.as_deref(), Some("vertical"));
    assert_eq!(checkout.scroll_position, None, "a board takes no position");
}

#[test]
fn an_instance_lists_only_its_own_interactions_not_its_components() {
    let mut components = components();
    components[1] = with(
        &components[1],
        "interactions",
        json!([{"actions": [{"type": "navigate", "dest": CHECKOUT}]}]),
    );
    let system = DesignSystem::from_json(&with(&tokens(), "$components", components))
        .unwrap_or_else(|error| panic!("the design system fixture must be read, but: {error}"));
    let scene = Scene::with_system(node(&page_json()), Assets::new(), system);

    let buy = inspect(&scene, BUY);
    assert_eq!(buy.interactions, Vec::<Value>::new());
}

#[test]
fn only_boards_and_frames_have_scrolling_and_a_frame_scrolls_by_default() {
    let scene = scene();

    let summary = inspect(&scene, SUMMARY);
    assert_eq!(summary.overflow.as_deref(), Some("none"));
    assert_eq!(summary.scroll_position.as_deref(), Some("scrolls"));

    let total = inspect(&scene, TOTAL);
    assert_eq!((total.overflow, total.scroll_position), (None, None));
    let buy = inspect(&scene, BUY);
    assert_eq!((buy.overflow, buy.scroll_position), (None, None));
}

// ---------------------------------------------------------------------------
// The WebAssembly export
// ---------------------------------------------------------------------------

/// The WebAssembly renderer that `make render-wasm` built before this test run.
fn render_wasm() {
    common::render_wasm();
}

#[test]
fn the_wasm_module_exports_the_inspection_and_the_mode_switch() {
    render_wasm();
    let path = common::repo_root().join("render-sandbox/build/omni_render.js");
    let loader =
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"));
    for export in ["_omni_render_inspect", "_omni_render_scene_set_mode"] {
        assert!(
            loader.contains(export),
            "render-sandbox/build/omni_render.js must export `{export}` after `make render-wasm`, \
             for the design viewer's inspect panel and mode switch"
        );
    }
}
