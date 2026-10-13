//! US-002: the design node types, the fields every node has, node IDs, and
//! which nodes hold children.

mod common;

use common::{
    assert_rejected, assert_round_trips, board, containing, flex_frame, frame, image, instance,
    page, parse, placed, text, vector, with, without,
};
use omni_core::design::{Node, NodeId, NodeKind};
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

fn child_ids(node: &Node) -> Vec<&str> {
    node.children()
        .iter()
        .map(|child| child.id().as_str())
        .collect()
}

fn written_child_ids(json: &Value) -> Vec<&str> {
    json["children"]
        .as_array()
        .expect("the serialized node lists its children")
        .iter()
        .map(|child| child["id"].as_str().expect("every child has an id"))
        .collect()
}

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
fn names_are_kept_exactly() {
    for name in [
        "Checkout — kassa 🛒",
        "Card / Header",
        "  padded  ",
        "\"quoted\" <b>",
    ] {
        let json = with(&frame("frame001"), "/name", json!(name));
        assert_eq!(parse(&json).name(), name);
        assert_round_trips(&json);
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
fn node_ids_are_8_lowercase_crockford_base32_characters() {
    for id in VALID_IDS {
        let parsed: NodeId = id
            .parse()
            .unwrap_or_else(|error| panic!("`{id}` is a valid node ID, but: {error:?}"));
        assert_eq!(parsed.as_str(), id, "a node ID reads back as written");
    }
    for (id, why) in INVALID_IDS {
        assert!(
            id.parse::<NodeId>().is_err(),
            "`{id}` is not a node ID: {why}"
        );
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
fn containers_keep_their_children_in_order() {
    let ids = [
        "frame003", "text0001", "frame001", "jpeg0001", "vec00001", "btn00001",
    ];
    let children = vec![
        frame("frame003"),
        text("text0001"),
        frame("frame001"),
        image("jpeg0001"),
        vector("vec00001"),
        instance("btn00001"),
    ];
    let parent = flex_frame("frame009", children.clone());
    let node = parse(&parent);
    assert_eq!(child_ids(&node), ids, "a frame keeps its children in order");
    let output = assert_round_trips(&parent);
    assert_eq!(
        written_child_ids(&output),
        ids,
        "children are written in order"
    );

    let mut reversed = children;
    reversed.reverse();
    assert_ne!(
        parse(&flex_frame("frame009", reversed)),
        node,
        "the order of children is part of the node"
    );

    let flex_board = with(&board("brd00001"), "/layout", json!({"mode": "flex"}));
    let board_json = containing(&flex_board, vec![text("text0002"), frame("frame002")]);
    assert_eq!(
        child_ids(&parse(&board_json)),
        ["text0002", "frame002"],
        "a board keeps its children in order"
    );
    assert_round_trips(&board_json);

    let page_json = page("page0001", vec![board("brd00002"), board("brd00001")]);
    let page_node = parse(&page_json);
    assert_eq!(
        child_ids(&page_node),
        ["brd00002", "brd00001"],
        "a page keeps its boards in order"
    );
    for child in page_node.children() {
        assert_eq!(child.kind(), NodeKind::Board);
    }
    assert_round_trips(&page_json);
}

#[test]
fn only_pages_boards_and_frames_have_children() {
    for json in [
        text("text0001"),
        image("jpeg0001"),
        vector("vec00001"),
        instance("btn00001"),
    ] {
        assert!(
            parse(&json).children().is_empty(),
            "a {} node has no children",
            json["type"]
        );
        assert_rejected(
            &containing(&json, vec![frame("frame001")]),
            &format!("a {} node cannot hold children", json["type"]),
        );
    }
    for json in [
        page("page0001", vec![]),
        board("brd00001"),
        frame("frame001"),
    ] {
        assert!(
            parse(&json).children().is_empty(),
            "a {} without `children` has none",
            json["type"]
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

/// A frame or board (`kind`) with every frame property set.
fn styled(kind: &str, id: &str) -> Value {
    json!({
        "id": id,
        "type": kind,
        "name": "Checkout",
        "x": 40,
        "y": 64,
        "width": {"fixed": 1280},
        "height": "hug",
        "minHeight": 720,
        "maxWidth": 1600,
        "layout": {
            "mode": "flex",
            "direction": "column",
            "gap": {"row": 24, "column": 24},
            "padding": {"top": 32, "right": 40, "bottom": 32, "left": 40},
            "alignItems": "center",
            "justifyContent": "space-between"
        },
        "fills": [{"type": "solid", "color": "#ffffff"}],
        "strokes": [{"color": "#e2e8f0", "width": 1}],
        "radius": {"topLeft": 12, "topRight": 12, "bottomRight": 12, "bottomLeft": 12},
        "shadows": [
            {"type": "drop", "color": "#0f172a1f", "offsetX": 2, "offsetY": 8, "blur": 24, "spread": -4}
        ],
        "opacity": 0.96,
        "clip": true,
        "children": [text("text0001"), frame("frame001")]
    })
}

#[test]
fn a_board_is_a_frame_on_a_page() {
    let as_frame = styled("frame", "frame009");
    let as_board = styled("board", "brd00001");
    assert_round_trips(&as_frame);
    assert_round_trips(&as_board);
    assert_eq!(parse(&as_board).kind(), NodeKind::Board);
    assert_eq!(parse(&as_frame).kind(), NodeKind::Frame);

    let on_page = page("page0001", vec![as_board]);
    let output = assert_round_trips(&on_page);
    assert_eq!(output["children"][0]["type"], "board");
    assert_eq!(
        parse(&on_page).children()[0].children().len(),
        2,
        "a board holds children like a frame"
    );
}
