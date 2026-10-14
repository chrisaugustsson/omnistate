//! Helpers for the US-102 converter tests: builders for synthetic Penpot
//! exports, and readers for the converted OmniState designs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use omni_core::design::{Node, NodeKind};
use omni_penpot::{Conversion, ReportEntry, convert_file};
use serde_json::{Value, json};

use super::{SyntheticPage, export_entries, shape_json, stored_zip, write_temp};

pub const FILE_ID: &str = "f0000000-0000-4000-8000-000000000002";

/// A 2 by 2 PNG: the first column red, the second blue.
pub const PNG: [u8; 74] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d,
    0x24, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x42, 0xff, 0x19, 0x60, 0x0c, 0x00, 0x43, 0xce, 0x07, 0xf9, 0x58, 0xed, 0x66, 0xa0, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// The id of the page of synthetic exports, and its name.
pub const PAGE_ID: &str = "a0000000-0000-4000-8000-000000000001";
pub const PAGE_NAME: &str = "Page 1";

/// A UUID for the shape number `n`. None of them is the id of the root frame.
pub fn uid(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

/// A shape of a synthetic export, with the shapes inside it.
pub struct S {
    pub json: Value,
    pub kids: Vec<S>,
}

/// A shape numbered `n` of type `kind` (as Penpot names it) in the box `bounds`
/// (`[x, y, width, height]`, in page coordinates), with no fill or stroke.
pub fn shape(n: u32, kind: &str, name: &str, bounds: [f64; 4]) -> S {
    S {
        json: shape_json(&uid(n), kind, name, &uid(0), bounds, &[]),
        kids: Vec::new(),
    }
}

impl S {
    /// Sets the fields of `extra` on the shape, replacing those it has.
    pub fn with(mut self, extra: Value) -> S {
        for (key, value) in extra.as_object().expect("`with` takes an object") {
            self.json[key] = value.clone();
        }
        self
    }

    /// Puts `kids` inside the shape, bottom of the stacking order first.
    pub fn kids(mut self, kids: Vec<S>) -> S {
        self.kids = kids;
        self
    }

    pub fn id(&self) -> String {
        self.json["id"].as_str().expect("an id").to_string()
    }
}

fn flatten(shape: S, parent: &str, out: &mut Vec<Value>) {
    let S { mut json, kids } = shape;
    json["parentId"] = json!(parent);
    json["frameId"] = json!(parent);
    let id = json["id"].as_str().expect("an id").to_string();
    if !kids.is_empty() {
        json["shapes"] = json!(kids.iter().map(S::id).collect::<Vec<_>>());
    }
    out.push(json);
    for kid in kids {
        flatten(kid, &id, out);
    }
}

/// The page of a synthetic export that has `top_level` directly on it.
pub fn page(top_level: Vec<S>) -> SyntheticPage {
    let ids = top_level.iter().map(S::id).collect();
    let mut shapes = Vec::new();
    for shape in top_level {
        flatten(shape, super::ROOT_ID, &mut shapes);
    }
    SyntheticPage {
        id: PAGE_ID.to_string(),
        name: PAGE_NAME.to_string(),
        index: 0,
        top_level: ids,
        shapes,
    }
}

/// The path of an export of one page that has `top_level` on it, written to
/// the temporary directory.
pub fn write_export(test: &str, top_level: Vec<S>) -> PathBuf {
    write_export_with(test, top_level, Vec::new())
}

/// As [`write_export`], with `extra` archive entries too.
pub fn write_export_with(test: &str, top_level: Vec<S>, extra: Vec<(String, Vec<u8>)>) -> PathBuf {
    let mut entries = export_entries(FILE_ID, "Synthetic", &[page(top_level)]);
    entries.extend(extra);
    write_temp(&format!("convert-{test}.penpot"), &stored_zip(&entries))
}

/// Converts an export of one page that has `top_level` on it. Every converted
/// design must pass OmniState's design validation, so this checks that each one
/// survives a trip through its JSON text.
pub fn convert(test: &str, top_level: Vec<S>) -> Conversion {
    convert_with(test, top_level, Vec::new())
}

/// As [`convert`], with `extra` archive entries too.
pub fn convert_with(test: &str, top_level: Vec<S>, extra: Vec<(String, Vec<u8>)>) -> Conversion {
    let path = write_export_with(test, top_level, extra);
    let conversion =
        convert_file(&path).unwrap_or_else(|error| panic!("cannot convert {path:?}: {error}"));
    for board in &conversion.boards {
        assert_validates(&board.design, &board.name);
    }
    conversion
}

/// Checks that `design` is accepted by OmniState's design validation, which is
/// what reading its JSON text does, and reads back as the same design.
pub fn assert_validates(design: &Node, what: &str) {
    let text = serde_json::to_string(design).expect("a design serializes");
    let back: Node = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("the design of {what:?} does not validate: {error}"));
    assert_eq!(
        &back, design,
        "the design of {what:?} changes in a round trip"
    );
}

/// The design of the board called `name`.
pub fn board<'a>(conversion: &'a Conversion, name: &str) -> &'a Node {
    let found: Vec<_> = conversion
        .boards
        .iter()
        .filter(|board| board.name == name)
        .collect();
    assert_eq!(found.len(), 1, "expected one board called {name:?}");
    &found[0].design
}

/// The board node of a design: the only child of its page.
pub fn board_node(design: &Node) -> &Node {
    assert_eq!(design.kind(), NodeKind::Page, "a design is a page");
    assert_eq!(design.children().len(), 1, "the page holds the board alone");
    let board = &design.children()[0];
    assert_eq!(board.kind(), NodeKind::Board);
    board
}

/// The node called `name` in the tree under `root`, which has exactly one.
pub fn node<'a>(root: &'a Node, name: &str) -> &'a Node {
    let found: Vec<_> = nodes(root)
        .into_iter()
        .filter(|node| node.name() == name)
        .collect();
    assert_eq!(found.len(), 1, "expected one node called {name:?}");
    found[0]
}

/// Whether the tree under `root` has a node called `name`.
pub fn has_node(root: &Node, name: &str) -> bool {
    nodes(root).into_iter().any(|node| node.name() == name)
}

/// `root` and every node below it, parents before children.
pub fn nodes(root: &Node) -> Vec<&Node> {
    let mut all = vec![root];
    for child in root.children() {
        all.extend(nodes(child));
    }
    all
}

/// The names of the children of `node`, in order.
pub fn child_names(node: &Node) -> Vec<&str> {
    node.children().iter().map(Node::name).collect()
}

/// The JSON form of `node`, which the tests read properties from.
pub fn json_of(node: &Node) -> Value {
    serde_json::to_value(node).expect("a node serializes")
}

/// The report entries of the shape `shape_id`.
pub fn entries_of<'a>(conversion: &'a Conversion, shape_id: &str) -> Vec<&'a ReportEntry> {
    conversion
        .report
        .entries
        .iter()
        .filter(|entry| entry.shape_id == shape_id)
        .collect()
}

/// Checks that the report lists `shape_id` in the board `board_id` called
/// `board`, under the name `name`, with a reason that says one of `words`.
pub fn assert_reported(
    conversion: &Conversion,
    shape_id: &str,
    board_id: &str,
    board: &str,
    name: &str,
    words: &[&str],
) {
    let entries = entries_of(conversion, shape_id);
    assert!(
        !entries.is_empty(),
        "the report does not list the shape {shape_id} ({name}); it has {:#?}",
        conversion.report.entries
    );
    let entry = entries
        .iter()
        .find(|entry| {
            let reason = entry.reason.to_lowercase();
            words.iter().any(|word| reason.contains(word))
        })
        .unwrap_or_else(|| {
            panic!("no reason of {entries:#?} says any of {words:?}");
        });
    assert_eq!(entry.board_id, board_id, "board id of {entry:?}");
    assert_eq!(entry.board, board, "board name of {entry:?}");
    assert_eq!(entry.name, name, "shape name of {entry:?}");
}

/// A number in a JSON design.
pub fn num(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("expected a number, got {value}"))
}

/// A number in a JSON design that is 0 when left out.
pub fn num_or_zero(value: &Value) -> f64 {
    if value.is_null() { 0.0 } else { num(value) }
}

pub fn assert_near(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: expected {expected} (within {tolerance}), got {actual}"
    );
}

/// How a node sizes an axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sz {
    Fixed(f64),
    Hug,
    Fill,
}

/// The `width` or `height` of a JSON node.
pub fn sizing(value: &Value) -> Sz {
    match value {
        Value::String(text) if text == "hug" => Sz::Hug,
        Value::String(text) if text == "fill" => Sz::Fill,
        Value::Object(map) if map.len() == 1 && map.contains_key("fixed") => {
            Sz::Fixed(num(&map["fixed"]))
        }
        other => panic!("not a sizing: {other}"),
    }
}

pub fn assert_sizing(actual: &Value, expected: Sz, what: &str) {
    match (sizing(actual), expected) {
        (Sz::Fixed(a), Sz::Fixed(e)) => assert_near(a, e, 0.01, what),
        (a, e) => assert_eq!(a, e, "{what}"),
    }
}

/// The red, green, blue and alpha bytes of a color written `#rrggbb` or
/// `#rrggbbaa`.
pub fn rgba(color: &Value) -> [u8; 4] {
    let text = color
        .as_str()
        .unwrap_or_else(|| panic!("expected a color, got {color}"));
    let hex = text
        .strip_prefix('#')
        .unwrap_or_else(|| panic!("{text} is not a color"));
    let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).expect("a hex digit pair");
    match hex.len() {
        6 => [byte(0), byte(2), byte(4), 255],
        8 => [byte(0), byte(2), byte(4), byte(6)],
        _ => panic!("{text} is not a color"),
    }
}

/// Checks that `color` is the color `expected` (red, green, blue) at `alpha`,
/// from 0 to 1, give or take one step of a byte.
pub fn assert_color(color: &Value, expected: [u8; 3], alpha: f64, what: &str) {
    let [r, g, b, a] = rgba(color);
    assert_eq!([r, g, b], expected, "{what}: the color of {color}");
    assert_near(f64::from(a), alpha * 255.0, 1.01, &format!("{what}: alpha"));
}

/// The numbers in SVG path data, in order.
pub fn path_numbers(path: &str) -> Vec<f64> {
    let mut numbers = Vec::new();
    let mut current = String::new();
    let mut flush = |current: &mut String| {
        if !current.is_empty() {
            numbers.push(
                current
                    .parse()
                    .unwrap_or_else(|_| panic!("number {current}")),
            );
            current.clear();
        }
    };
    for character in path.chars() {
        match character {
            '0'..='9' | '.' => current.push(character),
            '-' => {
                flush(&mut current);
                current.push('-');
            }
            _ => flush(&mut current),
        }
    }
    flush(&mut current);
    numbers
}

/// Every `attachment:<id>` that a JSON design mentions.
pub fn attachment_refs(value: &Value) -> Vec<String> {
    let mut found = Vec::new();
    collect_refs(value, &mut found);
    found
}

fn collect_refs(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::String(text) if text.starts_with("attachment:") => found.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| collect_refs(item, found)),
        Value::Object(map) => map.values().for_each(|item| collect_refs(item, found)),
        _ => {}
    }
}

/// The archive entries of an image in a synthetic export: its media record,
/// `media_id`, and the stored file `storage_id` holding the PNG.
pub fn media_entries(media_id: &str, storage_id: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        (
            format!("files/{FILE_ID}/media/{media_id}.json"),
            json!({
                "id": media_id,
                "name": "picture.png",
                "width": 2,
                "height": 2,
                "mtype": "image/png",
                "mediaId": storage_id
            })
            .to_string()
            .into_bytes(),
        ),
        (
            format!("objects/{storage_id}.json"),
            json!({
                "id": storage_id,
                "bucket": "file-media-object",
                "contentType": "image/png",
                "size": PNG.len()
            })
            .to_string()
            .into_bytes(),
        ),
        (format!("objects/{storage_id}.png"), PNG.to_vec()),
    ]
}

/// The fill that shows the media `media_id`.
pub fn image_fill(media_id: &str) -> Value {
    json!({
        "fillImage": {
            "id": media_id,
            "name": "picture.png",
            "width": 2,
            "height": 2,
            "mtype": "image/png",
            "keepAspectRatio": true
        },
        "fillOpacity": 1
    })
}

/// A flex layout of the fields a Penpot frame has for it, with Penpot's default
/// values. Combine with `S::with`.
pub fn flex(direction: &str) -> Value {
    json!({
        "layout": "flex",
        "layoutFlexDir": direction,
        "layoutWrapType": "nowrap",
        "layoutAlignItems": "start",
        "layoutAlignContent": "stretch",
        "layoutJustifyContent": "start",
        "layoutGapType": "multiple",
        "layoutGap": { "rowGap": 0, "columnGap": 0 },
        "layoutPaddingType": "simple",
        "layoutPadding": { "p1": 0, "p2": 0, "p3": 0, "p4": 0 }
    })
}

/// A text run: the characters, in the default style changed by `style`.
pub fn run(text: &str, style: Value) -> Value {
    let mut run = json!({
        "fontId": "gfont-inter",
        "fontFamily": "Inter",
        "fontSize": "16",
        "fontWeight": "400",
        "fontVariantId": "regular",
        "fontStyle": "normal",
        "lineHeight": "1.5",
        "letterSpacing": "0",
        "textAlign": "left",
        "textDecoration": "none",
        "textTransform": "none",
        "textDirection": "ltr",
        "fills": [{ "fillColor": "#102030", "fillOpacity": 1 }],
        "text": text
    });
    for (key, value) in style.as_object().expect("a style is an object") {
        run[key] = value.clone();
    }
    run
}

/// A paragraph of `runs`, which takes the style of its first run.
pub fn paragraph(runs: Vec<Value>) -> Value {
    let mut paragraph = runs[0].clone();
    paragraph.as_object_mut().expect("a run").remove("text");
    paragraph["type"] = json!("paragraph");
    paragraph["children"] = json!(runs);
    paragraph
}

/// The `content` of a text shape holding `paragraphs`.
pub fn text_content(paragraphs: Vec<Value>) -> Value {
    json!({
        "type": "root",
        "children": [{ "type": "paragraph-set", "children": paragraphs }]
    })
}

/// A text shape numbered `n` that holds one run of `text`.
pub fn text(n: u32, name: &str, bounds: [f64; 4], text: &str) -> S {
    shape(n, "text", name, bounds).with(json!({
        "growType": "fixed",
        "content": text_content(vec![paragraph(vec![run(text, json!({}))])])
    }))
}

/// A directory of its own in the temporary directory, empty.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("omni-penpot-cli-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.parent().expect("a parent")).expect("the temporary directory exists");
    dir
}

/// Runs `omni-penpot` with `args`.
pub fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omni-penpot"))
        .args(args)
        .output()
        .expect("omni-penpot can be started")
}

/// Runs `omni-penpot convert <file> --out <out>`.
pub fn run_convert(file: &Path, out: &Path) -> Output {
    run_cli(&[
        "convert",
        file.to_str().expect("a UTF-8 path"),
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ])
}

/// The files directly in `dir` with extension `extension`, by name.
pub fn files_with_extension(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot list {dir:?}: {error}"))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|found| found == extension))
        .collect();
    found.sort();
    found
}
