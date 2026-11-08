//! The player's behaviour: what a click, back and the actions do to the
//! frame, the history and the effects, ported from the behaviour tests of
//! FigCraft's `proto_run.rs` (MIT OR Apache-2.0; see `NOTICE`).

use omni_core::design::proto::{Direction, Easing, Transition};
use omni_core::design::{Node, NodeAddress, NodeData, NodeId};
use omni_proto::{Effect, KeyEvent, Player, PointerEvent, PointerKind, Rect};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

const B1: &str = "b0000001";
const B2: &str = "b0000002";
const B3: &str = "b0000003";
const GONE: &str = "b0000009";
const L1: &str = "a0000001";
const L2: &str = "a0000002";
const COMP: &str = "c0000001";
const COMP_ON: &str = "c0000002";
const INNER: &str = "d0000001";
const INST_A: &str = "e0000001";
const INST_B: &str = "e0000002";
const RESOLVED_A: &str = "f0000001";
const RESOLVED_B: &str = "f0000002";

fn size() -> Value {
    json!({"fixed": 100})
}

fn id(text: &str) -> NodeId {
    text.parse().unwrap()
}

fn node(value: Value) -> Node {
    serde_json::from_value(value).unwrap()
}

fn navigate(dest: &str) -> Value {
    json!({"actions": [{"type": "navigate", "dest": dest}]})
}

fn actions(actions: Value) -> Value {
    json!({"actions": actions})
}

fn layer(id: &str, interactions: Value) -> Value {
    json!({"id": id, "type": "frame", "name": id, "width": size(), "height": size(),
        "layout": {"mode": "none"}, "interactions": interactions})
}

fn board(id: &str, children: Vec<Value>) -> Value {
    json!({"id": id, "type": "board", "name": id, "width": size(), "height": size(),
        "layout": {"mode": "none"}, "children": children})
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn event(kind: PointerKind, x: f64, y: f64, at: f64) -> PointerEvent {
    PointerEvent { kind, x, y, at }
}

fn click(player: &mut Player, x: f64, y: f64, at: f64) -> Vec<Effect> {
    let mut effects = player.pointer(event(PointerKind::Down, x, y, at));
    effects.extend(player.pointer(event(PointerKind::Up, x, y, at + 10.0)));
    effects
}

/// A player on `B1` whose boards are `boards`, with a box of 100 × 100 at the
/// origin for every layer in `boxed`.
fn player(boards: Vec<Value>, components: Vec<Value>, boxed: &[(&str, Rect)]) -> Player {
    let page = node(json!({
        "id": "a0000000", "type": "page", "name": "Page",
        "children": boards,
    }));
    let NodeData::Page(page) = page.data() else {
        unreachable!("a page was built")
    };
    let components = components
        .into_iter()
        .map(node)
        .map(|node| (*node.id(), node))
        .collect();
    let mut player = Player::new(page, components, id(B1));
    let rects: HashMap<_, _> = boxed
        .iter()
        .map(|(layer, rect)| (id(layer), *rect))
        .collect();
    for board in &page.children {
        player.set_boxes(*board.id(), rects.clone(), HashMap::new());
    }
    player
}

fn two_boards(interactions: Value) -> Player {
    player(
        vec![board(B1, vec![layer(L1, interactions)]), board(B2, vec![])],
        vec![],
        &[(L1, rect(0.0, 0.0, 100.0, 100.0))],
    )
}

#[test]
fn a_click_navigates_with_the_interactions_transition() {
    let mut player = two_boards(json!([{
        "actions": [{"type": "navigate", "dest": B2}],
        "transition": {"type": "slideIn", "direction": "left"},
        "durationMs": 200,
        "easing": "linear"
    }]));
    player.pointer(event(PointerKind::Down, 10.0, 10.0, 0.0));
    let effects = player.pointer(event(PointerKind::Up, 12.0, 10.0, 50.0));

    assert_eq!(effects, vec![Effect::NeedsFrame]);
    let frame = player.frame();
    assert_eq!(frame.board, id(B2));
    assert_eq!(player.history(), &[id(B1)]);
    let running = frame.transition.unwrap();
    assert_eq!(running.from, id(B1));
    assert_eq!(running.to, id(B2));
    assert_eq!(
        running.transition,
        Transition::SlideIn {
            direction: Direction::Left
        }
    );
    assert_eq!(running.duration_ms, 200);
    assert_eq!(running.easing, Easing::Linear);
    assert!(!running.reversed);
    assert_eq!(running.started, 50.0);
}

#[test]
fn a_drag_and_a_click_on_nothing_change_nothing() {
    let mut player = two_boards(json!([navigate(B2)]));
    player.pointer(event(PointerKind::Down, 10.0, 10.0, 0.0));
    player.pointer(event(PointerKind::Move, 20.0, 10.0, 10.0));
    let dragged = player.pointer(event(PointerKind::Up, 20.0, 10.0, 20.0));
    let missed = click(&mut player, 500.0, 500.0, 100.0);

    assert!(dragged.is_empty());
    assert_eq!(missed, vec![Effect::Missed, Effect::NeedsFrame]);
    assert_eq!(player.frame().board, id(B1));
    assert!(player.history().is_empty());
}

#[test]
fn the_deepest_layer_with_the_trigger_wins() {
    let inner = layer(L2, json!([navigate(B3)]));
    let mut outer = layer(L1, json!([navigate(B2)]));
    outer["children"] = json!([inner]);
    let mut player = player(
        vec![board(B1, vec![outer]), board(B2, vec![]), board(B3, vec![])],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(0.0, 0.0, 50.0, 50.0)),
        ],
    );

    click(&mut player, 10.0, 10.0, 0.0);
    assert_eq!(player.frame().board, id(B3));
}

#[test]
fn back_undoes_a_navigate_with_its_transition_reversed() {
    let mut player = two_boards(json!([{
        "actions": [{"type": "navigate", "dest": B2}],
        "transition": {"type": "push", "direction": "right"},
        "durationMs": 120
    }]));
    player.back();
    assert_eq!(player.frame().board, id(B1));

    click(&mut player, 10.0, 10.0, 0.0);
    player.tick(500.0);
    player.back();

    let frame = player.frame();
    assert_eq!(frame.board, id(B1));
    assert!(player.history().is_empty());
    let running = frame.transition.unwrap();
    assert_eq!(running.from, id(B2));
    assert_eq!(running.to, id(B1));
    assert!(running.reversed);
    assert_eq!(running.duration_ms, 120);
    assert_eq!(
        running.transition,
        Transition::Push {
            direction: Direction::Left
        }
    );
    assert_eq!(running.started, 500.0);
}

#[test]
fn back_after_a_slide_in_plays_the_slide_out_the_other_way_with_its_duration_and_easing() {
    let mut player = two_boards(json!([{
        "actions": [{"type": "navigate", "dest": B2}],
        "transition": {"type": "slideIn", "direction": "left"},
        "durationMs": 300,
        "easing": "easeIn"
    }]));
    click(&mut player, 10.0, 10.0, 0.0);
    player.tick(500.0);

    player.back();

    let running = player.frame().transition.unwrap();
    assert_eq!(
        running.transition,
        Transition::SlideOut {
            direction: Direction::Right
        }
    );
    assert!(running.reversed);
    assert_eq!(running.duration_ms, 300);
    assert_eq!(running.easing, Easing::EaseIn);
}

#[test]
fn overlays_push_replace_and_pop_without_a_history_step() {
    let open = json!({"type": "openOverlay", "dest": B2, "position": "manual", "offset": [5, 6]});
    let mut player = player(
        vec![
            board(B1, vec![layer(L1, json!([actions(json!([open]))]))]),
            board(
                B2,
                vec![layer(
                    L2,
                    json!([actions(json!([{"type": "swapOverlay", "dest": B3}]))]),
                )],
            ),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
            (L2, rect(60.0, 0.0, 50.0, 50.0)),
            (B2, rect(0.0, 0.0, 100.0, 100.0)),
            (B3, rect(0.0, 0.0, 100.0, 100.0)),
        ],
    );

    let opened = click(&mut player, 10.0, 10.0, 0.0);
    assert_eq!(opened, vec![Effect::NeedsFrame]);
    let frame = player.frame();
    assert_eq!(frame.board, id(B1));
    assert_eq!(frame.overlays.len(), 1);
    assert_eq!(frame.overlays[0].board, id(B2));
    assert_eq!(frame.overlays[0].origin, [5.0, 6.0]);
    assert!(frame.transition.is_none());

    let swapped = click(&mut player, 70.0, 10.0, 100.0);
    assert_eq!(swapped, vec![Effect::NeedsFrame]);
    let frame = player.frame();
    assert_eq!(frame.overlays.len(), 1);
    assert!(frame.transition.is_none());
    assert_eq!(frame.overlays[0].board, id(B3));
    assert!(player.history().is_empty());
}

#[test]
fn close_overlay_pops_and_navigate_clears_overlays() {
    let mut player = player(
        vec![
            board(
                B1,
                vec![layer(
                    L1,
                    json!([actions(json!([{"type": "openOverlay", "dest": B2}]))]),
                )],
            ),
            board(
                B2,
                vec![
                    layer(
                        "a0000003",
                        json!([actions(json!([{"type": "closeOverlay"}]))]),
                    ),
                    layer("a0000004", json!([navigate(B3)])),
                ],
            ),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (B1, rect(0.0, 0.0, 300.0, 300.0)),
            (B2, rect(0.0, 0.0, 100.0, 200.0)),
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
            ("a0000003", rect(0.0, 100.0, 50.0, 50.0)),
            ("a0000004", rect(0.0, 0.0, 50.0, 50.0)),
        ],
    );

    // The overlay is centred, so its top-left corner is at 100, 50.
    click(&mut player, 10.0, 10.0, 0.0);
    let closed = click(&mut player, 110.0, 160.0, 100.0);
    assert_eq!(closed, vec![Effect::NeedsFrame]);
    assert!(player.frame().overlays.is_empty());

    click(&mut player, 10.0, 10.0, 200.0);
    assert_eq!(player.frame().overlays.len(), 1);
    click(&mut player, 110.0, 60.0, 300.0);
    let frame = player.frame();
    assert_eq!(frame.board, id(B3));
    assert!(frame.overlays.is_empty());
    assert_eq!(player.history(), &[id(B1)]);
}

#[test]
fn a_destination_outside_the_page_is_ignored() {
    let mut player = two_boards(json!([navigate(GONE)]));
    let before = player.frame();

    let effects = click(&mut player, 10.0, 10.0, 0.0);

    assert!(effects.is_empty());
    assert_eq!(player.frame(), before);
    assert!(player.history().is_empty());
}

#[test]
fn open_url_returns_an_effect_and_changes_nothing_else() {
    let mut player = two_boards(json!([actions(json!([
        {"type": "openUrl", "url": "https://example.com", "newTab": false}
    ]))]));
    let before = player.frame();

    let effects = click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(
        effects,
        vec![Effect::OpenUrl {
            url: "https://example.com".into(),
            new_tab: false
        }]
    );
    assert_eq!(player.frame(), before);
}

#[test]
fn every_action_of_an_interaction_runs_in_order() {
    let mut player = player(
        vec![
            board(
                B1,
                vec![layer(
                    L1,
                    json!([actions(json!([
                        {"type": "navigate", "dest": B2},
                        {"type": "openUrl", "url": "https://example.com"},
                        {"type": "navigate", "dest": B3}
                    ]))]),
                )],
            ),
            board(B2, vec![]),
            board(B3, vec![]),
        ],
        vec![],
        &[(L1, rect(0.0, 0.0, 100.0, 100.0))],
    );

    let effects = click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(effects.len(), 2);
    assert!(matches!(effects[0], Effect::OpenUrl { new_tab: true, .. }));
    assert_eq!(effects[1], Effect::NeedsFrame);
    assert_eq!(player.frame().board, id(B3));
    assert_eq!(player.history(), &[id(B1), id(B2)]);
}

#[test]
fn a_transition_ends_when_its_duration_has_passed() {
    let mut player = two_boards(json!([{
        "actions": [{"type": "navigate", "dest": B2}],
        "durationMs": 200
    }]));
    assert!(player.tick(0.0).is_empty());
    player.pointer(event(PointerKind::Down, 10.0, 10.0, 1000.0));
    player.pointer(event(PointerKind::Up, 10.0, 10.0, 1000.0));

    assert_eq!(player.tick(1100.0), vec![Effect::NeedsFrame]);
    assert!(player.frame().transition.is_some());
    assert_eq!(player.tick(1200.0), vec![Effect::NeedsFrame]);
    assert!(player.frame().transition.is_none());
    assert!(player.tick(1300.0).is_empty());
}

fn component_with_layer(component: &str, interactions: Value) -> Value {
    json!({
        "id": component,
        "type": "component",
        "name": "Toggle",
        "width": size(),
        "height": size(),
        "layout": {"mode": "none"},
        "children": [layer(INNER, interactions)]
    })
}

fn instance(instance: &str, interactions: Value) -> Value {
    json!({
        "id": instance,
        "type": "instance",
        "name": instance,
        "component": COMP,
        "interactions": interactions
    })
}

fn instance_player(instances: Vec<Value>, components: Vec<Value>) -> Player {
    let boxed = [
        (RESOLVED_A, rect(0.0, 0.0, 50.0, 50.0)),
        (RESOLVED_B, rect(100.0, 0.0, 50.0, 50.0)),
        (INST_A, rect(0.0, 0.0, 50.0, 50.0)),
        (INST_B, rect(100.0, 0.0, 50.0, 50.0)),
    ];
    let mut player = player(
        vec![board(B1, instances), board(B2, vec![])],
        components,
        &boxed,
    );
    let layers = HashMap::from([
        (
            id(RESOLVED_A),
            NodeAddress::new(Some(id(INST_A)), id(INNER)),
        ),
        (
            id(RESOLVED_B),
            NodeAddress::new(Some(id(INST_B)), id(INNER)),
        ),
    ]);
    let rects = boxed
        .iter()
        .map(|(layer, rect)| (id(layer), *rect))
        .collect();
    player.set_boxes(id(B1), rects, layers);
    player
}

#[test]
fn an_instance_plays_its_variants_interactions_unless_it_has_its_own() {
    let mut variant = component_with_layer(COMP, json!([]));
    variant["interactions"] = json!([navigate(B2)]);
    let mut player = instance_player(
        vec![
            instance(INST_A, json!([])),
            instance(
                INST_B,
                json!([actions(
                    json!([{"type": "openUrl", "url": "https://own.example.com"}])
                )]),
            ),
        ],
        vec![variant],
    );

    let own = click(&mut player, 110.0, 10.0, 0.0);
    assert!(matches!(own[0], Effect::OpenUrl { .. }));
    assert_eq!(player.frame().board, id(B1));

    click(&mut player, 10.0, 10.0, 100.0);
    assert_eq!(player.frame().board, id(B2));
}

#[test]
fn change_to_in_a_component_swaps_the_instance_that_holds_the_layer() {
    let change = json!([actions(json!([{"type": "changeTo", "dest": COMP_ON}]))]);
    let mut player = instance_player(
        vec![instance(INST_A, json!([])), instance(INST_B, json!([]))],
        vec![
            component_with_layer(COMP, change),
            component_with_layer(COMP_ON, json!([])),
        ],
    );

    let effects = click(&mut player, 110.0, 10.0, 0.0);

    assert_eq!(effects, vec![Effect::NeedsFrame]);
    let frame = player.frame();
    assert_eq!(frame.variant_swaps.len(), 1);
    assert_eq!(frame.variant_swaps[&id(INST_B)], id(COMP_ON));
    assert!(frame.transition.is_none());
    assert!(player.history().is_empty());

    // The instance now shows the variant, which has no interaction.
    let after = click(&mut player, 110.0, 10.0, 100.0);
    assert_eq!(after, vec![Effect::Missed, Effect::NeedsFrame]);
}

#[test]
fn change_to_fired_by_the_instance_itself_swaps_that_instance() {
    let change = json!([actions(json!([{"type": "changeTo", "dest": COMP_ON}]))]);
    let mut variant = component_with_layer(COMP, json!([]));
    variant["interactions"] = change.clone();
    let mut player = instance_player(
        vec![instance(INST_A, change), instance(INST_B, json!([]))],
        vec![variant, component_with_layer(COMP_ON, json!([]))],
    );

    // INST_A fires its own interaction, INST_B its variant's root one.
    click(&mut player, 10.0, 10.0, 0.0);
    click(&mut player, 110.0, 10.0, 100.0);

    let swaps = player.frame().variant_swaps;
    assert_eq!(swaps.len(), 2);
    assert_eq!(swaps[&id(INST_A)], id(COMP_ON));
    assert_eq!(swaps[&id(INST_B)], id(COMP_ON));
}

#[test]
fn change_to_a_component_the_design_system_lacks_is_ignored() {
    let change = json!([actions(json!([{"type": "changeTo", "dest": "c0000009"}]))]);
    let mut player = instance_player(
        vec![instance(INST_A, json!([]))],
        vec![component_with_layer(COMP, change)],
    );

    let effects = click(&mut player, 10.0, 10.0, 0.0);

    assert!(effects.is_empty());
    assert!(player.frame().variant_swaps.is_empty());
}

#[test]
fn hotspots_list_the_layers_with_interactions_and_their_given_boxes() {
    let mut player = player(
        vec![
            board(
                B1,
                vec![
                    layer(L1, json!([navigate(B2)])),
                    layer(L2, json!([])),
                    layer("a0000003", json!([navigate(B2)])),
                ],
            ),
            board(B2, vec![layer("a0000004", json!([navigate(B1)]))]),
        ],
        vec![],
        &[],
    );
    player.set_boxes(id(B1), HashMap::new(), HashMap::new());
    let boxes = HashMap::from([
        (id(L1), rect(1.0, 2.0, 3.0, 4.0)),
        (id(L2), rect(0.0, 0.0, 9.0, 9.0)),
        (id("a0000004"), rect(5.0, 5.0, 5.0, 5.0)),
    ]);

    assert_eq!(
        player.hotspots(&boxes),
        vec![(id(L1), rect(1.0, 2.0, 3.0, 4.0))]
    );

    player.pointer(event(PointerKind::Down, 0.0, 0.0, 0.0));
    player.restart(id(B2));
    assert_eq!(
        player.hotspots(&boxes),
        vec![(id("a0000004"), rect(5.0, 5.0, 5.0, 5.0))]
    );
}

#[test]
fn restart_clears_history_overlays_and_swaps() {
    let change = json!([actions(json!([{"type": "changeTo", "dest": COMP_ON}]))]);
    let open = json!([actions(json!([{"type": "openOverlay", "dest": B3}]))]);
    let mut player = player(
        vec![
            board(
                B1,
                vec![
                    instance(INST_A, json!([])),
                    layer(L1, json!([navigate(B2)])),
                ],
            ),
            board(B2, vec![layer(L2, open)]),
            board(B3, vec![]),
        ],
        vec![
            component_with_layer(COMP, change),
            component_with_layer(COMP_ON, json!([])),
        ],
        &[],
    );
    player.set_boxes(
        id(B1),
        HashMap::from([
            (id(RESOLVED_A), rect(0.0, 0.0, 50.0, 50.0)),
            (id(L1), rect(100.0, 0.0, 50.0, 50.0)),
        ]),
        HashMap::from([(
            id(RESOLVED_A),
            NodeAddress::new(Some(id(INST_A)), id(INNER)),
        )]),
    );
    player.set_boxes(
        id(B2),
        HashMap::from([(id(L2), rect(0.0, 0.0, 50.0, 50.0))]),
        HashMap::new(),
    );
    click(&mut player, 10.0, 10.0, 0.0);
    click(&mut player, 110.0, 10.0, 100.0);
    click(&mut player, 10.0, 10.0, 200.0);
    let before = player.frame();
    assert_eq!(before.board, id(B2));
    assert_eq!(before.overlays.len(), 1);
    // Navigating away ended the visit of B1, and its swap with it.
    assert!(before.variant_swaps.is_empty());

    player.restart(id(B1));

    let frame = player.frame();
    assert_eq!(frame.board, id(B1));
    assert!(player.history().is_empty());
    assert!(frame.overlays.is_empty());
    assert!(frame.variant_swaps.is_empty());
    assert!(frame.transition.is_none());
}

#[test]
fn closing_an_overlay_drops_the_swaps_made_in_it_and_keeps_those_of_the_board_under_it() {
    let change = json!([actions(json!([{"type": "changeTo", "dest": COMP_ON}]))]);
    let open = json!([actions(json!([{"type": "openOverlay", "dest": B2}]))]);
    let close = json!([actions(json!([{"type": "closeOverlay"}]))]);
    let mut player = player(
        vec![
            board(B1, vec![instance(INST_A, json!([])), layer(L1, open)]),
            board(B2, vec![instance(INST_B, json!([])), layer(L2, close)]),
        ],
        vec![
            component_with_layer(COMP, change),
            component_with_layer(COMP_ON, json!([])),
        ],
        &[],
    );
    let sizes = [
        (id(B1), rect(0.0, 0.0, 400.0, 800.0)),
        (id(B2), rect(0.0, 0.0, 200.0, 100.0)),
    ];
    let mut b1 = HashMap::from(sizes);
    b1.extend([
        (id(RESOLVED_A), rect(0.0, 0.0, 50.0, 50.0)),
        (id(L1), rect(100.0, 0.0, 50.0, 50.0)),
    ]);
    player.set_boxes(
        id(B1),
        b1,
        HashMap::from([(
            id(RESOLVED_A),
            NodeAddress::new(Some(id(INST_A)), id(INNER)),
        )]),
    );
    let mut b2 = HashMap::from(sizes);
    b2.extend([
        (id(RESOLVED_B), rect(0.0, 0.0, 50.0, 50.0)),
        (id(L2), rect(50.0, 0.0, 50.0, 50.0)),
    ]);
    player.set_boxes(
        id(B2),
        b2,
        HashMap::from([(
            id(RESOLVED_B),
            NodeAddress::new(Some(id(INST_B)), id(INNER)),
        )]),
    );
    click(&mut player, 10.0, 10.0, 0.0);
    click(&mut player, 110.0, 10.0, 100.0);
    // The overlay is centred, so its top-left corner is at 100, 350.
    click(&mut player, 110.0, 360.0, 200.0);
    assert_eq!(player.frame().variant_swaps.len(), 2);

    click(&mut player, 160.0, 360.0, 300.0);

    let frame = player.frame();
    assert!(frame.overlays.is_empty());
    assert_eq!(
        frame.variant_swaps,
        BTreeMap::from([(id(INST_A), id(COMP_ON))])
    );
}

#[test]
fn an_up_4px_from_the_down_is_still_a_click() {
    let mut player = two_boards(json!([navigate(B2)]));

    player.pointer(event(PointerKind::Down, 10.0, 10.0, 0.0));
    let effects = player.pointer(event(PointerKind::Up, 14.0, 10.0, 10.0));

    assert_eq!(effects, vec![Effect::NeedsFrame]);
    assert_eq!(player.frame().board, id(B2));
}

#[test]
fn a_press_that_moved_past_4px_is_no_click_even_back_at_its_start() {
    let mut player = two_boards(json!([navigate(B2)]));

    player.pointer(event(PointerKind::Down, 10.0, 10.0, 0.0));
    player.pointer(event(PointerKind::Move, 15.0, 10.0, 10.0));
    let effects = player.pointer(event(PointerKind::Up, 10.0, 10.0, 20.0));

    assert!(effects.is_empty());
    assert_eq!(player.frame().board, id(B1));
}

#[test]
fn leave_cancels_the_press() {
    let mut player = two_boards(json!([navigate(B2)]));

    player.pointer(event(PointerKind::Down, 10.0, 10.0, 0.0));
    player.pointer(event(PointerKind::Leave, 10.0, 10.0, 10.0));
    let effects = player.pointer(event(PointerKind::Up, 10.0, 10.0, 20.0));

    assert!(effects.is_empty());
    assert_eq!(player.frame().board, id(B1));
}

#[test]
fn at_equal_depth_the_later_sibling_wins() {
    let mut player = player(
        vec![
            board(
                B1,
                vec![
                    layer(L1, json!([navigate(B2)])),
                    layer(L2, json!([navigate(B3)])),
                ],
            ),
            board(B2, vec![]),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(0.0, 0.0, 100.0, 100.0)),
        ],
    );

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(player.frame().board, id(B3));
}

#[test]
fn a_hidden_layer_is_not_hit() {
    let mut hidden = layer(L2, json!([navigate(B3)]));
    hidden["visible"] = json!(false);
    let mut player = player(
        vec![
            board(B1, vec![layer(L1, json!([navigate(B2)])), hidden]),
            board(B2, vec![]),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(0.0, 0.0, 100.0, 100.0)),
        ],
    );

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(player.frame().board, id(B2));
}

#[test]
fn the_top_overlay_is_hit_at_its_origin_and_a_miss_inside_it_stays_in_it() {
    let open = json!({"type": "openOverlay", "dest": B2, "position": "manual", "offset": [200, 0]});
    let mut player = player(
        vec![
            board(
                B1,
                vec![
                    layer(L1, json!([actions(json!([open]))])),
                    layer(L2, json!([navigate(B3)])),
                ],
            ),
            board(
                B2,
                vec![layer(
                    "a0000003",
                    json!([actions(json!([{"type": "closeOverlay"}]))]),
                )],
            ),
            board(B3, vec![]),
        ],
        vec![],
        &[],
    );
    player.set_boxes(
        id(B1),
        HashMap::from([
            (id(L1), rect(0.0, 0.0, 50.0, 50.0)),
            (id(L2), rect(200.0, 0.0, 50.0, 50.0)),
            (id(B1), rect(0.0, 0.0, 300.0, 300.0)),
        ]),
        HashMap::new(),
    );
    player.set_boxes(
        id(B2),
        HashMap::from([
            (id("a0000003"), rect(0.0, 0.0, 20.0, 20.0)),
            (id(B2), rect(0.0, 0.0, 100.0, 100.0)),
        ]),
        HashMap::new(),
    );

    click(&mut player, 10.0, 10.0, 0.0);
    // The overlay's layer sits at its origin, over the board's L2.
    click(&mut player, 210.0, 10.0, 100.0);
    assert!(player.frame().overlays.is_empty());
    assert_eq!(player.frame().board, id(B1));

    click(&mut player, 10.0, 10.0, 200.0);
    // Nothing of the overlay is at this point of L2, and the board under it
    // does not take the click.
    click(&mut player, 240.0, 40.0, 300.0);
    assert_eq!(player.frame().board, id(B1));
    assert_eq!(player.frame().overlays.len(), 1);
}

#[test]
fn hotspots_are_the_top_overlays_layers() {
    let mut player = player(
        vec![
            board(
                B1,
                vec![layer(
                    L1,
                    json!([actions(json!([{"type": "openOverlay", "dest": B2}]))]),
                )],
            ),
            board(
                B2,
                vec![layer(
                    L2,
                    json!([actions(json!([{"type": "closeOverlay"}]))]),
                )],
            ),
        ],
        vec![],
        &[(L1, rect(0.0, 0.0, 50.0, 50.0))],
    );
    click(&mut player, 10.0, 10.0, 0.0);
    let boxes = HashMap::from([
        (id(L1), rect(0.0, 0.0, 50.0, 50.0)),
        (id(L2), rect(60.0, 0.0, 20.0, 20.0)),
    ]);

    let spots: HashMap<_, _> = player.hotspots(&boxes).into_iter().collect();

    assert_eq!(
        spots,
        HashMap::from([(id(L2), rect(60.0, 0.0, 20.0, 20.0))])
    );
}

#[test]
fn overlay_actions_with_nothing_to_act_on_change_nothing() {
    let mut player = player(
        vec![
            board(
                B1,
                vec![
                    layer(
                        L1,
                        json!([actions(json!([{"type": "swapOverlay", "dest": B2}]))]),
                    ),
                    layer(
                        L2,
                        json!([actions(json!([{"type": "openOverlay", "dest": GONE}]))]),
                    ),
                    layer(
                        "a0000004",
                        json!([actions(json!([{"type": "openOverlay", "dest": B2}]))]),
                    ),
                ],
            ),
            board(
                B2,
                vec![layer(
                    "a0000003",
                    json!([actions(json!([{"type": "swapOverlay", "dest": GONE}]))]),
                )],
            ),
        ],
        vec![],
        &[],
    );
    player.set_boxes(
        id(B1),
        HashMap::from([
            (id(L1), rect(0.0, 0.0, 50.0, 50.0)),
            (id(L2), rect(100.0, 0.0, 50.0, 50.0)),
            (id("a0000004"), rect(300.0, 0.0, 50.0, 50.0)),
            (id(B1), rect(0.0, 0.0, 400.0, 100.0)),
        ]),
        HashMap::new(),
    );
    player.set_boxes(
        id(B2),
        HashMap::from([
            (id("a0000003"), rect(0.0, 0.0, 50.0, 50.0)),
            (id(B2), rect(0.0, 0.0, 100.0, 100.0)),
        ]),
        HashMap::new(),
    );
    let before = player.frame();

    let swap_none = click(&mut player, 10.0, 10.0, 0.0);
    let open_gone = click(&mut player, 110.0, 10.0, 100.0);

    assert!(swap_none.is_empty());
    assert!(open_gone.is_empty());
    assert_eq!(player.frame(), before);

    click(&mut player, 310.0, 10.0, 200.0);
    // The overlay is centred, so its top-left corner is at 150, 0.
    let swap_gone = click(&mut player, 160.0, 10.0, 300.0);
    assert!(swap_gone.is_empty());
    let frame = player.frame();
    assert_eq!(frame.overlays.len(), 1);
    assert_eq!(frame.overlays[0].board, id(B2));
}

#[test]
fn an_instance_plays_the_variant_the_caller_set() {
    let mut on = component_with_layer(COMP_ON, json!([]));
    on["interactions"] = json!([navigate(B2)]);
    let mut player = instance_player(
        vec![instance(INST_A, json!([]))],
        vec![component_with_layer(COMP, json!([])), on],
    );
    player.set_variant(id(INST_A), id(COMP_ON));

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(player.frame().board, id(B2));
}

/// A player on a 400 × 800 `B1` whose `L1` (0, 0, 50 × 50) opens `open`, and
/// whose `L2` (300, 700, 50 × 50) navigates to `B3`. `B2` is 200 × 100 and
/// `B3` 100 × 300.
fn overlay_player(open: Value) -> Player {
    player(
        vec![
            board(
                B1,
                vec![
                    layer(L1, json!([actions(json!([open]))])),
                    layer(L2, json!([navigate(B3)])),
                ],
            ),
            board(B2, vec![]),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (B1, rect(0.0, 0.0, 400.0, 800.0)),
            (B2, rect(0.0, 0.0, 200.0, 100.0)),
            (B3, rect(0.0, 0.0, 100.0, 300.0)),
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
            (L2, rect(300.0, 700.0, 50.0, 50.0)),
        ],
    )
}

#[test]
fn a_click_outside_an_overlay_that_closes_on_outside_only_closes_it() {
    let mut player =
        overlay_player(json!({"type": "openOverlay", "dest": B2, "closeOnOutside": true}));
    click(&mut player, 10.0, 10.0, 0.0);

    // The centred overlay spans 100..300, 350..450; L2 under the click
    // navigates.
    let effects = click(&mut player, 310.0, 710.0, 100.0);

    assert_eq!(effects, vec![Effect::NeedsFrame]);
    let frame = player.frame();
    assert!(frame.overlays.is_empty());
    assert_eq!(frame.board, id(B1));
    assert!(frame.transition.is_none());
    assert!(player.history().is_empty());
}

#[test]
fn a_click_outside_an_overlay_that_does_not_close_on_outside_is_ignored() {
    let mut player =
        overlay_player(json!({"type": "openOverlay", "dest": B2, "closeOnOutside": false}));
    click(&mut player, 10.0, 10.0, 0.0);

    let effects = click(&mut player, 310.0, 710.0, 100.0);

    assert!(effects.is_empty());
    let frame = player.frame();
    assert_eq!(frame.board, id(B1));
    assert_eq!(frame.overlays.len(), 1);
    assert_eq!(frame.overlays[0].board, id(B2));
    assert!(player.history().is_empty());
}

#[test]
fn a_click_outside_the_top_overlay_closes_only_the_top_one() {
    let open_top = json!({"type": "openOverlay", "dest": B3, "position": "topLeft"});
    let mut player = player(
        vec![
            board(
                B1,
                vec![layer(
                    L1,
                    json!([actions(json!([{"type": "openOverlay", "dest": B2}]))]),
                )],
            ),
            board(B2, vec![layer(L2, json!([actions(json!([open_top]))]))]),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (B1, rect(0.0, 0.0, 400.0, 800.0)),
            (B2, rect(0.0, 0.0, 200.0, 100.0)),
            (B3, rect(0.0, 0.0, 50.0, 50.0)),
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
            (L2, rect(0.0, 0.0, 20.0, 20.0)),
        ],
    );
    click(&mut player, 10.0, 10.0, 0.0);
    // B2 is centred at 100, 350, so its L2 is at 100..120, 350..370.
    click(&mut player, 110.0, 360.0, 100.0);
    assert_eq!(player.frame().overlays.len(), 2);

    // Inside B2, outside B3 at 0..50, 0..50.
    click(&mut player, 250.0, 400.0, 200.0);

    let frame = player.frame();
    assert_eq!(frame.overlays.len(), 1);
    assert_eq!(frame.overlays[0].board, id(B2));
    assert_eq!(frame.overlays[0].origin, [100.0, 350.0]);
}

#[test]
fn an_overlay_opened_from_an_overlay_is_placed_on_the_trigger_as_drawn_on_screen() {
    let open_first =
        json!({"type": "openOverlay", "dest": B2, "position": "manual", "offset": [200, 0]});
    let open_under = json!({"type": "openOverlay", "dest": B3,
        "position": "bottomLeft", "anchor": "triggerOutside"});
    let mut player = player(
        vec![
            board(B1, vec![layer(L1, json!([actions(json!([open_first]))]))]),
            board(B2, vec![layer(L2, json!([actions(json!([open_under]))]))]),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (B1, rect(0.0, 0.0, 400.0, 800.0)),
            (B2, rect(0.0, 0.0, 200.0, 100.0)),
            (B3, rect(0.0, 0.0, 100.0, 300.0)),
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
            (L2, rect(10.0, 50.0, 30.0, 20.0)),
        ],
    );
    click(&mut player, 10.0, 10.0, 0.0);
    assert_eq!(player.frame().overlays[0].origin, [200.0, 0.0]);

    // L2 is drawn at 210..240, 50..70: B3 goes right under it.
    click(&mut player, 220.0, 60.0, 100.0);

    let frame = player.frame();
    assert_eq!(frame.overlays.len(), 2);
    assert_eq!(frame.overlays[1].board, id(B3));
    assert_eq!(frame.overlays[1].origin, [210.0, 70.0]);
}

#[test]
fn a_swapped_in_board_keeps_the_overlays_options_and_is_placed_for_its_size() {
    let open = json!({"type": "openOverlay", "dest": B2,
        "closeOnOutside": false, "background": "#00000080"});
    let mut player = player(
        vec![
            board(B1, vec![layer(L1, json!([actions(json!([open]))]))]),
            board(
                B2,
                vec![layer(
                    L2,
                    json!([actions(json!([{"type": "swapOverlay", "dest": B3}]))]),
                )],
            ),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (B1, rect(0.0, 0.0, 400.0, 800.0)),
            (B2, rect(0.0, 0.0, 200.0, 100.0)),
            (B3, rect(0.0, 0.0, 100.0, 300.0)),
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
            (L2, rect(0.0, 0.0, 20.0, 20.0)),
        ],
    );
    click(&mut player, 10.0, 10.0, 500.0);
    let opened = &player.frame().overlays[0];
    assert_eq!(opened.origin, [100.0, 350.0]);
    assert_eq!(opened.opened_at, 510.0);

    click(&mut player, 110.0, 360.0, 1000.0);

    let frame = player.frame();
    assert_eq!(frame.overlays.len(), 1);
    let swapped = &frame.overlays[0];
    assert_eq!(swapped.board, id(B3));
    assert_eq!(swapped.origin, [150.0, 250.0]);
    assert_eq!(swapped.background.as_deref(), Some("#00000080"));
    assert!(!swapped.close_on_outside);
    assert_eq!(swapped.opened_at, 1010.0);
    assert!(player.history().is_empty());
}

#[test]
fn back_with_overlays_open_goes_to_the_previous_board_and_closes_them() {
    let open = json!([actions(json!([{"type": "openOverlay", "dest": B3}]))]);
    let mut player = player(
        vec![
            board(B1, vec![layer(L1, json!([navigate(B2)]))]),
            board(B2, vec![layer(L2, open)]),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (B1, rect(0.0, 0.0, 400.0, 800.0)),
            (B2, rect(0.0, 0.0, 400.0, 800.0)),
            (B3, rect(0.0, 0.0, 100.0, 300.0)),
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
            (L2, rect(0.0, 0.0, 50.0, 50.0)),
        ],
    );
    click(&mut player, 10.0, 10.0, 0.0);
    click(&mut player, 10.0, 10.0, 100.0);
    assert_eq!(player.frame().overlays.len(), 1);
    assert_eq!(player.history(), &[id(B1)]);

    player.back();

    let frame = player.frame();
    assert_eq!(frame.board, id(B1));
    assert!(frame.overlays.is_empty());
    assert!(player.history().is_empty());
}

#[test]
fn back_on_the_first_board_leaves_its_overlays_open() {
    let mut player = overlay_player(json!({"type": "openOverlay", "dest": B2}));
    click(&mut player, 10.0, 10.0, 0.0);

    player.back();

    let frame = player.frame();
    assert_eq!(frame.board, id(B1));
    assert_eq!(frame.overlays.len(), 1);
    assert_eq!(frame.overlays[0].board, id(B2));
}

/// An interaction that `key` triggers, navigating to `dest`.
fn on_key(key: &str, dest: &str) -> Value {
    json!({"trigger": {"type": "keyPress", "key": key},
        "actions": [{"type": "navigate", "dest": dest}]})
}

/// `key` going down with no modifier held.
fn key_down(key: &str) -> KeyEvent {
    KeyEvent {
        key: key.to_string(),
        ctrl: false,
        alt: false,
        shift: false,
        cmd: false,
        down: true,
    }
}

fn three_boards(children: Vec<Value>) -> Player {
    player(
        vec![board(B1, children), board(B2, vec![]), board(B3, vec![])],
        vec![],
        &[],
    )
}

#[test]
fn shift_k_fires_the_shift_k_trigger_and_not_the_plain_k_one() {
    let mut player = three_boards(vec![
        layer(L1, json!([on_key("Shift+K", B3)])),
        layer(L2, json!([on_key("K", B2)])),
    ]);

    let effects = player.key(KeyEvent {
        shift: true,
        ..key_down("K")
    });

    assert_eq!(effects, vec![Effect::KeyHandled, Effect::NeedsFrame]);
    assert_eq!(player.frame().board, id(B3));
    assert_eq!(player.history(), &[id(B1)]);
}

#[test]
fn plain_k_fires_the_k_trigger_with_its_transition_and_not_the_shift_k_one() {
    let mut player = three_boards(vec![
        layer(
            L1,
            json!([{
                "trigger": {"type": "keyPress", "key": "K"},
                "actions": [{"type": "navigate", "dest": B2}],
                "transition": {"type": "slideIn", "direction": "left"},
                "durationMs": 200,
                "easing": "linear"
            }]),
        ),
        layer(L2, json!([on_key("Shift+K", B3)])),
    ]);

    let effects = player.key(key_down("K"));

    assert_eq!(effects, vec![Effect::KeyHandled, Effect::NeedsFrame]);
    let frame = player.frame();
    assert_eq!(frame.board, id(B2));
    let running = frame.transition.unwrap();
    assert_eq!(
        running.transition,
        Transition::SlideIn {
            direction: Direction::Left
        }
    );
    assert_eq!(running.duration_ms, 200);
    assert_eq!(running.easing, Easing::Linear);
}

#[test]
fn a_trigger_names_its_modifiers_in_any_order_and_its_key_in_any_case() {
    let mut player = three_boards(vec![layer(L1, json!([on_key("shift+CTRL+k", B2)]))]);

    let effects = player.key(KeyEvent {
        ctrl: true,
        shift: true,
        ..key_down("K")
    });

    assert_eq!(effects, vec![Effect::KeyHandled, Effect::NeedsFrame]);
    assert_eq!(player.frame().board, id(B2));
}

#[test]
fn a_trigger_may_name_a_key_by_its_other_names() {
    const B4: &str = "b0000004";
    let mut player = player(
        vec![
            board(B1, vec![layer(L1, json!([on_key("Esc", B2)]))]),
            board(B2, vec![layer(L2, json!([on_key("Return", B3)]))]),
            board(B3, vec![layer("a0000003", json!([on_key("Left", B4)]))]),
            board(B4, vec![layer("a0000004", json!([on_key("Space", B1)]))]),
        ],
        vec![],
        &[],
    );

    player.key(key_down("Escape"));
    assert_eq!(player.frame().board, id(B2));
    player.key(key_down("Enter"));
    assert_eq!(player.frame().board, id(B3));
    player.key(key_down("ArrowLeft"));
    assert_eq!(player.frame().board, id(B4));
    player.key(key_down(" "));
    assert_eq!(player.frame().board, id(B1));
}

#[test]
fn of_two_layers_naming_the_key_only_the_one_on_top_fires() {
    let mut player = three_boards(vec![
        layer(L1, json!([on_key("K", B2)])),
        layer(L2, json!([on_key("K", B3)])),
    ]);

    player.key(key_down("K"));

    assert_eq!(player.frame().board, id(B3));
    assert_eq!(player.history(), &[id(B1)]);
}

#[test]
fn a_key_fires_the_open_overlays_trigger_and_not_the_boards_under_it() {
    let open = json!({"type": "openOverlay", "dest": B2});
    let mut player = player(
        vec![
            board(
                B1,
                vec![
                    layer(L1, json!([actions(json!([open]))])),
                    layer(L2, json!([on_key("K", B3)])),
                ],
            ),
            board(
                B2,
                vec![layer(
                    "a0000003",
                    json!([{"trigger": {"type": "keyPress", "key": "K"},
                        "actions": [{"type": "closeOverlay"}]}]),
                )],
            ),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (B2, rect(0.0, 0.0, 100.0, 100.0)),
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
        ],
    );
    click(&mut player, 10.0, 10.0, 0.0);
    assert_eq!(player.frame().overlays.len(), 1);

    let effects = player.key(key_down("K"));

    assert_eq!(effects, vec![Effect::KeyHandled, Effect::NeedsFrame]);
    let frame = player.frame();
    assert_eq!(frame.board, id(B1));
    assert!(frame.overlays.is_empty());
    assert!(player.history().is_empty());
}

// US-36: the pointer and timer triggers, after FigCraft's rules.

const L3: &str = "a0000003";

fn on(trigger: Value, actions: Value) -> Value {
    json!({"trigger": trigger, "actions": actions})
}

fn go(dest: &str) -> Value {
    json!({"type": "navigate", "dest": dest})
}

fn link(name: &str) -> Value {
    json!({"type": "openUrl", "url": format!("https://example.com/{name}"), "newTab": false})
}

/// The names `link` gave the links among `effects`.
fn links(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::OpenUrl { url, .. } => Some(url.replace("https://example.com/", "")),
            _ => None,
        })
        .collect()
}

fn moved(player: &mut Player, x: f64, y: f64, at: f64) -> Vec<Effect> {
    player.pointer(event(PointerKind::Move, x, y, at))
}

/// A player on a 400 × 800 `B1` holding `b1`, a 200 × 100 `B2` holding `b2`
/// and an empty `B3`, with the layer boxes `boxed`.
fn pointer_player(b1: Vec<Value>, b2: Vec<Value>, boxed: &[(&str, Rect)]) -> Player {
    let mut boxes = vec![
        (B1, rect(0.0, 0.0, 400.0, 800.0)),
        (B2, rect(0.0, 0.0, 200.0, 100.0)),
        (B3, rect(0.0, 0.0, 100.0, 300.0)),
    ];
    boxes.extend_from_slice(boxed);
    player(
        vec![board(B1, b1), board(B2, b2), board(B3, vec![])],
        vec![],
        &boxes,
    )
}

#[test]
fn a_hover_navigate_holds_in_the_layers_box_and_returns_without_a_history_step() {
    let hover = json!({
        "trigger": "onHover",
        "actions": [go(B2)],
        "transition": {"type": "slideIn", "direction": "left"},
        "durationMs": 200,
        "easing": "linear"
    });
    let mut player = pointer_player(
        vec![layer(L1, json!([hover]))],
        vec![],
        &[(L1, rect(0.0, 0.0, 100.0, 100.0))],
    );

    moved(&mut player, 50.0, 50.0, 0.0);
    assert_eq!(player.frame().board, id(B2));
    // B2 draws nothing there: the box L1 had on B1 stays the hotspot.
    moved(&mut player, 90.0, 90.0, 10.0);
    assert_eq!(player.frame().board, id(B2));

    moved(&mut player, 150.0, 50.0, 20.0);

    let frame = player.frame();
    assert_eq!(frame.board, id(B1));
    assert!(player.history().is_empty());
    let running = frame.transition.unwrap();
    assert_eq!((running.from, running.to), (id(B2), id(B1)));
    assert!(running.reversed);
    assert_eq!(running.duration_ms, 200);
    assert_eq!(running.easing, Easing::Linear);
    assert_eq!(running.started, 20.0);
}

#[test]
fn a_mouse_enter_navigate_stays_when_the_pointer_leaves() {
    let mut player = pointer_player(
        vec![layer(L1, json!([on(json!("mouseEnter"), json!([go(B2)]))]))],
        vec![],
        &[(L1, rect(0.0, 0.0, 100.0, 100.0))],
    );

    moved(&mut player, 50.0, 50.0, 0.0);
    moved(&mut player, 150.0, 50.0, 10.0);

    assert_eq!(player.frame().board, id(B2));
    assert_eq!(player.history(), &[id(B1)]);
}

#[test]
fn moving_onto_a_child_enters_it_and_leaves_its_parent_entered() {
    let mut parent = layer(
        L1,
        json!([
            on(json!("mouseEnter"), json!([link("parent-enter")])),
            on(json!("mouseLeave"), json!([link("parent-leave")])),
        ]),
    );
    parent["children"] = json!([layer(
        L2,
        json!([on(json!("mouseEnter"), json!([link("child-enter")]))])
    )]);
    let mut player = pointer_player(
        vec![parent],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(0.0, 0.0, 50.0, 50.0)),
        ],
    );

    assert_eq!(
        links(&moved(&mut player, 75.0, 75.0, 0.0)),
        ["parent-enter"]
    );
    assert_eq!(
        links(&moved(&mut player, 25.0, 25.0, 10.0)),
        ["child-enter"]
    );
    assert_eq!(
        links(&moved(&mut player, 150.0, 150.0, 20.0)),
        ["parent-leave"]
    );
}

#[test]
fn a_parent_never_left_is_not_entered_or_left_again_when_its_childs_hover_reverts() {
    let mut parent = layer(
        L1,
        json!([
            on(json!("mouseEnter"), json!([link("parent-enter")])),
            on(json!("mouseLeave"), json!([link("parent-leave")])),
        ]),
    );
    parent["children"] = json!([layer(L2, json!([on(json!("onHover"), json!([go(B2)]))]))]);
    let mut player = pointer_player(
        vec![parent],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(0.0, 0.0, 50.0, 50.0)),
        ],
    );
    moved(&mut player, 75.0, 75.0, 0.0);
    moved(&mut player, 25.0, 25.0, 10.0);
    assert_eq!(player.frame().board, id(B2));

    let back_on_the_parent = moved(&mut player, 75.0, 75.0, 20.0);

    assert_eq!(player.frame().board, id(B1));
    assert!(links(&back_on_the_parent).is_empty());
    assert_eq!(
        links(&moved(&mut player, 150.0, 150.0, 30.0)),
        ["parent-leave"]
    );
}

#[test]
fn a_hover_revert_leaves_the_layers_hovered_on_the_board_it_returns_from() {
    let mut player = pointer_player(
        vec![layer(L1, json!([on(json!("onHover"), json!([go(B2)]))]))],
        vec![layer(
            L2,
            json!([on(json!("mouseLeave"), json!([link("b2-leave")]))]),
        )],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(0.0, 0.0, 200.0, 100.0)),
        ],
    );
    moved(&mut player, 50.0, 50.0, 0.0);
    moved(&mut player, 60.0, 60.0, 10.0);

    // Out of L1's box, still inside L2's box on B2.
    let effects = moved(&mut player, 150.0, 50.0, 20.0);

    assert_eq!(player.frame().board, id(B1));
    assert_eq!(links(&effects), ["b2-leave"]);
}

#[test]
fn a_press_revert_leaves_the_layers_hovered_on_the_board_it_returns_from() {
    let mut player = pointer_player(
        vec![layer(L1, json!([on(json!("onPress"), json!([go(B2)]))]))],
        vec![layer(
            L2,
            json!([on(json!("mouseLeave"), json!([link("b2-leave")]))]),
        )],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(0.0, 0.0, 200.0, 100.0)),
        ],
    );
    player.pointer(event(PointerKind::Down, 50.0, 50.0, 0.0));
    assert_eq!(player.frame().board, id(B2));
    moved(&mut player, 60.0, 60.0, 10.0);

    let effects = player.pointer(event(PointerKind::Up, 60.0, 60.0, 20.0));

    assert_eq!(player.frame().board, id(B1));
    assert!(player.history().is_empty());
    assert_eq!(links(&effects), ["b2-leave"]);
}

#[test]
fn mouse_up_fires_on_the_layer_under_the_pointer_at_release() {
    let mut player = pointer_player(
        vec![
            layer(L1, json!([on(json!("mouseUp"), json!([go(B2)]))])),
            layer(L2, json!([on(json!("mouseUp"), json!([go(B3)]))])),
        ],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(200.0, 0.0, 100.0, 100.0)),
        ],
    );

    player.pointer(event(PointerKind::Down, 50.0, 50.0, 0.0));
    moved(&mut player, 250.0, 50.0, 10.0);
    player.pointer(event(PointerKind::Up, 250.0, 50.0, 20.0));

    assert_eq!(player.frame().board, id(B3));
    assert_eq!(player.history(), &[id(B1)]);
}

#[test]
fn a_mouse_down_with_delay_ms_acts_that_long_after_the_down_and_once() {
    let mut player = pointer_player(
        vec![layer(
            L1,
            json!([{"trigger": "mouseDown", "delayMs": 300, "actions": [go(B2)]}]),
        )],
        vec![],
        &[(L1, rect(0.0, 0.0, 100.0, 100.0))],
    );

    player.pointer(event(PointerKind::Down, 50.0, 50.0, 1000.0));
    player.pointer(event(PointerKind::Up, 50.0, 50.0, 1010.0));
    assert_eq!(player.frame().board, id(B1));
    player.tick(1299.0);
    assert_eq!(player.frame().board, id(B1));

    player.tick(1300.0);
    assert_eq!(player.frame().board, id(B2));
    player.tick(2000.0);
    assert_eq!(player.history(), &[id(B1)]);
    assert!(player.timers().is_empty());
}

#[test]
fn delay_ms_on_a_hover_is_ignored() {
    let mut player = pointer_player(
        vec![layer(
            L1,
            json!([{"trigger": "onHover", "delayMs": 500, "actions": [go(B2)]}]),
        )],
        vec![],
        &[(L1, rect(0.0, 0.0, 100.0, 100.0))],
    );

    moved(&mut player, 50.0, 50.0, 0.0);

    assert_eq!(player.frame().board, id(B2));
}

#[test]
fn a_delayed_mouse_enter_is_dropped_when_its_board_goes_away() {
    let mut player = pointer_player(
        vec![layer(
            L1,
            json!([{"trigger": "mouseEnter", "delayMs": 500, "actions": [go(B3)]}]),
        )],
        vec![],
        &[(L1, rect(0.0, 0.0, 100.0, 100.0))],
    );
    moved(&mut player, 50.0, 50.0, 0.0);
    assert!(player.go(id(B2)));

    player.tick(500.0);

    assert_eq!(player.frame().board, id(B2));
    assert_eq!(player.history(), &[id(B1)]);
}

#[test]
fn a_delayed_mouse_leave_of_a_board_layer_outlives_an_overlay_closing_over_the_board() {
    let mut card = layer(
        L1,
        json!([{"trigger": "mouseLeave", "delayMs": 500, "actions": [go(B3)]}]),
    );
    card["children"] = json!([layer(
        L2,
        json!([actions(
            json!([{"type": "openOverlay", "dest": B2, "closeOnOutside": true}])
        )])
    )]);
    let mut player = pointer_player(
        vec![card],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(0.0, 0.0, 50.0, 50.0)),
        ],
    );
    moved(&mut player, 10.0, 10.0, 0.0);
    click(&mut player, 10.0, 10.0, 10.0);
    assert_eq!(player.frame().overlays.len(), 1);
    // The centred overlay spans 100..300, 350..450.
    moved(&mut player, 150.0, 150.0, 30.0);
    click(&mut player, 150.0, 150.0, 40.0);
    assert!(player.frame().overlays.is_empty());

    player.tick(530.0);

    assert_eq!(player.frame().board, id(B3));
}

#[test]
fn a_delayed_mouse_leave_of_an_overlay_layer_is_dropped_when_the_overlay_closes() {
    let mut player = pointer_player(
        vec![layer(
            L1,
            json!([actions(
                json!([{"type": "openOverlay", "dest": B2, "closeOnOutside": true}])
            )]),
        )],
        vec![layer(
            L3,
            json!([{"trigger": "mouseLeave", "delayMs": 500, "actions": [go(B3)]}]),
        )],
        &[
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
            (L3, rect(0.0, 0.0, 50.0, 50.0)),
        ],
    );
    click(&mut player, 10.0, 10.0, 0.0);
    // The centred overlay spans 100..300, 350..450, L3 100..150, 350..400.
    moved(&mut player, 110.0, 360.0, 10.0);
    moved(&mut player, 200.0, 420.0, 20.0);
    click(&mut player, 10.0, 700.0, 30.0);
    assert!(player.frame().overlays.is_empty());

    player.tick(520.0);

    assert_eq!(player.frame().board, id(B1));
    assert!(player.history().is_empty());
}

#[test]
fn after_delay_fires_its_ms_after_the_board_is_shown_once_per_visit() {
    let mut player = pointer_player(
        vec![layer(
            L1,
            json!([on(
                json!({"type": "afterDelay", "ms": 1000}),
                json!([link("splash")])
            )]),
        )],
        vec![],
        &[(L1, rect(0.0, 0.0, 100.0, 100.0))],
    );

    assert!(links(&player.tick(999.0)).is_empty());
    assert_eq!(links(&player.tick(1000.0)), ["splash"]);
    assert!(links(&player.tick(1500.0)).is_empty());

    assert!(player.go(id(B2)));
    player.back();

    assert!(links(&player.tick(2499.0)).is_empty());
    assert_eq!(links(&player.tick(2500.0)), ["splash"]);
}

#[test]
fn after_delay_keeps_counting_through_a_hover_navigate_and_its_revert() {
    let mut player = pointer_player(
        vec![
            layer(L1, json!([on(json!("onHover"), json!([go(B2)]))])),
            layer(
                L2,
                json!([on(
                    json!({"type": "afterDelay", "ms": 1000}),
                    json!([link("splash")])
                )]),
            ),
        ],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(200.0, 0.0, 100.0, 100.0)),
        ],
    );
    moved(&mut player, 50.0, 50.0, 100.0);
    moved(&mut player, 150.0, 150.0, 200.0);
    assert_eq!(player.frame().board, id(B1));

    assert_eq!(links(&player.tick(1000.0)), ["splash"]);
    assert!(links(&player.tick(1200.0)).is_empty());
}

#[test]
fn an_overlay_a_hover_opened_closes_when_the_hover_ends() {
    let mut player = pointer_player(
        vec![layer(
            L1,
            json!([on(
                json!("onHover"),
                json!([{"type": "openOverlay", "dest": B2}])
            )]),
        )],
        vec![],
        &[(L1, rect(0.0, 0.0, 50.0, 50.0))],
    );

    moved(&mut player, 10.0, 10.0, 0.0);
    assert_eq!(player.frame().overlays.len(), 1);
    moved(&mut player, 60.0, 60.0, 10.0);

    assert!(player.frame().overlays.is_empty());
    assert_eq!(player.frame().board, id(B1));
}

#[test]
fn a_hover_change_to_returns_to_the_variant_when_the_hover_ends() {
    let change = json!([on(
        json!("onHover"),
        json!([{"type": "changeTo", "dest": COMP_ON}])
    )]);
    let mut player = instance_player(
        vec![instance(INST_A, change)],
        vec![
            component_with_layer(COMP, json!([])),
            component_with_layer(COMP_ON, json!([])),
        ],
    );

    moved(&mut player, 10.0, 10.0, 0.0);
    assert_eq!(player.frame().variant_swaps[&id(INST_A)], id(COMP_ON));
    moved(&mut player, 300.0, 300.0, 10.0);

    assert!(player.frame().variant_swaps.is_empty());
}

#[test]
fn the_4px_of_a_click_are_screen_pixels() {
    let mut player = two_boards(json!([navigate(B2)]));
    player.set_scale(2.0);

    player.pointer(event(PointerKind::Down, 10.0, 10.0, 0.0));
    let effects = player.pointer(event(PointerKind::Up, 13.0, 10.0, 10.0));

    assert!(effects.is_empty());
    assert_eq!(player.frame().board, id(B1));
}

#[test]
fn an_up_within_4px_on_another_layer_is_no_click() {
    let mut player = pointer_player(
        vec![
            layer(L1, json!([navigate(B2)])),
            layer(L2, json!([navigate(B3)])),
        ],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 100.0, 100.0)),
            (L2, rect(101.0, 0.0, 100.0, 100.0)),
        ],
    );

    player.pointer(event(PointerKind::Down, 99.0, 10.0, 0.0));
    player.pointer(event(PointerKind::Up, 102.0, 10.0, 10.0));

    assert_eq!(player.frame().board, id(B1));
    assert!(player.history().is_empty());
}

fn drag_slide_in_left() -> Player {
    let drag = json!({
        "trigger": "onDrag",
        "actions": [go(B2)],
        "transition": {"type": "slideIn", "direction": "left"},
        "durationMs": 400,
        "easing": "linear"
    });
    pointer_player(
        vec![layer(L1, json!([drag]))],
        vec![],
        &[(L1, rect(0.0, 0.0, 100.0, 100.0))],
    )
}

#[test]
fn a_drag_against_a_slide_ins_direction_does_nothing() {
    let mut player = drag_slide_in_left();

    player.pointer(event(PointerKind::Down, 50.0, 50.0, 0.0));
    moved(&mut player, 60.0, 50.0, 10.0);
    player.pointer(event(PointerKind::Up, 60.0, 50.0, 20.0));

    let frame = player.frame();
    assert_eq!(frame.board, id(B1));
    assert!(frame.transition.is_none());
}

#[test]
fn a_drag_started_slide_follows_the_pointer_and_plays_the_rest_of_its_duration_on_release() {
    let mut player = drag_slide_in_left();
    player.pointer(event(PointerKind::Down, 50.0, 50.0, 0.0));
    moved(&mut player, 40.0, 50.0, 10.0);
    assert_eq!(player.frame().board, id(B2));
    assert!(player.frame().transition.unwrap().follows_pointer);

    // 50 of B2's 200 wide: a quarter of the 400 ms.
    moved(&mut player, 0.0, 50.0, 20.0);
    assert_eq!(player.frame().transition.unwrap().started, -80.0);

    player.pointer(event(PointerKind::Up, 0.0, 50.0, 30.0));
    let running = player.frame().transition.unwrap();
    assert!(!running.follows_pointer);
    assert_eq!(running.started, -70.0);

    player.tick(329.0);
    assert!(player.frame().transition.is_some());
    player.tick(330.0);
    assert!(player.frame().transition.is_none());
}
