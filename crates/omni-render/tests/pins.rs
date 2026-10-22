//! US-045: what the design viewer needs from the renderer to pin comments on
//! nodes. A comment pin sits on its node's box over the canvas and follows
//! zooming and panning, so the viewer asks where each commented node is on
//! the screen whenever the view changes.
//!
//! `omni_render::view::Scene::screen_box_of(&address, &viewport)` answers it:
//! the box of the node at `address` (a node ID, or
//! `<instanceId>/<componentNodeId>` for a node inside an instance) in screen
//! CSS pixels of `viewport`, from the screen's top-left corner, at the
//! viewport's zoom; `None` when no node of the scene has that address. It is
//! `Scene::bounds_of` as the viewport shows it.
//!
//! The WebAssembly module forwards it to the web app (the last test):
//!
//! - `omni_render_node_box(address, len) -> *const c_char`:
//!   `Scene::screen_box_of` of the loaded scene under the module's current
//!   view (the one that wheel and pan events change), as a NUL-ended JSON
//!   object `{"x", "y", "width", "height"}` that stays until the next call,
//!   or null when the address names no node.
//!
//! That pins follow the canvas in a browser, and that a comment on a node
//! inside an instance lands on that exact node, are the human checks.

mod common;
mod design;

use design::{fixed, node, with};
use omni_core::design::NodeAddress;
use omni_render::layout::LayoutBox;
use omni_render::paint::Assets;
use omni_render::system::DesignSystem;
use omni_render::view::{Scene, Viewport};
use serde_json::{Value, json};
use std::fs;
use std::process::Output;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// The design system and the page
// ---------------------------------------------------------------------------
//
// Components:
//
//   Cart (20 x 20), with one layer, Wheel (a 6 x 6 frame at 2, 12).
//   Button (120 x 40), with one layer, Icon (an instance of Cart at 10, 10).
//
// The page has two boards:
//
//   Checkout (0, 0, 400 x 300): Buy, an instance of Button at (20, 20), and
//   Summary, a 200 x 100 frame at (20, 100).
//   Receipt (500, 0, 200 x 200), empty.
//
// So in page coordinates Summary is (20, 100, 200 x 100), Buy (20, 20,
// 120 x 40), and the Wheel inside Buy's Icon (20 + 10 + 2, 20 + 10 + 12) =
// (32, 42, 6 x 6).

const CART: &str = "cc000001";
const WHEEL: &str = "cw000001";
const BUTTON: &str = "cb000001";
const ICON: &str = "cn000001";

const CHECKOUT: &str = "a0000001";
const RECEIPT: &str = "a0000002";
const BUY: &str = "e0000001";
const SUMMARY: &str = "f0000001";

fn solid(color: &str) -> Value {
    json!([{"type": "solid", "color": color}])
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
    let icon = json!({
        "id": ICON,
        "type": "instance",
        "name": "Icon",
        "component": CART,
        "x": 10,
        "y": 10
    });
    let button = json!({
        "id": BUTTON,
        "type": "component",
        "name": "Button",
        "width": fixed(120.0),
        "height": fixed(40.0),
        "layout": {"mode": "none"},
        "fills": solid("#ff0000"),
        "children": [icon]
    });
    json!([cart, button])
}

fn system() -> DesignSystem {
    let tokens = json!({"$extensions": {"omnistate": {"modes": ["light"]}}});
    DesignSystem::from_json(&with(&tokens, "$components", components()))
        .unwrap_or_else(|error| panic!("the design system fixture must be read, but: {error}"))
}

fn page_json() -> Value {
    json!({
        "id": "p0000001",
        "type": "page",
        "name": "Mobile",
        "children": [
            {
                "id": CHECKOUT,
                "type": "board",
                "name": "Checkout",
                "x": 0,
                "y": 0,
                "width": fixed(400.0),
                "height": fixed(300.0),
                "layout": {"mode": "none"},
                "fills": solid("#ffffff"),
                "children": [
                    {
                        "id": BUY,
                        "type": "instance",
                        "name": "Buy",
                        "component": BUTTON,
                        "x": 20,
                        "y": 20
                    },
                    {
                        "id": SUMMARY,
                        "type": "frame",
                        "name": "Summary",
                        "x": 20,
                        "y": 100,
                        "width": fixed(200.0),
                        "height": fixed(100.0),
                        "layout": {"mode": "none"}
                    }
                ]
            },
            {
                "id": RECEIPT,
                "type": "board",
                "name": "Receipt",
                "x": 500,
                "y": 0,
                "width": fixed(200.0),
                "height": fixed(200.0),
                "layout": {"mode": "none"},
                "fills": solid("#ffffff")
            }
        ]
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

fn screen_box(scene: &Scene, text: &str, viewport: &Viewport) -> LayoutBox {
    scene
        .screen_box_of(&address(text), viewport)
        .unwrap_or_else(|| {
            panic!("the scene has a node at `{text}`, so it has a box on the screen")
        })
}

fn assert_box(actual: LayoutBox, expected: (f32, f32, f32, f32), what: &str) {
    let close = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(
        close(actual.x, expected.0)
            && close(actual.y, expected.1)
            && close(actual.width, expected.2)
            && close(actual.height, expected.3),
        "{what}: expected (x {}, y {}, {} x {}) on the screen, got {actual:?}",
        expected.0,
        expected.1,
        expected.2,
        expected.3
    );
}

// ---------------------------------------------------------------------------
// The box of a node on the screen
// ---------------------------------------------------------------------------

#[test]
fn at_the_origin_and_100_percent_a_node_is_where_the_page_has_it() {
    let scene = scene();
    let viewport = Viewport::new(800.0, 600.0);
    assert_box(
        screen_box(&scene, SUMMARY, &viewport),
        (20.0, 100.0, 200.0, 100.0),
        "Summary",
    );
    assert_box(
        screen_box(&scene, RECEIPT, &viewport),
        (500.0, 0.0, 200.0, 200.0),
        "the board Receipt",
    );
}

#[test]
fn the_box_follows_the_zoom_and_the_pan() {
    let scene = scene();
    // Page point (10, 5) at the screen's top-left corner, at 200%.
    let mut viewport = Viewport::at(10.0, 5.0, 2.0, 800.0, 600.0);
    assert_box(
        screen_box(&scene, SUMMARY, &viewport),
        (20.0, 190.0, 400.0, 200.0),
        "Summary at 200% from (10, 5)",
    );

    // Panning 30 screen pixels right and 20 up moves the box the other way.
    viewport.pan_by(30.0, -20.0);
    assert_box(
        screen_box(&scene, SUMMARY, &viewport),
        (-10.0, 210.0, 400.0, 200.0),
        "Summary after a pan",
    );

    // Zooming out to 100% around the screen's top-left corner keeps the page
    // point there, (25, -5).
    viewport.zoom_at(0.5, (0.0, 0.0));
    assert_box(
        screen_box(&scene, SUMMARY, &viewport),
        (-5.0, 105.0, 200.0, 100.0),
        "Summary after zooming out",
    );
}

#[test]
fn a_node_inside_an_instance_has_its_own_box() {
    let scene = scene();
    let viewport = Viewport::at(10.0, 5.0, 2.0, 800.0, 600.0);
    assert_box(
        screen_box(&scene, BUY, &viewport),
        (20.0, 30.0, 240.0, 80.0),
        "the instance Buy",
    );
    assert_box(
        screen_box(&scene, &inside(BUY, WHEEL), &viewport),
        (44.0, 74.0, 12.0, 12.0),
        "the Wheel inside the Icon inside Buy",
    );
}

#[test]
fn an_address_that_names_no_node_has_no_box() {
    let scene = scene();
    let viewport = Viewport::new(800.0, 600.0);
    assert_eq!(scene.screen_box_of(&address("zzzzzzzz"), &viewport), None);
    assert_eq!(
        scene.screen_box_of(&address(&inside(SUMMARY, WHEEL)), &viewport),
        None,
        "Summary is no instance, so nothing is inside it"
    );
}

// ---------------------------------------------------------------------------
// The WebAssembly export
// ---------------------------------------------------------------------------

/// Runs `make render-wasm` once for every test in this file that needs it.
fn render_wasm() -> &'static Output {
    static BUILD: OnceLock<Output> = OnceLock::new();
    let output = BUILD.get_or_init(|| common::make(&["render-wasm"]));
    assert!(
        output.status.success(),
        "`make render-wasm` failed:\n{}",
        common::describe(output)
    );
    output
}

#[test]
fn the_wasm_module_exports_the_box_of_a_node_on_the_screen() {
    render_wasm();
    let path = common::repo_root().join("render-sandbox/build/omni_render.js");
    let loader =
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"));
    assert!(
        loader.contains("_omni_render_node_box"),
        "render-sandbox/build/omni_render.js must export `_omni_render_node_box` after \
         `make render-wasm`, for the design viewer's comment pins"
    );
}
