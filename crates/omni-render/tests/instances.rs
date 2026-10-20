//! US-032: the renderer draws component instances and resolves token
//! references, in every mode of the design system.
//!
//! `omni_render::system::DesignSystem::from_json` reads a design system
//! version as the API serves it: a DTCG token tree (`docs/formats/tokens.md`)
//! with the design system's `component` nodes in a list under `$components`.
//! `omni_render::view::Scene::with_system(page, assets, system)` lays a page
//! out and draws it with that design system, in its first mode. What the tests
//! expect:
//!
//! - An instance is drawn as its component's tree in the instance's box. An
//!   instance that leaves its size out takes its component's size. The
//!   component takes the instance's property values, and the defaults for the
//!   properties it leaves out:
//!   - `boolean`: a layer bound with `bind: {visible: "<prop>"}` is shown or
//!     hidden, and a hidden layer is neither drawn nor hit.
//!   - `text`: a text layer bound with `bind: {content: "<prop>"}` shows the
//!     value, and is laid out with it.
//!   - `slot`: an instance layer bound with `bind: {component: "<prop>"}`
//!     places the component whose node ID is the value.
//!   - `variant`: the components of a design system that have the same name
//!     and define the same variant properties (names and values) are the
//!     variants of one set, and each one's defaults are its own variant
//!     values. An instance shows the variant of its component's set whose
//!     defaults are the instance's variant values; a component of another
//!     name is never a variant of the set, even with the same properties.
//! - The instance's `overrides` apply on top, by the component node they
//!   name: `content` replaces a text, `fills` the fills of a node, `visible`
//!   shows or hides a node, and `component` swaps the component of an
//!   instance layer.
//! - Token references (`$color.brand`, `$spacing.pad`) are resolved in the
//!   scene's mode, in the design and in the components' layers alike, and
//!   for layout as well as for drawing. `Scene::set_mode` switches the mode,
//!   which lays out and draws the page again, and refuses a mode that the
//!   design system does not list, keeping the current one.
//! - A missing component (an instance of a component the design system does
//!   not have) and a missing token (a reference to a token the design system
//!   does not have) are drawn as a visible error placeholder in the node's
//!   box, and `Scene::errors` lists each with the node's address and what is
//!   missing (`Missing::Component`, `Missing::Token`), for the host page.
//!   A page with nothing missing has no errors.
//! - `Scene::node_at` gives the address of the node it finds, and a node
//!   inside an instance is `<instanceId>/<componentNodeId>`. `bounds_of` takes
//!   such an address and gives the node's box in page coordinates. Selection
//!   reports and outlines nodes inside instances by that address too.
//!
//! The WebAssembly exports and the sample page for the human check are covered
//! by `instances_sandbox.rs`. That the sample design looks right in light and
//! in dark mode is the human check.

mod design;

use design::{fixed, node, with};
use omni_core::design::{NodeAddress, NodeId, TokenRef};
use omni_render::layout::LayoutBox;
use omni_render::paint::Assets;
use omni_render::select::{Event, Modifiers, OutlineKind};
use omni_render::system::{DesignSystem, Missing, RenderError};
use omni_render::view::{Scene, Viewport};
use serde_json::{Value, json};

const WIDTH: u32 = 400;
const HEIGHT: u32 = 300;

const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];
/// `$color.brand` in light mode.
const RED: [u8; 3] = [255, 0, 0];
/// `$color.brand` in dark mode.
const BLUE: [u8; 3] = [0, 0, 255];
/// The fill of the secondary Button.
const ORANGE: [u8; 3] = [255, 128, 0];
/// The fill of the Chip, which is not a Button.
const YELLOW: [u8; 3] = [255, 255, 0];
/// The cart icon.
const GREEN: [u8; 3] = [0, 160, 0];
/// The heart icon.
const CYAN: [u8; 3] = [0, 255, 255];
/// The badge of a Button.
const GRAY: [u8; 3] = [32, 32, 32];

// ---------------------------------------------------------------------------
// The design system
// ---------------------------------------------------------------------------
//
// Tokens, in the modes light and dark:
//
//   color.surface  #ffffff / #000000
//   color.brand    #ff0000 / #0000ff
//   color.ink      #000000 / #ffffff
//   spacing.pad    10px    / 30px
//
// Components:
//
//   Cart (20 x 20, green) and Heart (20 x 20, cyan), with no layers.
//   Button (primary), 120 x 40, filled with $color.brand, its layers placed
//   at x and y:
//     Icon   (10, 10)  an instance of Cart, bound to showIcon and icon
//     Label  (40, 10)  a 16 px text "Buy" in $color.ink, bound to label
//     Badge  (100, 0)  an 8 x 8 frame, #202020
//   with the properties style (variant primary | secondary, default
//   primary), showIcon (boolean, true), label (text, "Buy") and icon (slot,
//   Cart).
//   Chip, 120 x 40, yellow: the same properties with style secondary, but
//   another name. It comes before the secondary Button in the list.
//   Button (secondary), 120 x 40, orange: the same properties with style
//   secondary.
//   Broken, 60 x 30, whose layer Stain (10, 5, 40 x 20) is filled with
//   $color.gone, a token the design system does not have.

const CART: &str = "cc000001";
const HEART: &str = "cc000002";
const BUTTON: &str = "cb000001";
const ICON: &str = "cn000001";
const LABEL: &str = "ct000001";
const BADGE: &str = "cg000001";
const CHIP: &str = "cd000001";
const BUTTON_SECONDARY: &str = "cb000002";
const BROKEN: &str = "ck000001";
const STAIN: &str = "ck000002";

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
            "ink": per_mode("#000000", "#ffffff")
        },
        "spacing": {
            "$type": "dimension",
            "pad": per_mode("10px", "30px")
        }
    })
}

fn button_props(style: &str) -> Value {
    json!({
        "style": {"type": "variant", "values": ["primary", "secondary"], "default": style},
        "showIcon": {"type": "boolean", "default": true},
        "label": {"type": "text", "default": "Buy"},
        "icon": {"type": "slot", "default": CART}
    })
}

fn component(id: &str, name: &str, width: f64, height: f64, fill: &str) -> Value {
    json!({
        "id": id,
        "type": "component",
        "name": name,
        "width": fixed(width),
        "height": fixed(height),
        "layout": {"mode": "none"},
        "fills": solid(fill)
    })
}

fn components() -> Vec<Value> {
    let icon = json!({
        "id": ICON,
        "type": "instance",
        "name": "Icon",
        "component": CART,
        "x": 10,
        "y": 10,
        "bind": {"visible": "showIcon", "component": "icon"}
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
    let badge = json!({
        "id": BADGE,
        "type": "frame",
        "name": "Badge",
        "x": 100,
        "y": 0,
        "width": fixed(8.0),
        "height": fixed(8.0),
        "layout": {"mode": "none"},
        "fills": solid("#202020")
    });
    let button = with(
        &with(
            &component(BUTTON, "Button", 120.0, 40.0, "$color.brand"),
            "props",
            button_props("primary"),
        ),
        "children",
        json!([icon, label, badge]),
    );
    let chip = with(
        &component(CHIP, "Chip", 120.0, 40.0, "#ffff00"),
        "props",
        button_props("secondary"),
    );
    let secondary = with(
        &component(BUTTON_SECONDARY, "Button", 120.0, 40.0, "#ff8000"),
        "props",
        button_props("secondary"),
    );
    let stain = json!({
        "id": STAIN,
        "type": "frame",
        "name": "Stain",
        "x": 10,
        "y": 5,
        "width": fixed(40.0),
        "height": fixed(20.0),
        "layout": {"mode": "none"},
        "fills": solid("$color.gone")
    });
    let broken = with(
        &component(BROKEN, "Broken", 60.0, 30.0, "#ffffff"),
        "children",
        json!([stain]),
    );
    vec![
        component(CART, "Cart", 20.0, 20.0, "#00a000"),
        component(HEART, "Heart", 20.0, 20.0, "#00ffff"),
        button,
        chip,
        secondary,
        broken,
    ]
}

fn system_json() -> Value {
    with(&tokens(), "$components", Value::Array(components()))
}

fn system() -> DesignSystem {
    DesignSystem::from_json(&system_json())
        .unwrap_or_else(|error| panic!("the design system fixture must be read, but: {error}"))
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------
//
// One board, 400 x 300 at the page origin, filled with $color.surface, with
// these instances of Button (each 120 x 40, at x and y on the board):
//
//   PLAIN      (20, 20)    no props and no overrides
//   SECONDARY  (160, 20)   style: secondary
//   NO_ICON    (20, 80)    showIcon: false
//   CHECKOUT   (160, 80)   label: "Checkout"
//   HEARTS     (20, 140)   icon: Heart
//   OVERRIDDEN (160, 140)  overrides: the Badge filled yellow, the Label's
//                          content "Pay now", the Icon hidden
//   PAY_NOW    (20, 200)   label: "Pay now"
//
// and PADDED (160, 200), a frame that hugs its one child, a 20 x 20 Swatch,
// with a padding of $spacing.pad on every side.
//
// Inside a Button at (x, y) the background shows at (x + 85, y + 35), the
// Icon at (x + 20, y + 20) and the Badge at (x + 104, y + 4).

const BOARD: &str = "a0000001";
const PLAIN: &str = "e0000001";
const SECONDARY: &str = "e0000002";
const NO_ICON: &str = "e0000003";
const CHECKOUT: &str = "e0000004";
const HEARTS: &str = "e0000005";
const OVERRIDDEN: &str = "e0000006";
const PAY_NOW: &str = "e0000007";
const PADDED: &str = "f0000001";
const SWATCH: &str = "f0000002";

fn instance(id: &str, x: f64, y: f64, props: Value, overrides: Value) -> Value {
    let mut instance = json!({
        "id": id,
        "type": "instance",
        "name": format!("Button {id}"),
        "component": BUTTON,
        "x": x,
        "y": y
    });
    if props != json!({}) {
        instance = with(&instance, "props", props);
    }
    if overrides != json!({}) {
        instance = with(&instance, "overrides", overrides);
    }
    instance
}

fn page_json() -> Value {
    let padded = json!({
        "id": PADDED,
        "type": "frame",
        "name": "Padded",
        "x": 160,
        "y": 200,
        "width": "hug",
        "height": "hug",
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
            "id": SWATCH,
            "type": "frame",
            "name": "Swatch",
            "width": fixed(20.0),
            "height": fixed(20.0),
            "layout": {"mode": "none"},
            "fills": solid("$color.ink")
        }]
    });
    let children = vec![
        instance(PLAIN, 20.0, 20.0, json!({}), json!({})),
        instance(
            SECONDARY,
            160.0,
            20.0,
            json!({"style": "secondary"}),
            json!({}),
        ),
        instance(NO_ICON, 20.0, 80.0, json!({"showIcon": false}), json!({})),
        instance(
            CHECKOUT,
            160.0,
            80.0,
            json!({"label": "Checkout"}),
            json!({}),
        ),
        instance(HEARTS, 20.0, 140.0, json!({"icon": HEART}), json!({})),
        instance(
            OVERRIDDEN,
            160.0,
            140.0,
            json!({}),
            json!({
                BADGE: {"fills": solid("#ffff00")},
                LABEL: {"content": "Pay now"},
                ICON: {"visible": false}
            }),
        ),
        instance(PAY_NOW, 20.0, 200.0, json!({"label": "Pay now"}), json!({})),
        padded,
    ];
    json!({
        "id": "p0000001",
        "type": "page",
        "name": "Page",
        "children": [{
            "id": BOARD,
            "type": "board",
            "name": "Board",
            "x": 0,
            "y": 0,
            "width": fixed(WIDTH as f64),
            "height": fixed(HEIGHT as f64),
            "layout": {"mode": "none"},
            "fills": solid("$color.surface"),
            "children": children
        }]
    })
}

fn scene() -> Scene {
    Scene::with_system(node(&page_json()), Assets::new(), system())
}

// ---------------------------------------------------------------------------
// A page with what is missing
// ---------------------------------------------------------------------------
//
// One white board, 400 x 300, with:
//
//   GHOST (20, 20, 100 x 40)   an instance of zzzzzzzz, which is no component
//   NOPE  (20, 100, 100 x 40)  a frame filled with $color.nope, no token
//   STAINED (20, 180)          an instance of Broken, whose Stain is filled
//                              with $color.gone, no token either

const MISSING_COMPONENT: &str = "zzzzzzzz";
const GHOST: &str = "e0000010";
const NOPE: &str = "f0000010";
const STAINED: &str = "e0000011";

fn broken_page_json() -> Value {
    json!({
        "id": "p0000002",
        "type": "page",
        "name": "Broken page",
        "children": [{
            "id": "a0000002",
            "type": "board",
            "name": "Broken board",
            "x": 0,
            "y": 0,
            "width": fixed(WIDTH as f64),
            "height": fixed(HEIGHT as f64),
            "layout": {"mode": "none"},
            "fills": solid("#ffffff"),
            "children": [
                {
                    "id": GHOST,
                    "type": "instance",
                    "name": "Ghost",
                    "component": MISSING_COMPONENT,
                    "x": 20,
                    "y": 20,
                    "width": fixed(100.0),
                    "height": fixed(40.0)
                },
                {
                    "id": NOPE,
                    "type": "frame",
                    "name": "Nope",
                    "x": 20,
                    "y": 100,
                    "width": fixed(100.0),
                    "height": fixed(40.0),
                    "layout": {"mode": "none"},
                    "fills": solid("$color.nope")
                },
                {
                    "id": STAINED,
                    "type": "instance",
                    "name": "Stained",
                    "component": BROKEN,
                    "x": 20,
                    "y": 180
                }
            ]
        }]
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn id(text: &str) -> NodeId {
    text.parse().expect("the fixture uses valid node IDs")
}

fn addr(node: &str) -> NodeAddress {
    NodeAddress::new(None, id(node))
}

/// The address of `node` of the component inside `instance`.
fn inside(instance: &str, node: &str) -> NodeAddress {
    NodeAddress::new(Some(id(instance)), id(node))
}

fn token(reference: &str) -> TokenRef {
    reference
        .parse()
        .expect("the fixture uses valid token references")
}

fn view() -> Viewport {
    Viewport::new(WIDTH as f32, HEIGHT as f32)
}

struct Pixels {
    rgba: Vec<u8>,
}

impl Pixels {
    fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        assert!(x < WIDTH && y < HEIGHT, "({x}, {y}) is off the canvas");
        let i = (y as usize * WIDTH as usize + x as usize) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }

    /// The share of the pixels of the box `[x, y, width, height]` that are
    /// clearly not white.
    fn marked_share(&self, [x, y, width, height]: [u32; 4]) -> f64 {
        let mut marked = 0;
        for py in y..y + height {
            for px in x..x + width {
                if self.rgb(px, py).iter().any(|&channel| channel < 192) {
                    marked += 1;
                }
            }
        }
        f64::from(marked) / f64::from(width * height)
    }
}

/// Draws what the scene shows at 100% from the page origin, one canvas pixel
/// to a CSS pixel.
fn render(scene: &mut Scene) -> Pixels {
    let (_, rgba) = scene.render_rgba(&view(), 1.0);
    Pixels { rgba }
}

#[track_caller]
fn assert_rgb(pixels: &Pixels, (x, y): (u32, u32), expected: [u8; 3], what: &str) {
    let actual = pixels.rgb(x, y);
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(&a, e)| a.abs_diff(e) <= 2),
        "{what}: the pixel at ({x}, {y}) must be {expected:?}, but it is {actual:?}"
    );
}

/// Where the background of a Button at (`x`, `y`) shows.
fn background(x: u32, y: u32) -> (u32, u32) {
    (x + 85, y + 35)
}

/// Where the Icon of a Button at (`x`, `y`) shows.
fn icon(x: u32, y: u32) -> (u32, u32) {
    (x + 20, y + 20)
}

/// Where the Badge of a Button at (`x`, `y`) shows.
fn badge(x: u32, y: u32) -> (u32, u32) {
    (x + 104, y + 4)
}

#[track_caller]
fn bounds(scene: &Scene, address: &NodeAddress) -> LayoutBox {
    scene
        .bounds_of(address)
        .unwrap_or_else(|| panic!("{address} must have bounds"))
}

fn page_box(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
    LayoutBox {
        x,
        y,
        width,
        height,
    }
}

// ---------------------------------------------------------------------------
// The design system
// ---------------------------------------------------------------------------

#[test]
fn a_design_system_version_is_read_with_its_modes() {
    let system = system();
    assert_eq!(system.modes(), ["light", "dark"], "the modes, in order");
}

#[test]
fn a_design_system_that_is_not_a_valid_version_is_refused() {
    let unknown_group = with(&system_json(), "sizes", json!({"$type": "dimension"}));
    assert!(
        DesignSystem::from_json(&unknown_group).is_err(),
        "a token tree that fails the token checks is refused"
    );
    let not_a_list = with(&system_json(), "$components", json!({"cb000001": {}}));
    assert!(
        DesignSystem::from_json(&not_a_list).is_err(),
        "`$components` is a list of component nodes"
    );
}

// ---------------------------------------------------------------------------
// Expanding instances
// ---------------------------------------------------------------------------

#[test]
fn an_instance_draws_its_components_tree_with_the_default_property_values() {
    let mut scene = scene();
    let pixels = render(&mut scene);
    assert_rgb(
        &pixels,
        background(20, 20),
        RED,
        "the Button's fill, $color.brand in light mode",
    );
    assert_rgb(
        &pixels,
        icon(20, 20),
        GREEN,
        "the Icon layer, which shows the default of the icon slot, Cart",
    );
    assert_rgb(&pixels, badge(20, 20), GRAY, "the Badge layer");
    assert_eq!(
        scene.bounds_of(&addr(PLAIN)),
        Some(page_box(20.0, 20.0, 120.0, 40.0)),
        "an instance that leaves its size out takes its component's size"
    );
}

#[test]
fn a_variant_value_shows_the_variant_of_the_set_with_that_value() {
    let mut scene = scene();
    let pixels = render(&mut scene);
    let at = background(160, 20);
    let actual = pixels.rgb(at.0, at.1);
    assert_ne!(
        actual, YELLOW,
        "the Chip has the same properties, but it is not a Button, so it is no variant of the Button set"
    );
    assert_rgb(
        &pixels,
        at,
        ORANGE,
        "style: secondary shows the Button whose style is secondary",
    );
    assert_rgb(
        &pixels,
        background(20, 20),
        RED,
        "the default style, primary, shows the instance's own component",
    );
}

#[test]
fn a_boolean_property_shows_or_hides_the_layer_bound_to_it() {
    let mut scene = scene();
    let pixels = render(&mut scene);
    assert_rgb(
        &pixels,
        icon(20, 20),
        GREEN,
        "showIcon defaults to true, so the Icon shows",
    );
    assert_rgb(
        &pixels,
        icon(20, 80),
        RED,
        "showIcon: false hides the Icon, so the Button's fill shows there",
    );
    let (x, y) = icon(20, 80);
    assert_eq!(
        scene.node_at((x as f32, y as f32)),
        Some(addr(NO_ICON)),
        "a hidden layer cannot be hit: the instance itself is under the pointer"
    );
}

#[test]
fn a_text_property_replaces_the_text_of_the_label_bound_to_it() {
    let scene = scene();
    let buy = bounds(&scene, &inside(PLAIN, LABEL));
    let checkout = bounds(&scene, &inside(CHECKOUT, LABEL));
    assert!(
        checkout.width > buy.width + 20.0,
        "label: \"Checkout\" lays the Label out with that text, wider than the default \"Buy\" ({} px), but it is {} px wide",
        buy.width,
        checkout.width
    );
    assert_eq!(
        (checkout.x, checkout.y, checkout.height),
        (200.0, 90.0, 20.0),
        "the Label stays where the component places it, one 20 px line tall"
    );
}

#[test]
fn a_slot_property_swaps_the_component_of_the_instance_layer_bound_to_it() {
    let mut scene = scene();
    let pixels = render(&mut scene);
    assert_rgb(
        &pixels,
        icon(20, 140),
        CYAN,
        "icon: Heart places the Heart in the Icon layer",
    );
}

#[test]
fn overrides_change_the_nodes_of_the_component_they_name() {
    let mut scene = scene();
    let pixels = render(&mut scene);
    assert_rgb(
        &pixels,
        badge(160, 140),
        YELLOW,
        "a fills override fills the Badge yellow",
    );
    assert_rgb(
        &pixels,
        badge(20, 20),
        GRAY,
        "an override changes only the instance that has it",
    );
    assert_rgb(
        &pixels,
        icon(160, 140),
        RED,
        "a visible override hides the Icon, so the Button's fill shows there",
    );
    let overridden = bounds(&scene, &inside(OVERRIDDEN, LABEL));
    let pay_now = bounds(&scene, &inside(PAY_NOW, LABEL));
    let buy = bounds(&scene, &inside(PLAIN, LABEL));
    assert!(
        (overridden.width - pay_now.width).abs() < 0.01 && overridden.width != buy.width,
        "a content override lays the Label out with \"Pay now\": {} px wide, as with label: \"Pay now\" ({} px), not \"Buy\" ({} px)",
        overridden.width,
        pay_now.width,
        buy.width
    );
}

// ---------------------------------------------------------------------------
// Tokens and modes
// ---------------------------------------------------------------------------

#[test]
fn token_references_are_resolved_in_the_first_mode_at_first() {
    let mut scene = scene();
    let pixels = render(&mut scene);
    assert_rgb(&pixels, (390, 290), WHITE, "the board's $color.surface");
    assert_rgb(&pixels, background(20, 20), RED, "$color.brand");
    assert_rgb(&pixels, (180, 220), BLACK, "the Swatch's $color.ink");
    assert_eq!(
        scene.bounds_of(&addr(SWATCH)),
        Some(page_box(170.0, 210.0, 20.0, 20.0)),
        "a padding of $spacing.pad, 10 px in light mode"
    );
    assert!(scene.errors().is_empty(), "nothing is missing");
}

#[test]
fn switching_the_mode_resolves_every_token_in_the_new_mode() {
    let mut scene = scene();
    render(&mut scene);
    scene
        .set_mode("dark")
        .unwrap_or_else(|error| panic!("dark is a mode of the design system, but: {error}"));
    let dark = render(&mut scene);
    assert_rgb(&dark, (390, 290), BLACK, "the board's $color.surface");
    assert_rgb(
        &dark,
        background(20, 20),
        BLUE,
        "$color.brand inside an instance",
    );
    assert_rgb(
        &dark,
        background(160, 20),
        ORANGE,
        "a literal color is the same in every mode",
    );
    assert_rgb(&dark, (200, 240), WHITE, "the Swatch's $color.ink");
    assert_eq!(
        scene.bounds_of(&addr(SWATCH)),
        Some(page_box(190.0, 230.0, 20.0, 20.0)),
        "a padding of $spacing.pad, 30 px in dark mode, lays the page out again"
    );
    assert!(scene.errors().is_empty(), "nothing is missing in dark mode");

    scene.set_mode("light").expect("light is a mode too");
    let light = render(&mut scene);
    assert_rgb(&light, background(20, 20), RED, "back in light mode");
}

#[test]
fn a_mode_the_design_system_does_not_list_is_refused() {
    let mut scene = scene();
    scene.set_mode("dark").expect("dark is a mode");
    assert!(
        scene.set_mode("sepia").is_err(),
        "sepia is not a mode of the design system"
    );
    let pixels = render(&mut scene);
    assert_rgb(
        &pixels,
        background(20, 20),
        BLUE,
        "the scene stays in dark mode",
    );
}

// ---------------------------------------------------------------------------
// What is missing
// ---------------------------------------------------------------------------

#[test]
fn a_missing_component_is_drawn_as_an_error_placeholder_and_reported() {
    let mut scene = Scene::with_system(node(&broken_page_json()), Assets::new(), system());
    let pixels = render(&mut scene);
    let share = pixels.marked_share([20, 20, 100, 40]);
    assert!(
        share >= 0.05,
        "the box of an instance of a missing component is visibly marked, but only {:.1}% of it differs from the white board",
        share * 100.0
    );
    assert_rgb(
        &pixels,
        (200, 40),
        WHITE,
        "the placeholder stays in the instance's box",
    );
    let expected = RenderError {
        node: addr(GHOST),
        missing: Missing::Component(id(MISSING_COMPONENT)),
    };
    assert!(
        scene.errors().contains(&expected),
        "the host page hears of {expected:?}, but the errors are {:?}",
        scene.errors()
    );
}

#[test]
fn a_missing_token_is_drawn_as_an_error_placeholder_and_reported() {
    let mut scene = Scene::with_system(node(&broken_page_json()), Assets::new(), system());
    let pixels = render(&mut scene);
    for (what, area) in [
        ("the frame filled with $color.nope", [20, 100, 100, 40]),
        (
            "the Stain inside the instance of Broken, filled with $color.gone",
            [30, 185, 40, 20],
        ),
    ] {
        let share = pixels.marked_share(area);
        assert!(
            share >= 0.05,
            "{what} is visibly marked, but only {:.1}% of its box differs from white",
            share * 100.0
        );
    }
    for expected in [
        RenderError {
            node: addr(NOPE),
            missing: Missing::Token(token("$color.nope")),
        },
        RenderError {
            node: inside(STAINED, STAIN),
            missing: Missing::Token(token("$color.gone")),
        },
    ] {
        assert!(
            scene.errors().contains(&expected),
            "the host page hears of {expected:?}, but the errors are {:?}",
            scene.errors()
        );
    }
}

// ---------------------------------------------------------------------------
// Addresses inside instances
// ---------------------------------------------------------------------------

#[test]
fn bounds_of_a_node_inside_an_instance_is_its_box_in_page_coordinates() {
    let scene = scene();
    assert_eq!(
        scene.bounds_of(&inside(PLAIN, BADGE)),
        Some(page_box(120.0, 20.0, 8.0, 8.0)),
        "the Badge, at (100, 0) in the instance at (20, 20)"
    );
    assert_eq!(
        scene.bounds_of(&inside(PLAIN, ICON)),
        Some(page_box(30.0, 30.0, 20.0, 20.0)),
        "the Icon, which takes the size of Cart"
    );
    let label = bounds(&scene, &inside(PLAIN, LABEL));
    assert_eq!(
        (label.x, label.y, label.height),
        (60.0, 30.0, 20.0),
        "the Label, at (40, 10) in the instance"
    );
    assert_eq!(
        scene.bounds_of(&inside(PLAIN, "zzzzzzzz")),
        None,
        "a node the component does not have"
    );
}

#[test]
fn node_at_gives_the_address_of_a_node_inside_an_instance() {
    let scene = scene();
    for (point, expected, case) in [
        ((124.0, 24.0), inside(PLAIN, BADGE), "the Badge"),
        ((40.0, 40.0), inside(PLAIN, ICON), "the Icon"),
        ((65.0, 40.0), inside(PLAIN, LABEL), "the Label"),
        (
            (105.0, 55.0),
            addr(PLAIN),
            "the instance, where no layer is",
        ),
        ((390.0, 290.0), addr(BOARD), "the board"),
    ] {
        assert_eq!(scene.node_at(point), Some(expected), "{case}");
    }
}

#[test]
fn selecting_a_node_inside_an_instance_reports_and_outlines_its_address() {
    let mut scene = scene();
    let view = view();
    scene.click(
        &view,
        (124.0, 24.0),
        Modifiers {
            ctrl: false,
            meta: true,
        },
    );
    assert_eq!(
        scene.take_events(),
        vec![Event::Select(Some(inside(PLAIN, BADGE)))],
        "Cmd + click selects the deepest node, the Badge inside the instance"
    );
    let outlines = scene.overlay(&view);
    let selection = outlines
        .iter()
        .find(|outline| outline.kind == OutlineKind::Selection)
        .expect("the selection is outlined");
    assert_eq!(selection.node, inside(PLAIN, BADGE));
    assert_eq!(selection.label, "Badge");
    assert_eq!(selection.rect, page_box(120.0, 20.0, 8.0, 8.0));
}
