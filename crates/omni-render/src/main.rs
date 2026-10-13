//! The omni-render WebAssembly module that `make render-wasm` builds.
//! Emscripten links programs, not libraries, into a JavaScript loader plus a
//! `.wasm` file, so this program exports the functions the browser calls:
//! rustc hands its `#[unsafe(no_mangle)]` functions to Emscripten's
//! `EXPORTED_FUNCTIONS`.

/// Draws the sandbox scene on the default framebuffer of the page's current
/// WebGL 2 context: `width` x `height` device pixels, with `pixel_ratio`
/// device pixels per canvas pixel. Returns whether Skia drew it.
#[cfg(target_os = "emscripten")]
#[unsafe(no_mangle)]
pub extern "C" fn omni_render_draw_sandbox(width: i32, height: i32, pixel_ratio: f32) -> bool {
    omni_render::web::draw_sandbox(width, height, pixel_ratio)
}

/// Nothing runs at startup: the page calls the exported functions once it has
/// made its WebGL context current.
fn main() {}
