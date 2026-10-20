//! US-054: the converter maps Penpot designs the way Penpot means them (Q48,
//! tasks/penpot-analysis.md), on small exports that the tests build and on the
//! real exports in fixtures/penpot/ (those tests skip when the folder is empty).
//!
//! - A child that fills an axis of a flex layout gets `minWidth` or `minHeight`
//!   0 on that axis, unless Penpot gives it a min of its own, so that it can
//!   shrink below its content as in Penpot.
//! - Penpot's `stretch` does not stretch a child that hugs its content on the
//!   cross axis; filling children still stretch.
//! - `layoutItemMargin` (`m1` top, `m2` right, `m3` bottom, `m4` left) becomes
//!   the node's `margin`, and a grid's `layoutAlignItems` and
//!   `layoutJustifyItems`, or a cell's own `alignSelf` and `justifySelf` when
//!   they are not `auto` (ASSUMED: every cell seen says `auto`), become the
//!   children's `alignSelf` and `justifySelf`. Dashed strokes and stroke
//!   alignments become the stroke's `dash` and `align`. None of these is listed
//!   in the report any more.
//! - A text's line height is the line pitch Penpot's browser laid it out at: the
//!   gap between the y of its first two lines in `positionData`, or the stored
//!   box height for a text of one line, written as
//!   `lineHeight = (pitch + 1/128) / fontSize` without rounding. A text with no
//!   `positionData` keeps its own multiple (see convert_synthetic.rs).
//! - A text box otherwise follows Chrome, not Penpot's whole-pixel rounding: a
//!   text that hugs its content, in a layout or not, gets no min size from the
//!   box Penpot stored, so it is as wide and tall as its content lays out.
//! - Shapes that sit directly on a page, outside every board, go into a board
//!   called "Unplaced" on that page, at the places Penpot gives them.
//! - Prototype interactions (a shape's `interactions`), prototype flows (a
//!   page's `flows`, each naming its starting board in `startingFrame`) and
//!   constraints other than Penpot's default left and top (`constraintsH`,
//!   `constraintsV`) are listed in the report and not converted. An interaction
//!   or a constraint is an entry for its shape whose reason says "interaction"
//!   (or "prototype link") or "constraint"; a flow is an entry for its starting
//!   board whose reason names the flow and says "flow". Each such entry has a
//!   property, because the shape itself is still converted.

mod common;

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use omni_core::design::{Node, NodeKind};
use omni_penpot::{Conversion, ConvertedBoard, convert_file};
use omni_render::layout::{self, LayoutBox};
use omni_render::paint::Assets;
use serde_json::{Value, json};

use common::convert::*;
use common::{SyntheticPage, export_entries, fixture, shape_json, stored_zip, write_temp};

/// The boxes layout gives the nodes of `design`, by node name.
fn boxes(design: &Node) -> HashMap<String, LayoutBox> {
    let assets = Assets::new();
    let computed = layout::compute(design, assets.fonts());
    nodes(design)
        .into_iter()
        .filter_map(|node| {
            computed
                .get(node.id())
                .map(|found| (node.name().to_string(), *found))
        })
        .collect()
}

fn layout_box<'a>(boxes: &'a HashMap<String, LayoutBox>, name: &str) -> &'a LayoutBox {
    boxes
        .get(name)
        .unwrap_or_else(|| panic!("layout gives {name:?} no box: {boxes:?}"))
}

/// The entries of the report whose reason says one of `words`.
fn entries_saying<'a>(
    conversion: &'a Conversion,
    words: &[&str],
) -> Vec<&'a omni_penpot::ReportEntry> {
    conversion
        .report
        .entries
        .iter()
        .filter(|entry| {
            let reason = entry.reason.to_lowercase();
            words.iter().any(|word| reason.contains(word))
        })
        .collect()
}

/// The boards called "Unplaced".
fn unplaced(conversion: &Conversion) -> Vec<&ConvertedBoard> {
    conversion
        .boards
        .iter()
        .filter(|board| board.name == "Unplaced")
        .collect()
}

/// A flex frame numbered `n` with a fixed box and `extra` fields.
fn flex_frame(n: u32, name: &str, direction: &str, bounds: [f64; 4], extra: Value) -> S {
    shape(n, "frame", name, bounds)
        .with(flex(direction))
        .with(json!({ "layoutItemHSizing": "fix", "layoutItemVSizing": "fix" }))
        .with(extra)
}

#[test]
fn fill_children_get_a_min_size_of_0_on_their_fill_axes() {
    let conversion = convert(
        "fix-fill-min",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 600.0, 400.0]).kids(vec![
                // A row 100 px wide whose only child fills it, around content
                // 150 px wide: Penpot's "Kund", 124 wide in Penpot and 148 in
                // OmniState before.
                flex_frame(2, "Row", "row", [0.0, 0.0, 100.0, 40.0], json!({})).kids(vec![
                    flex_frame(
                        3,
                        "Kund",
                        "row",
                        [0.0, 0.0, 100.0, 40.0],
                        json!({ "layoutItemHSizing": "fill" }),
                    )
                    .kids(vec![shape(
                        4,
                        "rect",
                        "Wide content",
                        [0.0, 0.0, 150.0, 40.0],
                    )]),
                ]),
                flex_frame(5, "Column", "column", [200.0, 0.0, 60.0, 100.0], json!({})).kids(vec![
                    flex_frame(
                        6,
                        "Tall",
                        "column",
                        [200.0, 0.0, 60.0, 100.0],
                        json!({ "layoutItemVSizing": "fill" }),
                    )
                    .kids(vec![shape(
                        7,
                        "rect",
                        "Tall content",
                        [200.0, 0.0, 60.0, 150.0],
                    )]),
                ]),
                flex_frame(
                    8,
                    "Bounded row",
                    "row",
                    [0.0, 200.0, 100.0, 40.0],
                    json!({}),
                )
                .kids(vec![
                    flex_frame(
                        9,
                        "Bounded",
                        "row",
                        [0.0, 200.0, 100.0, 40.0],
                        json!({ "layoutItemHSizing": "fill", "layoutItemMinW": 30 }),
                    )
                    .kids(vec![shape(
                        10,
                        "rect",
                        "Bounded content",
                        [0.0, 200.0, 150.0, 40.0],
                    )]),
                ]),
            ]),
        ],
    );
    let design = board(&conversion, "Home");
    let root = board_node(design);

    let kund = json_of(node(root, "Kund"));
    assert_sizing(&kund["width"], Sz::Fill, "Kund fills its width");
    assert_eq!(kund["minWidth"], json!(0), "a fill width gets minWidth 0");
    assert!(
        kund["minHeight"].is_null(),
        "a fixed height gets no min: {kund}"
    );
    let tall = json_of(node(root, "Tall"));
    assert_eq!(
        tall["minHeight"],
        json!(0),
        "a fill height gets minHeight 0"
    );
    assert!(
        tall["minWidth"].is_null(),
        "a fixed width gets no min: {tall}"
    );
    let bounded = json_of(node(root, "Bounded"));
    assert_near(
        num(&bounded["minWidth"]),
        30.0,
        0.001,
        "Penpot's own min of a fill child is kept",
    );

    let laid_out = boxes(design);
    assert_near(
        f64::from(layout_box(&laid_out, "Kund").width),
        100.0,
        0.01,
        "a fill child shrinks below its content to the row's width, as in Penpot",
    );
    assert_near(
        f64::from(layout_box(&laid_out, "Tall").height),
        100.0,
        0.01,
        "a fill child shrinks below its content to the column's height",
    );
}

#[test]
fn penpot_stretch_does_not_stretch_children_that_hug_their_content() {
    let conversion = convert(
        "fix-stretch",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 600.0, 400.0]).kids(vec![
                // Penpot's "Swatch" around "Meta": a column 300 px wide that
                // stretches its items. "Meta" hugs its content and stays 50 wide,
                // as in Penpot, where OmniState stretched it to the column.
                flex_frame(
                    2,
                    "Column",
                    "column",
                    [0.0, 0.0, 300.0, 200.0],
                    json!({ "layoutAlignItems": "stretch" }),
                )
                .kids(vec![
                    flex_frame(
                        3,
                        "Filling",
                        "row",
                        [0.0, 160.0, 300.0, 40.0],
                        json!({ "layoutItemHSizing": "fill" }),
                    ),
                    shape(4, "rect", "Fixed", [0.0, 100.0, 80.0, 40.0]),
                    flex_frame(
                        5,
                        "Meta",
                        "row",
                        [0.0, 0.0, 50.0, 40.0],
                        json!({ "layoutItemHSizing": "auto" }),
                    )
                    .kids(vec![shape(
                        6,
                        "rect",
                        "Meta content",
                        [0.0, 0.0, 50.0, 40.0],
                    )]),
                ]),
                // The same in a row, on the vertical axis.
                flex_frame(
                    7,
                    "Row",
                    "row",
                    [0.0, 250.0, 300.0, 100.0],
                    json!({ "layoutAlignItems": "stretch" }),
                )
                .kids(vec![
                    flex_frame(
                        8,
                        "Tall filler",
                        "row",
                        [100.0, 250.0, 40.0, 100.0],
                        json!({ "layoutItemVSizing": "fill" }),
                    ),
                    flex_frame(
                        9,
                        "Short hugger",
                        "row",
                        [0.0, 250.0, 40.0, 20.0],
                        json!({ "layoutItemVSizing": "auto" }),
                    )
                    .kids(vec![shape(
                        10,
                        "rect",
                        "Short content",
                        [0.0, 250.0, 40.0, 20.0],
                    )]),
                ]),
            ]),
        ],
    );
    let design = board(&conversion, "Home");
    let laid_out = boxes(design);
    let width = |name| f64::from(layout_box(&laid_out, name).width);
    let height = |name| f64::from(layout_box(&laid_out, name).height);
    assert_near(width("Meta"), 50.0, 0.01, "a hugging child keeps its width");
    assert_near(
        width("Filling"),
        300.0,
        0.01,
        "a filling child still stretches",
    );
    assert_near(width("Fixed"), 80.0, 0.01, "a fixed child keeps its width");
    assert_near(
        height("Short hugger"),
        20.0,
        0.01,
        "a hugging child keeps its height in a row",
    );
    assert_near(
        height("Tall filler"),
        100.0,
        0.01,
        "a filling child still stretches across the row",
    );
}

#[test]
fn item_margins_become_node_margins() {
    let conversion = convert(
        "fix-margins",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 600.0, 400.0]).kids(vec![
                // Penpot's "Händelse" with its dot "Punkt" (margin-top 5).
                flex_frame(2, "Händelse", "row", [0.0, 0.0, 300.0, 33.0], json!({})).kids(vec![
                    flex_frame(
                        3,
                        "Pushed",
                        "row",
                        [42.0, 0.0, 20.0, 20.0],
                        json!({
                            "layoutItemMargin": { "m2": 3, "m4": 26 },
                            "layoutItemMarginType": "multiple"
                        }),
                    ),
                    flex_frame(
                        4,
                        "Punkt",
                        "row",
                        [0.0, 5.0, 8.0, 8.0],
                        json!({
                            "layoutItemMargin": { "m1": 5 },
                            "layoutItemMarginType": "multiple"
                        }),
                    ),
                ]),
                flex_frame(5, "Bar", "row", [0.0, 100.0, 300.0, 40.0], json!({})).kids(vec![
                    flex_frame(
                        6,
                        "Rensa sökning",
                        "row",
                        [0.0, 96.0, 32.0, 32.0],
                        json!({
                            "layoutItemMargin": { "m1": -4, "m3": -4 },
                            "layoutItemMarginType": "simple"
                        }),
                    ),
                ]),
            ]),
        ],
    );
    let design = board(&conversion, "Home");
    let root = board_node(design);
    let side = |name: &str, key: &str| num_or_zero(&json_of(node(root, name))["margin"][key]);
    assert_near(
        side("Punkt", "top"),
        5.0,
        0.001,
        "Punkt: m1 is the top margin",
    );
    for key in ["right", "bottom", "left"] {
        assert_near(side("Punkt", key), 0.0, 0.001, &format!("Punkt: {key}"));
    }
    assert_near(
        side("Pushed", "right"),
        3.0,
        0.001,
        "m2 is the right margin",
    );
    assert_near(side("Pushed", "left"), 26.0, 0.001, "m4 is the left margin");
    assert_near(side("Rensa sökning", "top"), -4.0, 0.001, "a negative top");
    assert_near(
        side("Rensa sökning", "bottom"),
        -4.0,
        0.001,
        "m3 is the bottom margin",
    );

    let laid_out = boxes(design);
    assert_near(
        f64::from(layout_box(&laid_out, "Punkt").y),
        5.0,
        0.01,
        "Punkt sits 5 px down in its row that aligns to the start",
    );
    assert_near(
        f64::from(layout_box(&laid_out, "Pushed").x),
        8.0 + 26.0,
        0.01,
        "the left margin pushes Pushed along the row, after Punkt",
    );
    for (id, name) in [(3, "Pushed"), (4, "Punkt"), (6, "Rensa sökning")] {
        assert!(
            entries_of(&conversion, &uid(id)).is_empty(),
            "the margins of {name} are converted, not listed: {:#?}",
            conversion.report.entries
        );
    }
}

/// A grid frame numbered `n` of one cell per child, `columns` by one row, each
/// cell 40 by 40 px, that aligns its items by `align` and `justify`.
fn grid(n: u32, name: &str, cells: Value, columns: usize, align: &str, justify: &str) -> S {
    let width = 40.0 * columns as f64;
    shape(n, "frame", name, [0.0, 0.0, width, 40.0]).with(json!({
        "layout": "grid",
        "layoutGridDir": "row",
        "layoutGridColumns": vec![json!({ "type": "flex", "value": 1 }); columns],
        "layoutGridRows": [{ "type": "flex", "value": 1 }],
        "layoutGridCells": cells,
        "layoutItemHSizing": "fix",
        "layoutItemVSizing": "fix",
        "layoutGapType": "multiple",
        "layoutGap": { "rowGap": 0, "columnGap": 0 },
        "layoutPaddingType": "simple",
        "layoutPadding": { "p1": 0, "p2": 0, "p3": 0, "p4": 0 },
        "layoutAlignItems": align,
        "layoutJustifyItems": justify,
        "layoutJustifyContent": "stretch",
        "layoutAlignContent": "stretch"
    }))
}

fn cell(id: u32, column: u32, shape: u32, align: &str, justify: &str) -> Value {
    json!({
        "id": uid(id), "position": "manual", "column": column, "row": 1,
        "columnSpan": 1, "rowSpan": 1, "shapes": [uid(shape)],
        "alignSelf": align, "justifySelf": justify
    })
}

#[test]
fn grid_item_alignment_becomes_self_alignment() {
    let conversion = convert(
        "fix-grid-align",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 600.0, 400.0]).kids(vec![
                // Every grid of the real exports: items centered down and at
                // the start across.
                grid(
                    2,
                    "Line row",
                    json!({
                        uid(20): cell(20, 1, 3, "auto", "auto"),
                        uid(21): cell(21, 2, 4, "auto", "auto"),
                    }),
                    2,
                    "center",
                    "start",
                )
                .kids(vec![
                    shape(3, "rect", "Icon", [0.0, 8.0, 24.0, 24.0]),
                    flex_frame(
                        4,
                        "Label",
                        "row",
                        [40.0, 10.0, 20.0, 20.0],
                        json!({ "layoutItemHSizing": "auto", "layoutItemVSizing": "auto" }),
                    )
                    .kids(vec![shape(
                        5,
                        "rect",
                        "Label content",
                        [40.0, 10.0, 20.0, 20.0],
                    )]),
                ]),
                // A cell that aligns its own item overrides the grid's.
                grid(
                    6,
                    "Own cell",
                    json!({ uid(22): cell(22, 1, 7, "end", "center") }),
                    1,
                    "center",
                    "start",
                )
                .kids(vec![shape(7, "rect", "Corner", [8.0, 16.0, 24.0, 24.0])]),
            ]),
        ],
    );
    let design = board(&conversion, "Home");
    let root = board_node(design);
    for name in ["Icon", "Label"] {
        let json = json_of(node(root, name));
        assert_eq!(json["alignSelf"], "center", "{name}: {json}");
        assert!(
            json["justifySelf"].is_null() || json["justifySelf"] == "start",
            "{name} sits at the start across: {json}"
        );
    }
    let corner = json_of(node(root, "Corner"));
    assert_eq!(corner["alignSelf"], "end", "{corner}");
    assert_eq!(corner["justifySelf"], "center", "{corner}");

    let laid_out = boxes(design);
    let icon = layout_box(&laid_out, "Icon");
    assert_near(
        f64::from(icon.y),
        8.0,
        0.01,
        "Icon is centered down its cell",
    );
    assert_near(f64::from(icon.x), 0.0, 0.01, "Icon starts its cell across");
    let label = layout_box(&laid_out, "Label");
    assert_near(f64::from(label.y), 10.0, 0.01, "Label is centered down");
    assert_near(f64::from(label.x), 40.0, 0.01, "Label starts its cell");
    let corner = layout_box(&laid_out, "Corner");
    assert_near(f64::from(corner.x), 8.0, 0.01, "Corner is centered across");
    for (id, name) in [(2, "Line row"), (6, "Own cell")] {
        assert!(
            entries_of(&conversion, &uid(id)).is_empty(),
            "the item alignment of {name} is converted, not listed: {:#?}",
            conversion.report.entries
        );
    }
}

#[test]
fn dashed_strokes_and_stroke_alignments_are_converted() {
    let stroke = |style: &str, alignment: &str| {
        json!({ "strokes": [{
            "strokeColor": "#000000", "strokeOpacity": 1, "strokeWidth": 1,
            "strokeStyle": style, "strokeAlignment": alignment
        }] })
    };
    let conversion = convert(
        "fix-strokes",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "rect", "Dashed", [0.0, 0.0, 50.0, 50.0]).with(stroke("dashed", "inner")),
                shape(3, "rect", "Outside", [60.0, 0.0, 50.0, 50.0]).with(stroke("solid", "outer")),
                shape(4, "rect", "Centered", [120.0, 0.0, 50.0, 50.0])
                    .with(stroke("solid", "center")),
                shape(5, "path", "Dashed line", [0.0, 100.0, 40.0, 1.0])
                    .with(json!({
                        "x": null, "y": null, "width": null, "height": null,
                        "content": "M0,100.5L40,100.5"
                    }))
                    .with(stroke("dashed", "inner")),
            ]),
        ],
    );
    let root = board_node(board(&conversion, "Home"));
    let first_stroke = |name: &str| json_of(node(root, name))["strokes"][0].clone();
    for name in ["Dashed", "Dashed line"] {
        let dashed = first_stroke(name);
        assert!(
            num(&dashed["dash"]["length"]) > 0.0 && num(&dashed["dash"]["gap"]) > 0.0,
            "{name} is drawn in dashes: {dashed}"
        );
    }
    let dashed = first_stroke("Dashed");
    assert!(
        dashed["align"].is_null() || dashed["align"] == "inside",
        "an inner stroke lies inside: {dashed}"
    );
    assert_eq!(first_stroke("Outside")["align"], "outside");
    assert_eq!(first_stroke("Centered")["align"], "center");
    assert!(
        first_stroke("Outside")["dash"].is_null(),
        "a solid stroke has no dash"
    );
    assert!(
        conversion.report.entries.is_empty(),
        "dashes and alignments are converted, not listed: {:#?}",
        conversion.report.entries
    );
}

/// One line of a text's `positionData`: a run of `text` at `y`.
fn line(y: f64, text: &str, font_size: &str) -> Value {
    json!({
        "x": 10, "y": y, "width": 100, "height": 18.2, "text": text,
        "fontFamily": "Inter", "fontSize": font_size, "fontWeight": "400",
        "fontStyle": "normal", "lineHeight": "1.2", "letterSpacing": "0px",
        "textAlign": "left", "textDecoration": "none", "textTransform": "none",
        "direction": "ltr",
        "fills": [{ "fillColor": "#102030", "fillOpacity": 1 }]
    })
}

#[test]
fn line_heights_are_the_line_pitch_penpot_laid_the_text_out_at() {
    let style = |size: &str, height: &str| json!({ "fontSize": size, "lineHeight": height });
    let conversion = convert(
        "fix-line-height",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 600.0, 400.0]).kids(vec![
                // As in idag: the browser rounded every line of 14 x 1.45 =
                // 20.3 px to 20 px. The first line has two runs.
                shape(2, "text", "Whole pixels", [10.0, 801.0, 300.0, 40.0]).with(json!({
                    "growType": "auto-height",
                    "content": text_content(vec![
                        paragraph(vec![
                            run("One ", style("14", "1.45")),
                            run("more", style("14", "1.45")),
                        ]),
                        paragraph(vec![run("Two", style("14", "1.45"))]),
                    ]),
                    "positionData": [
                        line(819.9500122070312, "One ", "14px"),
                        line(819.9500122070312, "more", "14px"),
                        line(839.9500122070312, "Two", "14px"),
                    ]
                })),
                // As in ordrar: lines cut down to 1/64 px, 20.296875.
                shape(3, "text", "Sixty-fourths", [10.0, 100.0, 300.0, 41.0]).with(json!({
                    "growType": "auto-height",
                    "content": text_content(vec![
                        paragraph(vec![run("One", style("14", "1.45"))]),
                        paragraph(vec![run("Two", style("14", "1.45"))]),
                    ]),
                    "positionData": [
                        line(100.0, "One", "14px"),
                        line(120.296875, "Two", "14px"),
                    ]
                })),
                // One line of 16 x 1.2 = 19.2 px, in a box Penpot rounded up to
                // 20 px: the box height is the pitch.
                shape(4, "text", "One line", [10.0, 200.0, 15.0, 20.0]).with(json!({
                    "growType": "auto-width",
                    "content": text_content(vec![paragraph(vec![run("Hi", style("16", "1.2"))])]),
                    "positionData": [line(219.2, "Hi", "16px")]
                })),
            ]),
        ],
    );
    let design = board(&conversion, "Home");
    let root = board_node(design);
    let laid_out = boxes(design);
    for (name, pitch, size, lines) in [
        (
            "Whole pixels",
            839.9500122070312 - 819.9500122070312,
            14.0,
            2.0,
        ),
        ("Sixty-fourths", 20.296875, 14.0, 2.0),
        ("One line", 20.0, 16.0, 1.0),
    ] {
        let json = json_of(node(root, name));
        let expected = (pitch + 1.0 / 128.0) / size;
        let written = num(&json["lineHeight"]);
        assert!(
            (written - expected).abs() < 1e-9,
            "{name}: lineHeight is (pitch + 1/128) / fontSize = {expected}, written exactly, not {written}"
        );
        assert_near(
            f64::from(layout_box(&laid_out, name).height),
            pitch * lines,
            0.001,
            &format!("{name}: every line is as tall as Penpot's pitch"),
        );
    }
    // Text boxes follow Chrome: the width Penpot rounded up is not kept.
    let one_line = json_of(node(root, "One line"));
    assert_sizing(&one_line["width"], Sz::Hug, "an auto-width text hugs");
    assert_sizing(&one_line["height"], Sz::Hug, "an auto-width text hugs");
    assert!(
        conversion.report.entries.is_empty(),
        "{:#?}",
        conversion.report.entries
    );
}

#[test]
fn texts_that_hug_in_a_layout_follow_chrome_not_the_box_penpot_rounded_up() {
    let style = |size: &str, height: &str| json!({ "fontSize": size, "lineHeight": height });
    let hi = |n: u32, name: &str, bounds: [f64; 4]| {
        // "Hi" in Inter 16 is about 15 px wide in Chrome; Penpot stored a box
        // 40 px wide for it.
        shape(n, "text", name, bounds).with(json!({
            "growType": "auto-width",
            "layoutItemHSizing": "auto",
            "layoutItemVSizing": "auto",
            "content": text_content(vec![paragraph(vec![run("Hi", style("16", "1.2"))])]),
            "positionData": [line(bounds[1] + 19.2, "Hi", "16px")]
        }))
    };
    let conversion = convert(
        "fix-text-box-chrome",
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 600.0, 400.0]).kids(vec![
                // The same text outside any layout: the width Chrome measures.
                hi(2, "Loose Hi", [10.0, 300.0, 40.0, 20.0]),
                flex_frame(3, "Row", "row", [0.0, 0.0, 300.0, 40.0], json!({})).kids(vec![
                    // Penpot lists the children of a layout last first.
                    shape(5, "rect", "After", [40.0, 0.0, 20.0, 20.0]),
                    hi(4, "Hi", [0.0, 0.0, 40.0, 20.0]),
                ]),
                flex_frame(6, "Column", "column", [320.0, 0.0, 250.0, 200.0], json!({})).kids(
                    vec![
                        shape(8, "rect", "Below", [320.0, 41.0, 250.0, 10.0]),
                        // Two lines 20.296875 px apart, 40.59375 px in all, in a
                        // box Penpot rounded up to 41 px.
                        shape(7, "text", "Two lines", [320.0, 0.0, 250.0, 41.0]).with(json!({
                            "growType": "auto-height",
                            "layoutItemHSizing": "fix",
                            "layoutItemVSizing": "auto",
                            "content": text_content(vec![
                                paragraph(vec![run("One", style("14", "1.45"))]),
                                paragraph(vec![run("Two", style("14", "1.45"))]),
                            ]),
                            "positionData": [
                                line(0.0, "One", "14px"),
                                line(20.296875, "Two", "14px"),
                            ]
                        })),
                    ],
                ),
            ]),
        ],
    );
    let design = board(&conversion, "Home");
    let root = board_node(design);
    for name in ["Hi", "Two lines"] {
        let json = json_of(node(root, name));
        for min in ["minWidth", "minHeight"] {
            assert!(
                json.get(min).is_none(),
                "{name}: a text that hugs in a layout keeps no {min} from the box Penpot \
                 rounded up, it follows its content as Chrome lays it out: {json:#}"
            );
        }
    }

    let laid_out = boxes(design);
    let measured = f64::from(layout_box(&laid_out, "Loose Hi").width);
    assert!(
        measured < 39.0,
        "outside a layout the text is as wide as its content, not Penpot's 40 px: {measured}"
    );
    assert_near(
        f64::from(layout_box(&laid_out, "Hi").width),
        measured,
        0.01,
        "a hugging text in a row is as wide as Chrome measures its content",
    );
    assert_near(
        f64::from(layout_box(&laid_out, "After").x),
        measured,
        0.01,
        "the next child follows the measured text, not Penpot's whole-pixel box",
    );
    assert_near(
        f64::from(layout_box(&laid_out, "Two lines").height),
        2.0 * 20.296875,
        0.001,
        "a hugging text in a column is two line pitches tall, not Penpot's 41 px",
    );
    assert_near(
        f64::from(layout_box(&laid_out, "Below").y),
        2.0 * 20.296875,
        0.001,
        "the next child follows the text's lines, not Penpot's whole-pixel box",
    );
}

#[test]
fn shapes_outside_boards_go_into_a_board_called_unplaced() {
    let conversion = convert(
        "fix-unplaced",
        vec![
            text(1, "Loose label", [10.0, 20.0, 80.0, 24.0], "Loose"),
            shape(2, "rect", "Loose rect", [0.0, 40.0, 80.0, 20.0]),
            shape(3, "frame", "Home", [200.0, 0.0, 100.0, 100.0]),
            shape(4, "group", "Loose group", [300.0, 300.0, 50.0, 50.0]).kids(vec![shape(
                5,
                "rect",
                "Inside",
                [300.0, 300.0, 50.0, 50.0],
            )]),
        ],
    );
    assert_eq!(conversion.boards.len(), 2, "Home and Unplaced");
    let found = unplaced(&conversion);
    assert_eq!(found.len(), 1, "one Unplaced board for the page");
    let unplaced_board = found[0];
    assert_eq!(unplaced_board.page, PAGE_NAME);
    let root = board_node(&unplaced_board.design);
    assert_eq!(root.name(), "Unplaced");
    assert_eq!(
        child_names(root),
        ["Loose label", "Loose rect", "Loose group"],
        "the loose shapes, in their stacking order"
    );
    assert_eq!(node(root, "Loose label").kind(), NodeKind::Text);
    assert_eq!(node(root, "Loose rect").kind(), NodeKind::Frame);
    assert_eq!(child_names(node(root, "Loose group")), ["Inside"]);

    // Each sits where Penpot has it on the page.
    let board_json = json_of(root);
    let (board_x, board_y) = (num_or_zero(&board_json["x"]), num_or_zero(&board_json["y"]));
    for (name, x, y) in [
        ("Loose label", 10.0, 20.0),
        ("Loose rect", 0.0, 40.0),
        ("Loose group", 300.0, 300.0),
    ] {
        let json = json_of(node(root, name));
        assert_near(
            board_x + num_or_zero(&json["x"]),
            x,
            0.001,
            &format!("x of {name}"),
        );
        assert_near(
            board_y + num_or_zero(&json["y"]),
            y,
            0.001,
            &format!("y of {name}"),
        );
    }
    let laid_out = boxes(&unplaced_board.design);
    for (name, [x, y, width, height]) in [
        ("Loose rect", [0.0, 40.0, 80.0, 20.0]),
        ("Inside", [300.0, 300.0, 50.0, 50.0]),
    ] {
        let found = layout_box(&laid_out, name);
        assert_near(f64::from(found.x), x, 0.01, &format!("x of {name}"));
        assert_near(f64::from(found.y), y, 0.01, &format!("y of {name}"));
        assert_near(
            f64::from(found.width),
            width,
            0.01,
            &format!("width of {name}"),
        );
        assert_near(
            f64::from(found.height),
            height,
            0.01,
            &format!("height of {name}"),
        );
    }

    // They are converted, so the comparison sets them beside Penpot's boxes,
    // and none is listed as left out.
    let compared: BTreeSet<&str> = unplaced_board
        .shapes
        .iter()
        .map(|shape| shape.id.as_str())
        .collect();
    for n in [1, 2, 4, 5] {
        assert!(compared.contains(uid(n).as_str()), "shape {n} is compared");
        assert!(
            entries_of(&conversion, &uid(n))
                .iter()
                .all(|entry| entry.property.is_some()),
            "shape {n} is not left out: {:#?}",
            conversion.report.entries
        );
    }
}

#[test]
fn the_unplaced_board_is_on_the_page_of_its_shapes_and_only_there() {
    let first = page(vec![shape(
        1,
        "frame",
        "First board",
        [0.0, 0.0, 100.0, 100.0],
    )]);
    let mut loose = text(3, "Loose on two", [500.0, 10.0, 80.0, 24.0], "Loose").json;
    loose["parentId"] = json!(common::ROOT_ID);
    loose["frameId"] = json!(common::ROOT_ID);
    let second = SyntheticPage {
        id: "a0000000-0000-4000-8000-000000000002".to_string(),
        name: "Page 2".to_string(),
        index: 1,
        top_level: vec![uid(2), uid(3)],
        shapes: vec![
            shape_json(
                &uid(2),
                "frame",
                "Second board",
                common::ROOT_ID,
                [0.0, 0.0, 100.0, 100.0],
                &[],
            ),
            loose,
        ],
    };
    let entries = export_entries(FILE_ID, "Synthetic", &[first, second]);
    let path = write_temp("convert-fix-unplaced-pages.penpot", &stored_zip(&entries));
    let conversion = convert_file(&path).expect("the export converts");
    let found = unplaced(&conversion);
    assert_eq!(found.len(), 1, "only the page with a loose shape has one");
    assert_eq!(found[0].page, "Page 2");
    assert_eq!(child_names(board_node(&found[0].design)), ["Loose on two"]);
    for board in &conversion.boards {
        assert_validates(&board.design, &board.name);
    }
}

#[test]
fn interactions_flows_and_constraints_are_listed_and_not_converted() {
    let interaction = json!({ "interactions": [{
        "eventType": "click", "actionType": "navigate", "destination": uid(10),
        "positionRelativeTo": null, "preserveScroll": false,
        "animation": { "animationType": "dissolve", "duration": 160, "easing": "ease" }
    }] });
    let top_level = vec![
        shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
            shape(2, "rect", "Button", [0.0, 0.0, 50.0, 20.0]).with(interaction),
            shape(3, "rect", "Inert", [0.0, 30.0, 50.0, 20.0]).with(json!({ "interactions": [] })),
            shape(4, "rect", "Stretchy", [0.0, 60.0, 400.0, 20.0])
                .with(json!({ "constraintsH": "leftright", "constraintsV": "top" })),
            shape(5, "rect", "Scaled", [0.0, 90.0, 50.0, 20.0])
                .with(json!({ "constraintsH": "scale", "constraintsV": "scale" })),
            shape(6, "rect", "Footer", [0.0, 280.0, 50.0, 20.0])
                .with(json!({ "constraintsH": "left", "constraintsV": "bottom" })),
            shape(7, "rect", "Pinned", [0.0, 120.0, 50.0, 20.0])
                .with(json!({ "constraintsH": "left", "constraintsV": "top" })),
        ]),
        shape(10, "frame", "Details", [500.0, 0.0, 400.0, 300.0]),
    ];
    let mut entries = export_entries(FILE_ID, "Synthetic", &[page(top_level)]);
    let page_entry = format!("files/{FILE_ID}/pages/{PAGE_ID}.json");
    let page_json = entries
        .iter_mut()
        .find(|(name, _)| *name == page_entry)
        .expect("the page entry");
    page_json.1 = json!({
        "id": PAGE_ID, "name": PAGE_NAME, "index": 0,
        "flows": {
            uid(30): { "id": uid(30), "name": "Checkout flow", "startingFrame": uid(1) }
        }
    })
    .to_string()
    .into_bytes();
    let path = write_temp("convert-fix-prototype.penpot", &stored_zip(&entries));
    let conversion = convert_file(&path).expect("the export converts");

    assert_reported(
        &conversion,
        &uid(2),
        &uid(1),
        "Home",
        "Button",
        &["interaction", "prototype link"],
    );
    for (n, name) in [(4, "Stretchy"), (5, "Scaled"), (6, "Footer")] {
        assert_reported(&conversion, &uid(n), &uid(1), "Home", name, &["constraint"]);
    }
    for (n, name) in [(3, "Inert"), (7, "Pinned")] {
        assert!(
            entries_of(&conversion, &uid(n)).is_empty(),
            "{name} has no interaction and the default constraints: {:#?}",
            conversion.report.entries
        );
    }
    let flows: Vec<_> = conversion
        .report
        .entries
        .iter()
        .filter(|entry| {
            entry.reason.contains("Checkout flow") && entry.reason.to_lowercase().contains("flow")
        })
        .collect();
    assert_eq!(flows.len(), 1, "{:#?}", conversion.report.entries);
    assert_eq!(flows[0].shape_id, uid(1), "a flow names its starting board");
    for entry in &conversion.report.entries {
        assert!(
            entry.property.is_some(),
            "the shapes are still converted: {entry:?}"
        );
    }
    let root = board_node(board(&conversion, "Home"));
    for name in ["Button", "Stretchy", "Scaled", "Footer"] {
        assert!(has_node(root, name), "{name} is converted");
    }
}

// The real exports.

fn idag() -> Option<&'static Conversion> {
    static CELL: OnceLock<Option<Conversion>> = OnceLock::new();
    CELL.get_or_init(|| {
        fixture("tirebud-idag.penpot")
            .map(|path| convert_file(&path).expect("tirebud-idag converts"))
    })
    .as_ref()
}

fn ordrar() -> Option<&'static Conversion> {
    static CELL: OnceLock<Option<Conversion>> = OnceLock::new();
    CELL.get_or_init(|| {
        fixture("tirebud-ordrar.penpot")
            .map(|path| convert_file(&path).expect("tirebud-ordrar converts"))
    })
    .as_ref()
}

/// The distinct shapes with a report entry whose reason says one of `words`.
fn shapes_saying(conversion: &Conversion, words: &[&str]) -> BTreeSet<String> {
    entries_saying(conversion, words)
        .into_iter()
        .map(|entry| entry.shape_id.clone())
        .collect()
}

/// The page files of the export at `path`, as JSON.
fn raw_pages(path: &std::path::Path) -> Vec<Value> {
    use std::io::Read;

    let file = std::fs::File::open(path).expect("the export opens");
    let mut archive = zip::ZipArchive::new(file).expect("the export is a zip");
    let mut pages = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("an archive entry");
        let name = entry.name().expect("an entry name").to_string();
        if !(name.contains("/pages/") && name.matches('/').count() == 3) {
            continue;
        }
        let mut text = String::new();
        entry.read_to_string(&mut text).expect("a page is text");
        pages.push(serde_json::from_str(&text).expect("a page is JSON"));
    }
    pages
}

/// Checks that every flow of the export at `path` is listed for its starting
/// board, by name.
fn assert_flows_listed(conversion: &Conversion, path: &std::path::Path, count: usize) {
    let mut flows = 0;
    for page in raw_pages(path) {
        let Some(found) = page["flows"].as_object() else {
            continue;
        };
        for flow in found.values() {
            flows += 1;
            let name = flow["name"].as_str().expect("a flow has a name");
            let start = flow["startingFrame"].as_str().expect("a starting board");
            assert!(
                conversion.report.entries.iter().any(|entry| {
                    entry.shape_id == start
                        && entry.reason.contains(name)
                        && entry.reason.to_lowercase().contains("flow")
                }),
                "the flow {name:?} is not listed for its starting board {start}"
            );
        }
    }
    assert_eq!(flows, count, "the flows of {path:?}");
}

#[test]
fn idag_lists_its_7_interactions_2_flows_and_55_constraints() {
    let (Some(conversion), Some(path)) = (idag(), fixture("tirebud-idag.penpot")) else {
        return;
    };
    // Counted from the export's JSON: shapes with a non-empty `interactions`,
    // and shapes whose `constraintsH` is not left or `constraintsV` not top.
    assert_eq!(
        shapes_saying(conversion, &["interaction", "prototype link"]).len(),
        7
    );
    assert_eq!(shapes_saying(conversion, &["constraint"]).len(), 55);
    assert_flows_listed(conversion, &path, 2);
}

#[test]
fn ordrar_lists_its_2269_interactions_189_flows_and_991_constraints() {
    let (Some(conversion), Some(path)) = (ordrar(), fixture("tirebud-ordrar.penpot")) else {
        return;
    };
    assert_eq!(
        shapes_saying(conversion, &["interaction", "prototype link"]).len(),
        2_269
    );
    assert_eq!(shapes_saying(conversion, &["constraint"]).len(), 991);
    assert_flows_listed(conversion, &path, 189);
}

#[test]
fn idag_puts_its_8_loose_texts_into_an_unplaced_board() {
    let Some(conversion) = idag() else { return };
    let found = unplaced(conversion);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].page, "Idag");
    let root = board_node(&found[0].design);
    assert_eq!(root.children().len(), 8);
    assert!(
        root.children()
            .iter()
            .all(|child| child.kind() == NodeKind::Text)
    );
}

#[test]
fn ordrar_puts_its_232_loose_texts_into_two_unplaced_boards() {
    let Some(conversion) = ordrar() else { return };
    let found = unplaced(conversion);
    let per_page: Vec<(&str, usize)> = found
        .iter()
        .map(|board| {
            (
                board.page.as_str(),
                board_node(&board.design).children().len(),
            )
        })
        .collect();
    assert_eq!(per_page, [("Sidmallar", 1), ("Ordrar", 231)]);
}

/// Every node of every design of `conversion`.
fn all_nodes(conversion: &Conversion) -> Vec<Value> {
    conversion
        .boards
        .iter()
        .flat_map(|board| nodes(&board.design).into_iter().map(json_of))
        .collect()
}

#[test]
fn ordrar_keeps_the_margins_of_its_219_items_and_centers_its_170_grid_items() {
    let Some(conversion) = ordrar() else { return };
    let all = all_nodes(conversion);
    let with_margin: Vec<&Value> = all
        .iter()
        .filter(|node| {
            ["top", "right", "bottom", "left"]
                .iter()
                .any(|side| num_or_zero(&node["margin"][side]) != 0.0)
        })
        .collect();
    assert_eq!(with_margin.len(), 219, "the items with a margin");
    let dots: Vec<&Value> = all.iter().filter(|node| node["name"] == "Punkt").collect();
    assert_eq!(dots.len(), 189);
    for dot in dots {
        assert_near(num(&dot["margin"]["top"]), 5.0, 0.001, "Punkt's top margin");
    }
    let centered = all
        .iter()
        .filter(|node| node["alignSelf"] == "center")
        .count();
    assert_eq!(centered, 170, "the items of the 40 grids, centered down");
    for entry in &conversion.report.entries {
        let property = entry.property.as_deref().unwrap_or("");
        assert!(
            !matches!(
                property,
                "layoutItemMargin" | "layoutAlignItems" | "strokeStyle" | "strokeAlignment"
            ),
            "{entry:?} is converted now"
        );
    }
}

/// The text shapes of the export at `path` whose `positionData` has two lines
/// or more: id, the y of the first two lines, and the font size of the first
/// run.
fn multi_line_texts(path: &std::path::Path) -> Vec<(String, f64, f64, f64)> {
    use std::io::Read;

    let file = std::fs::File::open(path).expect("the export opens");
    let mut archive = zip::ZipArchive::new(file).expect("the export is a zip");
    let mut texts = Vec::new();
    let mut text = String::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("an archive entry");
        let name = entry.name().expect("an entry name").to_string();
        if !(name.contains("/pages/") && name.matches('/').count() == 4) {
            continue;
        }
        text.clear();
        entry.read_to_string(&mut text).expect("a shape is text");
        if !text.contains("\"positionData\"") {
            continue;
        }
        let shape: Value = serde_json::from_str(&text).expect("a shape is JSON");
        let mut ys: Vec<f64> = Vec::new();
        for item in shape["positionData"].as_array().into_iter().flatten() {
            let y = item["y"].as_f64().expect("a line has a y");
            if !ys.contains(&y) {
                ys.push(y);
            }
        }
        if ys.len() < 2 {
            continue;
        }
        let mut size = None;
        let mut stack = vec![&shape["content"]];
        while let Some(at) = stack.pop() {
            if let Some(text) = at["fontSize"].as_str() {
                if at.get("text").is_some() {
                    size = text.parse::<f64>().ok();
                    break;
                }
            }
            if let Some(children) = at["children"].as_array() {
                stack.extend(children.iter().rev());
            }
        }
        let id = shape["id"].as_str().expect("an id").to_string();
        texts.push((id, ys[0], ys[1], size.expect("a run has a font size")));
    }
    texts
}

#[test]
fn idag_multi_line_texts_take_the_line_pitch_of_their_position_data() {
    let (Some(conversion), Some(path)) = (idag(), fixture("tirebud-idag.penpot")) else {
        return;
    };
    // Shape id to the node it became.
    let mut node_of: HashMap<&str, Value> = HashMap::new();
    for board in &conversion.boards {
        let by_id: HashMap<String, &Node> = nodes(&board.design)
            .into_iter()
            .map(|node| (node.id().as_str().to_string(), node))
            .collect();
        for shape in &board.shapes {
            if let Some(node) = by_id.get(&shape.node_id) {
                node_of.insert(shape.id.as_str(), json_of(node));
            }
        }
    }
    let texts = multi_line_texts(&path);
    assert!(
        texts.len() >= 10,
        "idag has 10 multi-line texts: {}",
        texts.len()
    );
    for (id, first, second, size) in texts {
        let node = node_of
            .get(id.as_str())
            .unwrap_or_else(|| panic!("the text {id} is converted"));
        let expected = (second - first + 1.0 / 128.0) / size;
        assert_near(
            num(&node["lineHeight"]),
            expected,
            1e-9,
            &format!("the line height of {} ({id})", node["name"]),
        );
    }
}
