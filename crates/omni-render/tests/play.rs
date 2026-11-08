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
//! - US-40: controller buttons 0, 4 and 12 of the standard layout fire the
//!   `Gamepad A`, `Gamepad LB` and `Gamepad Up` key triggers as they go down.
//! - US-38: the board is resolved again with what `setVariable` and
//!   `setVariableMode` set: a Text bound to a number token shows it in its
//!   shortest form and the frame hugging it follows, a fill follows its colour
//!   token, and a board set to dark is drawn dark while other boards are not.
//!   Restart, and Exit followed by Play, draw the design system's values.
//! - US-33: a `changeTo` fired by a layer inside a component swaps the
//!   instance that holds the layer, and the board is laid out again: the
//!   instance grows to the destination variant and a frame hugging it follows,
//!   while other instances stay as designed. Inside an outer component that
//!   appears more than once, only the clicked one changes. The swapped
//!   instance keeps its other props and its overrides, and its variant props
//!   take the destination's defaults. A board that a transition leaves is
//!   drawn with its swaps until the transition ends, and as designed after.
//! - US-34: during a smart animate a layer on both boards is at the box and
//!   in the colour the eased progress mixes, the roots' fills mix, and a layer
//!   on one board only (hidden there, or of another kind) fades, beneath the
//!   new board's layers. A smart-animated change to animates the instance's
//!   layers the same way. A Text's content and what cannot be mixed switch at
//!   half the duration; `interpolate` switches them at a `t` of 0.5, mixes
//!   colours straight, alpha too, and carries a box on past 1.

mod design;

use design::{at, board, fixed, flex, frame, node, none, rect, text, with};
use omni_core::design::NodeAddress;
use omni_render::layout::LayoutBox;
use omni_render::paint::Assets;
use omni_render::play::{Session, Style, Tick, interpolate};
use omni_render::system::DesignSystem;
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

fn on_gamepad(button: &str, dest: &str) -> Value {
    json!([{
        "trigger": {"type": "keyPress", "key": format!("Gamepad {button}")},
        "actions": [{"type": "navigate", "dest": dest}]
    }])
}

#[test]
fn controller_buttons_0_4_and_12_fire_gamepad_a_lb_and_up_as_they_go_down() {
    let mut scene = scene_of(&page(vec![
        filled_board(
            B1,
            0.0,
            400.0,
            200.0,
            "#ffffff",
            vec![spot(
                "a0000011",
                0.0,
                0.0,
                100.0,
                100.0,
                on_gamepad("A", B2),
            )],
        ),
        filled_board(
            B2,
            1000.0,
            400.0,
            200.0,
            "#ffffff",
            vec![spot(
                "a0000012",
                0.0,
                0.0,
                100.0,
                100.0,
                on_gamepad("LB", B3),
            )],
        ),
        filled_board(
            B3,
            2000.0,
            400.0,
            200.0,
            "#ffffff",
            vec![spot(
                "a0000013",
                0.0,
                0.0,
                100.0,
                100.0,
                on_gamepad("Up", B1),
            )],
        ),
    ]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 200);

    assert!(
        !session.gamepad(0, false),
        "a button going up fires nothing"
    );
    assert!(session.gamepad(0, true));
    assert!(session.gamepad(4, true));
    assert!(session.gamepad(12, true));

    assert_eq!(
        session.state(),
        json!({"board": B1, "history": [B1, B2, B3], "overlays": [], "missed": false, "url": null})
    );
}

const COUNTER: &str = "a0000010";
const COUNT: &str = "a0000011";
const MODE: &str = "a0000012";
const SURFACE_WHITE: [u8; 3] = [0xff, 0xff, 0xff];
const SURFACE_DARK: [u8; 3] = [0x0f, 0x17, 0x2a];

/// `color.surface` is white in light and #0f172a in dark; `state.count` is 9.
fn play_system() -> DesignSystem {
    DesignSystem::from_json(&json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {"surface": {
            "$type": "color",
            "$value": "#ffffff",
            "$extensions": {"omnistate": {"modes": {"light": "#ffffff", "dark": "#0f172a"}}}
        }},
        "state": {
            "$extensions": {"omnistate": {"modes": []}},
            "count": {"$type": "number", "$value": 9}
        },
        "$components": []
    }))
    .unwrap_or_else(|error| panic!("the design system fixture must be read, but: {error}"))
}

/// A 400 x 300 board filled with `$color.surface`, with `children`.
fn surface_board(id: &str, x: f64, children: Vec<Value>) -> Value {
    solid(
        &at(
            &board(id, fixed(400.0), fixed(300.0), none(), children),
            x,
            0.0,
        ),
        "$color.surface",
    )
}

/// A frame at 0, 150 that hugs one Text showing `content`.
fn counter(content: &str) -> Value {
    at(
        &frame(
            COUNTER,
            json!("hug"),
            json!("hug"),
            flex(json!({})),
            vec![text(COUNT, content, 16.0, 20.0)],
        ),
        0.0,
        150.0,
    )
}

fn play_scene(boards: Vec<Value>) -> Scene {
    Scene::with_system(node(&page(boards)), Assets::new(), play_system())
}

fn width_of(scene: &Scene, board: &str, layer: &str) -> f32 {
    let (rects, _) = scene
        .board_boxes(&board.parse().unwrap())
        .expect("the scene has the board");
    rects[&layer.parse().unwrap()].width
}

fn set_surface_red() -> Value {
    json!([{"actions": [{"type": "setVariable", "variable": "color.surface", "value": "#ff0000"}]}])
}

#[test]
fn a_counter_text_shows_count_plus_one_as_a_whole_number_and_its_hugging_frame_follows() {
    let add = json!([{"actions": [{"type": "setVariable", "variable": "state.count",
        "expr": {"op": "add", "args": [{"variable": "state.count"}, {"value": 1}]}}]}]);
    let mut scene = play_scene(vec![surface_board(
        B1,
        0.0,
        vec![hotspot(0.0, 0.0, add), counter("$state.count")],
    )]);
    let mut designed_ten = play_scene(vec![surface_board(B1, 0.0, vec![counter("10")])]);
    while designed_ten.work() {}
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 300);
    assert!(click(&mut session, 50.0, 50.0));

    draw(&mut session, &mut scene, 400, 300);

    assert_eq!(
        width_of(&scene, B1, COUNTER),
        width_of(&designed_ten, B1, COUNTER),
        "9 + 1 is shown as 10, as wide as a Text written 10"
    );
}

#[test]
fn a_fill_follows_the_colour_token_an_action_sets() {
    let mut scene = play_scene(vec![surface_board(
        B1,
        0.0,
        vec![hotspot(0.0, 0.0, set_surface_red())],
    )]);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 300);
    assert!(click(&mut session, 50.0, 50.0));

    let stage = draw(&mut session, &mut scene, 400, 300);

    assert_eq!(stage.pixel(300, 250), [255, 0, 0]);
}

#[test]
fn set_variable_mode_draws_the_board_in_the_mode_and_leaves_the_other_boards_in_theirs() {
    let dark =
        json!([{"actions": [{"type": "setVariableMode", "collection": "color", "mode": "dark"}]}]);
    let mut scene = play_scene(vec![
        surface_board(
            B1,
            0.0,
            vec![
                hotspot(0.0, 0.0, dark),
                with(
                    &at(&rect(MODE, 100.0, 100.0), 200.0, 0.0),
                    "interactions",
                    navigate(B2),
                ),
            ],
        ),
        surface_board(B2, 1000.0, vec![]),
    ]);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 300);
    assert!(click(&mut session, 50.0, 50.0));

    let stage = draw(&mut session, &mut scene, 400, 300);
    assert_eq!(stage.pixel(300, 250), SURFACE_DARK, "the board set to dark");

    assert!(click(&mut session, 250.0, 50.0));
    let stage = draw(&mut session, &mut scene, 400, 300);
    assert_eq!(stage.pixel(300, 250), SURFACE_WHITE, "the board it went to");
}

#[test]
fn restart_draws_the_design_systems_values_again() {
    let mut scene = play_scene(vec![surface_board(
        B1,
        0.0,
        vec![hotspot(0.0, 0.0, set_surface_red())],
    )]);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 300);
    assert!(click(&mut session, 50.0, 50.0));
    draw(&mut session, &mut scene, 400, 300);

    session.restart(B1).expect("the page has the board");
    let stage = draw(&mut session, &mut scene, 400, 300);

    assert_eq!(stage.pixel(300, 250), SURFACE_WHITE);
}

#[test]
fn play_after_exit_draws_the_design_systems_values() {
    let mut scene = play_scene(vec![surface_board(
        B1,
        0.0,
        vec![hotspot(0.0, 0.0, set_surface_red())],
    )]);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 300);
    assert!(click(&mut session, 50.0, 50.0));
    draw(&mut session, &mut scene, 400, 300);

    session.end(&mut scene);
    let mut again = Session::start(&scene, B1).expect("the page has the board");
    let stage = draw(&mut again, &mut scene, 400, 300);

    assert_eq!(stage.pixel(300, 250), SURFACE_WHITE);
}

// US-36: the page's loop keeps ticking while a `delayMs` or `afterDelay` wait
// is pending, so that the wait is seen to run out.

const MOVE: u32 = 1;

fn link_to(url: &str) -> Value {
    json!({"type": "openUrl", "url": url, "newTab": false})
}

#[test]
fn a_pending_wait_asks_for_frames_until_it_has_run_out_also_on_a_frame_that_drew() {
    let first = json!([{"trigger": {"type": "afterDelay", "ms": 500},
        "actions": [link_to("https://example.com/splash")]}]);
    let second = json!([{"trigger": {"type": "afterDelay", "ms": 1000},
        "actions": [{"type": "navigate", "dest": B2}]}]);
    let mut scene = scene_of(&page(vec![
        filled_board(
            B1,
            0.0,
            400.0,
            200.0,
            "#ffffff",
            vec![
                spot("a0000011", 0.0, 0.0, 50.0, 50.0, first),
                spot("a0000012", 100.0, 0.0, 50.0, 50.0, second),
            ],
        ),
        filled_board(B2, 1000.0, 400.0, 200.0, "#ffffff", vec![]),
    ]));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 200);

    // The waits count from the first frame.
    assert_eq!(session.tick(1000.0), Tick::Waiting);
    assert_eq!(session.tick(1499.0), Tick::Waiting);
    assert_eq!(session.tick(1500.0), Tick::Animating);
    draw(&mut session, &mut scene, 400, 200);
    assert_eq!(session.tick(2000.0), Tick::Changed);
    assert_eq!(session.state()["board"], json!(B2));
}

#[test]
fn a_gesture_that_starts_a_wait_asks_for_a_frame() {
    let (_scene, mut session) = board_with_a_hotspot(json!([{
        "trigger": "mouseEnter",
        "delayMs": 500,
        "actions": [{"type": "navigate", "dest": B2}]
    }]));

    assert!(session.pointer(MOVE, 200.0, 100.0));
}

// US-33: a change to swaps the instance that holds the layer, and the board is
// laid out again with the swap (D19).

const BUTTON: &str = "cb000001";
const BUTTON_HOVER: &str = "cb000002";
const BUTTON_LABEL: &str = "ct000001";
const HOVER_LABEL: &str = "ct000002";
const HOVER_NOTE: &str = "ct000003";
const CARD: &str = "cc000001";
const CARD_BUTTON: &str = "cn000001";
const ROW: &str = "a0000020";
const FIRST: &str = "e0000001";
const OTHER: &str = "e0000002";
const GO_ON: &str = "a0000021";

/// A Button that hugs its Label with a padding of `pad`, in the variant
/// `state` of the set default | hover, whose `label` prop defaults to `label`.
fn button(id: &str, state: &str, label: &str, pad: f64, children: Vec<Value>) -> Value {
    json!({
        "id": id,
        "type": "component",
        "name": "Button",
        "width": "hug",
        "height": "hug",
        "layout": {"mode": "flex", "padding": {"top": pad, "right": pad, "bottom": pad, "left": pad}},
        "props": {
            "state": {"type": "variant", "values": ["default", "hover"], "default": state},
            "label": {"type": "text", "default": label}
        },
        "children": children
    })
}

/// A 16 px Label showing the `label` prop.
fn label(id: &str, content: &str) -> Value {
    with(
        &text(id, content, 16.0, 1.25),
        "bind",
        json!({"content": "label"}),
    )
}

/// Button (default): "Go" with a padding of 8, whose Label changes the
/// instance to the hover variant on a click. Button (hover): "Go further now"
/// and a Note "!!!!" with a padding of 20. Card hugs one default Button.
fn button_system() -> DesignSystem {
    let change = json!([{"actions": [{"type": "changeTo", "dest": BUTTON_HOVER}]}]);
    let default = button(
        BUTTON,
        "default",
        "Go",
        8.0,
        vec![with(&label(BUTTON_LABEL, "Go"), "interactions", change)],
    );
    let hover = button(
        BUTTON_HOVER,
        "hover",
        "Go further now",
        20.0,
        vec![
            label(HOVER_LABEL, "Go further now"),
            text(HOVER_NOTE, "!!!!", 16.0, 1.25),
        ],
    );
    let card = json!({
        "id": CARD,
        "type": "component",
        "name": "Card",
        "width": "hug",
        "height": "hug",
        "layout": {"mode": "flex"},
        "children": [{"id": CARD_BUTTON, "type": "instance", "name": "Button", "component": BUTTON}]
    });
    DesignSystem::from_json(&json!({"$components": [default, hover, card]}))
        .unwrap_or_else(|error| panic!("the design system fixture must be read, but: {error}"))
}

fn instance_of(id: &str, component: &str) -> Value {
    json!({"id": id, "type": "instance", "name": id, "component": component})
}

/// A row at 0, 0 that hugs `children`.
fn row(children: Vec<Value>) -> Value {
    at(
        &frame(ROW, json!("hug"), json!("hug"), flex(json!({})), children),
        0.0,
        0.0,
    )
}

/// A scene of one 400 x 300 board B1 with `children`, laid out.
fn button_scene(children: Vec<Value>) -> Scene {
    let board = at(
        &board(B1, fixed(400.0), fixed(300.0), none(), children),
        0.0,
        0.0,
    );
    let mut scene = Scene::with_system(node(&page(vec![board])), Assets::new(), button_system());
    while scene.work() {}
    scene
}

fn width_at(scene: &Scene, instance: Option<&str>, node: &str) -> f32 {
    let address = NodeAddress::new(
        instance.map(|id| id.parse().unwrap()),
        node.parse().unwrap(),
    );
    scene
        .bounds_of(&address)
        .unwrap_or_else(|| panic!("the scene has {node}"))
        .width
}

#[test]
fn a_change_to_grows_the_instance_that_holds_the_layer_and_the_row_hugging_it_but_not_its_sibling()
{
    let mut scene = button_scene(vec![row(vec![
        instance_of(FIRST, BUTTON),
        instance_of(OTHER, BUTTON),
    ])]);
    let designed = button_scene(vec![row(vec![
        instance_of(FIRST, BUTTON_HOVER),
        instance_of(OTHER, BUTTON),
    ])]);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 300);
    assert!(click(&mut session, 12.0, 18.0), "the first Button's Label");

    draw(&mut session, &mut scene, 400, 300);

    let widths = |scene: &Scene| {
        (
            width_at(scene, None, FIRST),
            width_at(scene, None, OTHER),
            width_at(scene, None, ROW),
        )
    };
    assert_eq!(widths(&scene), widths(&designed));
}

#[test]
fn a_change_to_inside_one_card_changes_only_that_cards_button() {
    let mut scene = button_scene(vec![row(vec![
        instance_of(FIRST, CARD),
        instance_of(OTHER, CARD),
    ])]);
    let designed = button_scene(vec![row(vec![
        instance_of(FIRST, BUTTON_HOVER),
        instance_of(OTHER, BUTTON),
    ])]);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 300);
    assert!(
        click(&mut session, 12.0, 18.0),
        "the first Card's Button's Label"
    );

    draw(&mut session, &mut scene, 400, 300);

    assert_eq!(
        (
            width_at(&scene, Some(FIRST), CARD_BUTTON),
            width_at(&scene, Some(OTHER), CARD_BUTTON),
        ),
        (
            width_at(&designed, None, FIRST),
            width_at(&designed, None, OTHER),
        )
    );
}

#[test]
fn a_swapped_instance_shows_the_destination_with_its_label_and_overrides_and_not_its_old_variant() {
    let overrides = json!({HOVER_NOTE: {"visible": false}});
    let send = |component: &str, props: Value| {
        with(
            &with(
                &at(&instance_of(FIRST, component), 0.0, 0.0),
                "props",
                props,
            ),
            "overrides",
            overrides.clone(),
        )
    };
    let mut scene = button_scene(vec![send(
        BUTTON,
        json!({"state": "default", "label": "Send"}),
    )]);
    let designed = button_scene(vec![send(BUTTON_HOVER, json!({"label": "Send"}))]);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 300);
    assert!(click(&mut session, 12.0, 18.0), "the Button's Label");

    draw(&mut session, &mut scene, 400, 300);

    assert_eq!(
        width_at(&scene, None, FIRST),
        width_at(&designed, None, FIRST)
    );
}

#[test]
fn a_board_a_dissolve_leaves_keeps_its_swaps_until_the_dissolve_ends() {
    let away = json!([{
        "actions": [{"type": "navigate", "dest": B2}],
        "transition": {"type": "dissolve"},
        "durationMs": 300
    }]);
    let b1 = at(
        &board(
            B1,
            fixed(400.0),
            fixed(300.0),
            none(),
            vec![
                at(&instance_of(FIRST, BUTTON), 0.0, 0.0),
                with(
                    &at(&rect(GO_ON, 100.0, 100.0), 200.0, 100.0),
                    "interactions",
                    away,
                ),
            ],
        ),
        0.0,
        0.0,
    );
    let b2 = at(
        &board(B2, fixed(400.0), fixed(300.0), none(), vec![]),
        1000.0,
        0.0,
    );
    let mut scene = Scene::with_system(node(&page(vec![b1, b2])), Assets::new(), button_system());
    while scene.work() {}
    let designed = button_scene(vec![at(&instance_of(FIRST, BUTTON), 0.0, 0.0)]);
    let swapped = button_scene(vec![at(&instance_of(FIRST, BUTTON_HOVER), 0.0, 0.0)]);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 300);
    session.tick(1000.0);
    assert!(click(&mut session, 12.0, 18.0), "the Button's Label");
    draw(&mut session, &mut scene, 400, 300);
    assert!(click(&mut session, 250.0, 150.0), "the hotspot to B2");
    session.tick(1000.0);

    session.tick(1150.0);
    draw(&mut session, &mut scene, 400, 300);
    assert_eq!(
        width_at(&scene, None, FIRST),
        width_at(&swapped, None, FIRST),
        "150 ms into the dissolve"
    );

    session.tick(1300.0);
    draw(&mut session, &mut scene, 400, 300);
    assert_eq!(
        width_at(&scene, None, FIRST),
        width_at(&designed, None, FIRST),
        "once the dissolve has ended"
    );
}

// US-34: smart animate (D16). A layer on both boards, by name under the same
// parent names, moves and restyles by the eased progress; a layer on one board
// only fades; the two roots are drawn as one layer whose fills mix. What
// cannot be mixed switches to the new board's at half the duration. easeOut
// at 200 of 400 ms is 0.685.

const BOX_OLD: &str = "a0000003";
const BOX_NEW: &str = "a0000013";

/// A `width` x `height` layer `id` named `name` at `x`, `y`, filled `color`.
fn named(id: &str, name: &str, (x, y, width, height): (f64, f64, f64, f64), color: &str) -> Value {
    solid(
        &with(&at(&rect(id, width, height), x, y), "name", json!(name)),
        color,
    )
}

fn smart_to(dest: &str) -> Value {
    json!([{
        "actions": [{"type": "navigate", "dest": dest}],
        "transition": {"type": "smartAnimate"},
        "durationMs": 400,
        "easing": "easeOut"
    }])
}

/// A black board B1 and a white board B2, 200 x 100 on a 200 x 100 stage,
/// after a click on B1 began a 400 ms easeOut smart animate to B2 at 1000 ms.
/// `b1` and `b2` are the layers each board has besides the hotspot on B1.
fn smart_animating(b1: Vec<Value>, b2: Vec<Value>) -> (Scene, Session) {
    let mut first = vec![spot(HOTSPOT, 0.0, 0.0, 200.0, 100.0, smart_to(B2))];
    first.extend(b1);
    let page = page(vec![
        filled_board(B1, 0.0, 200.0, 100.0, "#000000", first),
        filled_board(B2, 1000.0, 200.0, 100.0, "#ffffff", b2),
    ]);
    let mut scene = scene_of(&page);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 200, 100);
    session.tick(1000.0);
    click(&mut session, 100.0, 50.0);
    session.tick(1000.0);
    (scene, session)
}

/// Asserts that the pixel at `x`, `y` is `expected`, give or take 2 per
/// channel for blending.
#[track_caller]
fn assert_near(stage: &Stage, (x, y): (i32, i32), expected: [u8; 3], case: &str) {
    let actual = stage.pixel(x, y);
    let near = actual
        .iter()
        .zip(expected)
        .all(|(actual, expected)| actual.abs_diff(expected) <= 2);
    assert!(near, "{case}: {actual:?}, expected {expected:?}");
}

/// The stage 200 ms into a smart animate in which the Box moves from 0, 0,
/// 40 x 40 on B1 to 100, 40, 60 x 60 on B2 and turns from red to green; Old is
/// only on B1 at 150, 0 and New only on B2 at 0, 80; Hidden is hidden on B1 and
/// shown on B2 at 150, 30; Odd is a Text on B1 at 60, 82 and a frame on B2
/// at 100, 0; Under on B1 and Over on B2 both cover 150, 80.
fn smart_animate_at_200_ms() -> Stage {
    let (mut scene, mut session) = smart_animating(
        vec![
            named(BOX_OLD, "Box", (0.0, 0.0, 40.0, 40.0), "#ff0000"),
            named("a0000004", "Old", (150.0, 0.0, 50.0, 20.0), "#0000ff"),
            with(
                &named("a0000005", "Hidden", (150.0, 30.0, 50.0, 20.0), "#ff0000"),
                "visible",
                json!(false),
            ),
            named("a0000006", "Under", (150.0, 80.0, 50.0, 20.0), "#0000ff"),
            with(
                &at(&text("a0000008", "Odd", 16.0, 1.25), 60.0, 82.0),
                "name",
                json!("Odd"),
            ),
        ],
        vec![
            named(BOX_NEW, "Box", (100.0, 40.0, 60.0, 60.0), "#00ff00"),
            named("a0000014", "New", (0.0, 80.0, 50.0, 20.0), "#ff0000"),
            named("a0000015", "Hidden", (150.0, 30.0, 50.0, 20.0), "#ff0000"),
            named("a0000016", "Over", (150.0, 80.0, 50.0, 20.0), "#ff0000"),
            named("a0000018", "Odd", (100.0, 0.0, 40.0, 20.0), "#ff0000"),
        ],
    );
    session.tick(1200.0);
    draw(&mut session, &mut scene, 200, 100)
}

// The roots mix from black to white: 175 grey.
const ROOT_MIXED: [u8; 3] = [175, 175, 175];

#[test]
fn half_way_through_a_smart_animate_a_layer_on_both_boards_is_at_the_eased_box_and_colour() {
    let stage = smart_animate_at_200_ms();

    // The Box is at 68.5, 27.4, 53.7 x 53.7, red 31.5% and green 68.5%.
    let mixed = [80, 175, 0];
    assert_eq!(stage.pixel(70, 30), mixed, "the Box's top-left corner");
    assert_eq!(stage.pixel(120, 79), mixed, "the Box's bottom-right corner");
    assert_eq!(stage.pixel(66, 30), ROOT_MIXED, "left of the Box");
    assert_eq!(stage.pixel(90, 25), ROOT_MIXED, "above the Box");
    assert_eq!(stage.pixel(125, 50), ROOT_MIXED, "right of the Box");
    assert_eq!(stage.pixel(100, 85), ROOT_MIXED, "below the Box");
    assert_eq!(stage.pixel(30, 60), ROOT_MIXED, "the roots, mixed");
}

#[test]
fn half_way_through_a_smart_animate_a_layer_on_one_board_has_faded_by_the_eased_progress() {
    let stage = smart_animate_at_200_ms();

    // Blue at 31.5% over the grey, and red at 68.5%.
    assert_near(&stage, (175, 10), [120, 120, 200], "Old, fading out");
    assert_near(&stage, (25, 90), [230, 55, 55], "New, fading in");
    assert_near(
        &stage,
        (195, 40),
        [230, 55, 55],
        "Hidden, fading in at its own box",
    );
    assert_near(&stage, (130, 10), [230, 55, 55], "Odd, a frame fading in");
}

#[test]
fn a_layer_only_on_the_old_board_fades_out_beneath_one_only_on_the_new_board() {
    let stage = smart_animate_at_200_ms();

    // Red at 68.5% over blue at 31.5% over the grey.
    assert_near(&stage, (175, 90), [212, 38, 63], "Over, above Under");
}

/// The pixels of `stage` in `x` x `y` that are red rather than grey.
fn reddish(stage: &Stage, x: std::ops::Range<i32>, y: std::ops::Range<i32>) -> usize {
    y.flat_map(|y| x.clone().map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let [red, green, _] = stage.pixel(x, y);
            red > green.saturating_add(50)
        })
        .count()
}

#[test]
fn a_text_on_both_boards_switches_its_content_at_half_the_duration_not_at_half_the_progress() {
    let caption = |id: &str, content: &str| {
        with(
            &with(
                &at(&text(id, content, 40.0, 1.25), 10.0, 10.0),
                "name",
                json!("Caption"),
            ),
            "color",
            json!("#ff0000"),
        )
    };
    let (mut scene, mut session) = smart_animating(
        vec![caption("a0000007", ".")],
        vec![caption("a0000017", "WWWW")],
    );

    // easeOut at 150 of 400 ms is 0.54.
    session.tick(1150.0);
    let before = draw(&mut session, &mut scene, 200, 100);
    session.tick(1200.0);
    let after = draw(&mut session, &mut scene, 200, 100);

    assert_eq!(
        reddish(&before, 40..190, 0..100),
        0,
        "150 ms: the old \".\""
    );
    assert_ne!(
        reddish(&after, 40..190, 0..100),
        0,
        "200 ms: the new \"WWWW\""
    );
}

const TOGGLE_OFF: &str = "cg000001";
const TOGGLE_ON: &str = "cg000002";

/// An 80 x 40 Toggle in the variant `state` of the set off | on, filled
/// `track`, with a white 40 x 40 Knob at `knob`, 0 and a 10 x 10 Mark at 65, 28
/// filled `mark`.
fn toggle(id: &str, state: &str, track: &str, knob: f64, mark: Value, change: Value) -> Value {
    solid(
        &json!({
            "id": id,
            "type": "component",
            "name": "Toggle",
            "width": {"fixed": 80},
            "height": {"fixed": 40},
            "layout": {"mode": "none"},
            "props": {"state": {"type": "variant", "values": ["off", "on"], "default": state}},
            "children": [
                with(
                    &named(&format!("k{}", &id[1..]), "Knob", (knob, 0.0, 40.0, 40.0), "#ffffff"),
                    "interactions",
                    change
                ),
                with(
                    &named(&format!("m{}", &id[1..]), "Mark", (65.0, 28.0, 10.0, 10.0), "#ffffff"),
                    "fills",
                    mark
                )
            ]
        }),
        track,
    )
}

/// A black 200 x 100 board B1 on a 200 x 100 stage with an off Toggle at 0, 0,
/// after a click on its Knob began a 400 ms easeOut smart-animated change to
/// on at 1000 ms. The off Toggle is red with its Knob at 0 and a white Mark;
/// the on Toggle is green with its Knob at 40 and a white and a blue fill on
/// its Mark, which cannot be mixed.
fn toggling() -> (Scene, Session) {
    let change = json!([{
        "actions": [{"type": "changeTo", "dest": TOGGLE_ON}],
        "transition": {"type": "smartAnimate"},
        "durationMs": 400,
        "easing": "easeOut"
    }]);
    let white = json!({"type": "solid", "color": "#ffffff"});
    let blue = json!({"type": "solid", "color": "#0000ff"});
    let system = DesignSystem::from_json(&json!({"$components": [
        toggle(TOGGLE_OFF, "off", "#ff0000", 0.0, json!([white]), change),
        toggle(TOGGLE_ON, "on", "#00ff00", 40.0, json!([white, blue]), json!([])),
    ]}))
    .unwrap_or_else(|error| panic!("the design system fixture must be read, but: {error}"));
    let board = filled_board(
        B1,
        0.0,
        200.0,
        100.0,
        "#000000",
        vec![at(&instance_of(FIRST, TOGGLE_OFF), 0.0, 0.0)],
    );
    let mut scene = Scene::with_system(node(&page(vec![board])), Assets::new(), system);
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 200, 100);
    session.tick(1000.0);
    assert!(click(&mut session, 20.0, 20.0), "the Knob");
    session.tick(1000.0);
    draw(&mut session, &mut scene, 200, 100);
    (scene, session)
}

#[test]
fn half_way_through_a_smart_animated_change_to_the_instances_layers_are_at_the_eased_box_and_colour()
 {
    let (mut scene, mut session) = toggling();

    session.tick(1200.0);
    let stage = draw(&mut session, &mut scene, 200, 100);

    // The Knob is at 27.4..67.4 and the track red 31.5% and green 68.5%.
    let track = [80, 175, 0];
    let knob = [0xff, 0xff, 0xff];
    assert_eq!(stage.pixel(25, 20), track, "left of the Knob");
    assert_eq!(stage.pixel(29, 20), knob, "the Knob's left edge");
    assert_eq!(stage.pixel(66, 20), knob, "the Knob's right edge");
    assert_eq!(stage.pixel(70, 20), track, "right of the Knob");
    assert_eq!(
        stage.pixel(85, 20),
        [0, 0, 0],
        "the board, beside the instance"
    );
}

#[test]
fn a_smart_animated_change_to_switches_what_it_cannot_mix_at_half_the_duration() {
    let (mut scene, mut session) = toggling();

    // easeOut at 150 of 400 ms is 0.54.
    session.tick(1150.0);
    let before = draw(&mut session, &mut scene, 200, 100);
    session.tick(1200.0);
    let after = draw(&mut session, &mut scene, 200, 100);

    assert_eq!(
        before.pixel(70, 33),
        [0xff, 0xff, 0xff],
        "150 ms: the off Mark"
    );
    assert_eq!(
        after.pixel(70, 33),
        [0x00, 0x00, 0xff],
        "200 ms: the on Mark"
    );
}

fn layout_box(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
    LayoutBox {
        x,
        y,
        width,
        height,
    }
}

/// The style of a frame with `properties`.
fn style_of(properties: Value) -> Style {
    let mut frame = rect("a0000001", 10.0, 10.0);
    for (key, value) in properties.as_object().expect("properties are an object") {
        frame = with(&frame, key, value.clone());
    }
    Style::of(&node(&frame))
}

fn corners(radius: f64) -> Value {
    json!({"topLeft": radius, "topRight": radius, "bottomRight": radius, "bottomLeft": radius})
}

#[test]
fn interpolate_mixes_the_box_radius_opacity_and_a_solid_fill_straight_a_quarter_of_the_way() {
    let from = style_of(json!({
        "fills": [{"type": "solid", "color": "#ff000080"}],
        "radius": corners(8.0),
        "opacity": 1
    }));
    let to = style_of(json!({
        "fills": [{"type": "solid", "color": "#0000ff00"}],
        "radius": corners(16.0),
        "opacity": 0.5
    }));

    let mixed = interpolate(
        layout_box(0.0, 0.0, 40.0, 40.0),
        layout_box(100.0, 40.0, 60.0, 60.0),
        &from,
        &to,
        0.25,
    );

    // Each channel a quarter of the way, alpha too: not premultiplied.
    let expected = style_of(json!({
        "fills": [{"type": "solid", "color": "#bf004060"}],
        "radius": corners(10.0),
        "opacity": 0.875
    }));
    assert_eq!(mixed, (layout_box(25.0, 10.0, 45.0, 45.0), expected));
}

#[test]
fn interpolate_keeps_fills_it_cannot_mix_from_the_old_layer_until_half_way() {
    let gradient = json!({"type": "linear-gradient", "angle": 90,
        "stops": [{"position": 0, "color": "#000000"}, {"position": 1, "color": "#ffffff"}]});
    let red = json!({"type": "solid", "color": "#ff0000"});
    let green = json!({"type": "solid", "color": "#00ff00"});
    let shaded = style_of(json!({"fills": [gradient]}));
    let plain = style_of(json!({"fills": [green]}));
    let one = style_of(json!({"fills": [red]}));
    let two = style_of(json!({"fills": [red, green]}));
    let fills = |from: &Style, to: &Style, t: f64| {
        interpolate(LayoutBox::default(), LayoutBox::default(), from, to, t)
            .1
            .fills
    };

    assert_eq!(
        fills(&shaded, &plain, 0.49),
        shaded.fills,
        "a gradient, at 0.49"
    );
    assert_eq!(fills(&shaded, &plain, 0.5), plain.fills, "a solid, at 0.5");
    assert_eq!(fills(&one, &two, 0.49), one.fills, "one fill, at 0.49");
    assert_eq!(fills(&one, &two, 0.5), two.fills, "two fills, at 0.5");
}

#[test]
fn interpolate_mixes_the_colour_of_a_text() {
    let colour = |color: &str| {
        Style::of(&node(&with(
            &text("a0000001", "Go", 16.0, 1.25),
            "color",
            json!(color),
        )))
    };

    let mixed = interpolate(
        LayoutBox::default(),
        LayoutBox::default(),
        &colour("#000000"),
        &colour("#ff8040"),
        0.25,
    );

    assert_eq!(mixed.1, colour("#402010"));
}

#[test]
fn interpolate_past_the_end_carries_the_box_on_and_keeps_a_colour_in_range() {
    let from = style_of(json!({"fills": [{"type": "solid", "color": "#000000"}]}));
    let to = style_of(json!({"fills": [{"type": "solid", "color": "#ff8000"}]}));

    let mixed = interpolate(
        layout_box(0.0, 0.0, 40.0, 40.0),
        layout_box(100.0, 0.0, 60.0, 40.0),
        &from,
        &to,
        1.5,
    );

    let expected = style_of(json!({"fills": [{"type": "solid", "color": "#ffc000"}]}));
    assert_eq!(mixed, (layout_box(150.0, 0.0, 70.0, 40.0), expected));
}

const SECTION: &str = "a0000301";
const FIXED: &str = "a0000302";
const LATER: &str = "a0000303";
const PAGE_BACKGROUND: [u8; 3] = [0x33, 0x66, 0xcc];

/// A page shown on `prototype`'s device, on a #3366cc background.
fn device_page(prototype: Value, boards: Vec<Value>) -> Value {
    let mut settings = prototype;
    settings["background"] = json!("#3366cc");
    with(&page(boards), "prototype", settings)
}

fn custom(width: f64, height: f64) -> Value {
    json!({"device": "Custom", "customSize": [width, height]})
}

#[test]
fn a_fixed_frame_in_a_section_draws_above_the_sibling_of_the_section_scrolled_under_it() {
    // A 400 x 1000 board on a 400 x 400 screen: a section at the top holds a
    // red fixed frame at y 100..150, and a green frame after the section sits
    // at y 300..500.
    let fixed_frame = with(
        &solid(&at(&rect(FIXED, 400.0, 50.0), 0.0, 100.0), "#ff0000"),
        "scrollPosition",
        json!("fixed"),
    );
    let section = at(
        &frame(
            SECTION,
            fixed(400.0),
            fixed(300.0),
            none(),
            vec![fixed_frame],
        ),
        0.0,
        0.0,
    );
    let later = solid(&at(&rect(LATER, 400.0, 200.0), 0.0, 300.0), "#00ff00");
    let mut scene = scene_of(&device_page(
        custom(400.0, 400.0),
        vec![filled_board(
            B1,
            0.0,
            400.0,
            1000.0,
            "#ffffff",
            vec![section, later],
        )],
    ));
    let mut session = Session::start(&scene, B1).expect("the page has the board");
    draw(&mut session, &mut scene, 400, 400);

    // Scrolled 250, the green frame is at 50..250, under the fixed one.
    assert!(session.wheel(200.0, 350.0, 0.0, 250.0));
    let stage = draw(&mut session, &mut scene, 400, 400);

    assert_eq!(stage.pixel(200, 125), [255, 0, 0], "the fixed frame");
    assert_eq!(
        stage.pixel(200, 75),
        [0, 255, 0],
        "the green frame above it"
    );
}

#[test]
fn a_board_taller_than_the_device_screen_is_cut_at_the_screens_bottom_edge() {
    // A 200 x 200 screen on a 400 x 400 stage spans 100..300 on both axes.
    let mut scene = scene_of(&device_page(
        custom(200.0, 200.0),
        vec![filled_board(B1, 0.0, 200.0, 400.0, "#ff0000", vec![])],
    ));
    let mut session = Session::start(&scene, B1).expect("the page has the board");

    let stage = draw(&mut session, &mut scene, 400, 400);

    assert_eq!(stage.pixel(200, 295), [255, 0, 0], "inside the screen");
    assert_eq!(stage.pixel(200, 305), PAGE_BACKGROUND, "below the screen");
}

#[test]
fn at_50_percent_the_screen_is_drawn_at_half_its_size_in_the_middle() {
    // A 200 x 200 screen at half size on a 400 x 400 stage spans 150..250.
    let mut scene = scene_of(&device_page(
        custom(200.0, 200.0),
        vec![filled_board(B1, 0.0, 200.0, 200.0, "#ff0000", vec![])],
    ));
    let mut session = Session::start(&scene, B1).expect("the page has the board");

    session.set_scale("50").expect("50 is a scale");
    let stage = draw(&mut session, &mut scene, 400, 400);

    assert_eq!(stage.pixel(155, 155), [255, 0, 0], "inside its top-left");
    assert_eq!(
        stage.pixel(245, 245),
        [255, 0, 0],
        "inside its bottom-right"
    );
    assert_eq!(stage.pixel(145, 200), PAGE_BACKGROUND, "left of it");
    assert_eq!(stage.pixel(255, 200), PAGE_BACKGROUND, "right of it");
}

#[test]
fn at_100_percent_a_screen_wider_than_the_stage_is_not_scaled_down() {
    // An 800 x 400 screen at full size on a 400 x 400 stage fills it; at fit
    // it would be drawn at half size, at y 100..300.
    let mut scene = scene_of(&device_page(
        custom(800.0, 400.0),
        vec![filled_board(B1, 0.0, 800.0, 400.0, "#ff0000", vec![])],
    ));
    let mut session = Session::start(&scene, B1).expect("the page has the board");

    session.set_scale("100").expect("100 is a scale");
    let stage = draw(&mut session, &mut scene, 400, 400);

    assert_eq!(stage.pixel(200, 50), [255, 0, 0]);
}

#[test]
fn the_device_select_starts_at_the_pages_device_turned_when_the_page_says_landscape() {
    let scene = scene_of(&device_page(
        json!({"device": "iPhone 16", "rotation": "landscape"}),
        vec![filled_board(B1, 0.0, 400.0, 400.0, "#ffffff", vec![])],
    ));
    let session = Session::start(&scene, B1).expect("the page has the board");

    let screen = session.screen();
    let listed = |name: &str| {
        screen["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|device| device["name"] == name)
            .cloned()
    };

    assert_eq!(screen["device"], "iPhone 16");
    assert_eq!(
        listed("iPhone 16"),
        Some(json!({"name": "iPhone 16", "width": 852.0, "height": 393.0})),
        "a phone turns"
    );
    assert_eq!(
        listed("MacBook Air"),
        Some(json!({"name": "MacBook Air", "width": 1280.0, "height": 832.0})),
        "a laptop does not"
    );
}
