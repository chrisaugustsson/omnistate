# omni-render

The OmniState renderer. It draws with [Skia](https://skia.org) through the
`skia-safe` crate ([rust-skia](https://github.com/rust-skia/rust-skia)): natively
on a CPU raster surface (the tests), and in the browser on a WebGL 2 canvas,
compiled to WebAssembly with Emscripten for the `wasm32-unknown-emscripten`
target.

## Toolchain

A clean checkout builds on macOS with exactly these versions:

| Tool | Version | Installed by |
| --- | --- | --- |
| Rust (rustc, cargo, clippy, rustfmt) | 1.88.0 | rustup, from `rust-toolchain.toml` on first use |
| Rust standard library for `wasm32-unknown-emscripten` | 1.88.0 | rustup, from `rust-toolchain.toml` on first use |
| Emscripten SDK (emsdk, emcc) | 3.1.74 | the first `make render-wasm`, into `.emsdk/3.1.74/` |
| skia-safe and skia-bindings crates | 0.153.3 | Cargo, pinned by `Cargo.lock` |
| Skia binaries (Skia m153) | skia-bindings 0.153.3 release | the skia-bindings build script, from [rust-skia/skia-binaries](https://github.com/rust-skia/skia-binaries/releases/tag/0.153.3) |

Only [rustup](https://rustup.rs), Git, Python 3 and GNU Make (the 3.81 in the
Xcode Command Line Tools works) need to be installed beforehand.

### Skia binaries

skia-bindings downloads prebuilt Skia binaries for the target and feature set,
so neither a Skia build nor LLVM, Ninja or GNU binutils is needed. omni-render
enables the `gl` feature and keeps the default features, which selects these
archives of the 0.153.3 release:

- macOS on Apple silicon: `skia-binaries-b7f043e0b1e2a850e702-aarch64-apple-darwin-ganesh-gl-jpegd-jpege-pdf.tar.gz`
- macOS on Intel: `skia-binaries-b7f043e0b1e2a850e702-x86_64-apple-darwin-ganesh-gl-jpegd-jpege-pdf.tar.gz`
- WebAssembly: `skia-binaries-b7f043e0b1e2a850e702-wasm32-unknown-emscripten-ganesh-gl-jpegd-jpege-pdf.tar.gz`

Changing the skia-safe version or its features selects other binaries; a
combination without prebuilt binaries makes skia-bindings build Skia from
source, which needs far more tooling.

### Why Emscripten 3.1.74

The Skia WebAssembly binaries were compiled with emsdk 3.1.59 and use
WebAssembly thread-local storage. Emscripten 3.1.74, the last 3.1 release,
links them with Rust 1.88:

- It has the same libc++ (18.1.2) as 3.1.59, which the binaries were compiled
  against. Emscripten 4.0 moved to libc++ 19.
- Its linker places thread-local variables correctly in single-threaded
  programs (llvm/llvm-project#116136). With the 3.1.59 linker, Skia crashes
  with "divide by zero" as soon as it compiles its first shader.
- Its Binaryen accepts the WebAssembly feature names that Rust 1.87 and later
  (LLVM 20) emit, which the 3.1.59 Binaryen rejects.

Rust 1.88.0 uses LLVM 20, like this Emscripten, which is why
`rust-toolchain.toml` pins it. A Rust with a newer LLVM may emit WebAssembly
that Emscripten 3.1.74 cannot link.

## Build and run

```sh
make render-wasm   # installs emsdk on first use, then builds the module
python3 -m http.server 8000 --directory render-sandbox
```

Open http://localhost:8000. The page shows a WebGL canvas with a rectangle and
one line of text, drawn by Skia from Rust, and names the GPU underneath.

`make render-wasm` needs no sourced `emsdk_env.sh`: the Makefile points cargo at
the pinned `emcc`. It runs
`cargo build --release --target wasm32-unknown-emscripten -p omni-render --bin omni-render`
and copies the result to `render-sandbox/build/`:

- `omni_render.js`: an ES module whose default export, `createOmniRender()`,
  loads `omni_render.wasm` from the same folder.
- `omni_render.wasm`: Skia and omni-render.

`make test` runs every test, including `make render-wasm`. `make lint` runs
rustfmt and clippy, natively and for the WebAssembly target.

## Drawing in the browser

The page owns the canvas. It creates a WebGL 2 context with an alpha channel,
an 8-bit stencil buffer and no multisampling, registers it with
`renderer.GL.registerContext(gl, { majorVersion: 2 })`, makes it current with
`renderer.GL.makeContextCurrent()`, and then calls
`renderer._omni_render_draw_sandbox(width, height, pixelRatio)` with the
drawing buffer size in device pixels. Skia wraps the default framebuffer of that
context and draws `SandboxScene::demo()` (`src/sandbox.rs`), scaled by the
pixel ratio. Natively, `render_rgba` draws the same scene on a CPU raster
surface, which the tests check pixel by pixel.

## Fonts

There are no system fonts for Skia in the browser, so omni-render compiles in
Inter Regular (`fonts/Inter-Regular.ttf`, from the
[Inter 4.1 release](https://github.com/rsms/inter/releases/tag/v4.1)), under
the SIL Open Font License (`fonts/LICENSE.txt`). The WebAssembly build loads it
through FreeType; macOS loads it through CoreText.
