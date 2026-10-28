//! US-002: the design node types, the fields every node has, node IDs, and
//! which nodes hold children.

mod common;

use common::{
    assert_rejected, assert_round_trips, board, containing, flex_frame, frame, image, instance,
    page, parse, placed, text, vector, with, without,
};
use omni_core::design::NodeKind;
use serde_json::{Value, json};

/// One minimal node of every type, with the kind it parses to.
fn every_node_type() -> Vec<(NodeKind, Value)> {
    vec![
        (NodeKind::Page, page("page0001", vec![])),
        (NodeKind::Board, board("brd00001")),
        (NodeKind::Frame, frame("frame001")),
        (NodeKind::Text, text("text0001")),
        (NodeKind::Image, image("jpeg0001")),
        (NodeKind::Vector, vector("vec00001")),
        (NodeKind::Instance, instance("btn00001")),
    ]
}

/// Node IDs: 8 characters of the lowercase Crockford base32 alphabet
/// (`0123456789abcdefghjkmnpqrstvwxyz`). The first is the PRD's example.
const VALID_IDS: [&str; 7] = [
    "k3f9x2qa", "00000000", "zzzzzzzz", "01234567", "89abcdef", "ghjkmnpq", "rstvwxyz",
];

/// Strings that are not node IDs, and why.
const INVALID_IDS: [(&str, &str); 13] = [
    ("K3F9X2QA", "node IDs are lowercase"),
    ("k3f9x2q", "it has 7 characters, not 8"),
    ("k3f9x2qa0", "it has 9 characters, not 8"),
    ("", "it is empty"),
    ("k3f9x2qi", "`i` is not in the Crockford base32 alphabet"),
    ("k3f9x2ql", "`l` is not in the Crockford base32 alphabet"),
    ("k3f9x2qo", "`o` is not in the Crockford base32 alphabet"),
    ("k3f9x2qu", "`u` is not in the Crockford base32 alphabet"),
    ("k3f9-2qa", "`-` is not a base32 character"),
    ("k3f9_2qa", "`_` is not a base32 character"),
    ("k3f9 2qa", "a space is not a base32 character"),
    ("k3f9x2q=", "base32 padding is not part of an ID"),
    ("k3f9x2qå", "`å` is not a base32 character"),
];

#[test]
fn omni_core_defines_the_seven_design_node_types() {
    for (kind, json) in every_node_type() {
        let node = parse(&json);
        assert_eq!(
            node.kind(),
            kind,
            "a node of type {} must parse as NodeKind::{kind:?}",
            json["type"]
        );
        let output = assert_round_trips(&json);
        assert_eq!(
            output["type"], json["type"],
            "a {kind:?} node is written back with its type"
        );
    }
}

#[test]
fn an_instance_names_its_component() {
    let base = instance("btn00001");
    parse(&base);
    assert_rejected(
        &without(&base, "/component"),
        "an instance without the component it places",
    );
    assert_ne!(
        parse(&with(&base, "/component", json!("cmp00002"))),
        parse(&base),
        "the component is part of the instance"
    );
}

#[test]
fn unknown_node_types_are_rejected() {
    let base = frame("frame001");
    parse(&base);
    for unknown in ["rectangle", "group", "ellipse", "Frame", "FRAME", ""] {
        assert_rejected(
            &with(&base, "/type", json!(unknown)),
            &format!("`{unknown}` is not a design node type"),
        );
    }
}

#[test]
fn every_node_has_an_id_a_name_a_type_and_a_visible_flag() {
    for (kind, json) in every_node_type() {
        let node = parse(&json);
        assert_eq!(Some(node.id().as_str()), json["id"].as_str(), "{kind:?} id");
        assert_eq!(Some(node.name()), json["name"].as_str(), "{kind:?} name");
        assert!(node.visible(), "a {kind:?} without `visible` is visible");

        let hidden = with(&json, "/visible", json!(false));
        assert!(
            !parse(&hidden).visible(),
            "a {kind:?} with `visible: false` is hidden"
        );
        assert_round_trips(&hidden);

        let shown = with(&json, "/visible", json!(true));
        assert_eq!(
            parse(&shown),
            node,
            "`visible: true` means the same as leaving `visible` out on a {kind:?}"
        );
    }
}

#[test]
fn id_name_and_type_are_required() {
    for (kind, json) in every_node_type() {
        parse(&json);
        for field in ["id", "name", "type"] {
            assert_rejected(
                &without(&json, &format!("/{field}")),
                &format!("a {kind:?} node without `{field}` must be rejected"),
            );
        }
    }
}

#[test]
fn nodes_with_invalid_ids_are_rejected_with_an_error_naming_the_id() {
    for id in VALID_IDS {
        assert_eq!(parse(&frame(id)).id().as_str(), id);
    }
    for (id, why) in INVALID_IDS {
        let message = assert_rejected(
            &frame(id),
            &format!("`{id}` is not a node ID ({why}), so the node"),
        );
        assert!(
            message.contains(id),
            "the error must name the invalid ID `{id}`; it says: {message}"
        );
    }
    assert_rejected(
        &with(&frame("frame001"), "/id", json!(12345678)),
        "a node ID is a string, so a number",
    );
}

#[test]
fn only_pages_boards_and_frames_have_children() {
    for json in [
        text("text0001"),
        image("jpeg0001"),
        vector("vec00001"),
        instance("btn00001"),
    ] {
        parse(&json);
        assert_rejected(
            &containing(&json, vec![frame("frame001")]),
            &format!("a {} node cannot hold children", json["type"]),
        );
    }
}

#[test]
fn pages_hold_boards_and_boards_sit_only_on_pages() {
    assert_round_trips(&page(
        "page0001",
        vec![board("brd00001"), board("brd00002")],
    ));
    for child in [
        frame("frame001"),
        text("text0001"),
        image("jpeg0001"),
        instance("btn00001"),
    ] {
        assert_rejected(
            &page("page0001", vec![placed(&child, 10.0, 20.0)]),
            &format!("a page holds only boards, so a {} child", child["type"]),
        );
    }

    // A frame without layout places its children at x and y, so these
    // rejections are about the board itself.
    parse(&containing(
        &frame("frame001"),
        vec![placed(&frame("frame002"), 10.0, 20.0)],
    ));
    assert_rejected(
        &containing(&frame("frame001"), vec![board("brd00001")]),
        "a board is a top-level frame on a page, so a board inside a frame",
    );
    assert_rejected(
        &containing(&board("brd00002"), vec![board("brd00001")]),
        "a board is a top-level frame on a page, so a board inside a board",
    );

    assert_rejected(
        &page("page0001", vec![page("page0002", vec![])]),
        "a page inside a page",
    );
    assert_rejected(
        &flex_frame("frame001", vec![page("page0002", vec![])]),
        "a page inside a frame",
    );
}
