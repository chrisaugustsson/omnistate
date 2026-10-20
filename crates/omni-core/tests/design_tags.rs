//! US-027: the tag format for designs (Q26, FR-33). Agents read and write
//! designs as JSX-like tags, one tag per node with the node's properties as
//! attributes, and `omni_core::design::tags` converts between the tags and
//! the JSON node model without loss.
//!
//! The tests pin what the PRD states: the tag names (one per node type), the
//! example `<Frame id="k3f9x2qa" name="Card" dir="col" gap="$space.md"
//! pad={16} fill="$color.surface">` and what each of its attributes means,
//! and `x` for a node's position. How every other property is written is
//! the format's choice, which `docs/formats/design-tags.md` documents and the
//! round-trip tests hold to: printing a node and parsing it back gives the
//! same node.

mod common;

use common::{first_difference, frame, parse as from_json, to_json, with};
use omni_core::design::tags::{self, TagError, TagNode};
use omni_core::design::{Node, NodeId, NodeKind};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The example from the PRD, closed.
const PRD_EXAMPLE: &str = r#"<Frame id="k3f9x2qa" name="Card" dir="col" gap="$space.md" pad={16} fill="$color.surface">
</Frame>"#;

/// The node types and their tags.
const TAGS: [(&str, NodeKind); 7] = [
    ("Page", NodeKind::Page),
    ("Board", NodeKind::Board),
    ("Frame", NodeKind::Frame),
    ("Text", NodeKind::Text),
    ("Image", NodeKind::Image),
    ("Vector", NodeKind::Vector),
    ("Instance", NodeKind::Instance),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn id(text: &str) -> NodeId {
    text.parse().expect("a valid node ID")
}

/// Parses `source`, which must be valid tags.
fn parse_tags(source: &str) -> TagNode {
    tags::parse(source)
        .unwrap_or_else(|error| panic!("these tags must parse, but: {error}\n{source}"))
}

/// The node `source` describes, when every tag in it carries an `id`.
fn existing(source: &str) -> Node {
    parse_tags(source).assign_ids(|| panic!("every tag has an `id`, so no node is new"))
}

/// The node `source` describes, with IDs from `fresh` for its new nodes.
fn with_fresh_ids(source: &str, fresh: &[&str]) -> Node {
    let mut fresh = fresh.iter();
    parse_tags(source).assign_ids(|| {
        id(fresh
            .next()
            .expect("the server is asked for an ID once per new node"))
    })
}

/// Asserts that printing the node `json` describes and parsing the tags back
/// gives the same node, and that printing it again gives the same tags.
/// Returns the tags.
fn assert_tags_round_trip(json: &Value) -> String {
    let node = from_json(json);
    let printed = tags::print(&node);
    let tree = tags::parse(&printed).unwrap_or_else(|error| {
        panic!(
            "printed tags must parse again, but: {error}\n{}",
            excerpt(&printed, &error)
        )
    });
    let back =
        tree.assign_ids(|| panic!("printed tags carry the ID of every node, so no node is new"));
    let (expected, actual) = (to_json(&node), to_json(&back));
    if let Some(difference) = first_difference(&expected, &actual, "")
        .or_else(|| first_difference(&actual, &expected, ""))
    {
        panic!("printing and parsing the tags lost or changed a property: {difference}");
    }
    assert!(back == node, "the tags must parse back to the same node");
    assert!(
        tags::print(&back) == printed,
        "printing the parsed node again must give the same tags"
    );
    printed
}

/// The lines around `error` in `source`.
fn excerpt(source: &str, error: &TagError) -> String {
    let first = error.line().saturating_sub(3);
    source
        .lines()
        .enumerate()
        .skip(first)
        .take(5)
        .map(|(index, line)| format!("{:>6} | {line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The line and column, counting from 1 and in characters, of the start of
/// the `nth` (from 0) occurrence of `needle` in `source`.
fn position(source: &str, needle: &str, nth: usize) -> (usize, usize) {
    let (offset, _) = source
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("`{needle}` occurs fewer than {} times", nth + 1));
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    (line, source[line_start..offset].chars().count() + 1)
}

/// Asserts that `source` is rejected with an error at the `nth` occurrence of
/// `at`, whose message names each of `names` and whose fix mentions each of
/// `fixes`. Returns the error.
fn assert_error(
    source: &str,
    (at, nth): (&str, usize),
    names: &[&str],
    fixes: &[&str],
) -> TagError {
    let error = match tags::parse(source) {
        Ok(tree) => panic!("these tags must be rejected, but they parsed as {tree:?}:\n{source}"),
        Err(error) => error,
    };
    let (line, column) = position(source, at, nth);
    assert_eq!(
        (error.line(), error.column()),
        (line, column),
        "the error must point at `{at}` (line {line}, column {column}); it says: {error}\n{source}"
    );
    for name in names {
        assert!(
            error.message().contains(name),
            "the message must name `{name}`; it says: {}",
            error.message()
        );
    }
    assert!(
        !error.fix().trim().is_empty(),
        "the error must say how to fix it; it says: {error}"
    );
    for fix in fixes {
        assert!(
            error.fix().contains(fix),
            "the fix must mention `{fix}`; it says: {}",
            error.fix()
        );
    }
    let shown = error.to_string();
    for part in [
        format!("line {line}"),
        format!("column {column}"),
        error.message().to_string(),
        error.fix().to_string(),
    ] {
        assert!(
            shown.contains(&part),
            "the error as shown must include `{part}`; it shows: {shown}"
        );
    }
    error
}

/// A page with two boards that uses every node type, every layout mode and
/// every property, with literals and token references.
fn complete_design() -> Value {
    json!({
        "id": "page0001",
        "type": "page",
        "name": "Checkout — kassa 🛒",
        "children": [desktop_board(), mobile_board()]
    })
}

fn desktop_board() -> Value {
    json!({
        "id": "brd00001",
        "type": "board",
        "name": "Cart / desktop",
        "x": 40,
        "y": -120.5,
        "width": {"fixed": 1440},
        "height": "hug",
        "minHeight": 900,
        "layout": {
            "mode": "flex",
            "direction": "column",
            "gap": {"row": 32, "column": "$space.lg"},
            "padding": {"top": 48, "right": 64, "bottom": 56, "left": "$space.xl"},
            "alignItems": "center",
            "justifyContent": "space-between"
        },
        "fills": [
            {"type": "solid", "color": "#f8fafc"},
            {"type": "linear-gradient", "angle": 180, "stops": [
                {"position": 0, "color": "#ffffff00"},
                {"position": 0.35, "color": "#e0f2fe"},
                {"position": 1, "color": "$color.backdrop"}
            ]}
        ],
        "strokes": [
            {"color": "#cbd5e1", "width": 1, "align": "outside", "dash": {"length": 6, "gap": "$space.xs"}},
            {"color": "$color.focus", "width": 2, "align": "center"}
        ],
        "radius": {"topLeft": 24, "topRight": 24, "bottomRight": 8, "bottomLeft": 8},
        "clip": true,
        "children": [header(), product_grid()]
    })
}

fn header() -> Value {
    json!({
        "id": "frame001",
        "type": "frame",
        "name": "Header",
        "margin": {"top": -8, "right": "$space.sm", "bottom": 12, "left": 0},
        "width": "fill",
        "height": {"fixed": 72},
        "maxWidth": 1200,
        "layout": {
            "mode": "flex",
            "direction": "row-reverse",
            "wrap": "wrap",
            "gap": {"row": 8, "column": 16},
            "alignItems": "baseline",
            "justifyContent": "space-evenly"
        },
        "strokes": [{"color": "#e2e8f0", "width": 1, "align": "inside"}],
        "shadows": [
            {"type": "drop", "color": "#0f172a33", "offsetX": 2, "offsetY": 8, "blur": 24, "spread": -4},
            "$shadow.raised"
        ],
        "children": [title(), account_button(), notification_dot()]
    })
}

fn title() -> Value {
    json!({
        "id": "text0001",
        "type": "text",
        "name": "Title",
        "margin": {"left": 4},
        "width": "hug",
        "height": "hug",
        "content": "Din varukorg — 3 artiklar",
        "fontFamily": "IBM Plex Sans",
        "fontSize": 28,
        "fontWeight": 650,
        "fontStyle": "italic",
        "lineHeight": 1.25,
        "letterSpacing": -0.4,
        "textAlign": "right",
        "textDecoration": "underline",
        "color": "$color.text.strong"
    })
}

fn account_button() -> Value {
    json!({
        "id": "btn00001",
        "type": "instance",
        "name": "Account button",
        "component": "cmp00001",
        "width": "fill",
        "minWidth": 44,
        "maxWidth": 160
    })
}

fn notification_dot() -> Value {
    json!({
        "id": "vec00001",
        "type": "vector",
        "name": "Notification dot",
        "absolute": true,
        "x": 1180,
        "y": -6,
        "width": {"fixed": 12},
        "height": {"fixed": 12},
        "path": "M6 0a6 6 0 1 0 0.001 0Z",
        "fills": [{"type": "solid", "color": "#ef4444"}],
        "strokes": [{"color": "#ffffff", "width": 2, "dash": {"length": 1, "gap": 1.5}}],
        "shadows": [{"type": "drop", "color": "#00000040", "offsetX": 0, "offsetY": 1}],
        "opacity": 0.95
    })
}

fn product_grid() -> Value {
    json!({
        "id": "frame002",
        "type": "frame",
        "name": "Products",
        "width": "fill",
        "height": "hug",
        "minWidth": 640,
        "layout": {
            "mode": "grid",
            "columns": [{"fixed": 240}, {"fr": 1}, {"fr": 2.5}, "auto"],
            "rows": ["auto", {"fixed": "$size.row"}],
            "gap": {"row": 24, "column": 16},
            "padding": {"top": 8, "right": 12, "bottom": 16, "left": 20}
        },
        "children": [hero_photo(), price_card(), caption(), badge()]
    })
}

fn hero_photo() -> Value {
    json!({
        "id": "jpeg0001",
        "type": "image",
        "name": "Hero product",
        "gridColumn": {"start": 1, "span": 2},
        "gridRow": {"start": -1},
        "justifySelf": "stretch",
        "alignSelf": "start",
        "width": "fill",
        "height": "fill",
        "image": "attachment:0192f1c4-5b6a-7e11-8a22-b33cc44dd55e",
        "fit": "cover",
        "radius": {"topLeft": 16, "topRight": 16, "bottomRight": 16, "bottomLeft": 16},
        "strokes": [{"color": "#cbd5e1", "width": 0.5, "dash": {"length": "$dash.long", "gap": 2}}],
        "shadows": [
            {"type": "inner", "color": "#00000014", "offsetX": 1, "offsetY": 1, "blur": 2, "spread": 1}
        ],
        "opacity": 0.9
    })
}

fn price_card() -> Value {
    json!({
        "id": "frame003",
        "type": "frame",
        "name": "Price card",
        "visible": false,
        "gridColumn": {"span": 2},
        "justifySelf": "center",
        "alignSelf": "end",
        "width": {"fixed": "$size.card"},
        "height": {"fixed": 200},
        "layout": {"mode": "none"},
        "fills": [
            {"type": "image", "image": "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd", "fit": "contain"}
        ],
        "opacity": "$opacity.muted",
        "clip": false,
        "children": [price(), buy_button()]
    })
}

fn price() -> Value {
    json!({
        "id": "text0002",
        "type": "text",
        "name": "Price",
        "x": 16,
        "y": 24,
        "width": {"fixed": 200},
        "height": {"fixed": 40},
        "content": "1 299 kr\n<b>inkl. moms</b> & frakt",
        "fontFamily": "$font.family.mono",
        "fontSize": "$font.size.lg",
        "fontWeight": "$font.weight.bold",
        "fontStyle": "normal",
        "lineHeight": "$font.line-height.tight",
        "letterSpacing": "$font.tracking.wide",
        "textAlign": "justify",
        "textDecoration": "line-through",
        "color": "#16a34a",
        "opacity": 0.75
    })
}

fn buy_button() -> Value {
    json!({
        "id": "btn00002",
        "type": "instance",
        "name": "Buy button",
        "visible": false,
        "component": "cmp00002",
        "x": 16,
        "y": 140,
        "height": {"fixed": 48},
        "minHeight": "$size.touch"
    })
}

fn caption() -> Value {
    json!({
        "id": "text0003",
        "type": "text",
        "name": "Caption",
        "margin": {"top": "$space.sm", "bottom": -2},
        "alignSelf": "center",
        "width": "fill",
        "height": "hug",
        "maxHeight": 64,
        "content": "Fri frakt över 499 kr ✓",
        "fontFamily": "Inter",
        "fontSize": 14,
        "textAlign": "left",
        "textDecoration": "none"
    })
}

fn badge() -> Value {
    json!({
        "id": "frame005",
        "type": "frame",
        "name": "Badge",
        "absolute": true,
        "x": 8,
        "y": 8,
        "width": "hug",
        "height": "hug",
        "layout": {"mode": "flex"},
        "fills": [{"type": "solid", "color": "$color.accent"}],
        "radius": {"topLeft": "$radius.pill", "bottomRight": 0},
        "children": []
    })
}

fn mobile_board() -> Value {
    json!({
        "id": "brd00002",
        "type": "board",
        "name": "Cart / mobile (hidden)",
        "visible": false,
        "x": 1600,
        "y": 40,
        "width": {"fixed": "$size.mobile"},
        "height": {"fixed": 844},
        "maxHeight": 2400,
        "layout": {"mode": "none"},
        "fills": [{"type": "solid", "color": "$color.surface"}],
        "strokes": [{"color": "$color.border", "width": "$border.thin"}],
        "radius": {
            "topLeft": "$radius.device",
            "topRight": "$radius.device",
            "bottomRight": "$radius.device",
            "bottomLeft": "$radius.device"
        },
        "shadows": [
            {"type": "drop", "color": "$color.shadow", "offsetX": 4, "offsetY": 12, "blur": 32, "spread": 2}
        ],
        "opacity": 0.98,
        "children": [sheet()]
    })
}

fn sheet() -> Value {
    json!({
        "id": "frame004",
        "type": "frame",
        "name": "Sheet",
        "x": 12,
        "y": 480,
        "margin": {"top": 1, "right": 2, "bottom": 3, "left": 4},
        "width": {"fixed": 366},
        "height": "hug",
        "minWidth": "$size.sheet.min",
        "maxWidth": "$size.sheet.max",
        "minHeight": 120,
        "maxHeight": "$size.sheet.tall",
        "layout": {
            "mode": "flex",
            "direction": "column-reverse",
            "wrap": "wrap-reverse",
            "gap": {"row": "$space.sm"},
            "padding": {"top": "$space.md", "left": 20},
            "alignItems": "end",
            "justifyContent": "space-around"
        },
        "clip": true,
        "children": [grabber(), chevron(), thumbnail()]
    })
}

fn grabber() -> Value {
    json!({
        "id": "text0004",
        "type": "text",
        "name": "Grabber",
        "absolute": true,
        "x": 167,
        "y": -12,
        "width": {"fixed": 32},
        "height": {"fixed": 4},
        "content": "—",
        "fontFamily": "Inter",
        "fontSize": 12,
        "fontWeight": 300,
        "textAlign": "center",
        "color": "#94a3b8cc"
    })
}

fn chevron() -> Value {
    json!({
        "id": "vec00002",
        "type": "vector",
        "name": "Chevron",
        "visible": true,
        "width": {"fixed": 16},
        "height": {"fixed": 16},
        "path": "M4 6l4 4 4-4",
        "strokes": [{"color": "$color.icon", "width": "$border.icon"}]
    })
}

fn thumbnail() -> Value {
    json!({
        "id": "jpeg0002",
        "type": "image",
        "name": "Thumbnail",
        "width": "fill",
        "height": {"fixed": "$size.thumb"},
        "maxHeight": 96,
        "image": "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd",
        "fit": "contain"
    })
}

/// The number of nodes in `json`.
fn count(json: &Value) -> usize {
    1 + json["children"]
        .as_array()
        .map_or(0, |children| children.iter().map(count).sum())
}

// One tag is one node, attributes are its properties, and `id` refers to an
// existing node.

#[test]
fn the_prd_example_is_a_frame_whose_attributes_are_its_properties() {
    let node = existing(PRD_EXAMPLE);
    let json = to_json(&node);
    assert_eq!(json["id"], "k3f9x2qa", "`id` is the node's ID\n{json:#}");
    assert_eq!(json["type"], "frame", "`<Frame>` is a frame\n{json:#}");
    assert_eq!(json["name"], "Card", "`name` is the node's name\n{json:#}");
    assert_eq!(
        json["layout"]["mode"], "flex",
        "`dir` lays the children out in a flexbox\n{json:#}"
    );
    assert_eq!(
        json["layout"]["direction"], "column",
        "`dir=\"col\"` is the column direction\n{json:#}"
    );
    assert_eq!(
        json["layout"]["gap"],
        json!({"row": "$space.md", "column": "$space.md"}),
        "one `gap` is the gap between rows and between columns, as CSS `gap`\n{json:#}"
    );
    assert_eq!(
        json["layout"]["padding"],
        json!({"top": 16, "right": 16, "bottom": 16, "left": 16}),
        "one `pad` is the padding of every side, as CSS `padding`\n{json:#}"
    );
    assert_eq!(
        json["fills"],
        json!([{"type": "solid", "color": "$color.surface"}]),
        "`fill` is one solid fill\n{json:#}"
    );
    assert_eq!(
        json["children"],
        json!([]),
        "the frame has no children\n{json:#}"
    );
}

#[test]
fn a_tag_without_children_may_close_itself() {
    let self_closing = PRD_EXAMPLE.replace(">\n</Frame>", "/>");
    assert!(self_closing.ends_with("/>"));
    assert_eq!(
        existing(&self_closing),
        existing(PRD_EXAMPLE),
        "`<Frame .../>` is the same node as `<Frame ...></Frame>`"
    );
}

#[test]
fn an_id_refers_to_an_existing_node() {
    let tree = parse_tags(PRD_EXAMPLE);
    assert_eq!(tree.id(), Some(&id("k3f9x2qa")));
    assert!(!tree.is_new(), "a tag with an `id` is an existing node");
    assert_eq!(tree.kind(), NodeKind::Frame);
    assert_eq!(tree.name(), "Card");
}

#[test]
fn every_tag_is_one_node_and_its_children_are_the_node_children() {
    let source = r#"<Frame id="k3f9x2qa" name="Card" dir="col">
  <Frame id="m4n5p6q7" name="Header" dir="row">
    <Frame id="a1b2c3d4" name="Logo" dir="row"/>
    <Frame id="e5f6g7h8" name="Menu" dir="row"/>
  </Frame>
  <Frame id="r8s9t0v1" name="Body" dir="col"></Frame>
</Frame>"#;
    let json = to_json(&existing(source));
    let names = |json: &Value| -> Vec<String> {
        json["children"]
            .as_array()
            .expect("frames have children")
            .iter()
            .map(|child| child["name"].as_str().expect("a name").to_string())
            .collect()
    };
    assert_eq!(names(&json), ["Header", "Body"]);
    assert_eq!(names(&json["children"][0]), ["Logo", "Menu"]);
    assert_eq!(names(&json["children"][1]), Vec::<String>::new());
    assert_eq!(json["children"][0]["children"][1]["id"], "e5f6g7h8");
    assert_eq!(json["children"][0]["layout"]["direction"], "row");
}

#[test]
fn printing_writes_one_tag_per_node() {
    let design = complete_design();
    let printed = tags::print(&from_json(&design));
    let opened: usize = TAGS
        .iter()
        .map(|(tag, _)| {
            [" ", "\n", ">", "/"]
                .iter()
                .map(|after| printed.matches(&format!("<{tag}{after}")).count())
                .sum::<usize>()
        })
        .sum();
    assert_eq!(
        opened,
        count(&design),
        "every node is one tag, and every tag is one node\n{printed}"
    );
    for (tag, kind) in TAGS {
        let json_type = kind.as_str();
        let nodes = design
            .to_string()
            .matches(&format!("\"type\":\"{json_type}\""))
            .count();
        if nodes > 0 {
            assert!(
                printed.contains(&format!("<{tag}")),
                "{json_type} nodes are written as `<{tag}>` tags\n{printed}"
            );
        }
    }
}

#[test]
fn printing_writes_the_prd_example_as_the_prd_does() {
    let printed = tags::print(&existing(PRD_EXAMPLE));
    for attribute in [
        "<Frame",
        r#"id="k3f9x2qa""#,
        r#"name="Card""#,
        r#"dir="col""#,
        r#"gap="$space.md""#,
        "pad={16}",
        r#"fill="$color.surface""#,
    ] {
        assert!(
            printed.contains(attribute),
            "the printed frame must contain `{attribute}`; it is:\n{printed}"
        );
    }
}

// A parser and a printer convert between the tag format and the JSON node
// model without loss.

#[test]
fn a_complete_design_round_trips_through_tags() {
    assert_tags_round_trip(&complete_design());
}

#[test]
fn each_board_and_node_round_trips_on_its_own() {
    let design = complete_design();
    let mut pending = vec![design];
    while let Some(json) = pending.pop() {
        assert_tags_round_trip(&json);
        if let Some(children) = json["children"].as_array() {
            pending.extend(children.iter().cloned());
        }
    }
}

#[test]
fn the_node_examples_page_round_trips_through_tags() {
    let path = repo_root().join("render-sandbox/nodes.json");
    let text = fs::read_to_string(&path).expect("render-sandbox/nodes.json");
    let json: Value = serde_json::from_str(&text).expect("nodes.json is JSON");
    assert_tags_round_trip(&json);
}

#[test]
fn the_stress_file_round_trips_through_tags() {
    let out = std::env::temp_dir().join(format!(
        "omni-design-tags-stress-{}.json",
        std::process::id()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_stress-file"))
        .args(["--seed", "27", "--out"])
        .arg(&out)
        .output()
        .expect("the stress-file generator runs");
    assert!(
        output.status.success(),
        "the stress-file generator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(&out).expect("the stress file was written");
    let _ = fs::remove_file(&out);
    let json: Value = serde_json::from_str(&text).expect("the stress file is JSON");
    assert!(count(&json) >= 30_000, "the stress file has every node");
    assert_tags_round_trip(&json);
}

#[test]
fn texts_and_names_keep_every_character() {
    let awkward = [
        "",
        " ",
        "  leading and trailing spaces  ",
        "two\nlines",
        "a blank line\n\nbetween",
        "trailing newline\n",
        "\nleading newline",
        "tab\there",
        "carriage\r\nreturn",
        "\"double\" and 'single' quotes",
        "{braces} and {{double braces}}",
        "$color.surface is not a token here",
        "<b>markup</b> stays text",
        "</Text>",
        "<Frame name=\"inside\"/>",
        "& &amp; &lt; &#60; entities stay as written",
        "back\\slash and \\n",
        "emoji 🛒, å, ä, ö and 中文",
        "=\"/>",
    ];
    for content in awkward {
        assert_tags_round_trip(&with(&common::text("text0001"), "/content", json!(content)));
        assert_tags_round_trip(&with(&frame("frame001"), "/name", json!(content)));
    }
    for family in [
        "IBM Plex Sans",
        "Söhne \"Breit\"",
        "Font {with} <signs> & 'quotes'",
    ] {
        assert_tags_round_trip(&with(
            &common::text("text0001"),
            "/fontFamily",
            json!(family),
        ));
    }
    for path in ["M0,0 L10,10Z", "M 1 2\nL 3 4", "m6 0a6 6 0 1 0 .001 0z"] {
        assert_tags_round_trip(&with(&common::vector("vec00001"), "/path", json!(path)));
    }
}

/// Doubles that need all 17 significant digits.
const FULL_PRECISION: [f64; 4] = [12.345 * 3.0, 1439.1764831232663, 650.0 / 3.0, 20.0 / 13.0];

#[test]
fn every_number_keeps_its_exact_value() {
    let [length, wide, weight, ratio] = FULL_PRECISION;
    let fraction = 10.0 / 11.0;
    let tiny = 1e-7;
    let design = complete_design();
    for (pointer, value) in [
        ("/children/0/x", -wide),
        ("/children/0/y", length),
        ("/children/0/x", tiny),
        ("/children/0/y", 1e21),
        ("/children/0/width/fixed", wide),
        ("/children/0/minHeight", weight),
        ("/children/0/layout/gap/row", length),
        ("/children/0/layout/padding/top", 0.1 + 0.2),
        ("/children/0/fills/1/angle", -length),
        ("/children/0/fills/1/stops/1/position", fraction),
        ("/children/0/strokes/0/width", ratio),
        ("/children/0/strokes/0/dash/length", length),
        ("/children/0/radius/topLeft", length),
        ("/children/0/children/0/margin/top", -ratio),
        ("/children/0/children/0/maxWidth", wide),
        ("/children/0/children/0/shadows/0/offsetX", -length),
        ("/children/0/children/0/shadows/0/offsetY", length),
        ("/children/0/children/0/shadows/0/blur", length),
        ("/children/0/children/0/shadows/0/spread", -ratio),
        ("/children/0/children/0/children/0/fontSize", length),
        ("/children/0/children/0/children/0/fontWeight", weight),
        ("/children/0/children/0/children/0/lineHeight", ratio),
        ("/children/0/children/0/children/0/letterSpacing", -ratio),
        ("/children/0/children/0/children/2/opacity", fraction),
        ("/children/0/children/1/layout/columns/0/fixed", weight),
        ("/children/0/children/1/layout/columns/1/fr", ratio),
    ] {
        let json = with(&design, pointer, json!(value));
        assert_tags_round_trip(&json);
        let printed = tags::print(&from_json(&json));
        let back = to_json(&existing(&printed));
        assert_eq!(
            back.pointer(pointer).and_then(Value::as_f64),
            Some(value),
            "{pointer} = {value} survives the tags exactly"
        );
    }
}

#[test]
fn tags_parsed_and_printed_parse_back_to_the_same_node() {
    let source = r#"<Frame id="k3f9x2qa" name="Card" dir="col" gap="$space.md" pad={16} fill="$color.surface">
  <Frame id="m4n5p6q7" name="Row" dir="row" gap={8}>
    <Frame id="a1b2c3d4" name="Cell" dir="row"/>
  </Frame>
</Frame>"#;
    let node = existing(source);
    let printed = tags::print(&node);
    assert_eq!(
        existing(&printed),
        node,
        "printed tags parse back to the same node"
    );
    assert_eq!(
        tags::print(&existing(&printed)),
        printed,
        "printing is stable: the same node gives the same tags"
    );
}

#[test]
fn tags_are_shorter_than_the_json() {
    let node = from_json(&complete_design());
    let printed = tags::print(&node);
    let json = serde_json::to_string(&node).expect("the design serializes");
    assert!(
        printed.len() < json.len(),
        "the tag format is short: {} bytes of tags for {} bytes of compact JSON",
        printed.len(),
        json.len()
    );
}

// Nodes written without an `id` are marked new. IDs are always assigned by
// the server.

const PARTLY_NEW: &str = r#"<Frame id="k3f9x2qa" name="Card" dir="col">
  <Frame name="Header" dir="row">
    <Frame name="Logo" dir="row"/>
    <Frame id="m4n5p6q7" name="Menu" dir="row"/>
  </Frame>
  <Frame id="r8s9t0v1" name="Body" dir="col"/>
  <Frame name="Footer" dir="row"/>
</Frame>"#;

#[test]
fn nodes_without_an_id_are_new() {
    let tree = parse_tags(PARTLY_NEW);
    assert!(!tree.is_new(), "the card has an `id`");
    let [header, body, footer] = tree.children() else {
        panic!("the card has three children");
    };
    let [logo, menu] = header.children() else {
        panic!("the header has two children");
    };
    for (node, name, new) in [
        (header, "Header", true),
        (logo, "Logo", true),
        (menu, "Menu", false),
        (body, "Body", false),
        (footer, "Footer", true),
    ] {
        assert_eq!(node.name(), name);
        assert_eq!(
            node.is_new(),
            new,
            "`{name}` is new exactly when it has no `id`"
        );
        assert_eq!(
            node.id().is_none(),
            new,
            "`{name}` has an ID exactly when it is not new"
        );
    }
    assert_eq!(menu.id(), Some(&id("m4n5p6q7")));
    assert_eq!(body.id(), Some(&id("r8s9t0v1")));
}

/// The name and ID of `json` and every node below it, in document order.
fn names_and_ids(json: &Value) -> Vec<(String, String)> {
    let mut found = vec![(
        json["name"].as_str().expect("a name").to_string(),
        json["id"].as_str().expect("an ID").to_string(),
    )];
    for child in json["children"].as_array().into_iter().flatten() {
        found.extend(names_and_ids(child));
    }
    found
}

#[test]
fn the_server_assigns_ids_to_new_nodes_in_document_order() {
    let node = with_fresh_ids(PARTLY_NEW, &["new00001", "new00002", "new00003"]);
    let json = to_json(&node);
    assert_eq!(
        names_and_ids(&json),
        [
            ("Card", "k3f9x2qa"),
            ("Header", "new00001"),
            ("Logo", "new00002"),
            ("Menu", "m4n5p6q7"),
            ("Body", "r8s9t0v1"),
            ("Footer", "new00003"),
        ]
        .map(|(name, id)| (name.to_string(), id.to_string())),
        "new nodes take the server's IDs in document order, and the others keep theirs"
    );
}

#[test]
fn a_new_root_is_new_too() {
    let source = r#"<Frame name="Fresh" dir="col">
  <Frame id="m4n5p6q7" name="Kept" dir="row"/>
</Frame>"#;
    let tree = parse_tags(source);
    assert!(tree.is_new());
    assert_eq!(tree.id(), None);
    let json = to_json(&with_fresh_ids(source, &["new00001"]));
    assert_eq!(json["id"], "new00001");
    assert_eq!(json["children"][0]["id"], "m4n5p6q7");
}

#[test]
fn tags_that_all_have_ids_ask_the_server_for_none() {
    let mut asked = 0;
    let node = parse_tags(PRD_EXAMPLE).assign_ids(|| {
        asked += 1;
        id("new00001")
    });
    assert_eq!(asked, 0, "no node is new, so the server assigns no ID");
    assert_eq!(node.id(), &id("k3f9x2qa"));
}

// Unknown tags or attributes and invalid values fail with errors naming the
// line, the column and how to fix it.

#[test]
fn an_unknown_tag_names_the_tags_there_are() {
    let source =
        "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\">\n  <Box name=\"Inner\"/>\n</Frame>";
    assert_error(source, ("Box", 0), &["Box"], &["Frame", "Text", "Instance"]);
}

#[test]
fn an_unknown_attribute_names_the_attributes_the_tag_takes() {
    let source = r##"<Frame id="k3f9x2qa" name="Kassa — åäö" dir="col" colour="#ffffff"/>"##;
    let error = assert_error(source, ("colour", 0), &["colour"], &["fill"]);
    assert_eq!(
        error.column(),
        51,
        "columns count characters, not bytes: `—`, `å`, `ä` and `ö` are one column each"
    );
}

#[test]
fn an_unknown_attribute_on_a_later_line_is_found_there() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\">\n  <Frame name=\"Inner\" dir=\"row\"\n         wdith={10}/>\n</Frame>";
    assert_error(source, ("wdith", 0), &["wdith"], &[]);
}

#[test]
fn an_invalid_keyword_names_the_keywords_there_are() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"sideways\"/>";
    assert_error(source, ("\"sideways\"", 0), &["sideways"], &["row", "col"]);
}

#[test]
fn an_invalid_color_shows_how_colors_are_written() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\">\n  <Frame name=\"Swatch\" dir=\"row\"\n    fill=\"#12\"/>\n</Frame>";
    assert_error(source, ("\"#12\"", 0), &["#12"], &["#rrggbb"]);
}

#[test]
fn an_invalid_token_reference_shows_how_tokens_are_written() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\" gap=\"$space..md\"/>";
    assert_error(source, ("\"$space..md\"", 0), &["$space..md"], &["$"]);
}

#[test]
fn a_negative_padding_is_rejected() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\" pad={-4}/>";
    assert_error(source, ("{-4}", 0), &["-4"], &[]);
}

#[test]
fn an_opacity_out_of_range_names_the_range() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\">\n  <Frame name=\"Veil\" dir=\"row\" opacity={2}/>\n</Frame>";
    assert_error(source, ("{2}", 0), &["opacity", "2"], &["0 to 1"]);
}

#[test]
fn a_font_weight_out_of_range_names_the_range() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\">\n  <Text name=\"Title\" content=\"Din varukorg\" fontFamily=\"Inter\"\n    fontSize={24} fontWeight={2000}/>\n</Frame>";
    assert_error(
        source,
        ("{2000}", 0),
        &["fontWeight", "2000"],
        &["1 to 1000"],
    );
}

#[test]
fn an_opacity_below_the_range_names_the_range() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\" opacity={-0.5}/>";
    assert_error(source, ("{-0.5}", 0), &["opacity", "-0.5"], &["0 to 1"]);
}

#[test]
fn a_font_weight_below_the_range_names_the_range() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\">\n  <Text name=\"Title\" content=\"Din varukorg\" fontFamily=\"Inter\" fontSize={24} fontWeight={0}/>\n</Frame>";
    assert_error(source, ("{0}", 0), &["fontWeight", "0"], &["1 to 1000"]);
}

#[test]
fn an_invalid_id_is_rejected() {
    let source = "<Frame id=\"K3F9\" name=\"Card\" dir=\"col\"/>";
    assert_error(source, ("\"K3F9\"", 0), &["K3F9"], &[]);
}

#[test]
fn an_attribute_written_twice_is_rejected() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\" name=\"Again\"/>";
    assert_error(source, ("name", 1), &["name"], &[]);
}

#[test]
fn an_id_written_on_two_nodes_is_rejected() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\">\n  <Frame id=\"m4n5p6q7\" name=\"One\" dir=\"row\"/>\n  <Frame id=\"m4n5p6q7\" name=\"Two\" dir=\"row\"/>\n</Frame>";
    assert_error(source, ("\"m4n5p6q7\"", 1), &["m4n5p6q7"], &[]);
}

#[test]
fn a_tag_left_open_says_how_to_close_it() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\">\n  <Frame name=\"Inner\" dir=\"row\">\n  </Frame>\n";
    assert_error(source, ("Frame", 0), &["Frame"], &["</Frame>"]);
}

#[test]
fn a_closing_tag_that_does_not_match_says_which_one_to_write() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\">\n</Board>";
    assert_error(source, ("Board", 0), &["Board"], &["</Frame>"]);
}

#[test]
fn an_unterminated_value_points_at_its_opening_quote() {
    let source = "<Frame id=\"k3f9x2qa\" dir=\"col\" name=\"Card/>\n";
    assert_error(source, ("\"Card", 0), &[], &[]);
}

#[test]
fn a_board_inside_a_frame_is_rejected() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Card\" dir=\"col\">\n  <Board name=\"Nested\" dir=\"col\"/>\n</Frame>";
    let error = assert_error(source, ("Board", 0), &[], &[]);
    assert!(
        error.message().to_lowercase().contains("board"),
        "the message must say that boards do not go there; it says: {}",
        error.message()
    );
}

#[test]
fn a_position_on_a_child_that_flows_says_to_take_it_out_of_the_flow() {
    let source = "<Frame id=\"k3f9x2qa\" name=\"Row\" dir=\"row\">\n  <Frame name=\"Cell\" dir=\"row\" x={10}/>\n</Frame>";
    assert_error(source, ("x={10}", 0), &["x"], &["absolute"]);
}

#[test]
fn invalid_input_is_an_error_not_a_panic() {
    for source in [
        "",
        "   \n",
        "not tags",
        "<",
        "<Frame",
        "<Frame name=>",
        "<Frame name=\"Card\" dir=\"col\"",
        "<Frame name=\"A\" dir=\"col\"/><Frame name=\"B\" dir=\"col\"/>",
        "</Frame>",
        "<Frame name=\"Card\" dir=\"col\" pad={16/>",
        "<Frame name=\"Card\" dir=\"col\" pad={}/>",
        "<frame name=\"Card\" dir=\"col\"/>",
        "<Frame name=\"Card\" dir=\"col\"/> trailing text",
    ] {
        match tags::parse(source) {
            Ok(tree) => panic!("`{source}` is not one valid tag, but it parsed as {tree:?}"),
            Err(error) => {
                assert!(
                    error.line() >= 1 && error.column() >= 1,
                    "lines and columns count from 1 ({error})"
                );
                assert!(
                    !error.message().trim().is_empty() && !error.fix().trim().is_empty(),
                    "`{source}`: the error must say what is wrong and how to fix it"
                );
            }
        }
    }
}

// `docs/formats/design-tags.md` documents every tag and attribute.

#[test]
fn there_is_one_tag_per_node_type() {
    let names: BTreeSet<&str> = tags::all().iter().map(|tag| tag.name).collect();
    let expected: BTreeSet<&str> = TAGS.iter().map(|(tag, _)| *tag).collect();
    assert_eq!(names, expected);
    for tag in tags::all() {
        for attribute in ["id", "name"] {
            assert!(
                tag.attributes.contains(&attribute),
                "`<{}>` takes `{attribute}`",
                tag.name
            );
        }
        let unique: BTreeSet<&str> = tag.attributes.iter().copied().collect();
        assert_eq!(
            unique.len(),
            tag.attributes.len(),
            "`<{}>` lists each attribute once",
            tag.name
        );
    }
    for name in ["Board", "Frame"] {
        let tag = tags::all().iter().find(|tag| tag.name == name).unwrap();
        for attribute in ["dir", "gap", "pad", "fill", "x"] {
            assert!(
                tag.attributes.contains(&attribute),
                "`<{name}>` takes `{attribute}`"
            );
        }
    }
}

#[test]
fn the_format_doc_documents_every_tag_and_attribute() {
    let path = repo_root().join("docs/formats/design-tags.md");
    let doc = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("docs/formats/design-tags.md must document the tag format: {error}")
    });
    let mut missing = Vec::new();
    for tag in tags::all() {
        if !doc.contains(&format!("<{}", tag.name)) {
            missing.push(format!("the tag `<{}>`", tag.name));
        }
        for attribute in tag.attributes {
            if !doc.contains(&format!("`{attribute}`")) {
                missing.push(format!("the attribute `{attribute}` of `<{}>`", tag.name));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "docs/formats/design-tags.md must document every tag and attribute, but it lacks {}",
        missing.join(", ")
    );
}
