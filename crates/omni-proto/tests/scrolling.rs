//! US-37: scrolling in play mode. A top-level board bigger than the device
//! screen scrolls as a whole, a frame scrolls its content on the axes of its
//! `overflow`, fixed and sticky layers stay where the decisions put them, and
//! `scrollTo`, `navigate`'s `preserveScroll` and back set the offsets.

use omni_core::design::{Node, NodeData, NodeId};
use omni_proto::{Player, PointerEvent, PointerKind, Rect};
use serde_json::{Value, json};
use std::collections::HashMap;

const B1: &str = "b0000001";
const B2: &str = "b0000002";
const B3: &str = "b0000003";
const HEADER: &str = "a0000001";
const LIST: &str = "a0000002";
const ITEM: &str = "a0000003";
const ROW: &str = "a0000004";
const OTHER: &str = "a0000005";
const SECTION: &str = "a0000006";
const STICKY: &str = "a0000007";

fn id(text: &str) -> NodeId {
    text.parse().unwrap()
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// A frame with `fields` on top of its id, name, size and layout.
fn layer(id: &str, fields: Value) -> Value {
    let mut layer = json!({"id": id, "type": "frame", "name": id, "width": {"fixed": 100},
        "height": {"fixed": 100}, "layout": {"mode": "none"}});
    layer
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    layer
}

fn board(id: &str, children: Vec<Value>) -> Value {
    json!({"id": id, "type": "board", "name": id, "width": {"fixed": 100},
        "height": {"fixed": 100}, "layout": {"mode": "none"}, "children": children})
}

fn navigate(dest: &str) -> Value {
    json!([{"actions": [{"type": "navigate", "dest": dest}]}])
}

/// A player on `B1` with `boards`, each layer at the box `boxed` gives it, on
/// a device screen of `screen`.
fn player(boards: Vec<Value>, boxed: &[(&str, Rect)], screen: Option<(f64, f64)>) -> Player {
    let page: Node = serde_json::from_value(json!({
        "id": "a0000000", "type": "page", "name": "Page", "children": boards,
    }))
    .unwrap();
    let NodeData::Page(page) = page.data() else {
        unreachable!("a page was built")
    };
    let mut player = Player::new(page, HashMap::new(), id(B1));
    let rects: HashMap<_, _> = boxed
        .iter()
        .map(|(layer, rect)| (id(layer), *rect))
        .collect();
    for board in &page.children {
        player.set_boxes(*board.id(), rects.clone(), HashMap::new());
    }
    if let Some((width, height)) = screen {
        player.set_screen(width, height);
    }
    player
}

fn event(kind: PointerKind, x: f64, y: f64, at: f64) -> PointerEvent {
    PointerEvent { kind, x, y, at }
}

fn click(player: &mut Player, x: f64, y: f64, at: f64) {
    player.pointer(event(PointerKind::Down, x, y, at));
    player.pointer(event(PointerKind::Up, x, y, at + 10.0));
}

/// How far `layer` of `board` is moved from its design at `now`.
fn shift(player: &Player, board: &str, layer: &str, now: f64) -> [f64; 2] {
    player.scroll_view(id(board), now).shifts[&id(layer)]
}

/// `B1`, 400 x 1000 on a 400 x 500 screen, so that it scrolls 500 at most:
/// a fixed header at the top, 60 high, with `header`'s interactions; a list
/// at y 100, 300 high, scrolling vertically, whose 500 high item lets it
/// scroll 200; and a row at y 500, 100 high, with `row`'s. `B2` is as long,
/// with a row of its own at y 300; `B3` is empty.
fn phone(header: Value, row: Value) -> Player {
    let b1 = board(
        B1,
        vec![
            layer(
                HEADER,
                json!({"scrollPosition": "fixed", "interactions": header}),
            ),
            layer(
                LIST,
                json!({"overflow": "vertical", "children": [layer(ITEM, json!({}))]}),
            ),
            layer(ROW, json!({"interactions": row})),
        ],
    );
    let b2 = board(B2, vec![layer(OTHER, json!({}))]);
    player(
        vec![b1, b2, board(B3, vec![])],
        &[
            (B1, rect(0.0, 0.0, 400.0, 1000.0)),
            (B2, rect(0.0, 0.0, 400.0, 1000.0)),
            (HEADER, rect(0.0, 0.0, 400.0, 60.0)),
            (LIST, rect(0.0, 100.0, 400.0, 300.0)),
            (ITEM, rect(0.0, 100.0, 400.0, 500.0)),
            (ROW, rect(0.0, 500.0, 400.0, 100.0)),
            (OTHER, rect(0.0, 300.0, 400.0, 100.0)),
        ],
        Some((400.0, 500.0)),
    )
}

/// A point on the board that no scrolling frame covers at the top.
const ON_THE_BOARD: (f64, f64) = (200.0, 450.0);

/// A point on the list.
const ON_THE_LIST: (f64, f64) = (200.0, 200.0);

#[test]
fn a_board_scrolls_no_further_than_its_height_minus_the_screen_and_no_further_back_than_its_top() {
    let mut player = phone(json!([]), json!([]));
    let (x, y) = ON_THE_BOARD;

    player.scroll_at(x, y, 0.0, 800.0);
    assert_eq!(shift(&player, B1, ROW, 0.0), [0.0, -500.0], "past the end");

    player.scroll_at(x, y, 0.0, -1000.0);
    assert_eq!(shift(&player, B1, ROW, 0.0), [0.0, 0.0], "past the top");
}

#[test]
fn a_wheel_over_a_scrolling_frame_scrolls_it_to_its_end_and_the_rest_scrolls_the_board() {
    let mut player = phone(json!([]), json!([]));
    let (x, y) = ON_THE_LIST;

    player.scroll_at(x, y, 0.0, 300.0);

    assert_eq!(shift(&player, B1, LIST, 0.0), [0.0, -100.0], "the board");
    assert_eq!(
        shift(&player, B1, ITEM, 0.0),
        [0.0, -300.0],
        "the list's 200 and the board's 100"
    );
}

#[test]
fn a_drag_on_a_long_board_scrolls_it_by_the_distance_the_pointer_moved() {
    let mut player = phone(json!([]), json!([]));

    player.pointer(event(PointerKind::Down, 200.0, 480.0, 0.0));
    player.pointer(event(PointerKind::Move, 200.0, 440.0, 10.0));
    player.pointer(event(PointerKind::Move, 200.0, 380.0, 20.0));
    player.pointer(event(PointerKind::Up, 200.0, 380.0, 30.0));

    assert_eq!(shift(&player, B1, ROW, 30.0), [0.0, -100.0]);
}

#[test]
fn a_horizontal_frame_scrolls_its_content_sideways_and_not_down() {
    let strip = layer(
        LIST,
        json!({"overflow": "horizontal", "children": [layer(ITEM, json!({}))]}),
    );
    let mut player = player(
        vec![board(B1, vec![strip])],
        &[
            (B1, rect(0.0, 0.0, 400.0, 400.0)),
            (LIST, rect(0.0, 0.0, 400.0, 100.0)),
            (ITEM, rect(0.0, 0.0, 1000.0, 300.0)),
        ],
        None,
    );

    assert!(!player.scroll_at(50.0, 50.0, 0.0, 50.0), "down");
    assert!(player.scroll_at(50.0, 50.0, 50.0, 0.0), "sideways");
    assert_eq!(shift(&player, B1, ITEM, 0.0), [-50.0, 0.0]);
}

#[test]
fn a_fixed_header_stays_put_and_takes_the_click_over_the_row_scrolled_under_it() {
    let mut player = phone(navigate(B2), navigate(B3));
    let (x, y) = ON_THE_BOARD;
    // The row, at y 500, is drawn at 20..120 under the header at 0..60.
    player.scroll_at(x, y, 0.0, 480.0);

    assert_eq!(shift(&player, B1, HEADER, 0.0), [0.0, 0.0]);
    click(&mut player, 200.0, 30.0, 0.0);
    assert_eq!(player.frame().board, id(B2));
}

#[test]
fn back_opens_the_board_at_the_offsets_it_and_its_frames_had_when_it_was_left() {
    let mut player = phone(navigate(B2), json!([]));
    let (x, y) = ON_THE_LIST;
    player.scroll_at(x, y, 0.0, 150.0);
    let (x, y) = ON_THE_BOARD;
    player.scroll_at(x, y, 0.0, 50.0);
    click(&mut player, 200.0, 30.0, 0.0);
    assert_eq!(player.frame().board, id(B2));

    player.back();

    assert_eq!(player.frame().board, id(B1));
    assert_eq!(shift(&player, B1, LIST, 100.0), [0.0, -50.0], "the board");
    assert_eq!(
        shift(&player, B1, ITEM, 100.0),
        [0.0, -200.0],
        "the list's 150 and the board's 50"
    );
}

fn navigate_preserving(preserve: bool) -> Value {
    json!([{"actions": [{"type": "navigate", "dest": B2, "preserveScroll": preserve}]}])
}

#[test]
fn navigate_with_preserve_scroll_opens_the_destination_at_the_offset_the_board_had() {
    let mut player = phone(navigate_preserving(true), json!([]));
    let (x, y) = ON_THE_BOARD;
    player.scroll_at(x, y, 0.0, 150.0);

    click(&mut player, 200.0, 30.0, 0.0);

    assert_eq!(player.frame().board, id(B2));
    assert_eq!(shift(&player, B2, OTHER, 10.0), [0.0, -150.0]);
}

#[test]
fn navigate_without_preserve_scroll_opens_the_destination_at_the_top() {
    let mut player = phone(navigate_preserving(false), json!([]));
    let (x, y) = ON_THE_BOARD;
    player.scroll_at(x, y, 0.0, 150.0);

    click(&mut player, 200.0, 30.0, 0.0);

    assert_eq!(player.frame().board, id(B2));
    assert_eq!(shift(&player, B2, OTHER, 10.0), [0.0, 0.0]);
}

#[test]
fn scroll_to_glides_the_board_until_the_layer_sits_its_offset_below_the_top() {
    let mut player = phone(
        json!([{
            "actions": [{"type": "scrollTo", "dest": ROW, "offset": [0, 56]}],
            "durationMs": 200,
            "easing": "linear"
        }]),
        json!([]),
    );
    click(&mut player, 200.0, 30.0, 0.0);
    // The scroll counts from the first tick after the click.
    player.tick(100.0);

    // The row, at y 500, ends at 56: the board scrolls 444.
    assert_eq!(shift(&player, B1, ROW, 200.0), [0.0, -222.0], "half way");
    assert_eq!(shift(&player, B1, ROW, 300.0), [0.0, -444.0], "at the end");
}

#[test]
fn hotspot_outlines_are_cut_to_the_screen_and_one_below_it_has_none() {
    let player = player(
        vec![
            board(
                B1,
                vec![
                    layer(ROW, json!({"interactions": navigate(B2)})),
                    layer(OTHER, json!({"interactions": navigate(B2)})),
                ],
            ),
            board(B2, vec![]),
        ],
        &[
            (B1, rect(0.0, 0.0, 400.0, 1000.0)),
            (ROW, rect(0.0, 700.0, 100.0, 200.0)),
            (OTHER, rect(0.0, 900.0, 100.0, 50.0)),
        ],
        Some((400.0, 800.0)),
    );

    assert_eq!(player.hotspot_rects(), vec![rect(0.0, 700.0, 100.0, 100.0)]);
}

/// `B1`, 400 x 2000 on a 400 x 800 screen, with a section at y 600 holding a
/// sticky layer 60 below the section's top.
fn sticky_board() -> Player {
    let section = layer(
        SECTION,
        json!({"children": [layer(STICKY, json!({"scrollPosition": "sticky"}))]}),
    );
    player(
        vec![board(B1, vec![section])],
        &[
            (B1, rect(0.0, 0.0, 400.0, 2000.0)),
            (SECTION, rect(0.0, 600.0, 400.0, 400.0)),
            (STICKY, rect(0.0, 660.0, 400.0, 40.0)),
        ],
        Some((400.0, 800.0)),
    )
}

#[test]
fn a_sticky_layer_scrolls_until_it_is_its_offset_in_its_section_below_the_top_and_then_stays() {
    let mut player = sticky_board();

    player.scroll_at(200.0, 100.0, 0.0, 300.0);
    assert_eq!(
        shift(&player, B1, STICKY, 0.0),
        [0.0, -300.0],
        "at 360, still scrolling"
    );

    player.scroll_at(200.0, 100.0, 0.0, 700.0);
    assert_eq!(
        shift(&player, B1, STICKY, 0.0),
        [0.0, -600.0],
        "stuck at 60 rather than at -340"
    );
}

#[test]
fn a_stuck_sticky_layer_draws_above_the_boards_content_and_one_still_scrolling_does_not() {
    let mut player = sticky_board();

    player.scroll_at(200.0, 100.0, 0.0, 300.0);
    assert_eq!(player.scroll_view(id(B1), 0.0).raised, vec![], "scrolling");

    player.scroll_at(200.0, 100.0, 0.0, 700.0);
    assert_eq!(
        player.scroll_view(id(B1), 0.0).raised,
        vec![(id(STICKY), id(B1))],
        "stuck"
    );
}
