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
        // Reading, resolving, laying out and painting a design recurse into
        // its nodes, at about 5 KB of this stack for every level in a release
        // build. Emscripten's default of 64 KB overflowed at about 10 levels
        // and, as static data lies below the stack in an optimized build,
        // overwrote the allocator's state, which surfaced later as "memory
        // access out of bounds". 4 MB holds `nesting::MAX_NESTING` (128)
        // levels more than five times over; resolving refuses deeper designs.
        "STACK_SIZE=4194304",
        // Should the stack still overflow, abort with "stack overflow" at once
        // instead of corrupting memory: the module checks the stack pointer
        // whenever a function moves it.
        "STACK_OVERFLOW_CHECK=2",
        // Skia draws through a WebGL 2 context that the page creates and makes
        // current with the exported GL object.
        "MIN_WEBGL_VERSION=2",
        "MAX_WEBGL_VERSION=2",
        // The page registers its WebGL context through GL, copies designs,
        // images and fonts into the module's memory through HEAPU8, and reads
        // the text that the module returns with UTF8ToString.
        "EXPORTED_RUNTIME_METHODS=GL,HEAPU8,UTF8ToString",
    ] {
        println!("cargo::rustc-link-arg-bins=-s{setting}");
    }
}
