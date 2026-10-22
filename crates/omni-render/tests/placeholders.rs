//! US-058: a big design is shown board by board. The design viewer shows a
//! page of the version's outline first, every board a placeholder without its
//! content, asks the renderer which boards its view shows
//! (`Scene::visible_boards`) and reads those.
//!
//! - A board that hugs its content has no room of its own once its content is
//!   left out: without a padding it is laid out 0 x 0 and is never in view, so
//!   it would never be read. The API therefore gives each placeholder the box
//!   that the board is laid out in with its content (`view::board_box`), as a
//!   fixed width and height.
//! - `board_box` lays a board out as its page does: at its `x` and `y`, the
//!   size of its content when it hugs it, with its instances expanded and its
//!   tokens resolved against the design system (in its first mode).
//! - A page of such placeholders, spread as far apart as the Ordrar page of
//!   the TireBud design (boards from x = -3200 to 4900, y = -484 to 300,000),
//!   reports its boards in view once the view is fitted to it, and the boards
//!   under the view as the reviewer zooms in on them.

mod design;

use design::{at, board, fixed, flex, node, none, rect};
use omni_render::layout::LayoutBox;
use omni_render::paint::Assets;
use omni_render::system::DesignSystem;
use omni_render::view::{MIN_ZOOM, Scene, Viewport, board_box};
use serde_json::{Value, json};

const WIDTH: f32 = 1440.0;
const HEIGHT: f32 = 900.0;

const CARD: &str = "k0000001";

fn per_mode(light: &str, dark: &str) -> Value {
    json!({
        "$value": light,
        "$extensions": {"omnistate": {"modes": {"light": light, "dark": dark}}}
    })
}

/// A design system with the padding token `$spacing.pad` (10 px in its first
/// mode) and the component Card, 120 x 40.
fn system() -> DesignSystem {
    let json = json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "spacing": {"$type": "dimension", "pad": per_mode("10px", "30px")},
        "$components": [{
            "id": CARD,
            "type": "component",
            "name": "Card",
            "width": fixed(120.0),
            "height": fixed(40.0),
            "layout": {"mode": "none"}
        }]
    });
    DesignSystem::from_json(&json)
        .unwrap_or_else(|error| panic!("the design system fixture must be read, but: {error}"))
}

/// A board at `x`, `y` that hugs its content: a column with a padding of
/// `$spacing.pad` and a gap of 8, holding a Card and an 80 x 30 frame. Laid
/// out in the first mode it is 120 + 2 x 10 = 140 wide and
/// 40 + 8 + 30 + 2 x 10 = 98 tall.
fn hugging_board(id: &str, x: f64, y: f64) -> Value {
    let card = json!({"id": format!("e{}", &id[1..]), "type": "instance", "name": "Card", "component": CARD});
    let layout = flex(json!({
        "direction": "column",
        "gap": {"row": 8, "column": 8},
        "padding": {
            "top": "$spacing.pad",
            "right": "$spacing.pad",
            "bottom": "$spacing.pad",
            "left": "$spacing.pad"
        }
    }));
    at(
        &board(
            id,
            json!("hug"),
            json!("hug"),
            layout,
            vec![card, rect(&format!("r{}", &id[1..]), 80.0, 30.0)],
        ),
        x,
        y,
    )
}

/// `board` without its children, as the outline lists it.
fn placeholder_of(board: &Value) -> Value {
    let mut own = board.clone();
    own.as_object_mut()
        .expect("fixture boards are objects")
        .remove("children");
    own
}

/// The placeholder of `board` with the box `board_box` lays it out in, as the
/// API's outline gives it.
fn sized_placeholder(board: &Value, system: &DesignSystem) -> Value {
    let laid = board_box(node(board), &Assets::new(), Some(system))
        .unwrap_or_else(|| panic!("the board {} has a box", board["id"]));
    let mut own = placeholder_of(board);
    let object = own.as_object_mut().expect("fixture boards are objects");
    object.insert("width".to_string(), fixed(f64::from(laid.width)));
    object.insert("height".to_string(), fixed(f64::from(laid.height)));
    own
}

fn page(boards: Vec<Value>) -> Value {
    json!({"id": "p0000001", "type": "page", "name": "Ordrar", "children": boards})
}

fn visible(scene: &Scene, viewport: &Viewport) -> Vec<String> {
    scene
        .visible_boards(viewport)
        .iter()
        .map(|id| id.as_str().to_string())
        .collect()
}

/// Where the boards of the far-apart page are, like the Ordrar page's.
const SPREAD: [(&str, f64, f64); 6] = [
    ("b0000001", -3200.0, -484.0),
    ("b0000002", 0.0, 0.0),
    ("b0000003", 4900.0, 0.0),
    ("b0000004", 0.0, 2400.0),
    ("b0000005", -3200.0, 150_000.0),
    ("b0000006", 4900.0, 300_000.0),
];

#[test]
fn a_board_that_hugs_its_content_is_laid_out_in_the_box_of_that_content() {
    let system = system();
    let board = hugging_board("b0000001", 4900.0, 300_000.0);
    assert_eq!(
        board_box(node(&board), &Assets::new(), Some(&system)),
        Some(LayoutBox {
            x: 4900.0,
            y: 300_000.0,
            width: 140.0,
            height: 98.0,
        }),
        "the board at its x and y, its Card expanded and its padding token resolved"
    );
}

#[test]
fn a_board_of_a_fixed_size_keeps_it() {
    let board = at(
        &board("b0000001", fixed(1800.0), fixed(640.0), none(), Vec::new()),
        0.0,
        631.0,
    );
    assert_eq!(
        board_box(node(&board), &Assets::new(), None),
        Some(LayoutBox {
            x: 0.0,
            y: 631.0,
            width: 1800.0,
            height: 640.0,
        })
    );
}

#[test]
fn a_hugging_placeholder_without_its_content_is_never_in_view() {
    // Why the outline cannot hand the renderer the board's own JSON: left
    // without its content, a board that hugs it and has no padding has no
    // room, and no view shows it.
    let bare = placeholder_of(&at(
        &board(
            "b0000001",
            json!("hug"),
            json!("hug"),
            flex(json!({})),
            Vec::new(),
        ),
        100.0,
        100.0,
    ));
    let scene = Scene::new(node(&page(vec![bare])), Assets::new());
    let viewport = Viewport::at(0.0, 0.0, 1.0, WIDTH, HEIGHT);
    assert_eq!(visible(&scene, &viewport), Vec::<String>::new());
}

#[test]
fn the_placeholders_of_a_page_spread_far_apart_are_in_view_once_the_view_is_fitted() {
    let system = system();
    let boards: Vec<Value> = SPREAD
        .iter()
        .map(|(id, x, y)| sized_placeholder(&hugging_board(id, *x, *y), &system))
        .collect();
    for board in &boards {
        assert_eq!(board["width"], fixed(140.0), "{board}");
        assert_eq!(board["height"], fixed(98.0), "{board}");
    }
    let scene = Scene::with_system(node(&page(boards)), Assets::new(), system.clone());

    // The first frame fits the view to the boards, as the web build does.
    let mut viewport = Viewport::new(WIDTH, HEIGHT);
    viewport.fit(scene.bounds());
    assert_eq!(
        scene.bounds(),
        LayoutBox {
            x: -3200.0,
            y: -484.0,
            width: 8240.0,
            height: 300_582.0,
        }
    );
    // The page is too tall to fit at the smallest zoom (1%), so the view is
    // centered on it: on the board halfway down.
    assert_eq!(viewport.zoom(), MIN_ZOOM);
    assert_eq!(visible(&scene, &viewport), ["b0000005"]);

    // Zoomed in on each board in turn, that board is the one in view.
    for (id, x, y) in SPREAD {
        let viewport = Viewport::at(x as f32 - 20.0, y as f32 - 20.0, 1.0, WIDTH, HEIGHT);
        let shown = visible(&scene, &viewport);
        assert!(
            shown.contains(&id.to_string()),
            "the view at ({x}, {y}) shows {id}: {shown:?}"
        );
    }
}
