//! US-043, US-126: what the design viewer needs from the renderer for its
//! layers panel, which lists the boards of the page shown, folded, opens a
//! board's node tree, and keeps its selection in step with the canvas.
//!
//! `omni_render::view::Scene` answers both, with the types of
//! `omni_render::select`:
//!
//! - `Scene::layers()` lists the boards of the page in page order, each as a
//!   `Layer {address, name, kind, children}` with `children` empty.
//! - `Scene::layers_of(&address)` gives the node at the address as a `Layer`
//!   with every node below it in document order; `None` for an address that
//!   names no node of the page. A scene made with a design system lists the
//!   layers of each instance's component under the instance, addressed
//!   `<instanceId>/<componentNodeId>` (a node inside an instance inside an
//!   instance is addressed from the instance of the page, as `Scene::node_at`
//!   gives it). `kind` is the node's type as the design has it, so an
//!   instance is listed as an `instance` although it is drawn as a frame of
//!   its component's layers. A board not read yet (a placeholder) has no
//!   children.
//! - `Scene::board_of(&address)` names the board that holds the node, a
//!   board's own ID for the board; `None` for an address not on the page.
//! - `Scene::select_address(Some(&address))` selects the node at that
//!   address, as a click in the panel does: it is the selection from then on
//!   (`Scene::selected`, the `selection` outline of `Scene::overlay`), and an
//!   `Event::Select` with the address is queued for `take_events`, once per
//!   change, as for a click on the canvas. `None` clears the selection. An
//!   address that names no node of the scene changes nothing and gives
//!   `false`.
//!
//! The WebAssembly module forwards both to the web app (the last test):
//!
//! - `omni_render_scene_layers() -> *const c_char`: `Scene::layers` of the
//!   loaded scene as a NUL-ended JSON array that stays until the next call:
//!   `[{"address": "<address>", "name": "Cart", "type": "board", "children":
//!   []}]`.
//! - `omni_render_scene_layers_of(address, len)` and
//!   `omni_render_scene_board_of(address, len)`: `Scene::layers_of` as one
//!   such JSON object and `Scene::board_of` as text, null for no node.
//! - `omni_render_select_node(address, len) -> bool`:
//!   `Scene::select_address` for an address as text; empty text clears the
//!   selection.
//!
//! That selection stays in step between the canvas and the panel in a
//! browser is the human check.

mod common;
mod design;

use design::{fixed, node, with};
use omni_core::design::NodeAddress;
use omni_render::paint::Assets;
use omni_render::select::{Event, Layer, OutlineKind};
use omni_render::system::DesignSystem;
use omni_render::view::{Scene, Viewport};
use serde_json::{Value, json};
use std::fs;

// ---------------------------------------------------------------------------
// The design system and the page
// ---------------------------------------------------------------------------
//
// Components:
//
//   Cart (20 x 20), with one layer, Wheel (a 6 x 6 frame at 2, 12).
//   Button (120 x 40), with two layers: Label (a text at 40, 10) and Icon
//   (an instance of Cart at 10, 10).
//
// The page has two boards:
//
//   Checkout (0, 0, 400 x 300): Buy, an instance of Button at (20, 20), and
//   Summary, a 200 x 100 frame at (20, 100) holding the text Total.
//   Receipt (500, 0, 200 x 200), empty.

const CART: &str = "cc000001";
const WHEEL: &str = "cw000001";
const BUTTON: &str = "cb000001";
const LABEL: &str = "ct000001";
const ICON: &str = "cn000001";

const CHECKOUT: &str = "a0000001";
const RECEIPT: &str = "a0000002";
const BUY: &str = "e0000001";
const SUMMARY: &str = "f0000001";
const TOTAL: &str = "f0000002";

fn solid(color: &str) -> Value {
    json!([{"type": "solid", "color": color}])
}

fn tokens() -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": ["light"]}},
        "color": {
            "$type": "color",
            "ink": {
                "$value": "#000000",
                "$extensions": {"omnistate": {"modes": {"light": "#000000"}}}
            }
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
        "color": "$color.ink"
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
        "y": 20
    });
    let summary = json!({
        "id": SUMMARY,
        "type": "frame",
        "name": "Summary",
        "x": 20,
        "y": 100,
        "width": fixed(200.0),
        "height": fixed(100.0),
        "layout": {"mode": "none"},
        "children": [{
            "id": TOTAL,
            "type": "text",
            "name": "Total",
            "x": 10,
            "y": 10,
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
                "children": [buy, summary]
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

/// A layer, as the test expects it.
fn layer(address_text: &str, name: &str, kind: &str, children: Vec<Layer>) -> Layer {
    Layer {
        address: address(address_text),
        name: name.to_string(),
        kind: kind.to_string(),
        children,
    }
}

/// The layers as `address name kind` lines, indented by depth, so that a
/// failure shows the whole tree.
fn outline(layers: &[Layer]) -> String {
    fn walk(layers: &[Layer], depth: usize, out: &mut String) {
        for layer in layers {
            out.push_str(&format!(
                "{}{} {} {}\n",
                "  ".repeat(depth),
                layer.address,
                layer.name,
                layer.kind
            ));
            walk(&layer.children, depth + 1, out);
        }
    }
    let mut out = String::new();
    walk(layers, 0, &mut out);
    out
}

// ---------------------------------------------------------------------------
// The layers
// ---------------------------------------------------------------------------

#[test]
fn the_layers_are_the_boards_of_the_page_in_page_order_without_their_children() {
    let expected = vec![
        layer(CHECKOUT, "Checkout", "board", vec![]),
        layer(RECEIPT, "Receipt", "board", vec![]),
    ];
    assert_eq!(scene().layers(), expected);
}

#[test]
fn the_layers_of_a_board_are_every_node_below_it() {
    let expected = layer(
        CHECKOUT,
        "Checkout",
        "board",
        vec![
            layer(
                BUY,
                "Buy",
                "instance",
                vec![
                    layer(&inside(BUY, LABEL), "Label", "text", vec![]),
                    layer(
                        &inside(BUY, ICON),
                        "Icon",
                        "instance",
                        vec![layer(&inside(BUY, WHEEL), "Wheel", "frame", vec![])],
                    ),
                ],
            ),
            layer(
                SUMMARY,
                "Summary",
                "frame",
                vec![layer(TOTAL, "Total", "text", vec![])],
            ),
        ],
    );
    let layers = scene().layers_of(&address(CHECKOUT));
    assert_eq!(
        outline(layers.as_slice()),
        outline(&[expected.clone()]),
        "the layers of a board list every node below it, nodes inside instances by \
         `<instanceId>/<componentNodeId>`"
    );
    assert_eq!(layers, Some(expected));
}

#[test]
fn the_layers_of_a_node_inside_an_instance_are_its_subtree() {
    assert_eq!(
        scene().layers_of(&address(&inside(BUY, ICON))),
        Some(layer(
            &inside(BUY, ICON),
            "Icon",
            "instance",
            vec![layer(&inside(BUY, WHEEL), "Wheel", "frame", vec![])],
        ))
    );
}

#[test]
fn the_layers_of_an_address_not_on_the_page_are_none() {
    let scene = scene();
    assert_eq!(scene.layers_of(&address("zzzzzzzz")), None);
    assert_eq!(scene.layers_of(&address(&inside(BUY, "zzzzzzzz"))), None);
}

// ---------------------------------------------------------------------------
// The board that holds a node
// ---------------------------------------------------------------------------

#[test]
fn the_board_of_a_node_inside_an_instance_is_the_board_holding_the_instance() {
    assert_eq!(
        scene().board_of(&address(&inside(BUY, WHEEL))),
        Some(*address(CHECKOUT).node())
    );
}

#[test]
fn the_board_of_a_board_is_the_board_itself() {
    assert_eq!(
        scene().board_of(&address(RECEIPT)),
        Some(*address(RECEIPT).node())
    );
}

#[test]
fn the_board_of_an_address_not_on_the_page_is_none() {
    assert_eq!(scene().board_of(&address("zzzzzzzz")), None);
}

#[test]
fn every_layer_is_a_node_the_canvas_knows_by_the_same_address() {
    let scene = scene();
    fn walk(scene: &Scene, layers: &[Layer]) {
        for layer in layers {
            assert!(
                scene.bounds_of(&layer.address).is_some(),
                "the layer {} must name a node of the scene, so that selecting it in the \
                 panel selects it on the canvas",
                layer.address
            );
            walk(scene, &layer.children);
        }
    }
    let layers: Vec<Layer> = scene
        .layers()
        .iter()
        .filter_map(|board| scene.layers_of(&board.address))
        .collect();
    assert_eq!(layers.len(), 2, "the page has two boards");
    walk(&scene, &layers);
}

// ---------------------------------------------------------------------------
// Selecting by address
// ---------------------------------------------------------------------------

#[test]
fn selecting_an_address_selects_that_node_on_the_canvas() {
    let mut scene = scene();
    let label = address(&inside(BUY, LABEL));
    assert!(
        scene.select_address(Some(&label)),
        "a node inside an instance can be selected by its address"
    );
    assert_eq!(
        scene.take_events(),
        vec![Event::Select(Some(label.clone()))]
    );
    assert_eq!(scene.selected(), Some(*label.node()));

    let viewport = Viewport::at(0.0, 0.0, 2.0, 800.0, 600.0);
    let outlines = scene.overlay(&viewport);
    let selection: Vec<_> = outlines
        .iter()
        .filter(|outline| outline.kind == OutlineKind::Selection)
        .collect();
    assert_eq!(selection.len(), 1, "one selection outline: {outlines:?}");
    assert_eq!(selection[0].node, label);

    // Selecting it again changes nothing, so it says nothing.
    assert!(scene.select_address(Some(&label)));
    assert_eq!(scene.take_events(), vec![]);

    // Another node, a plain one this time.
    let total = address(TOTAL);
    assert!(scene.select_address(Some(&total)));
    assert_eq!(
        scene.take_events(),
        vec![Event::Select(Some(total.clone()))]
    );
    assert_eq!(scene.selected(), Some(*total.node()));
}

#[test]
fn selecting_none_clears_the_selection() {
    let mut scene = scene();
    assert!(scene.select_address(Some(&address(SUMMARY))));
    scene.take_events();
    assert!(scene.select_address(None));
    assert_eq!(scene.take_events(), vec![Event::Select(None)]);
    assert_eq!(scene.selected(), None);
    let viewport = Viewport::at(0.0, 0.0, 1.0, 800.0, 600.0);
    assert!(
        scene
            .overlay(&viewport)
            .iter()
            .all(|outline| outline.kind != OutlineKind::Selection),
        "nothing is outlined as selected"
    );
}

#[test]
fn an_address_that_names_no_node_changes_nothing() {
    let mut scene = scene();
    assert!(scene.select_address(Some(&address(SUMMARY))));
    scene.take_events();
    for unknown in ["zzzzzzzz", "e0000001/zzzzzzzz", "zzzzzzzz/ct000001"] {
        assert!(
            !scene.select_address(Some(&address(unknown))),
            "{unknown} names no node of the scene"
        );
    }
    assert_eq!(scene.take_events(), vec![]);
    assert_eq!(scene.selected(), Some(*address(SUMMARY).node()));
}

// ---------------------------------------------------------------------------
// The WebAssembly exports
// ---------------------------------------------------------------------------

/// The WebAssembly renderer that `make render-wasm` built before this test run.
fn render_wasm() {
    common::render_wasm();
}

#[test]
fn the_wasm_module_exports_the_layers_and_selection_by_address() {
    render_wasm();
    let path = common::repo_root().join("render-sandbox/build/omni_render.js");
    let loader =
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"));
    for export in [
        "_omni_render_scene_layers",
        "_omni_render_scene_layers_of",
        "_omni_render_scene_board_of",
        "_omni_render_select_node",
    ] {
        assert!(
            loader.contains(export),
            "render-sandbox/build/omni_render.js must export `{export}` after `make render-wasm`, \
             for the design viewer's layers panel"
        );
    }
}
