//! US-007: the WebAssembly module exposes selection to the host page, the
//! stress page in `render-sandbox/` lets the reviewer click, double-click and
//! press Esc on the stress file with outlines drawn above the canvas, and the
//! README documents the selection rules.
//!
//! The logic is native Rust and covered by `select.rs`; these are the exports
//! that forward it to the browser, which `make render-wasm` must build:
//!
//! - `omni_render_select_pointer_move(x, y, ctrl, meta)`,
//!   `omni_render_select_click(x, y, ctrl, meta)`,
//!   `omni_render_select_double_click(x, y)` and `omni_render_select_escape()`:
//!   `Scene::pointer_move`, `click`, `double_click` and `escape` for the
//!   scene of the page, in the view the page's own wheel and pan controls
//!   keep. Points are canvas CSS pixels.
//! - `omni_render_bounds_of(address, len) -> *const c_char`: `boundsOf` for an
//!   address as text (`<id>` or `<instanceId>/<componentNodeId>`), a NUL-ended
//!   JSON object that stays until the next call, `{"page": {"x": 20, "y": 20,
//!   "width": 200, "height": 200}, "screen": {...}}`, with the box in page
//!   coordinates and in canvas pixels at the current view. Null if the address
//!   names no node.
//! - `omni_render_take_events() -> *const c_char`: `Scene::take_events` as a
//!   NUL-ended JSON array that stays until the next call, oldest first:
//!   `[{"type": "hover", "node": "<address>" | null}, {"type": "select",
//!   "node": "<address>" | null}]`.
//! - `omni_render_overlay() -> *const c_char`: `Scene::overlay` for the
//!   current view as a NUL-ended JSON array that stays until the next call:
//!   `[{"kind": "hover" | "selection", "node": "<address>", "label": "Card",
//!   "x": 240, "y": 140, "width": 400, "height": 400}]`, in canvas pixels.
//!
//! The page draws those outlines and their labels in an element of its own
//! above the canvas, `id="overlay"`, redrawn with every frame so that they
//! follow the nodes while zooming and panning.
//!
//! What no test can see, the human check covers: that clicking,
//! double-clicking and Esc behave as listed on the stress file in a browser.

mod common;

use std::collections::BTreeSet;
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

/// The text of `stress.html` and of every local script it loads, directly or
/// through other scripts, joined.
fn page_source() -> String {
    let page = sandbox_dir().join("stress.html");
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
fn the_wasm_module_exports_the_selection_functions_the_stress_page_calls() {
    render_wasm();
    let loader = read_text(&sandbox_dir().join("build/omni_render.js"));
    for export in [
        "_omni_render_select_pointer_move",
        "_omni_render_select_click",
        "_omni_render_select_double_click",
        "_omni_render_select_escape",
        "_omni_render_bounds_of",
        "_omni_render_take_events",
        "_omni_render_overlay",
    ] {
        assert!(
            loader.contains(export),
            "render-sandbox/build/omni_render.js must export `{export}` after `make render-wasm`"
        );
    }
}

#[test]
fn the_stress_page_forwards_selection_input_and_draws_the_overlay_above_the_canvas() {
    let html = read_text(&sandbox_dir().join("stress.html"));
    let canvas = html.find("<canvas").expect("the page has a canvas");
    let overlay = html
        .find("id=\"overlay\"")
        .expect("the page needs an element with id=\"overlay\" for the outlines");
    assert!(
        overlay > canvas,
        "the overlay comes after the canvas, so that it is drawn above it"
    );
    let source = page_source();
    for (needle, why) in [
        ("click", "listen for clicks"),
        ("dblclick", "listen for double-clicks"),
        ("pointermove", "listen for the pointer moving, for hover"),
        ("Escape", "recognize the Esc key"),
        (
            "_omni_render_select_click",
            "forward clicks, with whether Ctrl or Cmd is held",
        ),
        ("_omni_render_select_double_click", "forward double-clicks"),
        (
            "_omni_render_select_pointer_move",
            "forward the pointer moving",
        ),
        ("_omni_render_select_escape", "forward Esc"),
        (
            "_omni_render_take_events",
            "pick up the hover and selection events",
        ),
        (
            "_omni_render_overlay",
            "ask for the outlines to draw above the canvas",
        ),
        (
            "_omni_render_bounds_of",
            "look up the bounds of a node address",
        ),
        ("overlay", "draw the outlines in the overlay element"),
    ] {
        assert!(
            source.contains(needle),
            "the page or the scripts it loads must {why} (`{needle}`)"
        );
    }
}

/// The lines of the README section whose heading contains `title` (case
/// insensitive), up to the next heading of the same or a higher level.
fn readme_section(readme: &str, title: &str) -> Option<String> {
    let mut section: Option<(usize, Vec<&str>)> = None;
    for line in readme.lines() {
        let level = line.chars().take_while(|&c| c == '#').count();
        let is_heading = level > 0 && line[level..].starts_with(' ');
        match (&mut section, is_heading) {
            (None, true) if line.to_lowercase().contains(title) => {
                section = Some((level, vec![line]));
            }
            (Some((start, _)), true) if level <= *start => break,
            (Some((_, lines)), _) => lines.push(line),
            _ => {}
        }
    }
    section.map(|(_, lines)| lines.join("\n"))
}

#[test]
fn the_readme_documents_selection_and_the_overlay_api() {
    let readme = read_text(&common::repo_root().join("crates/omni-render/README.md"));
    let section = readme_section(&readme, "selection")
        .unwrap_or_else(|| panic!("crates/omni-render/README.md needs a \"Selection\" section"));
    for (needle, why) in [
        ("nodeAt", "the call that finds the node under a point"),
        ("boundsOf", "the call that gives a node's bounds"),
        ("context", "the context that clicks select among"),
        ("double-click", "the double-click rule"),
        ("Esc", "the Esc rule"),
        ("Cmd", "the Cmd/Ctrl + click rule"),
        ("overlay", "the overlay the host page draws"),
        ("hover", "the hover events"),
    ] {
        assert!(
            section.contains(needle),
            "the Selection section must describe {why} (`{needle}`):\n{section}"
        );
    }
}
