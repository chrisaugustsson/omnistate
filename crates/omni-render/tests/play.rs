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
//! - US-35: an open overlay is drawn over the board at its place, with its
//!   `background` colour over the whole screen under it.
//! - US-39: a click on nothing outlines every hotspot of the board and its open
//!   overlays, the layers a pointer triggers and whose target is there, in
//!   #3b82f6 2 px outside their boxes, fading out over 1 s from the first
//!   frame after the click; frames are asked for until the fade ends.

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
        json!({"board": B2, "history": [B1], "overlays": [], "missed": false, "url": null})
    );
}

#[test]
fn a_click_beside_the_hotspot_of_a_centred_board_navigates_nowhere() {
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

    // A miss outlines the hotspots, so it changes what is drawn.
    assert!(click(&mut session, 90.0, 200.0));

    assert_eq!(
        session.state(),
        json!({"board": B1, "history": [], "overlays": [], "missed": true, "url": null})
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
        json!({"board": B3, "history": [B2], "overlays": [], "missed": false, "url": null})
    );

    assert!(session.back());
    assert_eq!(
        session.state(),
        json!({"board": B2, "history": [], "overlays": [], "missed": false, "url": null})
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

#[test]
fn an_overlay_is_drawn_over_its_background_sheet_which_covers_the_board_under_it() {
    let open = json!([{"actions": [
        {"type": "openOverlay", "dest": B2, "background": "#000000"}
    ]}]);
    let mut scene = scene_of(&page(vec![
        filled_board(
            B1,
            0.0,
            400.0,
            400.0,
            "#ff0000",
            vec![hotspot(0.0, 0.0, open)],
        ),
        filled_board(B2, 1000.0, 100.0, 100.0, "#00ff00", vec![]),
    ]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 400);
    assert!(click(&mut session, 50.0, 50.0));

    // The 100 x 100 overlay is centred on the 400 x 400 board: 150..250.
    let stage = draw(&mut session, &mut scene, 400, 400);

    assert_eq!(stage.pixel(200, 200), [0, 255, 0], "the overlay");
    assert_eq!(
        stage.pixel(155, 155),
        [0, 255, 0],
        "the overlay's top-left corner"
    );
    assert_eq!(
        stage.pixel(140, 200),
        [0, 0, 0],
        "the board left of the overlay"
    );
    assert_eq!(
        stage.pixel(395, 395),
        [0, 0, 0],
        "the board's bottom-right corner"
    );
}

const HINT: [u8; 3] = [0x3b, 0x82, 0xf6];
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];

/// A `width` x `height` layer `id` at `x`, `y` on its board with `interactions`.
fn spot(id: &str, x: f64, y: f64, width: f64, height: f64, interactions: Value) -> Value {
    with(
        &at(&rect(id, width, height), x, y),
        "interactions",
        interactions,
    )
}

/// A 400 x 200 white board on a 400 x 200 stage, so the board's points are the
/// stage's, with a hotspot at 150..250, 50..150.
fn board_with_a_hotspot(interactions: Value) -> (Scene, Session) {
    let mut scene = scene_of(&page(vec![
        filled_board(
            B1,
            0.0,
            400.0,
            200.0,
            "#ffffff",
            vec![hotspot(150.0, 50.0, interactions)],
        ),
        filled_board(B2, 1000.0, 400.0, 200.0, "#ffffff", vec![]),
    ]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 200);
    session.tick(1000.0);
    (scene, session)
}

#[test]
fn a_click_on_nothing_outlines_the_hotspot_2_px_outside_its_box_and_not_after_the_fade() {
    let (mut scene, mut session) = board_with_a_hotspot(navigate(B2));

    click(&mut session, 20.0, 20.0);
    session.tick(1016.0);
    let first = draw(&mut session, &mut scene, 400, 200);

    assert_eq!(first.pixel(148, 100), HINT, "2 px left of the box");
    assert_eq!(first.pixel(149, 100), HINT, "1 px left of the box");
    assert_eq!(first.pixel(147, 100), WHITE, "3 px left of the box");
    assert_eq!(first.pixel(251, 100), HINT, "2 px right of the box");
    assert_eq!(first.pixel(252, 100), WHITE, "3 px right of the box");

    session.tick(2216.0);
    let later = draw(&mut session, &mut scene, 400, 200);

    assert_eq!(later.pixel(149, 100), WHITE, "1.2 s after the first frame");
}

#[test]
fn a_click_on_nothing_asks_for_frames_until_the_outlines_have_faded() {
    let (mut scene, mut session) = board_with_a_hotspot(navigate(B2));

    click(&mut session, 20.0, 20.0);

    assert_eq!(session.tick(1016.0), Tick::Animating);
    draw(&mut session, &mut scene, 400, 200);
    assert_eq!(session.tick(1916.0), Tick::Animating);
    draw(&mut session, &mut scene, 400, 200);
    assert_ne!(session.tick(2116.0), Tick::Animating);
    draw(&mut session, &mut scene, 400, 200);
    assert_eq!(session.tick(2132.0), Tick::Idle);
}

#[test]
fn a_click_on_nothing_in_an_overlay_outlines_its_hotspots_where_the_overlay_is() {
    let open = json!([{"actions": [{"type": "openOverlay", "dest": B2}]}]);
    let mut scene = scene_of(&page(vec![
        filled_board(
            B1,
            0.0,
            400.0,
            400.0,
            "#ff0000",
            vec![hotspot(0.0, 0.0, open)],
        ),
        // Centred on the 400 x 400 board at 100..300: its hotspot at 50, 50
        // is drawn at 150..250.
        filled_board(
            B2,
            1000.0,
            200.0,
            200.0,
            "#ffffff",
            vec![spot("a0000002", 50.0, 50.0, 100.0, 100.0, navigate(B1))],
        ),
    ]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 400);
    session.tick(1000.0);
    click(&mut session, 50.0, 50.0);
    session.tick(1016.0);
    draw(&mut session, &mut scene, 400, 400);

    click(&mut session, 120.0, 120.0);
    session.tick(1032.0);
    let stage = draw(&mut session, &mut scene, 400, 400);

    assert_eq!(stage.pixel(149, 200), HINT, "left of the overlay's hotspot");
    assert_eq!(
        stage.pixel(251, 200),
        HINT,
        "right of the overlay's hotspot"
    );
}

#[test]
fn only_layers_a_pointer_triggers_and_whose_target_is_there_are_outlined() {
    // 50 x 50 layers at y 75..125 and x 25, 125, 225 and 325.
    let hover = json!([{"trigger": "onHover", "actions": [{"type": "navigate", "dest": B2}]}]);
    let key = json!([{
        "trigger": {"type": "keyPress", "key": "K"},
        "actions": [{"type": "navigate", "dest": B2}]
    }]);
    let delay = json!([{
        "trigger": {"type": "afterDelay", "ms": 60000},
        "actions": [{"type": "navigate", "dest": B2}]
    }]);
    let dangling = navigate("b0000009");
    let mut scene = scene_of(&page(vec![
        filled_board(
            B1,
            0.0,
            400.0,
            200.0,
            "#ffffff",
            vec![
                spot("a0000011", 25.0, 75.0, 50.0, 50.0, hover),
                spot("a0000012", 125.0, 75.0, 50.0, 50.0, key),
                spot("a0000013", 225.0, 75.0, 50.0, 50.0, delay),
                spot("a0000014", 325.0, 75.0, 50.0, 50.0, dangling),
            ],
        ),
        filled_board(B2, 1000.0, 400.0, 200.0, "#ffffff", vec![]),
    ]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 200);
    session.tick(1000.0);

    click(&mut session, 10.0, 10.0);
    session.tick(1016.0);
    let stage = draw(&mut session, &mut scene, 400, 200);

    assert_eq!(stage.pixel(24, 100), HINT, "the layer a hover triggers");
    assert_eq!(
        stage.pixel(124, 100),
        WHITE,
        "the layer only a key triggers"
    );
    assert_eq!(
        stage.pixel(224, 100),
        WHITE,
        "the layer only a delay triggers"
    );
    assert_eq!(
        stage.pixel(324, 100),
        WHITE,
        "the layer whose target is not there"
    );
}

// US-32: a change of board drawn as its transition plays. Directions name the
// way a board moves, so a slide in from the right is `direction: "left"`. A
// board moves by the stage's width times (1 − eased) in, or times eased out,
// and one that does not move stays where it rests. easeOut(0.5) is 0.685, so
// 150 ms into 300 ms on a 200 px stage a board coming in is 63 px from its
// place and one going out has gone 137 px.

const RED: [u8; 3] = [0xff, 0x00, 0x00];
const BLUE: [u8; 3] = [0x00, 0x00, 0xff];
const YELLOW: [u8; 3] = [0xff, 0xff, 0x00];
const SECOND: &str = "a0000002";

/// A red board B1 and a blue board B2, `width` x 100 on a 200 x 100 yellow
/// stage, after a click on B1 began a 300 ms easeOut change to B2 with
/// `transition` at 1000 ms. B2 changes to a green B3 with a 300 ms dissolve.
fn changing(transition: Value, width: f64) -> (Scene, Session) {
    let to_b2 = json!([{
        "actions": [{"type": "navigate", "dest": B2}],
        "transition": transition,
        "durationMs": 300,
        "easing": "easeOut"
    }]);
    let to_b3 = json!([{
        "actions": [{"type": "navigate", "dest": B3}],
        "transition": {"type": "dissolve"},
        "durationMs": 300,
        "easing": "easeOut"
    }]);
    let page = with(
        &page(vec![
            filled_board(
                B1,
                0.0,
                width,
                100.0,
                "#ff0000",
                vec![spot(HOTSPOT, 0.0, 0.0, width, 100.0, to_b2)],
            ),
            filled_board(
                B2,
                1000.0,
                width,
                100.0,
                "#0000ff",
                vec![spot(SECOND, 0.0, 0.0, width, 100.0, to_b3)],
            ),
            filled_board(B3, 2000.0, width, 100.0, "#00ff00", vec![]),
        ]),
        "prototype",
        json!({"background": "#ffff00"}),
    );
    let mut scene = scene_of(&page);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 200, 100);
    session.tick(1000.0);
    click(&mut session, 100.0, 50.0);
    session.tick(1000.0);
    (scene, session)
}

#[test]
fn a_slide_in_from_the_right_covers_the_old_board_from_its_right_edge_to_the_whole_stage() {
    let (mut scene, mut session) = changing(json!({"type": "slideIn", "direction": "left"}), 200.0);

    let start = draw(&mut session, &mut scene, 200, 100);
    assert_eq!(start.pixel(1, 50), RED, "0 ms: the old board's left edge");
    assert_eq!(
        start.pixel(198, 50),
        RED,
        "0 ms: the old board's right edge"
    );

    session.tick(1150.0);
    let middle = draw(&mut session, &mut scene, 200, 100);
    assert_eq!(middle.pixel(1, 50), RED, "150 ms: the old board stays");
    assert_eq!(
        middle.pixel(60, 50),
        RED,
        "150 ms: left of the new board's edge at 63"
    );
    assert_eq!(
        middle.pixel(66, 50),
        BLUE,
        "150 ms: right of the new board's edge at 63"
    );

    session.tick(1300.0);
    let end = draw(&mut session, &mut scene, 200, 100);
    assert_eq!(end.pixel(1, 50), BLUE, "300 ms: the new board's left edge");
    assert_eq!(
        end.pixel(198, 50),
        BLUE,
        "300 ms: the new board's right edge"
    );
}

#[test]
fn a_dissolve_half_way_blends_the_boards_by_the_eased_progress() {
    let (mut scene, mut session) = changing(json!({"type": "dissolve"}), 200.0);

    session.tick(1150.0);
    let stage = draw(&mut session, &mut scene, 200, 100);

    // 31.5% red and 68.5% blue.
    let [red, green, blue] = stage.pixel(100, 50);
    assert!((78..=83).contains(&red), "red {red}");
    assert_eq!(green, 0);
    assert!((172..=177).contains(&blue), "blue {blue}");
}

#[test]
fn a_push_moves_the_old_board_out_as_the_new_one_comes_in() {
    // At rest the 100 px boards sit at 50..150: the old one is at -87..13 and
    // the new one at 113..213.
    let (mut scene, mut session) = changing(json!({"type": "push", "direction": "left"}), 100.0);

    session.tick(1150.0);
    let stage = draw(&mut session, &mut scene, 200, 100);

    assert_eq!(stage.pixel(5, 50), RED, "the old board, moved out");
    assert_eq!(stage.pixel(100, 50), YELLOW, "between the two");
    assert_eq!(stage.pixel(120, 50), BLUE, "the new board, coming in");
}

#[test]
fn a_move_in_moves_only_the_new_board_over_the_old_one() {
    // The old 100 px board stays at 50..150 and the new one is at 113..213.
    let (mut scene, mut session) = changing(json!({"type": "moveIn", "direction": "left"}), 100.0);

    session.tick(1150.0);
    let stage = draw(&mut session, &mut scene, 200, 100);

    assert_eq!(stage.pixel(5, 50), YELLOW, "left of the old board");
    assert_eq!(stage.pixel(100, 50), RED, "the old board, where it rests");
    assert_eq!(
        stage.pixel(120, 50),
        BLUE,
        "the new board, over the old one"
    );
}

#[test]
fn a_move_out_moves_only_the_old_board_over_the_new_one() {
    // The new 160 px board stays at 20..180 and the old one is at -117..43.
    let (mut scene, mut session) = changing(json!({"type": "moveOut", "direction": "left"}), 160.0);

    session.tick(1150.0);
    let stage = draw(&mut session, &mut scene, 200, 100);

    assert_eq!(stage.pixel(10, 50), RED, "the old board, moved out");
    assert_eq!(stage.pixel(30, 50), RED, "the old board, over the new one");
    assert_eq!(stage.pixel(50, 50), BLUE, "the new board, where it rests");
    assert_eq!(stage.pixel(190, 50), YELLOW, "right of the new board");
}

#[test]
fn a_navigate_during_a_slide_starts_from_the_frame_on_screen_and_keeps_the_board_it_was_heading_to()
{
    let (mut scene, mut session) = changing(json!({"type": "slideIn", "direction": "left"}), 200.0);
    session.tick(1150.0);

    click(&mut session, 150.0, 50.0);
    session.tick(1150.0);
    let stage = draw(&mut session, &mut scene, 200, 100);

    assert_eq!(stage.pixel(60, 50), RED, "the slide, frozen at 150 ms");
    assert_eq!(stage.pixel(66, 50), BLUE, "the slide, frozen at 150 ms");
    assert_eq!(
        session.state(),
        json!({"board": B3, "history": [B1, B2], "overlays": [], "missed": false, "url": null})
    );
}
