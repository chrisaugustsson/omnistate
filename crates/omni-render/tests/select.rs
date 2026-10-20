//! US-007: click-to-select and the overlay API.
//!
//! `omni_render::view::Scene` answers which node is at a point and keeps the
//! hover and selection state of the reviewer, with the types of
//! `omni_render::select`. What the tests expect:
//!
//! - `Scene::node_at(point)` takes a page point and returns the address of
//!   the topmost visible node there: the deepest node whose box holds the point, later
//!   siblings above earlier ones, and nothing from a hidden node or its
//!   subtree. A node outside the box of an ancestor with `clip: true` cannot
//!   be hit there, and one that overflows an unclipped ancestor can. On the
//!   2,000-node board of the stress file an answer takes under 1 ms (99% of
//!   them, not counting the work of building the index), and it is the one a
//!   plain walk over the boxes gives.
//! - `Scene::bounds_of(&address)` is the box of a node in page coordinates,
//!   `None` for an address that names no node. Addresses inside component
//!   instances (`<instanceId>/<componentNodeId>`) are covered by
//!   `instances.rs` (US-032).
//! - Selection takes screen points and a `Viewport`, so a click lands on the
//!   node that is drawn under the pointer at any zoom and pan. The context is
//!   the node whose children a click selects among; it starts at the board
//!   (`Scene::context` is the board once a click has landed on one) and is
//!   `None` while nothing is selected.
//!   - `click`: selects the direct child of the context that holds the node
//!     under the pointer, or the board itself if the context has no child
//!     there. A click on another board starts over in that board.
//!   - `double_click`: selects like `click`, then makes the selected node the
//!     context and selects its child under the pointer. With no child under
//!     the pointer, selection and context stay as the click left them.
//!   - `escape`: selects the parent of the selection and moves the context up
//!     one level, so that the context is always the parent of the selection
//!     (the board is its own context). Escape with the board selected clears
//!     the selection, and with nothing selected does nothing.
//!   - `click` with Ctrl or Cmd: selects the deepest node under the pointer,
//!     and its parent becomes the context.
//!   - a click on no board clears the selection and resets the context.
//! - `Scene::pointer_move` sets the hovered node to the one a click at that
//!   point would select (the deepest one with Ctrl or Cmd held), `None` off
//!   the boards.
//! - `Scene::take_events` returns, oldest first, an `Event::Hover` each time
//!   the hovered node changes and an `Event::Select` each time the selected
//!   node changes, each with the node's `NodeAddress` (`None` for none), and
//!   drains them.
//! - `Scene::overlay(&viewport)` gives one `Outline` for the selected node and
//!   one for the hovered node, each labeled with the node's name and with its
//!   box in screen pixels of that viewport, so that the host page can draw
//!   them above the canvas and keep them aligned while it zooms and pans.
//!
//! The WebAssembly exports and the stress page that forward browser events to
//! these methods are covered by `select_sandbox.rs`. Clicking, double-clicking
//! and Esc on the stress file in a browser is the human check.

mod common;
mod design;

use design::{at, board, fixed, node, none, rect, with};
use omni_core::design::{Node, NodeData, NodeId};
use omni_render::layout::{self, LayoutBox};
use omni_render::paint::Assets;
use omni_render::select::{Event, Modifiers, NodeAddress, Outline, OutlineKind};
use omni_render::view::{Scene, Viewport};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

const WIDTH: f32 = 1440.0;
const HEIGHT: f32 = 900.0;

const NONE: Modifiers = Modifiers {
    ctrl: false,
    meta: false,
};
const CTRL: Modifiers = Modifiers {
    ctrl: true,
    meta: false,
};
const META: Modifiers = Modifiers {
    ctrl: false,
    meta: true,
};

// ---------------------------------------------------------------------------
// A page of two boards
// ---------------------------------------------------------------------------
//
// Board A, at (0, 0), 400 x 300:
//
//   Card (20, 20, 200 x 200)
//     Header (20, 20, 200 x 50)
//       Icon (30, 30, 30 x 30)
//     Body (20, 80, 200 x 100)
//       Button (40, 100, 80 x 40)
//   Side (260, 20, 100 x 200)
//
// Board B, at (500, 0), 400 x 400:
//
//   Panel (550, 50, 100 x 100), with a hidden Ghost of the same box above it
//   Back (700, 20, 150 x 150), with Front (760, 100, 120 x 150) above it
//   Clipper (520, 220, 100 x 50), which clips its child
//     Overflow (580, 230, 100 x 30), 40 px of it beyond the Clipper's edge
//   Holder (650, 320, 80 x 40), which does not clip its child
//     Stray (700, 330, 100 x 20), 70 px of it beyond the Holder's edge
//
// All boxes are in page coordinates. The boards are 100 px apart.

const BOARD_A: &str = "a0000001";
const CARD: &str = "a0000002";
const HEADER: &str = "a0000003";
const ICON: &str = "a0000004";
const BODY: &str = "a0000005";
const BUTTON: &str = "a0000006";
const SIDE: &str = "a0000007";

const BOARD_B: &str = "b0000001";
const PANEL: &str = "b0000002";
const GHOST: &str = "b0000003";
const BACK: &str = "b0000004";
const FRONT: &str = "b0000005";
const CLIPPER: &str = "b0000006";
const OVERFLOW: &str = "b0000007";
const HOLDER: &str = "b0000008";
const STRAY: &str = "b0000009";

fn named(json: &Value, name: &str) -> Value {
    with(json, "name", json!(name))
}

fn box_node(id: &str, name: &str, x: f64, y: f64, width: f64, height: f64) -> Value {
    named(&at(&rect(id, width, height), x, y), name)
}

fn holding(parent: &Value, children: Vec<Value>) -> Value {
    with(parent, "children", Value::Array(children))
}

fn board_a() -> Value {
    let header = holding(
        &box_node(HEADER, "Header", 0.0, 0.0, 200.0, 50.0),
        vec![box_node(ICON, "Icon", 10.0, 10.0, 30.0, 30.0)],
    );
    let body = holding(
        &box_node(BODY, "Body", 0.0, 60.0, 200.0, 100.0),
        vec![box_node(BUTTON, "Button", 20.0, 20.0, 80.0, 40.0)],
    );
    let card = holding(
        &box_node(CARD, "Card", 20.0, 20.0, 200.0, 200.0),
        vec![header, body],
    );
    let side = box_node(SIDE, "Side", 260.0, 20.0, 100.0, 200.0);
    named(
        &at(
            &board(
                BOARD_A,
                fixed(400.0),
                fixed(300.0),
                none(),
                vec![card, side],
            ),
            0.0,
            0.0,
        ),
        "Board A",
    )
}

fn board_b() -> Value {
    let ghost = with(
        &box_node(GHOST, "Ghost", 50.0, 50.0, 100.0, 100.0),
        "visible",
        json!(false),
    );
    let clipper = with(
        &holding(
            &box_node(CLIPPER, "Clipper", 20.0, 220.0, 100.0, 50.0),
            vec![box_node(OVERFLOW, "Overflow", 60.0, 10.0, 100.0, 30.0)],
        ),
        "clip",
        json!(true),
    );
    let holder = with(
        &holding(
            &box_node(HOLDER, "Holder", 150.0, 320.0, 80.0, 40.0),
            vec![box_node(STRAY, "Stray", 50.0, 10.0, 100.0, 20.0)],
        ),
        "clip",
        json!(false),
    );
    let children = vec![
        box_node(PANEL, "Panel", 50.0, 50.0, 100.0, 100.0),
        ghost,
        box_node(BACK, "Back", 200.0, 20.0, 150.0, 150.0),
        box_node(FRONT, "Front", 260.0, 100.0, 120.0, 150.0),
        clipper,
        holder,
    ];
    named(
        &at(
            &board(BOARD_B, fixed(400.0), fixed(400.0), none(), children),
            500.0,
            0.0,
        ),
        "Board B",
    )
}

fn two_boards() -> Value {
    json!({
        "id": "p0000001",
        "type": "page",
        "name": "Page",
        "children": [board_a(), board_b()]
    })
}

fn fresh() -> Scene {
    Scene::new(node(&two_boards()), Assets::new())
}

fn view() -> Viewport {
    Viewport::new(WIDTH, HEIGHT)
}

fn id(id: &str) -> NodeId {
    id.parse().expect("the fixture uses valid node IDs")
}

fn addr(id_text: &str) -> NodeAddress {
    NodeAddress::new(None, id(id_text))
}

fn page_box(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
    LayoutBox {
        x,
        y,
        width,
        height,
    }
}

/// Clicks the page point `at` in a view at 100% from the page origin, where
/// screen and page points are the same.
fn click(scene: &mut Scene, at: (f32, f32)) {
    scene.click(&view(), at, NONE);
}

fn double_click(scene: &mut Scene, at: (f32, f32)) {
    scene.double_click(&view(), at);
}

#[track_caller]
fn assert_state(scene: &Scene, selected: Option<&str>, context: Option<&str>, case: &str) {
    assert_eq!(
        scene.selected(),
        selected.map(id),
        "{case}: the selected node"
    );
    assert_eq!(scene.context(), context.map(id), "{case}: the context");
}

/// A point in each node of the page that no node above it covers.
const ICON_AT: (f32, f32) = (35.0, 35.0);
const HEADER_AT: (f32, f32) = (200.0, 30.0);
const BUTTON_AT: (f32, f32) = (60.0, 110.0);
const BODY_AT: (f32, f32) = (200.0, 170.0);
const CARD_AT: (f32, f32) = (100.0, 200.0);
const SIDE_AT: (f32, f32) = (300.0, 100.0);
const BOARD_A_AT: (f32, f32) = (380.0, 280.0);
const PANEL_AT: (f32, f32) = (600.0, 100.0);
const NO_BOARD_AT: (f32, f32) = (450.0, 100.0);

// ---------------------------------------------------------------------------
// Node addresses
// ---------------------------------------------------------------------------

#[test]
fn a_node_address_is_an_id_or_an_instance_and_a_component_node() {
    let plain: NodeAddress = "k3f9x2qa".parse().expect("an ID is an address");
    assert_eq!(plain.node(), &id("k3f9x2qa"));
    assert_eq!(plain.instance(), None);
    assert_eq!(plain.to_string(), "k3f9x2qa");

    let inside: NodeAddress = "a0000009/b000000z"
        .parse()
        .expect("<instanceId>/<componentNodeId> is an address");
    assert_eq!(inside.instance(), Some(&id("a0000009")));
    assert_eq!(inside.node(), &id("b000000z"));
    assert_eq!(inside.to_string(), "a0000009/b000000z");
    assert_eq!(
        inside,
        NodeAddress::new(Some(id("a0000009")), id("b000000z"))
    );
    assert_ne!(inside, plain);
}

#[test]
fn text_that_is_not_an_address_is_rejected() {
    for bad in [
        "",
        "/",
        "k3f9x2q",
        "k3f9x2qa/",
        "/k3f9x2qa",
        "k3f9x2qa/b000000z/c000000z",
        "k3f9x2qa b000000z",
        "K3F9X2QA",
        "k3f9x2ql",
        "k3f9x2qa/b00000i0",
    ] {
        assert!(
            bad.parse::<NodeAddress>().is_err(),
            "`{bad}` is not a node address"
        );
    }
}

// ---------------------------------------------------------------------------
// nodeAt
// ---------------------------------------------------------------------------

#[test]
fn node_at_returns_the_deepest_node_under_the_point() {
    let scene = fresh();
    for (point, expected, case) in [
        (ICON_AT, ICON, "the icon inside the header inside the card"),
        (HEADER_AT, HEADER, "the header beside the icon"),
        (BUTTON_AT, BUTTON, "the button inside the body"),
        (BODY_AT, BODY, "the body beside the button"),
        (CARD_AT, CARD, "the card below its body"),
        (SIDE_AT, SIDE, "the side frame"),
        (BOARD_A_AT, BOARD_A, "the board, where no node covers it"),
        (PANEL_AT, PANEL, "a node of the second board"),
    ] {
        assert_eq!(scene.node_at(point), Some(addr(expected)), "{case}");
    }
}

#[test]
fn node_at_is_none_where_there_is_no_board() {
    let scene = fresh();
    for point in [NO_BOARD_AT, (-5.0, 10.0), (200.0, 350.0), (5000.0, 5000.0)] {
        assert_eq!(scene.node_at(point), None, "{point:?} is on no board");
    }
}

#[test]
fn a_later_sibling_is_above_an_earlier_one_where_they_overlap() {
    let scene = fresh();
    // Back is 700-850 x 20-170 and Front is 760-880 x 100-250.
    for (point, expected) in [
        ((720.0, 50.0), BACK),
        ((800.0, 130.0), FRONT),
        ((870.0, 200.0), FRONT),
    ] {
        assert_eq!(scene.node_at(point), Some(addr(expected)), "{point:?}");
    }
}

#[test]
fn a_hidden_node_is_never_the_topmost_node() {
    let scene = fresh();
    // Ghost covers Panel's box and comes after it, but is hidden.
    assert_eq!(scene.node_at(PANEL_AT), Some(addr(PANEL)));
    assert_eq!(scene.node_at((560.0, 60.0)), Some(addr(PANEL)));
}

#[test]
fn a_hidden_node_hides_its_whole_subtree() {
    let hidden_group = with(
        &holding(
            &box_node("c0000002", "Group", 20.0, 20.0, 100.0, 100.0),
            vec![box_node("c0000003", "Inner", 10.0, 10.0, 50.0, 50.0)],
        ),
        "visible",
        json!(false),
    );
    let page = json!({
        "id": "p0000001",
        "type": "page",
        "name": "Page",
        "children": [named(
            &board("c0000001", fixed(200.0), fixed(200.0), none(), vec![hidden_group]),
            "Lone board"
        )]
    });
    let scene = Scene::new(node(&page), Assets::new());
    assert_eq!(
        scene.node_at((40.0, 40.0)),
        Some(addr("c0000001")),
        "neither the hidden group nor the node inside it can be hit"
    );
}

#[test]
fn a_node_cannot_be_hit_outside_a_clipping_ancestor_but_can_outside_an_unclipped_one() {
    let scene = fresh();
    // Overflow is 580-680 wide, its clipping parent only 520-620.
    assert_eq!(
        scene.node_at((600.0, 245.0)),
        Some(addr(OVERFLOW)),
        "inside both boxes"
    );
    assert_eq!(
        scene.node_at((650.0, 245.0)),
        Some(addr(BOARD_B)),
        "the part of Overflow that the Clipper clips away is not there"
    );
    // Stray is 700-800 wide, its unclipped parent only 650-730.
    assert_eq!(
        scene.node_at((780.0, 340.0)),
        Some(addr(STRAY)),
        "the part of Stray that overflows the Holder is visible, so it is hit"
    );
}

#[test]
fn bounds_of_a_node_is_its_box_in_page_coordinates() {
    let scene = fresh();
    for (node, expected) in [
        (BOARD_A, page_box(0.0, 0.0, 400.0, 300.0)),
        (CARD, page_box(20.0, 20.0, 200.0, 200.0)),
        (ICON, page_box(30.0, 30.0, 30.0, 30.0)),
        (BUTTON, page_box(40.0, 100.0, 80.0, 40.0)),
        (BOARD_B, page_box(500.0, 0.0, 400.0, 400.0)),
        (OVERFLOW, page_box(580.0, 230.0, 100.0, 30.0)),
        (STRAY, page_box(700.0, 330.0, 100.0, 20.0)),
    ] {
        assert_eq!(scene.bounds_of(&addr(node)), Some(expected), "node {node}");
    }
}

#[test]
fn bounds_of_an_address_that_names_nothing_is_none() {
    let scene = fresh();
    assert_eq!(scene.bounds_of(&addr("zzzzzzzz")), None, "an unknown ID");
    assert_eq!(
        scene.bounds_of(&NodeAddress::new(Some(id(ICON)), id(CARD))),
        None,
        "an instance part that is not an instance"
    );
    assert_eq!(
        scene.bounds_of(&NodeAddress::new(Some(id("zzzzzzzz")), id(CARD))),
        None,
        "an unknown instance"
    );
}

// ---------------------------------------------------------------------------
// nodeAt on the 2,000-node board of the stress file
// ---------------------------------------------------------------------------

/// The stress file `make stress-file` writes (its default seed).
fn stress_file() -> &'static str {
    static FILE: OnceLock<String> = OnceLock::new();
    FILE.get_or_init(|| {
        let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("select");
        fs::create_dir_all(&directory).expect("the scratch directory can be made");
        let path = directory.join("stress-file.json");
        let out = path.to_str().expect("the path is UTF-8").to_string();
        let output = common::make(&["stress-file", &format!("OUT={out}")]);
        assert!(
            output.status.success(),
            "`make stress-file` must write the stress file:\n{}",
            common::describe(&output)
        );
        fs::read_to_string(&path).expect("make stress-file wrote the file")
    })
}

fn count(node: &Node) -> usize {
    1 + node.children().iter().map(count).sum::<usize>()
}

/// The board of the stress file that has 2,000 nodes.
fn big_board() -> Node {
    let page: Node = serde_json::from_str(stress_file()).expect("the stress file is a design");
    page.children()
        .iter()
        .find(|board| count(board) == 2000)
        .expect("the stress file has one board of 2,000 nodes")
        .clone()
}

/// What a plain walk over the boxes gives: the topmost visible node at
/// `point`, the last child that has a hit first, descendants of a clipping
/// node only inside its box.
fn walk(
    node: &Node,
    boxes: &std::collections::HashMap<NodeId, LayoutBox>,
    point: (f32, f32),
) -> Option<NodeId> {
    if !node.visible() {
        return None;
    }
    let b = boxes[node.id()];
    let inside =
        point.0 >= b.x && point.0 < b.x + b.width && point.1 >= b.y && point.1 < b.y + b.height;
    let clips = matches!(
        node.data(),
        NodeData::Board(frame) | NodeData::Frame(frame) if frame.clip == Some(true)
    );
    if clips && !inside {
        return None;
    }
    node.children()
        .iter()
        .rev()
        .find_map(|child| walk(child, boxes, point))
        .or(inside.then(|| *node.id()))
}

/// `count` points across `area`, spread without a pattern a box edge could
/// line up with.
fn points_in(area: LayoutBox, count: usize) -> Vec<(f32, f32)> {
    (0..count)
        .map(|n| {
            let u = (n as f32 * 0.618_034 + 0.137).fract();
            let v = (n as f32 * 0.754_878 + 0.291).fract();
            (area.x + u * area.width, area.y + v * area.height)
        })
        .collect()
}

#[test]
fn node_at_on_the_2000_node_board_agrees_with_a_walk_over_the_boxes() {
    let board = big_board();
    let boxes = layout::compute(&board, Assets::new().fonts());
    let scene = Scene::new(board.clone(), Assets::new());
    let area = boxes[board.id()];
    let mut deepest = 0usize;
    for point in points_in(area, 600) {
        let expected = walk(&board, &boxes, point).map(|node| NodeAddress::new(None, node));
        let found = scene.node_at(point);
        assert_eq!(found, expected, "the node at {point:?}");
        deepest += usize::from(found.is_some_and(|node| node.node() != board.id()));
    }
    assert!(
        deepest > 300,
        "at least half the points of a dense board hit a node inside it, but only {deepest} of 600 did"
    );
}

#[test]
fn node_at_answers_in_under_a_millisecond_on_the_2000_node_board() {
    let board = big_board();
    let scene = Scene::new(board.clone(), Assets::new());
    let area = scene.bounds();
    let points = points_in(area, 3000);
    // Building the index is not what is timed: the first answer may be the
    // one that builds it.
    scene.node_at(points[0]);

    let mut times: Vec<f64> = points
        .iter()
        .map(|&point| {
            let started = Instant::now();
            let found = scene.node_at(point);
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            std::hint::black_box(found);
            elapsed
        })
        .collect();
    times.sort_by(f64::total_cmp);
    let p99 = times[times.len() * 99 / 100];
    assert!(
        p99 < 1.0,
        "99% of node_at calls must take under 1 ms on the 2,000-node board, but the 99th percentile took {p99:.3} ms (median {:.3} ms)",
        times[times.len() / 2]
    );
    assert!(
        points.iter().any(|&point| scene.node_at(point).is_some()),
        "node_at must find nodes on the board it is timed on"
    );
}

// ---------------------------------------------------------------------------
// Click
// ---------------------------------------------------------------------------

#[test]
fn nothing_is_selected_at_the_start() {
    let mut scene = fresh();
    assert_state(&scene, None, None, "a new scene");
    assert!(scene.take_events().is_empty(), "no events before any input");
}

#[test]
fn a_click_selects_the_direct_child_of_the_board_that_holds_the_node_under_the_pointer() {
    for (point, expected, case) in [
        (ICON_AT, CARD, "an icon two levels down selects its card"),
        (BUTTON_AT, CARD, "a button selects the card that holds it"),
        (CARD_AT, CARD, "the card itself"),
        (SIDE_AT, SIDE, "a child of the board with no children"),
        (PANEL_AT, PANEL, "a child of the second board"),
    ] {
        let mut scene = fresh();
        click(&mut scene, point);
        let board = if expected == PANEL { BOARD_B } else { BOARD_A };
        assert_state(&scene, Some(expected), Some(board), case);
    }
}

#[test]
fn a_click_on_the_board_where_no_child_is_selects_the_board_itself() {
    let mut scene = fresh();
    click(&mut scene, BOARD_A_AT);
    assert_state(&scene, Some(BOARD_A), Some(BOARD_A), "bare board");
}

#[test]
fn a_click_on_a_hidden_node_selects_what_is_below_it() {
    let mut scene = fresh();
    click(&mut scene, (560.0, 60.0));
    assert_state(&scene, Some(PANEL), Some(BOARD_B), "Ghost is hidden");
}

#[test]
fn a_click_lands_on_the_node_drawn_under_the_pointer_at_any_zoom_and_pan() {
    // At 200% from page point (-100, -50), the screen point (290, 190) is
    // the page point (45, 45), inside Icon.
    let viewport = Viewport::at(-100.0, -50.0, 2.0, WIDTH, HEIGHT);
    let mut scene = fresh();
    scene.click(&viewport, (290.0, 190.0), NONE);
    assert_state(&scene, Some(CARD), Some(BOARD_A), "zoomed in and panned");

    // At 50% the whole of board A is 200 x 150 screen px, and (190, 140) is
    // the page point (380, 280), the bare board.
    let viewport = Viewport::at(0.0, 0.0, 0.5, WIDTH, HEIGHT);
    scene.click(&viewport, (190.0, 140.0), NONE);
    assert_state(&scene, Some(BOARD_A), Some(BOARD_A), "zoomed out");

    // The same screen point is no node at 100%.
    scene.click(&view(), (190.0, 140.0), NONE);
    assert_state(&scene, Some(CARD), Some(BOARD_A), "back at 100%");
}

// ---------------------------------------------------------------------------
// Double-click and Esc
// ---------------------------------------------------------------------------

#[test]
fn a_double_click_makes_the_selection_the_context_and_selects_its_child_under_the_pointer() {
    let mut scene = fresh();
    click(&mut scene, ICON_AT);
    assert_state(&scene, Some(CARD), Some(BOARD_A), "after the click");

    double_click(&mut scene, ICON_AT);
    assert_state(&scene, Some(HEADER), Some(CARD), "one level in");

    double_click(&mut scene, ICON_AT);
    assert_state(&scene, Some(ICON), Some(HEADER), "two levels in");
}

#[test]
fn a_double_click_on_its_own_selects_the_same_as_a_click_and_a_double_click() {
    let mut scene = fresh();
    double_click(&mut scene, ICON_AT);
    assert_state(&scene, Some(HEADER), Some(CARD), "no click before");
}

#[test]
fn a_double_click_with_no_child_under_the_pointer_leaves_selection_and_context() {
    let mut scene = fresh();
    click(&mut scene, SIDE_AT);
    double_click(&mut scene, SIDE_AT);
    assert_state(&scene, Some(SIDE), Some(BOARD_A), "Side has no children");

    let mut scene = fresh();
    click(&mut scene, CARD_AT);
    double_click(&mut scene, CARD_AT);
    assert_state(
        &scene,
        Some(CARD),
        Some(BOARD_A),
        "no child of Card is at that point",
    );
}

#[test]
fn after_a_double_click_a_click_selects_among_the_children_of_the_new_context() {
    let mut scene = fresh();
    double_click(&mut scene, ICON_AT);
    assert_state(&scene, Some(HEADER), Some(CARD), "inside the card");

    click(&mut scene, BUTTON_AT);
    assert_state(
        &scene,
        Some(BODY),
        Some(CARD),
        "the card's child, not the button",
    );

    click(&mut scene, ICON_AT);
    assert_state(
        &scene,
        Some(HEADER),
        Some(CARD),
        "the other child of the card",
    );
}

#[test]
fn a_click_that_holds_no_child_of_the_context_selects_the_board_itself() {
    let mut scene = fresh();
    double_click(&mut scene, ICON_AT);
    assert_state(&scene, Some(HEADER), Some(CARD), "inside the card");

    click(&mut scene, CARD_AT);
    assert_state(
        &scene,
        Some(BOARD_A),
        Some(BOARD_A),
        "in the card, in none of its children",
    );

    double_click(&mut scene, ICON_AT);
    click(&mut scene, SIDE_AT);
    assert_state(&scene, Some(BOARD_A), Some(BOARD_A), "outside the card");
}

#[test]
fn a_click_on_another_board_starts_over_in_that_board() {
    let mut scene = fresh();
    double_click(&mut scene, ICON_AT);
    assert_state(&scene, Some(HEADER), Some(CARD), "inside the card");

    click(&mut scene, PANEL_AT);
    assert_state(&scene, Some(PANEL), Some(BOARD_B), "in the second board");

    click(&mut scene, ICON_AT);
    assert_state(
        &scene,
        Some(CARD),
        Some(BOARD_A),
        "and back, from the board",
    );
}

#[test]
fn escape_selects_the_parent_and_moves_the_context_up_one_level() {
    let mut scene = fresh();
    double_click(&mut scene, ICON_AT);
    double_click(&mut scene, ICON_AT);
    assert_state(&scene, Some(ICON), Some(HEADER), "two levels in");

    scene.escape();
    assert_state(&scene, Some(HEADER), Some(CARD), "first Esc");
    scene.escape();
    assert_state(&scene, Some(CARD), Some(BOARD_A), "second Esc");
    scene.escape();
    assert_state(
        &scene,
        Some(BOARD_A),
        Some(BOARD_A),
        "third Esc: the board is its own context",
    );
}

#[test]
fn escape_on_the_board_clears_the_selection_and_with_nothing_selected_does_nothing() {
    let mut scene = fresh();
    click(&mut scene, BOARD_A_AT);
    assert_state(
        &scene,
        Some(BOARD_A),
        Some(BOARD_A),
        "the board is selected",
    );
    scene.escape();
    assert_state(&scene, None, None, "Esc on the board");

    scene.take_events();
    scene.escape();
    assert_state(&scene, None, None, "Esc with nothing selected");
    assert!(
        scene.take_events().is_empty(),
        "Esc with nothing selected changes nothing, so it tells nobody"
    );
}

#[test]
fn after_escape_a_click_selects_among_the_children_of_the_new_context() {
    let mut scene = fresh();
    double_click(&mut scene, ICON_AT);
    double_click(&mut scene, ICON_AT);
    scene.escape();
    assert_state(&scene, Some(HEADER), Some(CARD), "back to the header");

    click(&mut scene, BODY_AT);
    assert_state(&scene, Some(BODY), Some(CARD), "a child of the card");
}

// ---------------------------------------------------------------------------
// Ctrl/Cmd + click and empty canvas
// ---------------------------------------------------------------------------

#[test]
fn ctrl_or_cmd_click_selects_the_deepest_node_under_the_pointer() {
    for (keys, name) in [(CTRL, "Ctrl"), (META, "Cmd")] {
        let mut scene = fresh();
        scene.click(&view(), ICON_AT, keys);
        assert_state(
            &scene,
            Some(ICON),
            Some(HEADER),
            &format!("{name} + click on the icon"),
        );

        scene.click(&view(), BUTTON_AT, keys);
        assert_state(
            &scene,
            Some(BUTTON),
            Some(BODY),
            &format!("{name} + click on the button"),
        );

        scene.click(&view(), BOARD_A_AT, keys);
        assert_state(
            &scene,
            Some(BOARD_A),
            Some(BOARD_A),
            &format!("{name} + click on the bare board"),
        );

        scene.click(&view(), PANEL_AT, keys);
        assert_state(
            &scene,
            Some(PANEL),
            Some(BOARD_B),
            &format!("{name} + click on the second board"),
        );
    }
}

#[test]
fn escape_after_a_deep_selection_selects_its_parent() {
    let mut scene = fresh();
    scene.click(&view(), BUTTON_AT, CTRL);
    scene.escape();
    assert_state(&scene, Some(BODY), Some(CARD), "Esc after Ctrl + click");
}

#[test]
fn a_click_on_empty_canvas_clears_the_selection_and_resets_the_context() {
    let mut scene = fresh();
    double_click(&mut scene, ICON_AT);
    double_click(&mut scene, ICON_AT);
    assert_state(&scene, Some(ICON), Some(HEADER), "two levels in");

    click(&mut scene, NO_BOARD_AT);
    assert_state(&scene, None, None, "after clicking between the boards");

    click(&mut scene, ICON_AT);
    assert_state(
        &scene,
        Some(CARD),
        Some(BOARD_A),
        "the context starts at the board again",
    );
}

#[test]
fn ctrl_click_on_empty_canvas_clears_the_selection_too() {
    let mut scene = fresh();
    scene.click(&view(), ICON_AT, CTRL);
    assert_state(&scene, Some(ICON), Some(HEADER), "the icon is selected");
    scene.click(&view(), NO_BOARD_AT, CTRL);
    assert_state(&scene, None, None, "Ctrl + click on no board");
}

// ---------------------------------------------------------------------------
// Hover
// ---------------------------------------------------------------------------

#[test]
fn hover_is_the_node_a_click_would_select() {
    let mut scene = fresh();
    scene.pointer_move(&view(), ICON_AT, NONE);
    assert_eq!(scene.hovered(), Some(id(CARD)), "from the board");

    scene.pointer_move(&view(), ICON_AT, CTRL);
    assert_eq!(
        scene.hovered(),
        Some(id(ICON)),
        "with Ctrl held, the deepest"
    );
    scene.pointer_move(&view(), ICON_AT, META);
    assert_eq!(
        scene.hovered(),
        Some(id(ICON)),
        "with Cmd held, the deepest"
    );

    scene.pointer_move(&view(), SIDE_AT, NONE);
    assert_eq!(scene.hovered(), Some(id(SIDE)));

    scene.pointer_move(&view(), NO_BOARD_AT, NONE);
    assert_eq!(scene.hovered(), None, "off every board");

    double_click(&mut scene, ICON_AT);
    scene.pointer_move(&view(), ICON_AT, NONE);
    assert_eq!(scene.hovered(), Some(id(HEADER)), "in the card's context");
}

#[test]
fn hover_follows_the_screen_point_through_the_viewport() {
    let viewport = Viewport::at(-100.0, -50.0, 2.0, WIDTH, HEIGHT);
    let mut scene = fresh();
    scene.pointer_move(&viewport, (290.0, 190.0), NONE);
    assert_eq!(scene.hovered(), Some(id(CARD)));
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[test]
fn hover_events_tell_each_change_of_the_hovered_node_once() {
    let mut scene = fresh();
    scene.pointer_move(&view(), ICON_AT, NONE);
    scene.pointer_move(&view(), (40.0, 40.0), NONE);
    scene.pointer_move(&view(), BUTTON_AT, NONE);
    assert_eq!(
        scene.take_events(),
        vec![Event::Hover(Some(addr(CARD)))],
        "three moves over the same card are one event"
    );

    scene.pointer_move(&view(), SIDE_AT, NONE);
    scene.pointer_move(&view(), NO_BOARD_AT, NONE);
    scene.pointer_move(&view(), NO_BOARD_AT, NONE);
    assert_eq!(
        scene.take_events(),
        vec![Event::Hover(Some(addr(SIDE))), Event::Hover(None)],
        "onto the side frame, then off the boards"
    );
}

#[test]
fn selection_events_tell_each_change_of_the_selected_node_once() {
    let mut scene = fresh();
    click(&mut scene, ICON_AT);
    click(&mut scene, BUTTON_AT);
    click(&mut scene, SIDE_AT);
    click(&mut scene, NO_BOARD_AT);
    assert_eq!(
        scene.take_events(),
        vec![
            Event::Select(Some(addr(CARD))),
            Event::Select(Some(addr(SIDE))),
            Event::Select(None),
        ],
        "the second click selects the card that is already selected"
    );
}

#[test]
fn a_double_click_and_escape_tell_the_selection_they_leave() {
    let mut scene = fresh();
    double_click(&mut scene, ICON_AT);
    let last_selected = scene
        .take_events()
        .into_iter()
        .rfind(|event| matches!(event, Event::Select(_)));
    assert_eq!(
        last_selected,
        Some(Event::Select(Some(addr(HEADER)))),
        "a double-click ends with the header selected"
    );

    scene.escape();
    assert_eq!(
        scene.take_events(),
        vec![Event::Select(Some(addr(CARD)))],
        "Esc moves the selection from the header to the card"
    );
}

#[test]
fn events_are_drained_when_taken() {
    let mut scene = fresh();
    scene.pointer_move(&view(), ICON_AT, NONE);
    click(&mut scene, ICON_AT);
    assert!(!scene.take_events().is_empty());
    assert!(scene.take_events().is_empty(), "taken events are gone");
}

#[test]
fn hover_and_selection_events_come_in_the_order_they_happened() {
    let mut scene = fresh();
    scene.pointer_move(&view(), SIDE_AT, NONE);
    click(&mut scene, SIDE_AT);
    scene.pointer_move(&view(), NO_BOARD_AT, NONE);
    assert_eq!(
        scene.take_events(),
        vec![
            Event::Hover(Some(addr(SIDE))),
            Event::Select(Some(addr(SIDE))),
            Event::Hover(None),
        ]
    );
}

// ---------------------------------------------------------------------------
// Overlay
// ---------------------------------------------------------------------------

fn outline(overlay: &[Outline], kind: OutlineKind) -> Option<&Outline> {
    overlay.iter().find(|outline| outline.kind == kind)
}

#[track_caller]
fn assert_near(actual: LayoutBox, expected: LayoutBox, case: &str) {
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(
        near(actual.x, expected.x)
            && near(actual.y, expected.y)
            && near(actual.width, expected.width)
            && near(actual.height, expected.height),
        "{case}: expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn there_is_no_overlay_while_nothing_is_hovered_or_selected() {
    let scene = fresh();
    assert!(scene.overlay(&view()).is_empty());
}

#[test]
fn the_overlay_outlines_the_hovered_and_selected_nodes_labeled_with_their_names() {
    let mut scene = fresh();
    click(&mut scene, ICON_AT);
    scene.pointer_move(&view(), SIDE_AT, NONE);
    let overlay = scene.overlay(&view());
    assert_eq!(overlay.len(), 2, "one outline each: {overlay:?}");

    let selection = outline(&overlay, OutlineKind::Selection).expect("the selection is outlined");
    assert_eq!(selection.node, addr(CARD));
    assert_eq!(selection.label, "Card");
    assert_near(
        selection.rect,
        page_box(20.0, 20.0, 200.0, 200.0),
        "selection",
    );

    let hover = outline(&overlay, OutlineKind::Hover).expect("the hovered node is outlined");
    assert_eq!(hover.node, addr(SIDE));
    assert_eq!(hover.label, "Side");
    assert_near(hover.rect, page_box(260.0, 20.0, 100.0, 200.0), "hover");
}

#[test]
fn the_overlay_has_only_the_outlines_that_still_apply() {
    let mut scene = fresh();
    click(&mut scene, ICON_AT);
    scene.pointer_move(&view(), NO_BOARD_AT, NONE);
    let overlay = scene.overlay(&view());
    assert_eq!(overlay.len(), 1);
    assert_eq!(overlay[0].kind, OutlineKind::Selection);

    click(&mut scene, NO_BOARD_AT);
    assert!(
        scene.overlay(&view()).is_empty(),
        "cleared selection, no outline"
    );
}

#[test]
fn outlines_stay_aligned_with_their_nodes_while_zooming_and_panning() {
    let mut scene = fresh();
    scene.click(&view(), BUTTON_AT, CTRL);
    assert_eq!(scene.selected(), Some(id(BUTTON)));

    // The button's page box is (40, 100, 80 x 40).
    let at_200 = Viewport::at(-100.0, -50.0, 2.0, WIDTH, HEIGHT);
    let overlay = scene.overlay(&at_200);
    let selection = outline(&overlay, OutlineKind::Selection).expect("outlined");
    assert_near(
        selection.rect,
        page_box(280.0, 300.0, 160.0, 80.0),
        "at 200% from (-100, -50)",
    );

    let mut viewport = view();
    viewport.zoom_at(0.5, (700.0, 400.0));
    viewport.pan_by(30.0, -20.0);
    let zoom = viewport.zoom();
    let origin = viewport.to_screen((40.0, 100.0));
    let overlay = scene.overlay(&viewport);
    let selection = outline(&overlay, OutlineKind::Selection).expect("outlined");
    assert_near(
        selection.rect,
        page_box(origin.0, origin.1, 80.0 * zoom, 40.0 * zoom),
        "after zoom_at and pan_by",
    );
    assert!(zoom < 1.0, "the view really is zoomed out");
}

#[test]
fn a_hover_outline_stays_aligned_too() {
    let mut scene = fresh();
    let viewport = Viewport::at(-100.0, -50.0, 2.0, WIDTH, HEIGHT);
    scene.pointer_move(&viewport, (290.0, 190.0), NONE);
    let overlay = scene.overlay(&viewport);
    let hover = outline(&overlay, OutlineKind::Hover).expect("hover is outlined");
    assert_eq!(hover.label, "Card");
    assert_near(
        hover.rect,
        page_box(240.0, 140.0, 400.0, 400.0),
        "card at 200%",
    );

    let moved = Viewport::at(0.0, 0.0, 1.0, WIDTH, HEIGHT);
    let overlay = scene.overlay(&moved);
    let hover = outline(&overlay, OutlineKind::Hover).expect("hover is outlined");
    assert_near(
        hover.rect,
        page_box(20.0, 20.0, 200.0, 200.0),
        "card at 100%",
    );
}
