//! Skia renderer for OmniState designs, built natively and to WebAssembly
//! (Emscripten) so it can draw through WebGL in the browser.

pub mod sandbox;
#[cfg(target_os = "emscripten")]
pub mod web;
