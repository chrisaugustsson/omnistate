//! Emscripten link settings for the omni-render WebAssembly module.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("emscripten") {
        return;
    }
    for setting in [
        // An ES module whose default export, createOmniRender(), instantiates it.
        "MODULARIZE=1",
        "EXPORT_ES6=1",
        "EXPORT_NAME=createOmniRender",
        "ENVIRONMENT=web",
        "ALLOW_MEMORY_GROWTH=1",
        // Skia draws through a WebGL 2 context that the page creates and makes
        // current with the exported GL object.
        "MIN_WEBGL_VERSION=2",
        "MAX_WEBGL_VERSION=2",
        "EXPORTED_RUNTIME_METHODS=GL",
    ] {
        println!("cargo::rustc-link-arg-bins=-s{setting}");
    }
}
