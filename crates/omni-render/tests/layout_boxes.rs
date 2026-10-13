//! US-003: the layout module turns a node tree into a Taffy tree and returns a
//! box (x, y, width, height) for every node ID, in page coordinates, with the
//! fractions a browser keeps.

mod common;
mod design;

use design::{
    absolute, assert_box, at, board, box_of, fixed, flex, frame, grid, ids_in, lay_out, none, rect,
    sized, text, with,
};
use serde_json::{Value, json};

/// The attachment the image fixtures show.
const PHOTO: &str = "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd";

/// An image node sized `width` x `height`.
fn image(id: &str, width: Value, height: Value) -> Value {
    json!({
        "id": id,
        "type": "image",
        "name": format!("Image {id}"),
        "width": width,
        "height": height,
        "image": PHOTO,
        "fit": "cover"
    })
}

/// A square vector node, `size` px wide.
fn vector(id: &str, size: f64) -> Value {
    json!({
        "id": id,
        "type": "vector",
        "name": format!("Vector {id}"),
        "width": fixed(size),
        "height": fixed(size),
        "path": "M0 0 L24 24"
    })
}

/// An instance of `component` that leaves its size to the component.
fn instance(id: &str, component: &str) -> Value {
    json!({
        "id": id,
        "type": "instance",
        "name": format!("Instance {id}"),
        "component": component
    })
}

/// A page with every node type, every layout mode, nested frames, an
/// absolute child, a hidden frame, an instance without a size and a token
/// reference that layout cannot resolve.
fn every_kind_of_node() -> Value {
    let row = frame(
        "frm00001",
        json!("fill"),
        json!("hug"),
        flex(json!({"gap": {"column": 8}, "alignItems": "center"})),
        vec![
            image("jpg00001", fixed(64.0), fixed(64.0)),
            vector("vec00001", 24.0),
            with(
                &with(&instance("nst00001", "cmp00001"), "width", fixed(120.0)),
                "height",
                fixed(40.0),
            ),
            instance("nst00002", "cmp00002"),
            with(
                &text(
                    "txt00002",
                    "A longer label that wraps inside the row",
                    14.0,
                    1.4,
                ),
                "width",
                json!("fill"),
            ),
        ],
    );
    let cells = frame(
        "frm00002",
        json!("fill"),
        fixed(200.0),
        grid(json!({
            "columns": [{"fixed": 100}, {"fr": 1}],
            "rows": ["auto"],
            "gap": {"row": "$space.md", "column": 12}
        })),
        vec![
            with(
                &sized("frm00003", json!("fill"), json!("fill")),
                "gridColumn",
                json!({"start": 2}),
            ),
            text("txt00003", "Cell", 16.0, 1.5),
            with(
                &frame(
                    "frm00004",
                    fixed(10.0),
                    fixed(10.0),
                    none(),
                    vec![rect("rct00001", 4.0, 4.0)],
                ),
                "visible",
                json!(false),
            ),
        ],
    );
    let canvas = frame(
        "frm00005",
        fixed(200.0),
        fixed(100.0),
        none(),
        vec![
            at(&vector("vec00002", 20.0), 10.0, 10.0),
            at(
                &frame(
                    "frm00006",
                    json!("hug"),
                    json!("hug"),
                    flex(json!({})),
                    vec![text("txt00004", "Inside", 12.0, 1.25)],
                ),
                50.0,
                20.0,
            ),
        ],
    );
    let first = at(
        &board(
            "brd00001",
            fixed(800.0),
            fixed(600.0),
            flex(json!({
                "direction": "column",
                "gap": {"row": 16},
                "padding": {"top": 24, "right": 24, "bottom": 24, "left": 24}
            })),
            vec![
                text("txt00001", "Every node gets a box", 24.0, 1.2),
                row,
                cells,
                canvas,
                absolute(&text("txt00005", "Pinned", 16.0, 1.5), 700.0, 10.0),
            ],
        ),
        0.0,
        0.0,
    );
    let second = at(
        &board(
            "brd00002",
            json!("hug"),
            json!("hug"),
            grid(json!({"columns": ["auto", "auto"]})),
            vec![
                text("txt00006", "Left", 16.0, 1.5),
                text("txt00007", "Right", 16.0, 1.5),
                with(
                    &image("jpg00002", json!("fill"), fixed(50.0)),
                    "gridColumn",
                    json!({"span": 2}),
                ),
            ],
        ),
        900.0,
        0.0,
    );
    json!({
        "id": "page0001",
        "type": "page",
        "name": "Page",
        "children": [first, second]
    })
}

#[test]
fn every_node_of_the_tree_gets_a_box_by_its_id() {
    let page = every_kind_of_node();
    let boxes = lay_out(&page);
    let expected = ids_in(&page);
    let returned: std::collections::BTreeSet<String> =
        boxes.keys().map(|id| id.as_str().to_string()).collect();
    assert_eq!(
        returned, expected,
        "layout must return one box for each node of the tree, keyed by node ID, and no other"
    );
    for (id, node_box) in &boxes {
        assert!(
            [node_box.x, node_box.y, node_box.width, node_box.height]
                .iter()
                .all(|value| value.is_finite())
                && node_box.width >= 0.0
                && node_box.height >= 0.0,
            "node {id} must get a box of finite numbers and a size of 0 or more, but layout gave {node_box:?}"
        );
    }
}

#[test]
fn boxes_are_in_page_coordinates() {
    let placed_board = at(
        &board(
            "brd00001",
            fixed(400.0),
            fixed(300.0),
            flex(json!({"padding": {"top": 10, "left": 20}})),
            vec![frame(
                "frm00001",
                fixed(50.0),
                fixed(40.0),
                none(),
                vec![at(&rect("a0000001", 10.0, 10.0), 5.0, 6.0)],
            )],
        ),
        100.0,
        200.0,
    );
    let hugging_board = at(
        &board(
            "brd00002",
            json!("hug"),
            json!("hug"),
            flex(json!({"padding": {"top": 8, "right": 8, "bottom": 8, "left": 8}})),
            vec![rect("b0000001", 30.0, 20.0)],
        ),
        -600.0,
        50.5,
    );
    let page = json!({
        "id": "page0001",
        "type": "page",
        "name": "Page",
        "children": [placed_board, hugging_board]
    });

    let boxes = lay_out(&page);
    let case = "a board at (100, 200) on its page";
    assert_box(&boxes, "brd00001", [100.0, 200.0, 400.0, 300.0], case);
    assert_box(&boxes, "frm00001", [120.0, 210.0, 50.0, 40.0], case);
    assert_box(&boxes, "a0000001", [125.0, 216.0, 10.0, 10.0], case);
    let case = "a board at (-600, 50.5) that hugs its padded content";
    assert_box(&boxes, "brd00002", [-600.0, 50.5, 46.0, 36.0], case);
    assert_box(&boxes, "b0000001", [-592.0, 58.5, 30.0, 20.0], case);

    let alone = lay_out(&placed_board);
    for id in ["brd00001", "frm00001", "a0000001"] {
        assert_eq!(
            box_of(&alone, id),
            box_of(&boxes, id),
            "a board laid out on its own sits where it sits on its page, so node {id} gets the same box"
        );
    }
}

#[test]
fn sizes_include_the_padding() {
    let boxes = lay_out(&board(
        "brd00001",
        fixed(200.0),
        fixed(100.0),
        flex(json!({"padding": {"top": 10, "right": 20, "bottom": 30, "left": 40}})),
        vec![sized("a0000001", json!("fill"), json!("fill"))],
    ));
    let case = "a 200 x 100 board with padding 10 20 30 40 and a child that fills it";
    assert_box(&boxes, "brd00001", [0.0, 0.0, 200.0, 100.0], case);
    assert_box(&boxes, "a0000001", [40.0, 10.0, 140.0, 60.0], case);
}

#[test]
fn boxes_keep_their_fractions() {
    let third = 100.0_f32 / 3.0;
    let boxes = lay_out(&board(
        "brd00001",
        fixed(110.25),
        fixed(30.0),
        flex(json!({})),
        vec![
            rect("a0000001", 10.25, 20.75),
            sized("b0000001", json!("fill"), fixed(30.0)),
            sized("c0000001", json!("fill"), fixed(30.0)),
            sized("d0000001", json!("fill"), fixed(30.0)),
        ],
    ));
    let case = "a 10.25 x 20.75 box and three fill children sharing the other 100 px of a row";
    assert_box(&boxes, "a0000001", [0.0, 0.0, 10.25, 20.75], case);
    assert_box(&boxes, "b0000001", [10.25, 0.0, third, 30.0], case);
    assert_box(&boxes, "c0000001", [10.25 + third, 0.0, third, 30.0], case);
    assert_box(
        &boxes,
        "d0000001",
        [10.25 + 2.0 * third, 0.0, third, 30.0],
        case,
    );
}

#[test]
fn hidden_nodes_take_no_space() {
    let hidden = |json: Value| with(&json, "visible", json!(false));
    let boxes = lay_out(&board(
        "brd00001",
        fixed(300.0),
        fixed(100.0),
        flex(json!({"gap": {"column": 10}})),
        vec![
            rect("a0000001", 50.0, 20.0),
            hidden(rect("b0000001", 60.0, 20.0)),
            rect("c0000001", 70.0, 20.0),
        ],
    ));
    let case = "a row of three boxes whose middle one is hidden";
    assert_box(&boxes, "a0000001", [0.0, 0.0, 50.0, 20.0], case);
    assert_box(&boxes, "c0000001", [60.0, 0.0, 70.0, 20.0], case);
    // A hidden node still gets a box, like every node of the tree.
    box_of(&boxes, "b0000001");

    let boxes = lay_out(&board(
        "brd00002",
        json!("hug"),
        json!("hug"),
        flex(json!({"gap": {"column": 10}})),
        vec![
            rect("d0000001", 50.0, 20.0),
            hidden(frame(
                "e0000001",
                fixed(200.0),
                fixed(200.0),
                flex(json!({})),
                vec![rect("e1000001", 100.0, 100.0)],
            )),
            rect("f0000001", 70.0, 30.0),
        ],
    ));
    let case = "a row that hugs two boxes and a hidden 200 x 200 frame";
    assert_box(&boxes, "brd00002", [0.0, 0.0, 130.0, 30.0], case);
    assert_box(&boxes, "f0000001", [60.0, 0.0, 70.0, 30.0], case);
    // So do the nodes below a hidden node.
    box_of(&boxes, "e0000001");
    box_of(&boxes, "e1000001");
}

#[test]
fn omni_render_lays_out_with_taffy() {
    let output = common::cargo(&["metadata", "--no-deps", "--format-version", "1"]);
    assert!(
        output.status.success(),
        "`cargo metadata` failed at the repository root:\n{}",
        common::describe(&output)
    );
    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata prints JSON");
    let omni_render = metadata["packages"]
        .as_array()
        .expect("cargo metadata lists packages")
        .iter()
        .find(|package| package["name"] == "omni-render")
        .expect("omni-render is a workspace member");
    let dependencies = omni_render["dependencies"]
        .as_array()
        .expect("every package lists its dependencies");
    for crate_name in ["taffy", "omni-core"] {
        assert!(
            dependencies
                .iter()
                .any(|dependency| dependency["name"] == crate_name && dependency["kind"].is_null()),
            "omni-render lays out the omni-core node tree with Taffy, so it needs a normal dependency on the `{crate_name}` crate"
        );
    }
}
