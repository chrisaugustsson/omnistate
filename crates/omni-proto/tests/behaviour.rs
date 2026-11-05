//! The player's behaviour: what a click, back and the actions do to the
//! frame, the history and the effects, ported from the behaviour tests of
//! FigCraft's `proto_run.rs` (MIT OR Apache-2.0; see `NOTICE`).

use omni_core::design::proto::{Direction, Easing, Transition};
use omni_core::design::{Node, NodeAddress, NodeData, NodeId};
use omni_proto::{Effect, Player, PointerEvent, PointerKind, Rect};
use serde_json::{Value, json};
use std::collections::HashMap;

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
    assert!(missed.is_empty());
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
            direction: Direction::Right
        }
    );
    assert_eq!(running.started, 500.0);
}

#[test]
fn overlays_push_replace_and_pop_without_a_history_step() {
    let open = json!({"type": "openOverlay", "dest": B2, "position": "manual", "offset": [5, 6]});
    let mut player = player(
        vec![
            board(
                B1,
                vec![
                    layer(L1, json!([actions(json!([open]))])),
                    layer(
                        L2,
                        json!([actions(json!([{"type": "swapOverlay", "dest": B3}]))]),
                    ),
                ],
            ),
            board(B2, vec![]),
            board(B3, vec![]),
        ],
        vec![],
        &[
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
            (L2, rect(60.0, 0.0, 50.0, 50.0)),
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
                vec![
                    layer(
                        L1,
                        json!([actions(json!([{"type": "openOverlay", "dest": B2}]))]),
                    ),
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
        &[
            (L1, rect(0.0, 0.0, 50.0, 50.0)),
            (L2, rect(60.0, 0.0, 50.0, 50.0)),
            ("a0000003", rect(0.0, 100.0, 50.0, 50.0)),
        ],
    );

    click(&mut player, 10.0, 10.0, 0.0);
    let closed = click(&mut player, 10.0, 110.0, 100.0);
    assert_eq!(closed, vec![Effect::NeedsFrame]);
    assert!(player.frame().overlays.is_empty());

    click(&mut player, 10.0, 10.0, 200.0);
    assert_eq!(player.frame().overlays.len(), 1);
    click(&mut player, 70.0, 10.0, 300.0);
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
    assert!(after.is_empty());
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
    assert_eq!(before.variant_swaps.len(), 1);

    player.restart(id(B1));

    let frame = player.frame();
    assert_eq!(frame.board, id(B1));
    assert!(player.history().is_empty());
    assert!(frame.overlays.is_empty());
    assert!(frame.variant_swaps.is_empty());
    assert!(frame.transition.is_none());
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
fn the_top_overlay_is_hit_first_at_its_origin_and_a_miss_falls_through() {
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
        ]),
        HashMap::new(),
    );
    player.set_boxes(
        id(B2),
        HashMap::from([(id("a0000003"), rect(0.0, 0.0, 20.0, 20.0))]),
        HashMap::new(),
    );

    click(&mut player, 10.0, 10.0, 0.0);
    // The overlay's layer sits at its origin, over the board's L2.
    click(&mut player, 210.0, 10.0, 100.0);
    assert!(player.frame().overlays.is_empty());
    assert_eq!(player.frame().board, id(B1));

    click(&mut player, 10.0, 10.0, 200.0);
    // Nothing of the overlay is at this point of L2, so the board's L2 is hit.
    click(&mut player, 240.0, 40.0, 300.0);
    assert_eq!(player.frame().board, id(B3));
}

#[test]
fn hotspots_include_the_open_overlays_layers() {
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

    assert_eq!(spots, boxes);
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
                        "a0000003",
                        json!([actions(json!([{"type": "swapOverlay", "dest": GONE}]))]),
                    ),
                    layer(
                        "a0000004",
                        json!([actions(json!([{"type": "openOverlay", "dest": B2}]))]),
                    ),
                ],
            ),
            board(B2, vec![]),
        ],
        vec![],
        &[],
    );
    player.set_boxes(
        id(B1),
        HashMap::from([
            (id(L1), rect(0.0, 0.0, 50.0, 50.0)),
            (id(L2), rect(100.0, 0.0, 50.0, 50.0)),
            (id("a0000003"), rect(200.0, 0.0, 50.0, 50.0)),
            (id("a0000004"), rect(300.0, 0.0, 50.0, 50.0)),
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
    let swap_gone = click(&mut player, 210.0, 10.0, 300.0);
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
