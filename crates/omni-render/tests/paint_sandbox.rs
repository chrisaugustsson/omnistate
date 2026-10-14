//! US-005: the visual test page in `render-sandbox/` shows one example of every
//! node type and property, and the README records how far the WebAssembly
//! build (FreeType) and the native build (CoreText) lay those examples out
//! apart.
//!
//! The page is `render-sandbox/nodes.html`. It draws the design in
//! `render-sandbox/nodes.json` on a WebGL canvas through the module that `make
//! render-wasm` builds, which exports the functions the host page calls:
//!
//! - `omni_render_add_image` and `omni_render_add_font`: the page supplies the
//!   images that `attachment:<id>` references name, and font files.
//! - `omni_render_draw_design`: draws a design on the page's WebGL canvas.
//! - `omni_render_layout_design`: lays a design out and returns its boxes, so
//!   the page can measure them against the native build's.
//!
//! The design is a page of boards in the format of `omni_core::design`, with
//! literal values only, and no instances (US-032). Its examples are drawn
//! natively here too, to check that every board shows something.
//!
//! What no test can see, the human check covers: that every example on the
//! page looks right in a browser.

mod common;

use omni_core::design::{Node, NodeData};
use omni_render::layout;
use omni_render::paint::{self, Assets};
use serde_json::Value;
use skia_safe::{EncodedImageFormat, Paint, surfaces};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::OnceLock;

fn sandbox_dir() -> PathBuf {
    common::repo_root().join("render-sandbox")
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

fn nodes_json() -> Value {
    let text = read_text(&sandbox_dir().join("nodes.json"));
    serde_json::from_str(&text).expect("render-sandbox/nodes.json is JSON")
}

// ---------------------------------------------------------------------------
// The page and the module it loads
// ---------------------------------------------------------------------------

/// Runs `make render-wasm` once for every test in this file that needs it.
fn render_wasm() -> &'static Output {
    static BUILD: OnceLock<Output> = OnceLock::new();
    let output = BUILD.get_or_init(|| common::make(&["render-wasm"]));
    assert!(
        output.status.success(),
        "`make render-wasm` failed:\n{}",
        common::describe(output)
    );
    output
}

/// The values of the `src` attributes in `html`.
fn script_sources(html: &str) -> Vec<String> {
    let mut sources = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("src=") {
        rest = &rest[at + 4..];
        let Some(quote) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) else {
            continue;
        };
        if let Some(end) = rest[1..].find(quote) {
            sources.push(rest[1..1 + end].to_string());
        }
    }
    sources
}

/// Relative paths written as quoted strings in `text` that end in one of
/// `extensions`.
fn quoted_paths(text: &str, extensions: &[&str]) -> Vec<String> {
    let mut paths = Vec::new();
    for quote in ['"', '\'', '`'] {
        for (index, piece) in text.split(quote).enumerate() {
            if index % 2 == 1
                && !piece.is_empty()
                && !piece.contains(char::is_whitespace)
                && extensions
                    .iter()
                    .any(|extension| piece.ends_with(extension))
            {
                paths.push(piece.to_string());
            }
        }
    }
    paths
}

/// The text of the page and of every local script it loads, directly or
/// through other scripts, joined.
fn page_source() -> String {
    let page = sandbox_dir().join("nodes.html");
    let html = read_text(&page);
    let mut all = html.clone();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: Vec<(PathBuf, String)> = script_sources(&html)
        .into_iter()
        .map(|source| (page.clone(), source))
        .collect();
    while let Some((from, reference)) = queue.pop() {
        let Some(directory) = from.parent() else {
            continue;
        };
        let Ok(path) = directory.join(&reference).canonicalize() else {
            continue;
        };
        if !path.is_file() || !path.starts_with(sandbox_dir()) || !seen.insert(path.clone()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for next in quoted_paths(&text, &[".js", ".mjs"]) {
            queue.push((path.clone(), next));
        }
        all.push('\n');
        all.push_str(&text);
    }
    all
}

#[test]
fn the_visual_test_page_draws_the_example_design_on_a_webgl_canvas() {
    let page = sandbox_dir().join("nodes.html");
    assert!(
        page.is_file(),
        "render-sandbox/nodes.html is the visual test page"
    );
    let html = read_text(&page);
    assert!(
        html.to_lowercase().contains("<canvas"),
        "the page must contain a <canvas>:\n{html}"
    );
    let source = page_source();
    let lower = source.to_lowercase();
    for (needle, why) in [
        ("webgl", "draw through a WebGL context"),
        (
            "omni_render.js",
            "load the module that make render-wasm builds",
        ),
        ("nodes.json", "load the example design"),
        (
            "_omni_render_draw_design",
            "draw the design with the module's exported function",
        ),
        (
            "_omni_render_add_image",
            "supply the images that the design's attachments name",
        ),
        (
            "_omni_render_layout_design",
            "measure the layout of the WebAssembly build",
        ),
    ] {
        assert!(
            lower.contains(needle),
            "the page or the scripts it loads must {why} (`{needle}`)"
        );
    }
}

#[test]
fn the_wasm_module_exports_the_functions_the_page_calls() {
    render_wasm();
    let loader = read_text(&sandbox_dir().join("build/omni_render.js"));
    for export in [
        "_omni_render_add_image",
        "_omni_render_add_font",
        "_omni_render_draw_design",
        "_omni_render_layout_design",
    ] {
        assert!(
            loader.contains(export),
            "render-sandbox/build/omni_render.js must export `{export}` after `make render-wasm`"
        );
    }
}

// ---------------------------------------------------------------------------
// The example design
// ---------------------------------------------------------------------------

/// Every node of `json` and below, with the type it says it has.
fn nodes_of(json: &Value) -> Vec<&Value> {
    let mut nodes = vec![json];
    let mut next = 0;
    while next < nodes.len() {
        if let Some(children) = nodes[next].get("children").and_then(Value::as_array) {
            let children: Vec<&Value> = children.iter().collect();
            nodes.extend(children);
        }
        next += 1;
    }
    nodes
}

/// Strings in `json` that start with `$`, other than in the free text of
/// `content` and `name`: token references.
fn token_references(json: &Value, key: &str, found: &mut Vec<String>) {
    match json {
        Value::String(text) if text.starts_with('$') && key != "content" && key != "name" => {
            found.push(text.clone());
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| token_references(item, key, found)),
        Value::Object(map) => map
            .iter()
            .for_each(|(key, value)| token_references(value, key, found)),
        _ => {}
    }
}

#[test]
fn the_example_design_is_a_page_the_design_model_accepts_with_literal_values_and_no_instances() {
    let json = nodes_json();
    assert_eq!(
        json["type"], "page",
        "the example design is one page whose children are the boards"
    );
    let design: Node = serde_json::from_value(json.clone())
        .unwrap_or_else(|error| panic!("the design model must accept nodes.json: {error}"));
    assert!(
        matches!(design.data(), NodeData::Page(page) if page.children.len() >= 3),
        "the page needs several boards, to show everything"
    );

    let mut tokens = Vec::new();
    token_references(&json, "", &mut tokens);
    assert!(
        tokens.is_empty(),
        "token references are out of scope here (US-032), but the design has {tokens:?}"
    );
    let instances = nodes_of(&json)
        .into_iter()
        .filter(|node| node["type"] == "instance")
        .count();
    assert_eq!(instances, 0, "instances are out of scope here (US-032)");
}

/// What a design shows.
#[derive(Default)]
struct Seen {
    types: BTreeSet<String>,
    /// `(node type, property)` pairs.
    properties: BTreeSet<(String, String)>,
    /// The values of keywords, by name such as `fill type`.
    values: BTreeMap<&'static str, BTreeSet<String>>,
    /// The keys of the corner radii, shadows and strokes seen.
    keys: BTreeMap<&'static str, BTreeSet<String>>,
}

impl Seen {
    fn value(&mut self, what: &'static str, value: &Value) {
        let text = match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        self.values.entry(what).or_default().insert(text);
    }

    fn keys_of(&mut self, what: &'static str, object: &Value) {
        if let Some(map) = object.as_object() {
            self.keys
                .entry(what)
                .or_default()
                .extend(map.keys().cloned());
        }
    }

    fn has_value(&self, what: &str, value: &str) -> bool {
        self.values
            .get(what)
            .is_some_and(|values| values.contains(value))
    }

    /// Whether a board or a frame has `property`.
    fn container_has(&self, property: &str) -> bool {
        ["board", "frame"]
            .iter()
            .any(|kind| self.has(kind, property))
    }

    fn has(&self, kind: &str, property: &str) -> bool {
        self.properties
            .contains(&(kind.to_string(), property.to_string()))
    }
}

fn survey(json: &Value) -> Seen {
    let mut seen = Seen::default();
    for node in nodes_of(json) {
        let kind = node["type"].as_str().expect("every node has a type");
        seen.types.insert(kind.to_string());
        for key in node.as_object().expect("nodes are objects").keys() {
            seen.properties.insert((kind.to_string(), key.clone()));
        }
        if node["visible"] == false {
            seen.value("visible", &node["visible"]);
        }
        if let Some(mode) = node.get("layout").and_then(|layout| layout.get("mode")) {
            seen.value("layout mode", mode);
        }
        for fill in node
            .get("fills")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            seen.value("fill type", &fill["type"]);
            if let Some(fit) = fill.get("fit") {
                seen.value("fit", fit);
            }
        }
        if let Some(fit) = node.get("fit") {
            seen.value("fit", fit);
        }
        for shadow in node
            .get("shadows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            seen.value("shadow type", &shadow["type"]);
            seen.keys_of("shadow", shadow);
        }
        for stroke in node
            .get("strokes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            seen.keys_of("stroke", stroke);
        }
        if let Some(radius) = node.get("radius") {
            seen.keys_of("radius", radius);
        }
        for (key, what) in [
            ("textAlign", "text align"),
            ("textDecoration", "text decoration"),
            ("fontStyle", "font style"),
            ("fontFamily", "font family"),
        ] {
            if kind == "text" {
                if let Some(value) = node.get(key) {
                    seen.value(what, value);
                }
            }
        }
    }
    seen
}

#[test]
fn the_example_design_shows_one_example_of_every_node_type_and_property() {
    let seen = survey(&nodes_json());
    let mut missing: Vec<String> = Vec::new();
    let mut need = |ok: bool, what: &str| {
        if !ok {
            missing.push(what.to_string());
        }
    };

    for kind in ["page", "board", "frame", "text", "image", "vector"] {
        need(seen.types.contains(kind), &format!("a {kind} node"));
    }
    for mode in ["none", "flex", "grid"] {
        need(
            seen.has_value("layout mode", mode),
            &format!("a `{mode}` layout"),
        );
    }
    need(seen.has_value("visible", "false"), "a hidden node");

    for property in ["fills", "strokes", "radius", "shadows", "opacity", "clip"] {
        need(
            seen.container_has(property),
            &format!("`{property}` on a board or a frame"),
        );
    }
    for fill in ["solid", "linear-gradient", "image"] {
        need(
            seen.has_value("fill type", fill),
            &format!("a `{fill}` fill"),
        );
    }
    for corner in ["topLeft", "topRight", "bottomRight", "bottomLeft"] {
        need(
            seen.keys
                .get("radius")
                .is_some_and(|keys| keys.contains(corner)),
            &format!("a `{corner}` radius"),
        );
    }
    for shadow in ["drop", "inner"] {
        need(
            seen.has_value("shadow type", shadow),
            &format!("a {shadow} shadow"),
        );
    }
    for key in ["offsetX", "offsetY", "blur", "spread"] {
        need(
            seen.keys
                .get("shadow")
                .is_some_and(|keys| keys.contains(key)),
            &format!("a shadow with `{key}`"),
        );
    }
    need(seen.keys.contains_key("stroke"), "a stroke");

    for property in [
        "content",
        "fontFamily",
        "fontSize",
        "fontWeight",
        "fontStyle",
        "lineHeight",
        "letterSpacing",
        "textAlign",
        "textDecoration",
        "color",
        "opacity",
    ] {
        need(
            seen.has("text", property),
            &format!("`{property}` on a text"),
        );
    }
    for align in ["left", "center", "right", "justify"] {
        need(
            seen.has_value("text align", align),
            &format!("`{align}` text"),
        );
    }
    for decoration in ["underline", "line-through"] {
        need(
            seen.has_value("text decoration", decoration),
            &format!("`{decoration}` text"),
        );
    }
    need(seen.has_value("font style", "italic"), "italic text");
    need(seen.has_value("font family", "Inter"), "a text in Inter");
    need(
        seen.values
            .get("font family")
            .is_some_and(|families| families.iter().any(|family| family != "Inter")),
        "a text in a font family that the renderer does not have (set in Inter instead)",
    );

    for property in ["image", "fit", "strokes", "radius", "shadows", "opacity"] {
        need(
            seen.has("image", property),
            &format!("`{property}` on an image"),
        );
    }
    for fit in ["cover", "contain"] {
        need(seen.has_value("fit", fit), &format!("a `{fit}` image fit"));
    }
    for property in ["path", "fills", "strokes", "shadows", "opacity"] {
        need(
            seen.has("vector", property),
            &format!("`{property}` on a vector"),
        );
    }

    assert!(
        missing.is_empty(),
        "render-sandbox/nodes.json must show one example of every node type and property, but it has no:\n  {}",
        missing.join("\n  ")
    );
}

/// The PNG of an image `colors.len()` pixels wide and 2 tall, each column one
/// of `colors`.
fn png(colors: &[[u8; 3]]) -> Vec<u8> {
    let mut surface = surfaces::raster_n32_premul((colors.len() as i32, 2)).expect("a surface");
    for (column, [r, g, b]) in colors.iter().enumerate() {
        let mut paint = Paint::default();
        paint.set_color(skia_safe::Color::from_rgb(*r, *g, *b));
        surface.canvas().draw_rect(
            skia_safe::Rect::from_xywh(column as f32, 0.0, 1.0, 2.0),
            &paint,
        );
    }
    surface
        .image_snapshot()
        .encode(
            None::<&mut skia_safe::gpu::DirectContext>,
            EncodedImageFormat::PNG,
            None,
        )
        .expect("the image encodes as PNG")
        .as_bytes()
        .to_vec()
}

/// Every attachment that an image node or an image fill of `json` shows.
fn attachments(json: &Value) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for node in nodes_of(json) {
        if node["type"] == "image" {
            found.insert(node["image"].as_str().expect("an attachment").to_string());
        }
        for fill in node
            .get("fills")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if fill["type"] == "image" {
                found.insert(fill["image"].as_str().expect("an attachment").to_string());
            }
        }
    }
    found
}

#[test]
fn every_board_of_the_example_design_draws_something_when_the_host_supplies_its_images() {
    let json = nodes_json();
    let design: Node = serde_json::from_value(json.clone()).expect("the design model accepts it");
    let mut assets = Assets::new();
    for reference in attachments(&json) {
        let attachment = reference.parse().expect("a valid attachment reference");
        assets
            .add_image(
                &attachment,
                &png(&[[220, 40, 40], [40, 160, 60], [40, 80, 220], [240, 200, 30]]),
            )
            .expect("the host can supply a PNG");
    }

    let boxes = layout::compute(&design, assets.fonts());
    let NodeData::Page(page) = design.data() else {
        panic!("the example design is a page");
    };
    let board_boxes: Vec<_> = page
        .children
        .iter()
        .map(|board| (board.name().to_string(), boxes[board.id()]))
        .collect();
    let width = board_boxes
        .iter()
        .map(|(_, b)| (b.x + b.width).ceil() as u32)
        .max()
        .expect("boards");
    let height = board_boxes
        .iter()
        .map(|(_, b)| (b.y + b.height).ceil() as u32)
        .max()
        .expect("boards");
    assert!(
        (1..=4096).contains(&width) && (1..=8192).contains(&height),
        "the boards of the sandbox page lie within 4096 x 8192 px, but they need {width} x {height}"
    );

    let rgba = paint::render_rgba(&design, &assets, width, height);
    assert_eq!(rgba.len(), width as usize * height as usize * 4);
    for (name, board) in board_boxes {
        assert!(
            board.width > 0.0 && board.height > 0.0,
            "board {name:?} has a size"
        );
        let (x0, y0) = (board.x.floor() as u32, board.y.floor() as u32);
        let (x1, y1) = (
            ((board.x + board.width).ceil() as u32).min(width),
            ((board.y + board.height).ceil() as u32).min(height),
        );
        let drawn = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| rgba[(y as usize * width as usize + x as usize) * 4 + 3] > 0)
            .count();
        assert!(drawn > 0, "board {name:?} draws nothing in its box");
    }
}

// ---------------------------------------------------------------------------
// The README
// ---------------------------------------------------------------------------

#[test]
fn the_readme_records_the_largest_layout_difference_between_wasm_and_native() {
    let readme = read_text(&common::repo_root().join("crates/omni-render/README.md"));
    let paragraph = readme
        .split("\n\n")
        .find(|paragraph| paragraph.to_lowercase().contains("largest difference"))
        .unwrap_or_else(|| {
            panic!(
                "crates/omni-render/README.md must record the largest difference between the layout boxes of the WASM build and the native build, in a paragraph that says \"largest difference\""
            )
        });
    for word in ["FreeType", "CoreText", "nodes.json"] {
        assert!(
            paragraph.contains(word),
            "the paragraph on the largest difference must mention {word}:\n{paragraph}"
        );
    }
    let lower = paragraph.to_lowercase();
    let after = &lower[lower.find("largest difference").expect("found above")..];
    let words: Vec<&str> = after
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
        .map(|word| word.trim_end_matches(['.', ';', ':']))
        .filter(|word| !word.is_empty())
        .collect();
    let measured: Vec<f64> = words
        .iter()
        .enumerate()
        .filter_map(|(index, word)| {
            let followed = words.get(index + 1) == Some(&"px");
            let number = match word.strip_suffix("px") {
                Some(number) => number,
                None if followed => word,
                None => return None,
            };
            number.trim_matches('*').parse::<f64>().ok()
        })
        .collect();
    assert!(
        measured
            .iter()
            .any(|difference| difference.is_finite() && *difference >= 0.0),
        "the paragraph must state the largest difference as a number of px, such as `0.04 px`:\n{paragraph}"
    );
}
