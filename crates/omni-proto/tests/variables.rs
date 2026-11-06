//! US-38: variables at play time. `setVariable`, `setVariableMode` and
//! `conditional` change what the session holds (`Player::variables`), read
//! through `omni_proto::variables::Eval`; the design system's tokens are only
//! read.
//!
//! What the tests expect, from the story and its decisions:
//!
//! - A division by zero is 0, and the interaction goes on with its next action.
//! - An expression that fails for another reason (a boolean added to a number)
//!   skips that one action; a condition that is not a boolean counts as false.
//! - `conditional` runs the first block whose condition holds, else the block
//!   with no condition.
//! - `{mode: "color"}` is the board's mode, the collection's first listed mode
//!   for a board without `modes`; `{inMode}` is the design system's value in
//!   that mode, whatever play set.
//! - `setVariableMode` sets the mode on the board under any open overlay,
//!   starting from that board's own `modes`.
//! - A layer whose `visible` token is false cannot be clicked.

use omni_core::design::tokens::{ModeChoice, TokenTree};
use omni_core::design::{Node, NodeData, NodeId};
use omni_proto::variables::TokenValue;
use omni_proto::{Player, PointerEvent, PointerKind, Rect};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

const B1: &str = "b0000001";
const B2: &str = "b0000002";
const B3: &str = "b0000003";
const L1: &str = "a0000001";
const L2: &str = "a0000002";

fn id(text: &str) -> NodeId {
    text.parse().unwrap()
}

/// Modes light and dark for `color` (the root list) and comfortable and
/// compact for `density`; `state` has no modes.
fn tokens() -> TokenTree {
    TokenTree::from_json(&json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "surface": {
                "$type": "color",
                "$value": "#ffffff",
                "$extensions": {"omnistate": {"modes": {"light": "#ffffff", "dark": "#0f172a"}}}
            }
        },
        "density": {
            "$extensions": {"omnistate": {"modes": ["comfortable", "compact"]}},
            "space": {
                "$type": "dimension",
                "$value": "16px",
                "$extensions": {"omnistate": {"modes": {"comfortable": "16px", "compact": "12px"}}}
            }
        },
        "state": {
            "$extensions": {"omnistate": {"modes": []}},
            "count": {"$type": "number", "$value": 4},
            "ratio": {"$type": "number", "$value": 5},
            "isOpen": {"$type": "boolean", "$value": false},
            "label": {"$type": "string", "$value": "none"}
        }
    }))
    .unwrap()
}

fn layer(id: &str, actions: Value) -> Value {
    json!({"id": id, "type": "frame", "name": id, "width": {"fixed": 100}, "height": {"fixed": 100},
        "layout": {"mode": "none"}, "interactions": [{"actions": actions}]})
}

fn board(id: &str, children: Vec<Value>) -> Value {
    json!({"id": id, "type": "board", "name": id, "width": {"fixed": 100}, "height": {"fixed": 100},
        "layout": {"mode": "none"}, "children": children})
}

fn rect(x: f64, y: f64) -> Rect {
    Rect {
        x,
        y,
        width: 50.0,
        height: 50.0,
    }
}

/// A player on `B1` with the tokens of `tokens()`, `L1` at the origin and
/// `L2` right of it on every board.
fn player(boards: Vec<Value>) -> Player {
    let page: Node = serde_json::from_value(json!({
        "id": "a0000000", "type": "page", "name": "Page", "children": boards,
    }))
    .unwrap();
    let NodeData::Page(page) = page.data() else {
        unreachable!("a page was built")
    };
    let mut player = Player::new(page, HashMap::new(), id(B1));
    player.set_tokens(tokens());
    let rects = HashMap::from([(id(L1), rect(0.0, 0.0)), (id(L2), rect(60.0, 0.0))]);
    for board in &page.children {
        player.set_boxes(*board.id(), rects.clone(), HashMap::new());
    }
    player
}

/// A player whose board `B1` runs `actions` when `L1` is clicked.
fn on_click(actions: Value) -> Player {
    player(vec![board(B1, vec![layer(L1, actions)]), board(B2, vec![])])
}

fn click(player: &mut Player, x: f64, y: f64, at: f64) {
    for kind in [PointerKind::Down, PointerKind::Up] {
        player.pointer(PointerEvent { kind, x, y, at });
    }
}

fn values(player: &Player) -> BTreeMap<String, TokenValue> {
    player.variables().values.clone()
}

fn set(variable: &str, value: Value) -> Value {
    json!({"type": "setVariable", "variable": variable, "value": value})
}

fn set_expr(variable: &str, expr: Value) -> Value {
    json!({"type": "setVariable", "variable": variable, "expr": expr})
}

fn var(path: &str) -> Value {
    json!({"variable": path})
}

fn op(op: &str, args: Value) -> Value {
    json!({"op": op, "args": args})
}

fn modes(pairs: &[(&str, &str)]) -> ModeChoice {
    ModeChoice(
        pairs
            .iter()
            .map(|(collection, mode)| (collection.to_string(), mode.to_string()))
            .collect(),
    )
}

#[test]
fn a_division_by_zero_sets_0_and_the_next_action_still_runs() {
    let mut player = on_click(json!([
        set_expr(
            "state.ratio",
            op("divide", json!([var("state.count"), {"value": 0}]))
        ),
        set("state.label", json!("after")),
    ]));

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(
        values(&player),
        BTreeMap::from([
            (
                "state.label".to_string(),
                TokenValue::String("after".into())
            ),
            ("state.ratio".to_string(), TokenValue::Number(0.0)),
        ])
    );
}

#[test]
fn an_expression_that_fails_skips_its_action_and_the_rest_runs() {
    let mut player = on_click(json!([
        set_expr(
            "state.count",
            op("add", json!([var("state.isOpen"), {"value": 1}]))
        ),
        set("state.label", json!("after")),
    ]));

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(
        values(&player),
        BTreeMap::from([(
            "state.label".to_string(),
            TokenValue::String("after".into())
        )])
    );
}

#[test]
fn a_conditional_runs_only_the_first_block_whose_condition_holds() {
    let holds = op("greater", json!([var("state.count"), {"value": 3}]));
    let mut player = on_click(json!([{"type": "conditional", "blocks": [
        {"condition": op("less", json!([var("state.count"), {"value": 3}])), "actions": [set("state.label", json!("less"))]},
        {"condition": holds, "actions": [set("state.label", json!("first"))]},
        {"condition": holds, "actions": [set("state.label", json!("second"))]},
        {"actions": [set("state.label", json!("else"))]},
    ]}]));

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(
        values(&player),
        BTreeMap::from([(
            "state.label".to_string(),
            TokenValue::String("first".into())
        )])
    );
}

#[test]
fn a_condition_that_is_not_a_boolean_counts_as_false_and_the_else_block_runs() {
    let mut player = on_click(json!([{"type": "conditional", "blocks": [
        {"condition": var("state.count"), "actions": [set("state.label", json!("count"))]},
        {"actions": [set("state.label", json!("else"))]},
    ]}]));

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(
        values(&player),
        BTreeMap::from([("state.label".to_string(), TokenValue::String("else".into()))])
    );
}

#[test]
fn not_toggles_a_boolean_for_the_session_on_each_click() {
    let mut player = on_click(json!([set_expr(
        "state.isOpen",
        op("not", json!([var("state.isOpen")]))
    )]));

    click(&mut player, 10.0, 10.0, 0.0);
    assert_eq!(values(&player)["state.isOpen"], TokenValue::Boolean(true));

    click(&mut player, 10.0, 10.0, 100.0);
    assert_eq!(values(&player)["state.isOpen"], TokenValue::Boolean(false));
}

#[test]
fn the_mode_of_a_board_without_modes_is_the_collections_first_listed_mode() {
    let mut player = on_click(json!([set_expr("state.label", json!({"mode": "color"})),]));

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(
        values(&player)["state.label"],
        TokenValue::String("light".into())
    );
}

#[test]
fn the_mode_follows_set_variable_mode_on_the_board() {
    let mut player = on_click(json!([
        {"type": "setVariableMode", "collection": "color", "mode": "dark"},
        set_expr("state.label", json!({"mode": "color"})),
    ]));

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(
        values(&player)["state.label"],
        TokenValue::String("dark".into())
    );
}

#[test]
fn in_mode_reads_the_design_systems_value_and_not_the_one_play_set() {
    let mut player = on_click(json!([
        set("color.surface", json!("#ff0000")),
        set_expr(
            "state.label",
            json!({"inMode": {"variable": "color.surface", "mode": "dark"}})
        ),
    ]));

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(
        values(&player)["state.label"],
        TokenValue::String("#0f172a".into())
    );
}

#[test]
fn set_variable_mode_starts_from_the_boards_own_modes_and_leaves_other_boards_alone() {
    let mut own = board(
        B1,
        vec![layer(
            L1,
            json!([{"type": "setVariableMode", "collection": "color", "mode": "dark"}]),
        )],
    );
    own["modes"] = json!({"density": "compact"});
    let mut player = player(vec![own, board(B2, vec![])]);

    click(&mut player, 10.0, 10.0, 0.0);

    assert_eq!(
        player.variables().modes,
        BTreeMap::from([(id(B1), modes(&[("color", "dark"), ("density", "compact")]))])
    );
}

#[test]
fn set_variable_mode_from_an_overlay_sets_the_mode_on_the_board_under_it() {
    let open = json!({"type": "openOverlay", "dest": B2, "position": "manual", "offset": [0, 200]});
    let mut player = player(vec![
        board(B1, vec![layer(L1, json!([open]))]),
        board(
            B2,
            vec![layer(
                L2,
                json!([{"type": "setVariableMode", "collection": "color", "mode": "dark"}]),
            )],
        ),
    ]);
    player.set_boxes(
        id(B2),
        HashMap::from([(id(L2), rect(0.0, 0.0)), (id(B2), rect(0.0, 0.0))]),
        HashMap::new(),
    );

    click(&mut player, 10.0, 10.0, 0.0);
    assert_eq!(player.frame().overlays.len(), 1);
    let origin = player.frame().overlays[0].origin;
    click(&mut player, origin[0] + 10.0, origin[1] + 10.0, 100.0);

    assert_eq!(
        player.variables().modes,
        BTreeMap::from([(id(B1), modes(&[("color", "dark")]))])
    );
}

#[test]
fn a_layer_hidden_by_its_visible_token_cannot_be_clicked_until_play_shows_it() {
    let mut gated = layer(L2, json!([{"type": "navigate", "dest": B3}]));
    gated["visible"] = json!("$state.isOpen");
    let mut player = player(vec![
        board(
            B1,
            vec![layer(L1, json!([set("state.isOpen", json!(true))])), gated],
        ),
        board(B2, vec![]),
        board(B3, vec![]),
    ]);

    click(&mut player, 70.0, 10.0, 0.0);
    assert_eq!(player.frame().board, id(B1));

    click(&mut player, 10.0, 10.0, 100.0);
    click(&mut player, 70.0, 10.0, 200.0);
    assert_eq!(player.frame().board, id(B3));
}
