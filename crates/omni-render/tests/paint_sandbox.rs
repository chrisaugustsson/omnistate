//! US-005: the WebAssembly module that `make render-wasm` builds for the
//! visual test page in `render-sandbox/` exports the functions the host page
//! calls:
//!
//! - `omni_render_add_image` and `omni_render_add_font`: the page supplies the
//!   images that `attachment:<id>` references name, and font files.
//! - `omni_render_draw_design`: draws a design on the page's WebGL canvas.
//! - `omni_render_layout_design`: lays a design out and returns its boxes, so
//!   the page can measure them against the native build's.
//!
//! What no test can see, the human check covers: that every example on the
//! page looks right in a browser.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

fn sandbox_dir() -> PathBuf {
    common::repo_root().join("render-sandbox")
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

// ---------------------------------------------------------------------------
// The page and the module it loads
// ---------------------------------------------------------------------------

/// The WebAssembly renderer that `make render-wasm` built before this test run.
fn render_wasm() {
    common::render_wasm();
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
