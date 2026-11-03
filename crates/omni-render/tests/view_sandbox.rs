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

use std::fs;
use std::path::{Path, PathBuf};

fn sandbox_dir() -> PathBuf {
    common::repo_root().join("render-sandbox")
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

/// The WebAssembly renderer that `make render-wasm` built before this test run.
fn render_wasm() {
    common::render_wasm();
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
