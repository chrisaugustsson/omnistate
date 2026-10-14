//! US-006: the stress page in `render-sandbox/` lets the reviewer zoom and pan
//! around the stress file in a browser, and the README records which level of
//! detail technique the renderer uses when zoomed out.
//!
//! The page is `render-sandbox/stress.html`. It draws `build/stress-file.json`
//! (which `make stress-file` writes) on a WebGL canvas through the module that
//! `make render-wasm` builds, and forwards the browser's wheel, pointer, touch
//! and key events to `omni_render::view::Controls` through these exports,
//! whose logic the native tests cover (`viewport.rs`, `scene.rs`):
//!
//! - `omni_render_scene_load(design, len) -> bool`: parses the design (a
//!   page, as JSON) into the module's scene and fits the view to its boards.
//! - `omni_render_scene_draw(width, height, pixel_ratio) -> bool`: draws the
//!   scene in the current viewport on the canvas of `width` x `height` device
//!   pixels, after resizing the viewport to that size in CSS pixels.
//! - `omni_render_scene_stats() -> *const c_char`: the last frame's
//!   `FrameStats` as JSON, `{"boardsDrawn": 3, "nodesPainted": 640,
//!   "detail": "full"}` (`detail` is `"full"` or `"reduced"`).
//! - `omni_render_view_wheel(delta_x, delta_y, x, y, ctrl, meta) -> bool`,
//!   `omni_render_view_pinch(scale, x, y) -> bool`,
//!   `omni_render_view_pointer_down(x, y) -> bool`,
//!   `omni_render_view_pointer_move(x, y) -> bool`,
//!   `omni_render_view_pointer_up()` and `omni_render_view_space(held)`.
//!
//! The README has a "Level of detail" section that names the technique, so
//! that US-007 and US-008 build on a documented choice.
//!
//! What no test can see, the human check covers: that zooming and panning
//! around the stress file feel smooth in a browser.

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
fn the_wasm_module_exports_the_functions_the_stress_page_calls() {
    render_wasm();
    let loader = read_text(&sandbox_dir().join("build/omni_render.js"));
    for export in [
        "_omni_render_scene_load",
        "_omni_render_scene_draw",
        "_omni_render_scene_stats",
        "_omni_render_view_wheel",
        "_omni_render_view_pinch",
        "_omni_render_view_pointer_down",
        "_omni_render_view_pointer_move",
        "_omni_render_view_pointer_up",
        "_omni_render_view_space",
    ] {
        assert!(
            loader.contains(export),
            "render-sandbox/build/omni_render.js must export `{export}` after `make render-wasm`"
        );
    }
}

#[test]
fn the_stress_page_draws_the_stress_file_and_forwards_zoom_and_pan_events() {
    let page = sandbox_dir().join("stress.html");
    assert!(
        page.is_file(),
        "render-sandbox/stress.html is the page for zooming and panning around the stress file"
    );
    let html = read_text(&page);
    assert!(
        html.to_lowercase().contains("<canvas"),
        "the page must contain a <canvas>:\n{html}"
    );
    let source = page_source();
    assert!(
        source.to_lowercase().contains("webgl"),
        "the page must draw through a WebGL context"
    );
    for (needle, why) in [
        (
            "omni_render.js",
            "load the module that make render-wasm builds",
        ),
        (
            "stress-file.json",
            "load the file that make stress-file writes to render-sandbox/build/",
        ),
        ("_omni_render_scene_load", "load the design into the scene"),
        ("_omni_render_scene_draw", "draw the scene"),
        ("_omni_render_scene_stats", "show what the frame did"),
        ("requestAnimationFrame", "draw frames as the browser paints"),
        ("wheel", "listen for wheel events"),
        (
            "_omni_render_view_wheel",
            "forward wheel events (Ctrl/Cmd zoom, plain pan) to the controls",
        ),
        (
            "ctrlKey",
            "tell a zooming wheel event from a panning one by Ctrl",
        ),
        (
            "metaKey",
            "tell a zooming wheel event from a panning one by Cmd",
        ),
        ("passive", "listen for wheel events as non-passive"),
        (
            "preventDefault",
            "keep the browser from zooming or scrolling the page",
        ),
        ("_omni_render_view_pinch", "forward touch pinches"),
        ("pointerdown", "listen for the pointer going down"),
        ("pointermove", "listen for the pointer moving"),
        ("pointerup", "listen for the pointer going up"),
        (
            "_omni_render_view_pointer_down",
            "forward the pointer going down",
        ),
        (
            "_omni_render_view_pointer_move",
            "forward the pointer moving",
        ),
        (
            "_omni_render_view_pointer_up",
            "forward the pointer going up",
        ),
        ("keydown", "listen for the space key going down"),
        ("keyup", "listen for the space key going up"),
        ("Space", "recognize the space key"),
        ("_omni_render_view_space", "forward whether space is held"),
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
fn the_readme_documents_the_level_of_detail_technique() {
    let readme = read_text(&common::repo_root().join("crates/omni-render/README.md"));
    let section = readme_section(&readme, "level of detail").unwrap_or_else(|| {
        panic!("crates/omni-render/README.md needs a \"Level of detail\" section")
    });
    let lower = section.to_lowercase();
    assert!(
        ["tile", "snapshot", "simplif"]
            .iter()
            .any(|technique| lower.contains(technique)),
        "the section must name the technique chosen (cached tiles, board snapshots or simplified drawing):\n{section}"
    );
    assert!(
        lower.contains("stress file"),
        "the section must say how the technique holds 60 frames per second on the stress file:\n{section}"
    );
    assert!(
        lower.contains("60"),
        "the section must state the 60 frames per second it holds:\n{section}"
    );
    assert!(
        lower.contains('%'),
        "the section must state the zoom (a percentage) below which it applies:\n{section}"
    );
    assert!(
        readme.contains("stress.html"),
        "the README must say how to try zooming and panning: open render-sandbox/stress.html"
    );
}
