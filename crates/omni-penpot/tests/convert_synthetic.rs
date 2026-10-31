//! US-102: `omni_penpot::convert_file` on small exports that the tests build
//! themselves, for the behaviour the fixtures do not show or show only in bulk.
//! These tests do not need fixtures/penpot/.
//!
//! `convert_file(path)` returns a `Conversion { boards, attachments, report }`.
//! `boards` has one `ConvertedBoard { page, id, name, design }` per top-level
//! frame of every page, in page order and then stacking order: `design` is an
//! OmniState page node that holds the board alone, and the board sits on it at
//! the x and y that Penpot gives it on the page. Shapes of Penpot's types map to
//! nodes like this (every shape inside a board becomes exactly one node, with
//! the Penpot shape's name):
//!
//! | Penpot | OmniState |
//! | --- | --- |
//! | top-level frame | `board` |
//! | frame, group, rect | `frame` (a group and a rect have layout `none`) |
//! | circle | `vector` (an ellipse path in the shape's own box) |
//! | text | `text` |
//! | path | `vector` (its path, moved into the shape's own box) |
//! | image | `image` |
//!
//! A shape of any other type, such as a boolean (`bool`), is not converted but
//! listed in `conversion.report.entries`, each a `ReportEntry { board,
//! board_id, shape_id, name, property, reason }`. `property` is None when the
//! whole shape was left out and names the property when only that was. The
//! board is the top-level board that holds the shape. Shapes that sit directly
//! on a page, outside every board, go into a board called "Unplaced" on that
//! page (US-054, see convert_penpot_fixes.rs).
//!
//! Where a flex or grid layout lays out its children, the converted children
//! carry no x and y. In every flex frame of the real exports, the order of the
//! `shapes` list is the reverse of the order the children appear in (left to
//! right in a row, top to bottom in a column), so the converter reverses it.
//! Penpot's padding fields run p1 top, p2 right, p3 bottom, p4 left; its corner
//! radii r1 top left, r2 top right, r3 bottom right, r4 bottom left.
//!
//! ASSUMED, not observed (neither fixture holds one of these, so the test data
//! follows Penpot's public naming and the format document says so): linear
//! gradient fills (`fillColorGradient` with `type`, `startX`, `startY`, `endX`,
//! `endY`, `stops` of `color`, `opacity` and `offset`, the points in fractions
//! of the shape's box), image fills (`fillImage` with the media id) and `image`
//! shapes (`metadata` with the media id), whose media record is
//! `files/<file>/media/<media id>.json` with `mediaId` naming the stored file
//! `objects/<media id>.<extension>`; `inner-shadow` shadows; `blur`;
//! `maskedGroup`; and text decorations `underline` and `line-through`.

mod common;

use omni_core::design::NodeKind;
use omni_penpot::{Conversion, convert_file};
use omni_render::paint::Assets;
use serde_json::{Value, json};

use common::convert::*;

#[test]
fn every_top_level_frame_becomes_a_design_with_one_board() {
    let conversion = convert(
        "boards",
        vec![
            shape(1, "frame", "Home", [100.0, 200.0, 400.0, 300.0]),
            text(2, "Label", [0.0, 0.0, 50.0, 20.0], "A label on the page"),
            shape(3, "frame", "About", [900.0, 50.0, 320.0, 480.5]),
        ],
    );
    // The label on the page goes into the Unplaced board (US-054).
    assert_eq!(conversion.boards.len(), 3);
    let boards: Vec<_> = conversion
        .boards
        .iter()
        .filter(|b| b.name != "Unplaced")
        .collect();
    let names: Vec<_> = boards.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Home", "About"]);
    assert_eq!(boards[0].id, uid(1));
    assert_eq!(boards[1].id, uid(3));
    assert_eq!(boards[0].page, PAGE_NAME);

    let home = json_of(board_node(board(&conversion, "Home")));
    assert_eq!(home["name"], "Home");
    assert_eq!(home["type"], "board");
    assert_near(num(&home["x"]), 100.0, 0.001, "board x");
    assert_near(num(&home["y"]), 200.0, 0.001, "board y");
    assert_sizing(&home["width"], Sz::Fixed(400.0), "board width");
    assert_sizing(&home["height"], Sz::Fixed(300.0), "board height");
    let about = json_of(board_node(board(&conversion, "About")));
    assert_near(num(&about["x"]), 900.0, 0.001, "board x");
    assert_near(num(&about["y"]), 50.0, 0.001, "board y");
    assert_sizing(&about["height"], Sz::Fixed(480.5), "board height");
}

#[test]
fn shapes_become_the_matching_node_types() {
    let conversion = convert(
        "kinds",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "frame", "Card", [10.0, 10.0, 200.0, 100.0]).kids(vec![shape(
                    3,
                    "rect",
                    "Swatch",
                    [20.0, 20.0, 30.0, 30.0],
                )]),
                shape(4, "group", "Cluster", [220.0, 10.0, 100.0, 50.0]).kids(vec![shape(
                    5,
                    "rect",
                    "Chip",
                    [220.0, 10.0, 100.0, 50.0],
                )]),
                shape(6, "circle", "Dot", [10.0, 150.0, 40.0, 40.0]),
                text(7, "Title", [60.0, 150.0, 120.0, 24.0], "Hello"),
                shape(8, "path", "Arrow", [200.0, 150.0, 20.0, 20.0]).with(json!({
                    "x": null, "y": null, "width": null, "height": null,
                    "content": "M200,150L220,160L200,170Z"
                })),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    for (name, kind) in [
        ("Card", NodeKind::Frame),
        ("Swatch", NodeKind::Frame),
        ("Cluster", NodeKind::Frame),
        ("Chip", NodeKind::Frame),
        ("Dot", NodeKind::Vector),
        ("Title", NodeKind::Text),
        ("Arrow", NodeKind::Vector),
    ] {
        assert_eq!(node(root, name).kind(), kind, "the type of {name}");
    }
    assert_eq!(
        nodes(root).len(),
        8,
        "one node for the board and each shape"
    );
    // A group and a rect have no layout of their own.
    for name in ["Cluster", "Swatch", "Chip"] {
        let json = json_of(node(root, name));
        assert_eq!(json["layout"]["mode"], "none", "the layout of {name}");
    }
    assert_eq!(child_names(node(root, "Card")), ["Swatch"]);
    assert_eq!(child_names(node(root, "Cluster")), ["Chip"]);
}

#[test]
fn children_of_frames_without_layout_keep_their_positions_as_x_and_y() {
    let conversion = convert(
        "absolute",
        vec![
            shape(1, "frame", "Home", [100.0, 200.0, 400.0, 300.0]).kids(vec![
                shape(2, "rect", "Direct", [110.0, 230.0, 50.0, 20.0]),
                shape(3, "frame", "Inner", [150.0, 260.0, 200.0, 100.0]).kids(vec![shape(
                    4,
                    "rect",
                    "Nested",
                    [165.0, 275.5, 40.0, 10.0],
                )]),
                shape(5, "group", "Group", [300.0, 400.0, 60.0, 40.0]).kids(vec![shape(
                    6,
                    "rect",
                    "InGroup",
                    [310.0, 405.0, 20.0, 20.0],
                )]),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let expect = [
        ("Direct", 10.0, 30.0, 50.0, 20.0),
        ("Inner", 50.0, 60.0, 200.0, 100.0),
        ("Nested", 15.0, 15.5, 40.0, 10.0),
        ("Group", 200.0, 200.0, 60.0, 40.0),
        ("InGroup", 10.0, 5.0, 20.0, 20.0),
    ];
    for (name, x, y, width, height) in expect {
        let json = json_of(node(root, name));
        assert_near(num(&json["x"]), x, 0.001, &format!("x of {name}"));
        assert_near(num(&json["y"]), y, 0.001, &format!("y of {name}"));
        assert_sizing(
            &json["width"],
            Sz::Fixed(width),
            &format!("width of {name}"),
        );
        assert_sizing(
            &json["height"],
            Sz::Fixed(height),
            &format!("height of {name}"),
        );
    }
}

#[test]
fn a_flex_frame_becomes_a_flex_layout_with_gap_padding_and_alignment() {
    let conversion = convert(
        "flex",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 600.0, 400.0]).kids(vec![
                shape(2, "frame", "Row", [0.0, 0.0, 300.0, 100.0])
                    .with(flex("row"))
                    .with(json!({
                        "layoutWrapType": "wrap",
                        "layoutGap": { "rowGap": 4, "columnGap": 6 },
                        "layoutPaddingType": "multiple",
                        "layoutPadding": { "p1": 1, "p2": 2, "p3": 3, "p4": 4 },
                        "layoutAlignItems": "center",
                        "layoutJustifyContent": "space-between"
                    })),
                shape(3, "frame", "Column", [0.0, 150.0, 100.0, 200.0])
                    .with(flex("column"))
                    .with(json!({
                        "layoutAlignItems": "end",
                        "layoutJustifyContent": "center"
                    })),
                shape(4, "frame", "Plain stretch", [300.0, 150.0, 100.0, 200.0])
                    .with(flex("column"))
                    .with(json!({ "layoutAlignItems": "stretch" })),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let row = json_of(node(root, "Row"))["layout"].clone();
    assert_eq!(row["mode"], "flex");
    assert_eq!(row["direction"], "row");
    assert_eq!(row["wrap"], "wrap");
    assert_eq!(row["alignItems"], "center");
    assert_eq!(row["justifyContent"], "space-between");
    assert_near(num(&row["gap"]["row"]), 4.0, 0.001, "row gap");
    assert_near(num(&row["gap"]["column"]), 6.0, 0.001, "column gap");
    let padding = &row["padding"];
    assert_near(num(&padding["top"]), 1.0, 0.001, "padding top");
    assert_near(num(&padding["right"]), 2.0, 0.001, "padding right");
    assert_near(num(&padding["bottom"]), 3.0, 0.001, "padding bottom");
    assert_near(num(&padding["left"]), 4.0, 0.001, "padding left");

    let column = json_of(node(root, "Column"))["layout"].clone();
    assert_eq!(column["mode"], "flex");
    assert_eq!(column["direction"], "column");
    assert_eq!(column["alignItems"], "end");
    assert_eq!(column["justifyContent"], "center");
    assert_ne!(column["wrap"], "wrap", "a nowrap layout does not wrap");
    assert_near(
        num_or_zero(&column["padding"]["top"]),
        0.0,
        0.001,
        "padding",
    );
    // Penpot's stretch does not stretch children that hug their content, so
    // it becomes start; filling children still stretch (US-054).
    assert_eq!(
        json_of(node(root, "Plain stretch"))["layout"]["alignItems"],
        "start"
    );
}

#[test]
fn flex_children_come_in_the_order_they_appear_in_not_the_stacking_order() {
    // Penpot lists the children of a flex frame last-appearing first.
    let conversion = convert(
        "flex-order",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 600.0, 400.0]).kids(vec![
                shape(2, "frame", "Row", [0.0, 0.0, 300.0, 40.0])
                    .with(flex("row"))
                    .kids(vec![
                        shape(3, "rect", "Third", [200.0, 0.0, 40.0, 40.0]),
                        shape(4, "rect", "Second", [100.0, 0.0, 40.0, 40.0]),
                        shape(5, "rect", "First", [0.0, 0.0, 40.0, 40.0]),
                    ]),
                shape(6, "frame", "Column", [0.0, 100.0, 40.0, 300.0])
                    .with(flex("column"))
                    .kids(vec![
                        shape(7, "rect", "Bottom", [0.0, 360.0, 40.0, 40.0]),
                        shape(8, "rect", "Top", [0.0, 100.0, 40.0, 40.0]),
                    ]),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    assert_eq!(child_names(node(root, "Row")), ["First", "Second", "Third"]);
    assert_eq!(child_names(node(root, "Column")), ["Top", "Bottom"]);
    // The layout computes their positions.
    for name in ["First", "Second", "Third", "Top", "Bottom"] {
        let json = json_of(node(root, name));
        assert!(
            json["x"].is_null() && json["y"].is_null(),
            "{name} has x or y"
        );
    }
}

#[test]
fn children_taken_out_of_a_flex_flow_keep_their_positions() {
    let conversion = convert(
        "flex-absolute",
        vec![
            shape(1, "frame", "Home", [100.0, 100.0, 600.0, 400.0]).kids(vec![
                shape(2, "frame", "Row", [100.0, 100.0, 300.0, 100.0])
                    .with(flex("row"))
                    .kids(vec![
                        shape(3, "rect", "Badge", [370.0, 110.0, 20.0, 20.0])
                            .with(json!({ "layoutItemAbsolute": true })),
                        shape(4, "rect", "In flow", [100.0, 100.0, 40.0, 40.0]),
                    ]),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let badge = json_of(node(root, "Badge"));
    assert_eq!(badge["absolute"], true);
    assert_near(num(&badge["x"]), 270.0, 0.001, "x of the badge");
    assert_near(num(&badge["y"]), 10.0, 0.001, "y of the badge");
    let in_flow = json_of(node(root, "In flow"));
    assert_ne!(in_flow["absolute"], true);
    assert!(in_flow["x"].is_null() && in_flow["y"].is_null());
}

#[test]
fn item_sizing_becomes_fixed_hug_or_fill() {
    let sized = |n, name: &str, h: &str, v: &str| {
        shape(n, "frame", name, [0.0, 0.0, 50.0, 20.0])
            .with(json!({ "layoutItemHSizing": h, "layoutItemVSizing": v }))
    };
    let conversion = convert(
        "sizing",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 600.0, 400.0])
                .with(json!({ "layoutItemHSizing": "auto" }))
                .with(flex("column"))
                .kids(vec![
                    sized(2, "Fixed", "fix", "fix"),
                    sized(3, "Hugging", "auto", "auto"),
                    sized(4, "Filling", "fill", "fill"),
                    sized(5, "Mixed", "fill", "auto"),
                    shape(6, "rect", "Unsized", [0.0, 0.0, 33.0, 22.0]),
                ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let board_json = json_of(root);
    assert_sizing(&board_json["width"], Sz::Hug, "a board that hugs its width");
    assert_sizing(&board_json["height"], Sz::Fixed(400.0), "board height");
    for (name, width, height) in [
        ("Fixed", Sz::Fixed(50.0), Sz::Fixed(20.0)),
        ("Hugging", Sz::Hug, Sz::Hug),
        ("Filling", Sz::Fill, Sz::Fill),
        ("Mixed", Sz::Fill, Sz::Hug),
        ("Unsized", Sz::Fixed(33.0), Sz::Fixed(22.0)),
    ] {
        let json = json_of(node(root, name));
        assert_sizing(&json["width"], width, &format!("width of {name}"));
        assert_sizing(&json["height"], height, &format!("height of {name}"));
    }
}

#[test]
fn texts_size_by_how_they_grow() {
    let grows = |n, name: &str, grow: &str| {
        text(n, name, [0.0, 0.0, 80.0, 24.0], "Some text").with(json!({ "growType": grow }))
    };
    let conversion = convert(
        "text-growth",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                grows(2, "Auto width", "auto-width"),
                grows(3, "Auto height", "auto-height"),
                grows(4, "Fixed", "fixed"),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let width = json_of(node(root, "Auto width"));
    assert_sizing(&width["width"], Sz::Hug, "auto-width text, width");
    assert_sizing(&width["height"], Sz::Hug, "auto-width text, height");
    let height = json_of(node(root, "Auto height"));
    assert_sizing(&height["width"], Sz::Fixed(80.0), "auto-height text, width");
    assert_sizing(&height["height"], Sz::Hug, "auto-height text, height");
    let fixed = json_of(node(root, "Fixed"));
    assert_sizing(&fixed["width"], Sz::Fixed(80.0), "fixed text, width");
    assert_sizing(&fixed["height"], Sz::Fixed(24.0), "fixed text, height");
}

#[test]
fn a_grid_frame_becomes_a_grid_layout_with_placed_children() {
    let conversion = convert(
        "grid",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 600.0, 400.0]).kids(vec![
                shape(2, "frame", "Grid", [0.0, 0.0, 300.0, 200.0])
                    .with(json!({
                        "layout": "grid",
                        "layoutGridDir": "row",
                        "layoutGridColumns": [
                            { "type": "flex", "value": 1 },
                            { "type": "auto", "value": null }
                        ],
                        "layoutGridRows": [
                            { "type": "auto", "value": null },
                            { "type": "flex", "value": 2 }
                        ],
                        "layoutGridCells": {
                            uid(20): {
                                "id": uid(20), "position": "manual", "column": 2, "row": 1,
                                "columnSpan": 1, "rowSpan": 1, "shapes": [uid(3)],
                                "alignSelf": "auto", "justifySelf": "auto"
                            },
                            uid(21): {
                                "id": uid(21), "position": "manual", "column": 1, "row": 2,
                                "columnSpan": 2, "rowSpan": 1, "shapes": [uid(4)],
                                "alignSelf": "auto", "justifySelf": "auto"
                            }
                        },
                        "layoutGapType": "multiple",
                        "layoutGap": { "rowGap": 10, "columnGap": 20 },
                        "layoutPaddingType": "multiple",
                        "layoutPadding": { "p1": 4, "p2": 5, "p3": 6, "p4": 7 },
                        "layoutAlignItems": "start",
                        "layoutJustifyContent": "stretch",
                        "layoutAlignContent": "stretch"
                    }))
                    .kids(vec![
                        shape(3, "rect", "Top right", [100.0, 0.0, 50.0, 50.0]),
                        shape(4, "rect", "Bottom wide", [0.0, 100.0, 300.0, 50.0]),
                    ]),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let layout = json_of(node(root, "Grid"))["layout"].clone();
    assert_eq!(layout["mode"], "grid");
    let columns = layout["columns"].as_array().expect("columns");
    assert_eq!(columns.len(), 2);
    assert_near(num(&columns[0]["fr"]), 1.0, 0.001, "first column");
    assert_eq!(columns[1], "auto");
    let rows = layout["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], "auto");
    assert_near(num(&rows[1]["fr"]), 2.0, 0.001, "second row");
    assert_near(num(&layout["gap"]["row"]), 10.0, 0.001, "row gap");
    assert_near(num(&layout["gap"]["column"]), 20.0, 0.001, "column gap");
    assert_near(num(&layout["padding"]["top"]), 4.0, 0.001, "padding top");
    assert_near(
        num(&layout["padding"]["right"]),
        5.0,
        0.001,
        "padding right",
    );
    assert_near(
        num(&layout["padding"]["bottom"]),
        6.0,
        0.001,
        "padding bottom",
    );
    assert_near(num(&layout["padding"]["left"]), 7.0, 0.001, "padding left");

    let line = |value: &serde_json::Value, key: &str| num(&value[key]);
    let top_right = json_of(node(root, "Top right"));
    assert_near(
        line(&top_right["gridColumn"], "start"),
        2.0,
        0.001,
        "column start",
    );
    assert_near(
        num_or_one(&top_right["gridColumn"]["span"]),
        1.0,
        0.001,
        "column span",
    );
    assert_near(
        line(&top_right["gridRow"], "start"),
        1.0,
        0.001,
        "row start",
    );
    let bottom = json_of(node(root, "Bottom wide"));
    assert_near(
        line(&bottom["gridColumn"], "start"),
        1.0,
        0.001,
        "column start",
    );
    assert_near(
        num(&bottom["gridColumn"]["span"]),
        2.0,
        0.001,
        "column span",
    );
    assert_near(line(&bottom["gridRow"], "start"), 2.0, 0.001, "row start");
    assert!(top_right["x"].is_null() && bottom["y"].is_null());
}

fn num_or_one(value: &serde_json::Value) -> f64 {
    if value.is_null() { 1.0 } else { num(value) }
}

#[test]
fn solid_fills_keep_their_color_and_opacity() {
    let conversion = convert(
        "solid",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0])
                .with(json!({ "fills": [{ "fillColor": "#336699", "fillOpacity": 0.5 }] }))
                .kids(vec![
                    shape(2, "rect", "Opaque", [0.0, 0.0, 20.0, 20.0])
                        .with(json!({ "fills": [{ "fillColor": "#ff8800", "fillOpacity": 1 }] })),
                    shape(3, "rect", "Bare", [0.0, 0.0, 20.0, 20.0]),
                ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let board_fills = json_of(root)["fills"].clone();
    assert_eq!(board_fills.as_array().map(Vec::len), Some(1));
    assert_eq!(board_fills[0]["type"], "solid");
    assert_color(
        &board_fills[0]["color"],
        [0x33, 0x66, 0x99],
        0.5,
        "board fill",
    );
    let opaque = json_of(node(root, "Opaque"))["fills"].clone();
    assert_eq!(opaque[0]["type"], "solid");
    assert_color(&opaque[0]["color"], [0xff, 0x88, 0x00], 1.0, "rect fill");
    let bare = json_of(node(root, "Bare"));
    assert!(
        bare["fills"].is_null() || bare["fills"].as_array().is_some_and(Vec::is_empty),
        "a shape with no fill has none"
    );
}

#[test]
fn linear_gradients_become_gradient_fills_with_an_angle() {
    let gradient = |start: [f64; 2], end: [f64; 2]| {
        json!({
            "fillColorGradient": {
                "type": "linear",
                "startX": start[0], "startY": start[1], "endX": end[0], "endY": end[1],
                "width": 1,
                "stops": [
                    { "color": "#ff0000", "opacity": 1, "offset": 0 },
                    { "color": "#0000ff", "opacity": 0.5, "offset": 1 }
                ]
            },
            "fillOpacity": 1
        })
    };
    let conversion = convert(
        "gradient",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "rect", "Down", [0.0, 0.0, 100.0, 100.0])
                    .with(json!({ "fills": [gradient([0.5, 0.0], [0.5, 1.0])] })),
                shape(3, "rect", "Right", [0.0, 0.0, 100.0, 100.0])
                    .with(json!({ "fills": [gradient([0.0, 0.5], [1.0, 0.5])] })),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    for (name, angle) in [("Down", 180.0), ("Right", 90.0)] {
        let fill = json_of(node(root, name))["fills"][0].clone();
        assert_eq!(fill["type"], "linear-gradient", "fill of {name}");
        assert_near(num(&fill["angle"]), angle, 0.5, &format!("angle of {name}"));
        let stops = fill["stops"].as_array().expect("stops");
        assert_eq!(stops.len(), 2);
        assert_near(num(&stops[0]["position"]), 0.0, 0.001, "first stop");
        assert_near(num(&stops[1]["position"]), 1.0, 0.001, "last stop");
        assert_color(&stops[0]["color"], [0xff, 0x00, 0x00], 1.0, "first stop");
        assert_color(&stops[1]["color"], [0x00, 0x00, 0xff], 0.5, "last stop");
    }
}

#[test]
fn images_in_the_export_become_attachments_that_the_renderer_loads() {
    let (photo, logo) = (uid(100), uid(101));
    let conversion = convert_with(
        "images",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "rect", "Photo", [0.0, 0.0, 100.0, 100.0])
                    .with(json!({ "fills": [image_fill(&photo)] })),
                shape(3, "image", "Logo", [120.0, 0.0, 64.0, 64.0]).with(json!({
                    "metadata": { "id": logo, "width": 2, "height": 2, "mtype": "image/png" }
                })),
            ]),
        ],
        [
            media_entries(&photo, &uid(200)),
            media_entries(&logo, &uid(201)),
        ]
        .concat(),
    );
    let root = board_node(board(&conversion, "Home"));
    let photo_node = json_of(node(root, "Photo"));
    assert_eq!(photo_node["fills"][0]["type"], "image");
    assert_eq!(node(root, "Logo").kind(), NodeKind::Image);
    let logo_node = json_of(node(root, "Logo"));
    assert_sizing(&logo_node["width"], Sz::Fixed(64.0), "image width");

    let refs = attachment_refs(&json_of(root));
    assert_eq!(refs.len(), 2, "the design refers to two images: {refs:?}");
    for reference in &refs {
        let id = reference.strip_prefix("attachment:").expect("a prefix");
        let attachment = conversion
            .attachments
            .iter()
            .find(|attachment| attachment.id == id)
            .unwrap_or_else(|| panic!("no attachment for {reference}"));
        assert_eq!(attachment.extension, "png");
        assert_eq!(attachment.bytes, PNG, "the image bytes of {reference}");
        Assets::new()
            .add_image(
                &reference.parse().expect("an attachment reference"),
                &attachment.bytes,
            )
            .unwrap_or_else(|error| panic!("the renderer cannot load {reference}: {error}"));
    }
}

#[test]
fn strokes_keep_their_color_and_width() {
    let stroke = |color: &str, width: f64, style: &str, alignment: &str| {
        json!({ "strokes": [{
            "strokeColor": color, "strokeOpacity": 0.5, "strokeWidth": width,
            "strokeStyle": style, "strokeAlignment": alignment
        }] })
    };
    let conversion = convert(
        "strokes",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "rect", "Outlined", [0.0, 0.0, 50.0, 50.0])
                    .with(stroke("#112233", 2.0, "solid", "inner")),
                shape(3, "path", "Line", [60.0, 0.0, 40.0, 1.0]).with(json!({
                    "x": null, "y": null, "width": null, "height": null,
                    "content": "M60,0.5L100,0.5",
                    "strokes": [{
                        "strokeColor": "#a1a1aa", "strokeOpacity": 1, "strokeWidth": 1.4,
                        "strokeStyle": "solid", "strokeAlignment": "inner"
                    }]
                })),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let outlined = json_of(node(root, "Outlined"))["strokes"].clone();
    assert_eq!(outlined.as_array().map(Vec::len), Some(1));
    assert_color(&outlined[0]["color"], [0x11, 0x22, 0x33], 0.5, "stroke");
    assert_near(num(&outlined[0]["width"]), 2.0, 0.001, "stroke width");
    let line = json_of(node(root, "Line"))["strokes"].clone();
    assert_color(&line[0]["color"], [0xa1, 0xa1, 0xaa], 1.0, "line");
    assert_near(num(&line[0]["width"]), 1.4, 0.001, "line width");
    assert!(
        conversion.report.entries.is_empty(),
        "solid inner strokes convert fully: {:#?}",
        conversion.report.entries
    );
}

#[test]
fn a_circle_stroke_keeps_its_alignment_and_lies_inside_without_one() {
    let stroke = |alignment: Option<&str>| {
        let mut stroke = json!({
            "strokeColor": "#000000", "strokeOpacity": 1, "strokeWidth": 2,
            "strokeStyle": "solid"
        });
        if let Some(alignment) = alignment {
            stroke["strokeAlignment"] = json!(alignment);
        }
        json!({ "strokes": [stroke] })
    };
    let conversion = convert(
        "circle-strokes",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "circle", "Inner", [0.0, 0.0, 40.0, 40.0]).with(stroke(Some("inner"))),
                shape(3, "circle", "Default", [50.0, 0.0, 40.0, 40.0]).with(stroke(None)),
                shape(4, "circle", "Outer", [100.0, 0.0, 40.0, 40.0]).with(stroke(Some("outer"))),
                shape(5, "circle", "Center", [150.0, 0.0, 40.0, 40.0]).with(stroke(Some("center"))),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let align = |name: &str| json_of(node(root, name))["strokes"][0]["align"].clone();
    assert_eq!(align("Inner"), "inside");
    assert_eq!(align("Default"), "inside");
    assert_eq!(align("Outer"), "outside");
    assert_eq!(align("Center"), "center");
}

/// A black centered stroke of `width` and `style` with the properties of
/// `caps` set.
fn capped_stroke(width: f64, style: &str, caps: Value) -> Value {
    let mut stroke = json!({
        "strokeColor": "#000000", "strokeOpacity": 1, "strokeWidth": width,
        "strokeStyle": style, "strokeAlignment": "center"
    });
    for (key, value) in caps.as_object().expect("caps are an object") {
        stroke[key] = value.clone();
    }
    stroke
}

/// A Penpot path along the top of a 40 x 1 box at (0, `y`) with `stroke`.
fn open_path(n: u32, name: &str, y: f64, stroke: Value) -> S {
    shape(n, "path", name, [0.0, y, 40.0, 1.0]).with(json!({
        "x": null, "y": null, "width": null, "height": null,
        "content": format!("M0,{y}L40,{y}"),
        "strokes": [stroke]
    }))
}

/// The reasons the report gives for the shape `shape_id`.
fn reasons<'a>(conversion: &'a Conversion, shape_id: &str) -> Vec<&'a str> {
    entries_of(conversion, shape_id)
        .into_iter()
        .map(|entry| entry.reason.as_str())
        .collect()
}

#[test]
fn a_marker_at_one_end_is_reported_with_the_cap_the_stroke_actually_gets() {
    let conversion = convert(
        "marker-ends",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                open_path(
                    2,
                    "Round to arrow",
                    0.0,
                    capped_stroke(
                        2.0,
                        "solid",
                        json!({"strokeCapStart": "round", "strokeCapEnd": "line-arrow"}),
                    ),
                ),
                open_path(
                    3,
                    "Arrow to round",
                    10.0,
                    capped_stroke(
                        2.0,
                        "solid",
                        json!({"strokeCapStart": "triangle-arrow", "strokeCapEnd": "round"}),
                    ),
                ),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let cap = |name: &str| json_of(node(root, name))["strokes"][0]["cap"].clone();

    assert_eq!(cap("Round to arrow"), "round");
    let round_start = reasons(&conversion, &uid(2));
    assert!(
        round_start
            .iter()
            .any(|reason| reason.contains("line-arrow")),
        "the arrow at the end is reported: {round_start:#?}"
    );
    assert!(
        !round_start.iter().any(|reason| reason.contains("no cap")),
        "the stroke gets the round cap, so no report says it gets none: {round_start:#?}"
    );

    assert_eq!(cap("Arrow to round"), Value::Null);
    let arrow_start = reasons(&conversion, &uid(3));
    assert!(
        arrow_start
            .iter()
            .any(|reason| reason.contains("triangle-arrow")),
        "the arrow at the start is reported: {arrow_start:#?}"
    );
    assert!(
        arrow_start.iter().all(|reason| reason.contains("no cap")),
        "every report says the stroke gets no cap: {arrow_start:#?}"
    );
}

#[test]
fn a_vector_stroke_takes_the_cap_of_its_start_and_a_different_end_is_reported() {
    let solid = |caps: Value| capped_stroke(2.0, "solid", caps);
    let conversion = convert(
        "caps",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                open_path(
                    2,
                    "Round",
                    0.0,
                    solid(json!({"strokeCapStart": "round", "strokeCapEnd": "round"})),
                ),
                open_path(
                    3,
                    "Square",
                    10.0,
                    solid(json!({"strokeCapStart": "square", "strokeCapEnd": "square"})),
                ),
                open_path(4, "Absent", 20.0, solid(json!({}))),
                open_path(
                    5,
                    "Null",
                    30.0,
                    solid(json!({"strokeCapStart": null, "strokeCapEnd": null})),
                ),
                open_path(
                    6,
                    "Round to square",
                    40.0,
                    solid(json!({"strokeCapStart": "round", "strokeCapEnd": "square"})),
                ),
                open_path(
                    7,
                    "None to round",
                    50.0,
                    solid(json!({"strokeCapStart": null, "strokeCapEnd": "round"})),
                ),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let cap = |name: &str| json_of(node(root, name))["strokes"][0]["cap"].clone();

    assert_eq!(cap("Round"), "round");
    assert_eq!(cap("Square"), "square");
    assert_eq!(cap("Absent"), Value::Null);
    assert_eq!(cap("Null"), Value::Null);
    assert_eq!(reasons(&conversion, &uid(2)), Vec::<&str>::new());
    assert_eq!(reasons(&conversion, &uid(3)), Vec::<&str>::new());
    assert_eq!(reasons(&conversion, &uid(4)), Vec::<&str>::new());
    assert_eq!(reasons(&conversion, &uid(5)), Vec::<&str>::new());

    assert_eq!(cap("Round to square"), "round");
    let round_to_square = reasons(&conversion, &uid(6));
    assert!(
        round_to_square
            .iter()
            .any(|reason| reason.contains("round") && reason.contains("square")),
        "the report names both ends: {round_to_square:#?}"
    );

    assert_eq!(cap("None to round"), Value::Null);
    assert_reported(
        &conversion,
        &uid(7),
        &uid(1),
        "Home",
        "None to round",
        &["round"],
    );
}

#[test]
fn a_dotted_vector_stroke_becomes_round_dots_twice_its_width_apart_whatever_its_caps() {
    let conversion = convert(
        "dotted-vectors",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                open_path(
                    2,
                    "Dotted line",
                    0.0,
                    capped_stroke(
                        2.0,
                        "dotted",
                        json!({"strokeCapStart": "square", "strokeCapEnd": "round"}),
                    ),
                ),
                shape(3, "circle", "Dotted ring", [0.0, 10.0, 20.0, 20.0]).with(json!({
                    "strokes": [capped_stroke(3.0, "dotted", json!({}))]
                })),
                open_path(
                    4,
                    "Dotted arrow",
                    40.0,
                    capped_stroke(
                        2.0,
                        "dotted",
                        json!({"strokeCapStart": "round", "strokeCapEnd": "diamond-marker"}),
                    ),
                ),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let stroke = |name: &str| json_of(node(root, name))["strokes"][0].clone();

    let line = stroke("Dotted line");
    assert_eq!(num(&line["dash"]["length"]), 0.0);
    assert_eq!(num(&line["dash"]["gap"]), 4.0);
    assert_eq!(line["cap"], "round");
    let ring = stroke("Dotted ring");
    assert_eq!(num(&ring["dash"]["length"]), 0.0);
    assert_eq!(num(&ring["dash"]["gap"]), 6.0);
    assert_eq!(ring["cap"], "round");
    assert_eq!(reasons(&conversion, &uid(2)), Vec::<&str>::new());
    assert_eq!(reasons(&conversion, &uid(3)), Vec::<&str>::new());

    assert_eq!(stroke("Dotted arrow")["cap"], "round");
    let arrow = entries_of(&conversion, &uid(4));
    assert_eq!(arrow.len(), 1, "only the marker is reported: {arrow:#?}");
    assert_eq!(arrow[0].property.as_deref(), Some("strokeCapEnd"));
    assert!(arrow[0].reason.contains("diamond-marker"), "{arrow:#?}");
}

#[test]
fn the_stroke_of_a_board_frame_or_rect_gets_no_cap_and_no_report_about_it() {
    let conversion = convert(
        "frame-caps",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "frame", "Dotted card", [0.0, 0.0, 50.0, 50.0]).with(json!({
                    "strokes": [capped_stroke(
                        2.0,
                        "dotted",
                        json!({"strokeCapStart": "round", "strokeCapEnd": "square"}),
                    )]
                })),
            ]),
        ],
    );
    let card = json_of(node(board_node(board(&conversion, "Home")), "Dotted card"));
    assert_eq!(card["strokes"][0]["dash"], Value::Null);
    assert_eq!(card["strokes"][0]["cap"], Value::Null);
    let entries = entries_of(&conversion, &uid(2));
    assert_eq!(
        entries.len(),
        1,
        "only the dotted style is reported: {entries:#?}"
    );
    assert_eq!(entries[0].property.as_deref(), Some("strokeStyle"));
    assert!(entries[0].reason.contains("dotted"), "{entries:#?}");
}

#[test]
fn corner_radii_map_to_the_four_corners() {
    let conversion = convert(
        "radii",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0])
                .with(json!({ "r1": 8, "r2": 8, "r3": 8, "r4": 8 }))
                .kids(vec![
                    shape(2, "rect", "Uneven", [0.0, 0.0, 50.0, 50.0])
                        .with(json!({ "r1": 1, "r2": 2, "r3": 3, "r4": 4 })),
                    shape(3, "rect", "Square", [60.0, 0.0, 50.0, 50.0])
                        .with(json!({ "r1": 0, "r2": 0, "r3": 0, "r4": 0 })),
                ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let board_radius = json_of(root)["radius"].clone();
    for corner in ["topLeft", "topRight", "bottomRight", "bottomLeft"] {
        assert_near(num(&board_radius[corner]), 8.0, 0.001, corner);
    }
    let uneven = json_of(node(root, "Uneven"))["radius"].clone();
    assert_near(num(&uneven["topLeft"]), 1.0, 0.001, "top left");
    assert_near(num(&uneven["topRight"]), 2.0, 0.001, "top right");
    assert_near(num(&uneven["bottomRight"]), 3.0, 0.001, "bottom right");
    assert_near(num(&uneven["bottomLeft"]), 4.0, 0.001, "bottom left");
    let square = json_of(node(root, "Square"))["radius"].clone();
    for corner in ["topLeft", "topRight", "bottomRight", "bottomLeft"] {
        assert_near(num_or_zero(&square[corner]), 0.0, 0.001, corner);
    }
}

#[test]
fn opacity_is_kept() {
    let conversion = convert(
        "opacity",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "frame", "Faded", [0.0, 0.0, 50.0, 50.0]).with(json!({ "opacity": 0.7 })),
                shape(3, "rect", "Solid", [60.0, 0.0, 50.0, 50.0]),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    assert_near(
        num(&json_of(node(root, "Faded"))["opacity"]),
        0.7,
        0.001,
        "opacity",
    );
    let solid = json_of(node(root, "Solid"));
    assert_near(
        if solid["opacity"].is_null() {
            1.0
        } else {
            num(&solid["opacity"])
        },
        1.0,
        0.001,
        "opacity of a shape with none",
    );
}

#[test]
fn drop_and_inner_shadows_are_converted_and_hidden_ones_are_not() {
    let shadow = |style: &str, hidden: bool, color: &str, opacity: f64, offsets: [f64; 4]| {
        json!({
            "id": uid(900), "style": style, "hidden": hidden,
            "color": { "color": color, "opacity": opacity },
            "offsetX": offsets[0], "offsetY": offsets[1], "blur": offsets[2], "spread": offsets[3]
        })
    };
    let conversion = convert(
        "shadows",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "frame", "Raised", [10.0, 10.0, 100.0, 100.0]).with(json!({ "shadow": [
                    shadow("drop-shadow", false, "#000000", 0.55, [2.0, 24.0, 64.0, 3.0]),
                    shadow("drop-shadow", true, "#00ff00", 1.0, [9.0, 9.0, 9.0, 9.0]),
                    shadow("inner-shadow", false, "#ff0000", 1.0, [1.0, 2.0, 3.0, 0.0]),
                ] })),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let shadows = json_of(node(root, "Raised"))["shadows"].clone();
    let shadows = shadows.as_array().expect("shadows");
    assert_eq!(
        shadows.len(),
        2,
        "the hidden shadow is left out: {shadows:#?}"
    );
    assert_eq!(shadows[0]["type"], "drop");
    assert_near(num(&shadows[0]["offsetX"]), 2.0, 0.001, "offset x");
    assert_near(num(&shadows[0]["offsetY"]), 24.0, 0.001, "offset y");
    assert_near(num(&shadows[0]["blur"]), 64.0, 0.001, "blur");
    assert_near(num(&shadows[0]["spread"]), 3.0, 0.001, "spread");
    assert_color(&shadows[0]["color"], [0, 0, 0], 0.55, "drop shadow");
    assert_eq!(shadows[1]["type"], "inner");
    assert_near(num(&shadows[1]["offsetY"]), 2.0, 0.001, "offset y");
    assert_near(num(&shadows[1]["blur"]), 3.0, 0.001, "blur");
    assert_color(&shadows[1]["color"], [0xff, 0, 0], 1.0, "inner shadow");
}

#[test]
fn a_text_keeps_its_visible_drop_shadows_without_spread_and_drops_an_inner_one() {
    let shadow = |style: &str, hidden: bool, color: &str, spread: f64| {
        json!({
            "id": uid(900), "style": style, "hidden": hidden,
            "color": { "color": color, "opacity": 1 },
            "offsetX": 4, "offsetY": 6, "blur": 8, "spread": spread
        })
    };
    let conversion = convert(
        "text-shadows",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                text(2, "Heading", [10.0, 10.0, 200.0, 40.0], "Hej").with(json!({
                    "shadow": [
                        shadow("drop-shadow", false, "#ff0000", 0.0),
                        shadow("drop-shadow", true, "#00ff00", 0.0),
                        shadow("inner-shadow", false, "#0000ff", 0.0),
                        shadow("drop-shadow", false, "#000000", 3.0),
                        shadow("drop-shadow", false, "#ffffff", -2.0),
                    ],
                    "strokes": [{ "strokeColor": "#000000", "strokeWidth": 1 }]
                })),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let shadows = json_of(node(root, "Heading"))["shadows"].clone();
    let shadows = shadows.as_array().expect("shadows");
    assert_eq!(shadows.len(), 3, "the visible drop shadows: {shadows:#?}");
    assert_color(&shadows[0]["color"], [0xff, 0, 0], 1.0, "first shadow");
    assert_color(&shadows[1]["color"], [0, 0, 0], 1.0, "second shadow");
    assert_color(
        &shadows[2]["color"],
        [0xff, 0xff, 0xff],
        1.0,
        "third shadow",
    );
    for shadow in shadows {
        assert_eq!(shadow["type"], "drop");
        assert_near(num(&shadow["offsetX"]), 4.0, 0.001, "offset x");
        assert_near(num(&shadow["offsetY"]), 6.0, 0.001, "offset y");
        assert_near(num(&shadow["blur"]), 8.0, 0.001, "blur");
        assert_near(num(&shadow["spread"]), 0.0, 0.001, "spread");
    }

    let entries = entries_of(&conversion, &uid(2));
    let reported = |property: &str, words: &[&str]| {
        entries
            .iter()
            .filter(|entry| {
                let reason = entry.reason.to_lowercase();
                entry.property.as_deref() == Some(property)
                    && words.iter().all(|word| reason.contains(word))
            })
            .count()
    };
    assert_eq!(entries.len(), 4, "{entries:#?}");
    assert_eq!(reported("strokes", &["strokes on text"]), 1, "{entries:#?}");
    assert_eq!(reported("shadow", &["inner"]), 1, "{entries:#?}");
    assert_eq!(reported("shadow", &["spread", "3"]), 1, "{entries:#?}");
    assert_eq!(reported("shadow", &["spread", "-2"]), 1, "{entries:#?}");
    assert!(
        entries.iter().all(|entry| entry.name == "Heading"),
        "{entries:#?}"
    );
}

#[test]
fn text_keeps_its_characters_and_style() {
    let style = json!({
        "fontFamily": "Geist Mono", "fontSize": "18", "fontWeight": "500",
        "fontStyle": "italic", "lineHeight": "1.4", "letterSpacing": "0.5",
        "textAlign": "center", "textDecoration": "underline",
        "fills": [{ "fillColor": "#f4f4f5", "fillOpacity": 1 }]
    });
    let conversion = convert(
        "text-style",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "text", "Styled", [10.0, 10.0, 100.0, 26.0]).with(json!({
                    "growType": "fixed",
                    "content": text_content(vec![paragraph(vec![run("Hej världen", style)])])
                })),
                text(3, "Plain", [10.0, 50.0, 100.0, 24.0], "Plain text"),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let styled = json_of(node(root, "Styled"));
    assert_eq!(styled["content"], "Hej världen");
    assert_eq!(styled["fontFamily"], "Geist Mono");
    assert_near(num(&styled["fontSize"]), 18.0, 0.001, "font size");
    assert_near(num(&styled["fontWeight"]), 500.0, 0.001, "font weight");
    assert_eq!(styled["fontStyle"], "italic");
    assert_near(num(&styled["lineHeight"]), 1.4, 0.001, "line height");
    assert_near(num(&styled["letterSpacing"]), 0.5, 0.001, "letter spacing");
    assert_eq!(styled["textAlign"], "center");
    assert_eq!(styled["textDecoration"], "underline");
    assert_color(&styled["color"], [0xf4, 0xf4, 0xf5], 1.0, "text color");
    let plain = json_of(node(root, "Plain"));
    assert_eq!(plain["content"], "Plain text");
    assert_eq!(plain["fontFamily"], "Inter");
    assert_near(num(&plain["fontSize"]), 16.0, 0.001, "font size");
    assert_color(&plain["color"], [0x10, 0x20, 0x30], 1.0, "text color");
    assert!(
        conversion.report.entries.is_empty(),
        "{:#?}",
        conversion.report
    );
}

#[test]
fn paragraphs_become_lines_and_runs_of_one_style_join_up() {
    let conversion = convert(
        "text-paragraphs",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "text", "Two paragraphs", [0.0, 0.0, 100.0, 48.0]).with(json!({
                    "growType": "fixed",
                    "content": text_content(vec![
                        paragraph(vec![run("One", json!({}))]),
                        paragraph(vec![run("Two", json!({}))]),
                    ])
                })),
                shape(3, "text", "Split runs", [0.0, 60.0, 100.0, 24.0]).with(json!({
                    "growType": "fixed",
                    "content": text_content(vec![paragraph(vec![
                        run("Same ", json!({})),
                        run("style", json!({})),
                    ])])
                })),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    assert_eq!(json_of(node(root, "Two paragraphs"))["content"], "One\nTwo");
    assert_eq!(json_of(node(root, "Split runs"))["content"], "Same style");
    assert!(
        conversion.report.entries.is_empty(),
        "runs that look alike are not mixed styles: {:#?}",
        conversion.report.entries
    );
}

#[test]
fn paths_are_moved_into_the_box_of_their_vector() {
    let conversion = convert(
        "paths",
        vec![
            shape(1, "frame", "Home", [100.0, 200.0, 400.0, 300.0]).kids(vec![
                shape(2, "path", "Square path", [110.0, 230.0, 20.0, 20.0]).with(json!({
                    "x": null, "y": null, "width": null, "height": null,
                    "content": "M110,230L130,230L130,250L110,250Z",
                    "fills": [{ "fillColor": "#ff0000", "fillOpacity": 1 }]
                })),
                shape(3, "circle", "Oval", [120.0, 240.0, 30.0, 10.0])
                    .with(json!({ "fills": [{ "fillColor": "#00ff00", "fillOpacity": 1 }] })),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let square = json_of(node(root, "Square path"));
    assert_eq!(square["type"], "vector");
    assert_near(num(&square["x"]), 10.0, 0.001, "x");
    assert_near(num(&square["y"]), 30.0, 0.001, "y");
    assert_sizing(&square["width"], Sz::Fixed(20.0), "width");
    assert_sizing(&square["height"], Sz::Fixed(20.0), "height");
    let path = square["path"].as_str().expect("path data");
    let numbers = path_numbers(path);
    assert!(numbers.len() >= 8, "path {path} lost points");
    for number in &numbers {
        assert!(
            (-0.01..=20.01).contains(number),
            "path {path} is not in the vector's own 20 by 20 box"
        );
    }
    assert_eq!(square["fills"][0]["type"], "solid");

    let oval = json_of(node(root, "Oval"));
    assert_eq!(oval["type"], "vector");
    assert_near(num(&oval["x"]), 20.0, 0.001, "x");
    assert_near(num(&oval["y"]), 40.0, 0.001, "y");
    assert_sizing(&oval["width"], Sz::Fixed(30.0), "width");
    assert_sizing(&oval["height"], Sz::Fixed(10.0), "height");
    let path = oval["path"].as_str().expect("path data");
    let numbers = path_numbers(path);
    assert!(!numbers.is_empty(), "the ellipse has a path");
    // Whether the path uses curves or arcs, its numbers (coordinates, radii)
    // stay within the 30 by 10 box, and it reaches the box's right edge.
    for number in &numbers {
        assert!(
            (-0.01..=30.01).contains(number),
            "{path} leaves the 30 by 10 box"
        );
    }
    assert!(
        numbers.iter().any(|number| *number > 29.99),
        "{path} does not reach the right edge of its box"
    );
    assert_eq!(oval["fills"][0]["type"], "solid");
}

#[test]
fn hidden_shapes_become_invisible_nodes() {
    let conversion = convert(
        "hidden",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "rect", "Gone", [0.0, 0.0, 20.0, 20.0]).with(json!({ "hidden": true })),
                shape(3, "rect", "Shown", [30.0, 0.0, 20.0, 20.0]).with(json!({ "hidden": false })),
                shape(4, "rect", "Unmarked", [60.0, 0.0, 20.0, 20.0]),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    assert!(!node(root, "Gone").visible());
    assert!(node(root, "Shown").visible());
    assert!(node(root, "Unmarked").visible());
}

#[test]
fn a_board_with_only_convertible_shapes_has_an_empty_report() {
    let conversion = convert(
        "clean",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0])
                .with(flex("column"))
                .kids(vec![
                    shape(2, "rect", "Rect", [0.0, 0.0, 20.0, 20.0]),
                    text(3, "Text", [0.0, 30.0, 60.0, 20.0], "Text"),
                ]),
        ],
    );
    assert!(
        conversion.report.entries.is_empty(),
        "{:#?}",
        conversion.report.entries
    );
}

#[test]
fn boolean_shapes_masks_blur_and_radial_gradients_are_listed_in_the_report() {
    let conversion =
        convert(
            "report",
            vec![
                shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "rect", "Before", [0.0, 0.0, 20.0, 20.0]),
                shape(3, "bool", "Union", [30.0, 0.0, 40.0, 20.0])
                    .with(json!({
                        "boolType": "union",
                        "content": "M30,0L70,0L70,20L30,20Z"
                    }))
                    .kids(vec![
                        shape(4, "rect", "Operand A", [30.0, 0.0, 20.0, 20.0]),
                        shape(5, "rect", "Operand B", [50.0, 0.0, 20.0, 20.0]),
                    ]),
                shape(6, "rect", "After", [80.0, 0.0, 20.0, 20.0]),
                shape(7, "group", "Masked", [0.0, 50.0, 60.0, 60.0])
                    .with(json!({ "maskedGroup": true }))
                    .kids(vec![shape(8, "rect", "Under mask", [0.0, 50.0, 60.0, 60.0])]),
                shape(9, "frame", "Blurred", [100.0, 50.0, 60.0, 60.0]).with(json!({
                    "blur": { "id": uid(901), "type": "layer-blur", "value": 8, "hidden": false }
                })),
                shape(10, "rect", "Radial", [180.0, 50.0, 60.0, 60.0]).with(json!({ "fills": [{
                    "fillColorGradient": {
                        "type": "radial", "startX": 0.5, "startY": 0.5, "endX": 1.0, "endY": 0.5,
                        "width": 1,
                        "stops": [
                            { "color": "#ffffff", "opacity": 1, "offset": 0 },
                            { "color": "#000000", "opacity": 1, "offset": 1 }
                        ]
                    },
                    "fillOpacity": 1
                }] })),
            ]),
            ],
        );
    assert_reported(&conversion, &uid(3), &uid(1), "Home", "Union", &["bool"]);
    assert_reported(&conversion, &uid(7), &uid(1), "Home", "Masked", &["mask"]);
    assert_reported(&conversion, &uid(9), &uid(1), "Home", "Blurred", &["blur"]);
    assert_reported(
        &conversion,
        &uid(10),
        &uid(1),
        "Home",
        "Radial",
        &["radial"],
    );
    // One bad shape never stops the others, and the board is still converted.
    let root = board_node(board(&conversion, "Home"));
    assert!(has_node(root, "Before") && has_node(root, "After"));
    // Shapes with only a property left out are still converted.
    assert!(has_node(root, "Blurred") && has_node(root, "Radial"));
    for entry in &conversion.report.entries {
        assert!(!entry.reason.trim().is_empty(), "{entry:?} has no reason");
    }
    assert!(
        entries_of(&conversion, &uid(2)).is_empty() && entries_of(&conversion, &uid(6)).is_empty(),
        "convertible shapes are not listed"
    );
}

#[test]
fn mixed_text_styles_unknown_types_and_missing_shapes_are_listed() {
    let conversion = convert(
        "report-more",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "text", "Mixed", [0.0, 0.0, 100.0, 24.0]).with(json!({
                    "growType": "fixed",
                    "content": text_content(vec![paragraph(vec![
                        run("Hello ", json!({ "fontWeight": "700" })),
                        run("world", json!({ "fontWeight": "400" })),
                    ])])
                })),
                shape(3, "svg-raw", "Raw svg", [0.0, 40.0, 30.0, 30.0]),
                shape(4, "frame", "Broken", [100.0, 40.0, 30.0, 30.0])
                    .with(json!({ "shapes": [uid(99)] })),
                shape(5, "rect", "Fine", [200.0, 40.0, 30.0, 30.0]),
            ]),
        ],
    );
    assert_reported(&conversion, &uid(2), &uid(1), "Home", "Mixed", &["mixed"]);
    assert_reported(
        &conversion,
        &uid(3),
        &uid(1),
        "Home",
        "Raw svg",
        &["svg-raw"],
    );
    let missing = entries_of(&conversion, &uid(99));
    assert_eq!(missing.len(), 1, "the missing shape is listed once");
    assert_eq!(missing[0].board_id, uid(1));
    assert!(missing[0].property.is_none(), "the whole shape is missing");
    let root = board_node(board(&conversion, "Home"));
    assert_eq!(json_of(node(root, "Mixed"))["content"], "Hello world");
    assert!(has_node(root, "Broken") && has_node(root, "Fine"));
}

#[test]
fn a_problem_deep_in_a_board_names_the_top_level_board() {
    let conversion = convert(
        "report-deep",
        vec![
            shape(1, "frame", "Outer board", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "frame", "Level one", [0.0, 0.0, 200.0, 200.0]).kids(vec![
                    shape(3, "group", "Level two", [0.0, 0.0, 100.0, 100.0]).kids(vec![shape(
                        4,
                        "bool",
                        "Deep union",
                        [0.0, 0.0, 10.0, 10.0],
                    )]),
                ]),
            ]),
            shape(5, "frame", "Other board", [500.0, 0.0, 100.0, 100.0]),
        ],
    );
    assert_reported(
        &conversion,
        &uid(4),
        &uid(1),
        "Outer board",
        "Deep union",
        &["bool"],
    );
    assert_eq!(conversion.report.entries.len(), 1);
}

/// The component fields of a component's root shape, as the real exports store
/// them: `componentId` names the component, `componentRoot` marks the root of a
/// copy or of the main instance, and `shapeRef` points a copy at the main.
fn component_root(component: u32, shape_ref: Option<u32>) -> serde_json::Value {
    let mut fields = json!({
        "componentId": uid(component),
        "componentFile": FILE_ID,
        "componentRoot": true
    });
    match shape_ref {
        Some(main) => fields["shapeRef"] = json!(uid(main)),
        None => fields["mainInstance"] = json!(true),
    }
    fields
}

#[test]
fn component_roots_are_listed_in_the_report_and_still_converted() {
    let conversion = convert(
        "report-components",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "frame", "Button copy", [0.0, 0.0, 120.0, 40.0])
                    .with(component_root(500, Some(4)))
                    .kids(vec![
                        text(3, "Button label", [10.0, 10.0, 100.0, 20.0], "Save")
                            .with(json!({ "shapeRef": uid(5) })),
                    ]),
                shape(4, "frame", "Button main", [0.0, 100.0, 120.0, 40.0])
                    .with(component_root(500, None))
                    .kids(vec![text(
                        5,
                        "Main label",
                        [10.0, 110.0, 100.0, 20.0],
                        "Save",
                    )]),
                shape(6, "rect", "Plain", [200.0, 0.0, 40.0, 40.0]),
            ]),
            shape(7, "frame", "Card", [500.0, 0.0, 200.0, 100.0])
                .with(component_root(501, Some(99))),
        ],
    );
    for (n, board_n, board_name, name, component) in [
        (2, 1, "Home", "Button copy", 500),
        (4, 1, "Home", "Button main", 500),
        (7, 7, "Card", "Card", 501),
    ] {
        assert_reported(
            &conversion,
            &uid(n),
            &uid(board_n),
            board_name,
            name,
            &["component"],
        );
        let entry = entries_of(&conversion, &uid(n))
            .into_iter()
            .find(|entry| entry.reason.to_lowercase().contains("component"))
            .expect("a component entry");
        assert!(
            entry.reason.contains(&uid(component)),
            "the reason {:?} names the component {}",
            entry.reason,
            uid(component)
        );
        assert!(
            entry.property.is_some(),
            "{entry:?}: the shape is still converted, only its component link is left out"
        );
    }
    // The component's shapes are converted as the plain shapes they are.
    let home = board_node(board(&conversion, "Home"));
    assert_eq!(node(home, "Button copy").kind(), NodeKind::Frame);
    assert_eq!(child_names(node(home, "Button copy")), ["Button label"]);
    assert_eq!(child_names(node(home, "Button main")), ["Main label"]);
    assert_eq!(board_node(board(&conversion, "Card")).name(), "Card");
    // Only the roots are listed: not the shapes inside them, nor plain shapes.
    for n in [3, 5, 6] {
        assert!(
            entries_of(&conversion, &uid(n)).is_empty(),
            "shape {n} is listed: {:#?}",
            conversion.report.entries
        );
    }
}

#[test]
fn converting_twice_gives_the_same_result() {
    let top = || {
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0])
                .with(flex("row"))
                .kids(vec![
                    shape(2, "rect", "A", [0.0, 0.0, 20.0, 20.0]),
                    shape(3, "bool", "B", [30.0, 0.0, 20.0, 20.0]),
                    text(4, "C", [60.0, 0.0, 40.0, 20.0], "C"),
                ]),
            shape(5, "frame", "Second", [500.0, 0.0, 100.0, 100.0]),
        ]
    };
    let first = convert("determinism", top());
    let again = convert_file(&write_export("determinism", top())).expect("converts");
    assert_eq!(first, again);
    assert_eq!(
        serde_json::to_string(&first.boards[0].design).unwrap(),
        serde_json::to_string(&again.boards[0].design).unwrap()
    );
}
