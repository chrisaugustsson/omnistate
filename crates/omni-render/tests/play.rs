//! US-31: playing a page's prototype on a canvas (`omni_render::play::Session`),
//! which `web::play_*` and the `omni_render_play_*` exports wrap for the
//! viewer's play mode.
//!
//! What the tests expect, from the story:
//!
//! - The board the player shows is drawn at its own size in the middle of the
//!   stage, on the page's `prototype.background`, and scaled down to fit when
//!   it is bigger than the stage. A click on the stage lands on the layer drawn
//!   under it, wherever the board was placed.
//! - A click on a hotspot moves to its destination, and the state reports the
//!   new board with the board it left in the history.
//! - The page's loop runs only while `tick` says a transition is playing: an
//!   instant navigate is drawn once.
//! - A link the prototype asks to open is in the state once.
//! - Going to a board as Previous and Next do pushes the board it leaves, so
//!   that Back returns there.
//! - A board ID that is not a board of the page is refused with "`<id>` is not
//!   a board of the shown page".

mod design;

use design::{at, board, fixed, node, none, rect, with};
use omni_render::paint::Assets;
use omni_render::play::{Session, Tick};
use omni_render::view::Scene;
use serde_json::{Value, json};
use skia_safe::{AlphaType, ColorType, ImageInfo, surfaces};

const B1: &str = "b0000001";
const B2: &str = "b0000002";
const B3: &str = "b0000003";
const HOTSPOT: &str = "a0000001";

const DOWN: u32 = 0;
const UP: u32 = 2;

fn solid(json: &Value, color: &str) -> Value {
    with(json, "fills", json!([{"type": "solid", "color": color}]))
}

/// A `width` x `height` board at `x`, `y` on the page, filled with `color`.
fn filled_board(
    id: &str,
    x: f64,
    width: f64,
    height: f64,
    color: &str,
    children: Vec<Value>,
) -> Value {
    solid(
        &at(
            &board(id, fixed(width), fixed(height), none(), children),
            x,
            0.0,
        ),
        color,
    )
}

/// A 100 x 100 layer at `x`, `y` on its board with `interactions`.
fn hotspot(x: f64, y: f64, interactions: Value) -> Value {
    with(
        &at(&rect(HOTSPOT, 100.0, 100.0), x, y),
        "interactions",
        interactions,
    )
}

fn navigate(dest: &str) -> Value {
    json!([{"actions": [{"type": "navigate", "dest": dest}]}])
}

fn page(boards: Vec<Value>) -> Value {
    json!({"id": "p0000001", "type": "page", "name": "Page", "children": boards})
}

fn scene_of(page: &Value) -> Scene {
    Scene::new(node(page), Assets::new())
}

/// The stage, `width` x `height` CSS pixels at a pixel ratio of 1, after the
/// session drew on it.
struct Stage {
    width: i32,
    rgba: Vec<u8>,
}

impl Stage {
    fn pixel(&self, x: i32, y: i32) -> [u8; 3] {
        let at = ((y * self.width + x) * 4) as usize;
        [self.rgba[at], self.rgba[at + 1], self.rgba[at + 2]]
    }
}

fn draw(session: &mut Session, scene: &mut Scene, width: i32, height: i32) -> Stage {
    let mut surface = surfaces::raster_n32_premul((width, height)).expect("Skia creates a surface");
    session.draw(scene, surface.canvas(), (width as f32, height as f32), 1.0);
    let info = ImageInfo::new(
        (width, height),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    let mut rgba = vec![0; info.compute_min_byte_size()];
    assert!(surface.read_pixels(&info, &mut rgba, info.min_row_bytes(), (0, 0)));
    Stage { width, rgba }
}

fn click(session: &mut Session, x: f32, y: f32) -> bool {
    session.pointer(DOWN, x, y);
    session.pointer(UP, x, y)
}

#[test]
fn a_board_smaller_than_the_stage_is_drawn_at_its_own_size_in_the_middle_on_the_page_background() {
    let page = with(
        &page(vec![filled_board(B1, 0.0, 200.0, 100.0, "#ff0000", vec![])]),
        "prototype",
        json!({"background": "#3366cc"}),
    );
    let mut scene = scene_of(&page);
    let mut session = Session::start(&scene, B1).expect("the page has the board");

    // A 200 x 100 board on a 400 x 400 stage spans x 100..300 and y 150..250.
    let stage = draw(&mut session, &mut scene, 400, 400);

    assert_eq!(
        stage.pixel(105, 155),
        [255, 0, 0],
        "inside the board's top-left corner"
    );
    assert_eq!(
        stage.pixel(295, 245),
        [255, 0, 0],
        "inside the board's bottom-right corner"
    );
    assert_eq!(
        stage.pixel(95, 200),
        [0x33, 0x66, 0xcc],
        "left of the board"
    );
    assert_eq!(stage.pixel(200, 145), [0x33, 0x66, 0xcc], "above the board");
    assert_eq!(
        stage.pixel(305, 200),
        [0x33, 0x66, 0xcc],
        "right of the board"
    );
    assert_eq!(stage.pixel(200, 255), [0x33, 0x66, 0xcc], "below the board");
}

#[test]
fn a_click_on_a_board_scaled_down_to_fit_lands_on_the_hotspot_drawn_under_it() {
    // An 800 x 400 board on a 400 x 400 stage is drawn at half size, at y
    // 100..300: its hotspot at 400, 200 (100 x 100) is drawn at 200..250, 200..250.
    let mut scene = scene_of(&page(vec![
        filled_board(
            B1,
            0.0,
            800.0,
            400.0,
            "#ffffff",
            vec![hotspot(400.0, 200.0, navigate(B2))],
        ),
        filled_board(B2, 1000.0, 800.0, 400.0, "#ffffff", vec![]),
    ]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 400);

    assert!(
        click(&mut session, 225.0, 225.0),
        "the click changed what is shown"
    );

    assert_eq!(
        session.state(),
        json!({"board": B2, "history": [B1], "overlays": [], "url": null})
    );
}

#[test]
fn a_click_beside_the_hotspot_of_a_centred_board_changes_nothing() {
    // A 200 x 100 board on a 400 x 400 stage spans x 100..300: its hotspot at
    // 0, 0 is drawn at 100..200, 150..250, and x 90 is beside the board.
    let mut scene = scene_of(&page(vec![
        filled_board(
            B1,
            0.0,
            200.0,
            100.0,
            "#ffffff",
            vec![hotspot(0.0, 0.0, navigate(B2))],
        ),
        filled_board(B2, 1000.0, 200.0, 100.0, "#ffffff", vec![]),
    ]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 400);

    assert!(!click(&mut session, 90.0, 200.0));

    assert_eq!(
        session.state(),
        json!({"board": B1, "history": [], "overlays": [], "url": null})
    );
}

#[test]
fn an_instant_navigate_is_drawn_once_and_asks_for_no_further_frame() {
    let mut scene = scene_of(&page(vec![
        filled_board(
            B1,
            0.0,
            200.0,
            100.0,
            "#ffffff",
            vec![hotspot(0.0, 0.0, navigate(B2))],
        ),
        filled_board(B2, 1000.0, 200.0, 100.0, "#ffffff", vec![]),
    ]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 200, 100);
    assert_eq!(session.tick(1000.0), Tick::Idle);

    click(&mut session, 50.0, 50.0);

    assert_eq!(session.tick(1016.0), Tick::Changed);
    draw(&mut session, &mut scene, 200, 100);
    assert_eq!(session.tick(1032.0), Tick::Idle);
}

#[test]
fn a_dissolve_asks_for_frames_until_its_duration_has_passed() {
    let interactions = json!([{
        "actions": [{"type": "navigate", "dest": B2}],
        "transition": {"type": "dissolve"},
        "durationMs": 200
    }]);
    let mut scene = scene_of(&page(vec![
        filled_board(
            B1,
            0.0,
            200.0,
            100.0,
            "#ffffff",
            vec![hotspot(0.0, 0.0, interactions)],
        ),
        filled_board(B2, 1000.0, 200.0, 100.0, "#ffffff", vec![]),
    ]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 200, 100);
    session.tick(1000.0);

    click(&mut session, 50.0, 50.0);

    assert_eq!(session.tick(1100.0), Tick::Animating);
    draw(&mut session, &mut scene, 200, 100);
    assert_ne!(session.tick(1300.0), Tick::Animating);
}

#[test]
fn a_link_the_prototype_opens_is_in_the_state_once() {
    let interactions = json!([{"actions": [
        {"type": "openUrl", "url": "https://example.com/terms", "newTab": false}
    ]}]);
    let mut scene = scene_of(&page(vec![filled_board(
        B1,
        0.0,
        200.0,
        100.0,
        "#ffffff",
        vec![hotspot(0.0, 0.0, interactions)],
    )]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 200, 100);

    assert!(
        click(&mut session, 50.0, 50.0),
        "the page is told to read the state"
    );

    assert_eq!(
        session.state()["url"],
        json!({"url": "https://example.com/terms", "newTab": false})
    );
    assert_eq!(session.state()["url"], Value::Null);
}

#[test]
fn going_to_a_board_pushes_the_board_it_leaves_so_back_returns_there() {
    let scene = scene_of(&page(vec![
        filled_board(B1, 0.0, 200.0, 100.0, "#ffffff", vec![]),
        filled_board(B2, 300.0, 200.0, 100.0, "#ffffff", vec![]),
        filled_board(B3, 600.0, 200.0, 100.0, "#ffffff", vec![]),
    ]));
    let mut session = Session::start(&scene, B2).expect("the page has the board");

    assert_eq!(session.go(B3), Ok(true));
    assert_eq!(
        session.state(),
        json!({"board": B3, "history": [B2], "overlays": [], "url": null})
    );

    assert!(session.back());
    assert_eq!(
        session.state(),
        json!({"board": B2, "history": [], "overlays": [], "url": null})
    );
}

#[test]
fn a_session_on_an_id_that_is_not_a_board_of_the_page_is_refused() {
    let scene = scene_of(&page(vec![filled_board(
        B1,
        0.0,
        200.0,
        100.0,
        "#ffffff",
        vec![hotspot(0.0, 0.0, navigate(B2))],
    )]));

    let refused = Session::start(&scene, HOTSPOT).err();

    assert_eq!(
        refused.as_deref(),
        Some("`a0000001` is not a board of the shown page")
    );
}
