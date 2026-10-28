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
