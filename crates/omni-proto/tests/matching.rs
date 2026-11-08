//! US-34: which layers of one board are the same layer on another, for smart
//! animate (`omni_proto::matching::match_layers`).
//!
//! What the tests expect, from the story and its decisions:
//!
//! - A layer pairs with the layer of the same name under the same chain of
//!   parent names. The chain starts below the two roots, whose own names are
//!   left out, and the roots are not returned as a pair.
//! - The pairs come in the document order of `from`: depth first, a parent
//!   before its children, siblings in order.
//! - Two layers of one name under one parent pair first with first, and a
//!   layer pairs only under the layer its parent paired with.
//! - Pairing goes by name alone: a Text and a Frame of one name pair.

use omni_core::design::{Node, NodeId};
use omni_proto::matching::match_layers;
use serde_json::{Value, json};

fn id(text: &str) -> NodeId {
    text.parse().unwrap()
}

fn node(value: Value) -> Node {
    serde_json::from_value(value).unwrap()
}

fn frame(id: &str, name: &str, children: Vec<Value>) -> Value {
    json!({"id": id, "type": "frame", "name": name, "width": {"fixed": 10}, "height": {"fixed": 10},
        "layout": {"mode": "none"}, "children": children})
}

fn board(id: &str, name: &str, children: Vec<Value>) -> Value {
    json!({"id": id, "type": "board", "name": name, "width": {"fixed": 100}, "height": {"fixed": 100},
        "layout": {"mode": "none"}, "children": children})
}

fn pairs(list: &[(&str, &str)]) -> Vec<(NodeId, NodeId)> {
    list.iter().map(|(from, to)| (id(from), id(to))).collect()
}

#[test]
fn layers_of_one_name_under_the_same_parent_names_pair_in_the_order_of_the_old_board() {
    let from = node(board(
        "b0000001",
        "Home",
        vec![
            frame("a0000001", "Title", vec![]),
            frame("a0000002", "Card", vec![frame("a0000003", "Price", vec![])]),
            frame("a0000004", "Panel", vec![frame("a0000005", "Item", vec![])]),
        ],
    ));
    let to = node(board(
        "b0000002",
        "Details",
        vec![
            frame("a0000012", "Card", vec![frame("a0000013", "Price", vec![])]),
            frame("a0000011", "Title", vec![]),
            frame("a0000014", "Pane", vec![frame("a0000015", "Item", vec![])]),
        ],
    ));

    assert_eq!(
        match_layers(&from, &to),
        pairs(&[
            ("a0000001", "a0000011"),
            ("a0000002", "a0000012"),
            ("a0000003", "a0000013"),
        ])
    );
}

#[test]
fn siblings_of_one_name_pair_first_with_first_and_only_under_the_parent_theirs_paired_with() {
    let from = node(board(
        "b0000001",
        "Board",
        vec![
            frame(
                "a0000001",
                "List",
                vec![
                    frame("a0000002", "Row", vec![]),
                    frame("a0000003", "Row", vec![]),
                ],
            ),
            frame("a0000004", "List", vec![frame("a0000005", "Row", vec![])]),
        ],
    ));
    let to = node(board(
        "b0000002",
        "Board",
        vec![
            frame(
                "a0000011",
                "List",
                vec![
                    frame("a0000012", "Row", vec![]),
                    frame("a0000013", "Row", vec![]),
                    frame("a0000016", "Row", vec![]),
                ],
            ),
            frame("a0000014", "List", vec![]),
            frame("a0000017", "List", vec![frame("a0000015", "Row", vec![])]),
        ],
    ));

    assert_eq!(
        match_layers(&from, &to),
        pairs(&[
            ("a0000001", "a0000011"),
            ("a0000002", "a0000012"),
            ("a0000003", "a0000013"),
            ("a0000004", "a0000014"),
        ])
    );
}

#[test]
fn a_text_and_a_frame_of_one_name_pair() {
    let from = node(board(
        "b0000001",
        "Board",
        vec![
            json!({"id": "a0000001", "type": "text", "name": "Label", "width": "hug", "height": "hug",
            "content": "Off", "fontFamily": "Inter", "fontSize": 16, "lineHeight": 1.25}),
        ],
    ));
    let to = node(board(
        "b0000002",
        "Board",
        vec![frame("a0000011", "Label", vec![])],
    ));

    assert_eq!(match_layers(&from, &to), pairs(&[("a0000001", "a0000011")]));
}
