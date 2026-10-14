# omni-render

The OmniState renderer. It draws with [Skia](https://skia.org) through the
`skia-safe` crate ([rust-skia](https://github.com/rust-skia/rust-skia)): natively
on a CPU raster surface (the tests), and in the browser on a WebGL 2 canvas,
compiled to WebAssembly with Emscripten for the `wasm32-unknown-emscripten`
target. It lays designs out with [Taffy](https://github.com/DioxusLabs/taffy)
(see [Layout](#layout)).

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
enables the `gl` feature, and the `textlayout` feature for Skia's paragraph
layout, and keeps the default features, which selects these archives of the
0.153.3 release:

- macOS on Apple silicon: `skia-binaries-b7f043e0b1e2a850e702-aarch64-apple-darwin-ganesh-gl-jpegd-jpege-pdf-textlayout.tar.gz`
- macOS on Intel: `skia-binaries-b7f043e0b1e2a850e702-x86_64-apple-darwin-ganesh-gl-jpegd-jpege-pdf-textlayout.tar.gz`
- WebAssembly: `skia-binaries-b7f043e0b1e2a850e702-wasm32-unknown-emscripten-ganesh-gl-jpegd-jpege-pdf-textlayout.tar.gz`

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

## Drawing nodes

`paint::draw` paints a laid-out design on a Skia canvas: every node in the
box that `layout::compute` gave it, one canvas pixel to a CSS pixel. The same
function draws on the CPU raster surface of `paint::render_rgba` (the tests,
which look at its pixels) and on the WebGL canvas of the browser. Its module
documentation lists what the design format leaves open: the paint order of a
frame, strokes inside the edge of boxes but centered on vectors, shadows as CSS
`box-shadow`, opacity as a group, and images fitted like CSS `object-fit`.

- Frames and boards paint shadows, fills (solid, linear gradient along CSS's
  gradient line, image), inner shadows, strokes and then their children, clipped
  to the rounded box when `clip` is set. Images paint like frames with one image,
  and vectors fill and stroke their SVG path, in pixels from the corner of their
  box.
- Instances and token references are out of scope until US-032: an instance is
  not drawn, and a color that is a token is not painted.
- The host supplies images (`Assets::add_image`, PNG or JPEG bytes for an
  `attachment:<id>`) and font files (`Assets::add_font`, for a family name). An
  image that was not supplied draws nothing. Text in a family that was not
  supplied is measured and drawn in the bundled Inter, so only Inter Regular is
  there until more weights are bundled: Skia slants it for italic text, and the
  WebAssembly build also thickens it for heavier weights, which the native build
  does not.
- Text is drawn in the lines that layout computed (`ShapedText`, `src/text.rs`):
  Skia shapes and paints each run of a line, but never breaks a line, so the lines
  are the ones Chrome breaks. Lines align left, center, right or justified inside
  the text's box, and underlines and lines through text span the whole line.

### The nodes test page

`render-sandbox/nodes.html` draws `render-sandbox/nodes.json` on a WebGL canvas
through the WebAssembly module: one example of every node type and property, in
six boards (fills, radii and strokes; shadows; opacity, clip and visibility;
layouts; text; images and vectors), each captioned with what it should look
like. Build the module and serve the folder as above, then open
http://localhost:8000/nodes.html. The page generates the images itself, because
the host supplies them, and has a file input that supplies a font for the family
"Missing Sans".

The module exports the functions that the page calls, and
`omni_render_alloc`, `omni_render_free` and `omni_render_error` to hand text and
bytes across (`src/main.rs`; the page copies them in through the exported
`HEAPU8`):

- `_omni_render_add_image` and `_omni_render_add_font` supply an image or a font.
- `_omni_render_draw_design` draws a design on the current WebGL context.
- `_omni_render_layout_design` lays a design out and returns the box of every
  node as JSON.

`make nodes-boxes` writes the boxes that the native build gives the same design
to `render-sandbox/build/nodes-boxes.json`, and the page lines them up with its
own: the page reports the largest difference of any x, y, width or height of any
node. Measured in Chrome on macOS, the largest difference between the layout
boxes of the WebAssembly build (FreeType) and the native build (CoreText) over
the 189 nodes of `nodes.json` is 0 px: both read the same Inter file with linear
metrics and no hinting, so the boxes match exactly. The measurement needs
a browser, so run it again after changing the examples, the fonts or the layout.

## Zooming, panning and level of detail

`view::Viewport` is the part of the page that the screen shows: a zoom and the
page point at the screen's top-left corner. `view::Controls` turns the page's
events into changes of it: Ctrl or Cmd with the wheel zooms around the pointer
(which is also how browsers report a trackpad pinch), a touch pinch zooms around
the fingers, the wheel alone pans, and so does dragging while the space key is
held. Zoom is exponential in the wheel delta, so a notch of 100 zooms by 22% and
events add up however they are split, and it stays between 1% and 6400%. All of
it is plain Rust, so the tests drive it natively and the browser page only
forwards events.

`view::Scene` holds a page laid out once and draws it in any viewport. It draws
the boards that reach into the view and no others, where a board reaches as far
as its drop shadows do, so culling never changes the picture. `FrameStats` says
which boards a frame drew, how many nodes it painted and at what detail.

### Level of detail

The technique is board snapshots, on top of cached pictures:

- **Full detail** (device scale of 50% or more, the zoom times the pixel ratio):
  each board in view is recorded once into a Skia picture, with a bounding box
  hierarchy, and every frame after that replays the picture. Panning and zooming
  never shape text or walk the design again, and the replay skips what is out of
  view.
- **Reduced detail** (below 50%): each board in view is drawn from a snapshot, a
  raster image of the whole board at 50% (at most 2,048 pixels on its longest
  side), made once. A snapshot is only ever reduced, so it is composited with
  mipmaps and looks like the board at any zoom down to the 1% minimum. A frame
  then draws one image per board and no nodes, however many nodes the boards
  have.

A snapshot is drawn from the board's nodes at reduced detail
(`paint::draw_reduced`), not from its picture, because it is only ever seen
small: text is not shaped but shown as a bar, its box filled with the text color
at 35% opacity, and shadows are left out. Everything else is drawn as at full
detail. Shaping the text and working out the shadows of a board is most of what
drawing it costs, and a board of 50% or less shows next to nothing of either.

Snapshots made for one zoom serve every other, so the cost is paid once per board:
the first frame at a reduced zoom paints every node of the boards in view to make
them (all 36,000 nodes of the stress file, if it is fitted in the view at the
start, in about a tenth of a second natively), and `nodes_painted` counts those.
After that a frame of the stress file fitted in view draws 100 images and paints
no nodes. That is how it holds 60 frames per second on the stress file where
drawing 36,000 nodes a frame would not: the tests pin the budget at no more than
5,000 nodes painted per frame after the first. The renderer gate measures it on a
GPU (see [Renderer gate A](#renderer-gate-a)), and the smoothness in a browser is
the human check of US-006.

Drawing a design shapes its text only once. `layout::compute_shaped` returns the
text that layout shaped to measure it, the scene keeps it, and recording a board
draws from it.

The browser module keeps one Skia GPU context for the page's WebGL context, so
the compiled shaders and the textures of the snapshots stay on the GPU between
frames. It draws the text of a view from signed distance fields, which Skia makes
once for each glyph and scales on the GPU, from 1 px up (`ContextOptions::
min_distance_field_font_size`, and device-independent fonts on the surface). Glyph
bitmaps are made for one scale, so while zooming every step made them again for
every glyph in view: at a zoom between 25% and 100% that was 20 to 90 ms a frame
on the stress file.

### The stress page

`render-sandbox/stress.html` draws the stress file on a WebGL canvas through the
WebAssembly module, and lets you zoom and pan around it. It shows the zoom, the
detail, how many boards were drawn, and how many nodes the last frame painted:

```sh
make stress-file render-wasm
python3 -m http.server 8000 --directory render-sandbox
```

Open http://localhost:8000/stress.html. Zoom with a pinch or Ctrl/Cmd + wheel,
pan with the wheel, or drag while holding Space. The page forwards its events
to the exports `_omni_render_view_wheel`, `_view_pinch`, `_view_pointer_down`,
`_view_pointer_move`, `_view_pointer_up` and `_view_space`, loads the file with
`_omni_render_scene_load`, draws with `_omni_render_scene_draw` and reads
`_omni_render_scene_stats`. Supply images and fonts before loading a scene: the
scene keeps the ones it was loaded with.

## Renderer gate A

`make bench-render` is the pass/fail performance test of the renderer (US-008,
Q35: if it cannot meet these there is no product). It needs a Mac with a GPU and
Chrome. It builds the module and the stress file, installs Playwright with npm
into `render-sandbox/` (`playwright-core`, which downloads no browser), and runs
`render-sandbox/bench-render.mjs`: headed Chrome driven by Playwright at 1440 x
900 and a device pixel ratio of 2. It stops if the WebGL renderer string shows
software rendering, such as SwiftShader, and takes each measurement 5 times from a
cold page load (a new Chrome with an empty profile for every one) on
`render-sandbox/bench.html`:

| Measurement | Limit |
| --- | --- |
| The 2,000-node board: from its data being loaded in the page to its first complete frame | under 500 ms |
| The 100-board file: from its data being loaded to the first frame showing every board | under 2 s |
| A scripted path that zooms in on the middle of the screen to 100%, pans across the boards and zooms out again: the 95th-percentile frame time | 16.7 ms or less (60 frames per second) |

A frame is complete when the GPU has finished it: the page draws and then waits
for `gl.finish()` and a pixel read-back. The first frame of a measurement includes
what Skia does once, such as making the GPU context and compiling shaders. The
verdict is the median of the 5 runs, which the Gate A section of
`docs/renderer-gate.md` reports with every run, the machine and the WebGL renderer
string. `render-sandbox/bench-render-lib.mjs` holds the numbers and the report, and
the tests run it in Node.

## Selection

Comments pin to the exact node that was clicked, so `view::Scene` answers which
node that is and keeps the reviewer's hover and selection (the types are in
`select`).

`nodeAt` (`Scene::node_at`) takes a page point and returns the topmost visible
node there: the deepest node whose box holds the point, with later siblings
above earlier ones. Hidden nodes and their subtrees cannot be hit, a node cannot
be hit outside an ancestor that clips, and a node that overflows an ancestor that
does not clip can. It is answered from an index of node boxes built when the
scene is: every node knows the extent of itself and its visible descendants, so a
query skips whole subtrees and takes microseconds on the 2,000-node board of the
stress file (the tests pin the 99th percentile under 1 ms).

The selection follows these rules. The context is the node whose children a click
selects among, always the parent of the selection (the board is its own context):

- A click selects the direct child of the context that holds the node under the
  pointer, or the board itself if no child of the context does. The context
  starts at the board, and a click on another board starts over there.
- A double-click makes the selected node the context and selects its child under
  the pointer. With no child there, nothing changes.
- Esc selects the parent and moves the context up one level. On a board it clears
  the selection, and with nothing selected it does nothing.
- Cmd or Ctrl + click selects the deepest node under the pointer, and its parent
  becomes the context.
- Clicking where there is no board clears the selection and resets the context.

Hover is what a click would select, so it follows the context and goes to the
deepest node while Cmd or Ctrl is held. Selection input takes screen points and a
`Viewport`, so a click lands on the node that is drawn under the pointer at any
zoom and pan.

The host page hears about changes through `Scene::take_events`, which gives an
`Event::Hover` or `Event::Select` with the node's address each time the hovered
or the selected node actually changes, oldest first. `boundsOf`
(`Scene::bounds_of`) gives the box of a node address in page coordinates. An
address is `<id>`, or `<instanceId>/<componentNodeId>` for a node inside a
component instance; the bounds of the latter wait for components to be resolved
(US-032).

The overlay is the outlines of the hovered and the selected node, labeled with
the node's name: `Scene::overlay(&viewport)` gives their boxes in screen pixels
of that viewport, so the page draws them in an element above the canvas and, by
asking again for every frame, keeps them aligned while zooming and panning.

The WebAssembly module exports `_omni_render_select_pointer_move`,
`_select_click`, `_select_double_click` and `_select_escape` for the input,
`_omni_render_take_events` and `_omni_render_overlay` for what the page draws and
hears (JSON arrays), and `_omni_render_bounds_of` (the box of an address, in page
and in canvas pixels, as JSON). The stress page wires them to click, dblclick,
pointermove and Escape listeners, draws the outlines in `#overlay`, and shows the
selected node and its size.

## Layout

`layout::compute` gives every node of a design its box (x, y, width and
height, in page coordinates) by the CSS that `docs/formats/layout.md` maps
each layout property to, so designs lay out the way a browser lays out the
same design written as HTML and CSS. Taffy runs the flexbox, grid and
absolute positioning; `src/layout/style.rs` holds the mapping.

Text is shaped by Skia's paragraph layout (`src/text.rs`), which measures
each piece of a text between two places where a line may break. Lines then
take as many pieces as fit, the way Chrome fills lines, with line heights and
baselines rounded the way Chrome rounds them. Skia's own line breaking cannot
stand in for Chrome's: it breaks words that are wider than their line, and
breaks after slashes. So `src/text/breaks.rs` finds the places where Chrome
breaks lines: Blink's rules between two ASCII characters, and Unicode's line
breaking algorithm (UAX #14, through the `unicode-linebreak` crate) next to
any other character.

Layout owns its Taffy tree (`src/layout/tree.rs`) instead of using Taffy's
`TaffyTree`, for three reasons:

- Where the space available decides the width of a row flex container, CSS
  gives the row its fit-content width, bounded by the row's min and max
  widths. Taffy makes a single-line row as wide as its items' flex bases add
  up to and a row that wraps as wide as its widest line, and it wraps the
  items at the row's max width while it measures them. The tree lays such a
  row out at its CSS width.
- Taffy lets a parent's layout make a box smaller than its padding, such as a
  column that stretches a child across less than its padding. CSS never
  does, and neither does the tree.
- Taffy's cache keeps 9 measured sizes per node, too few for nested rows and
  columns, which then measure the same sizes again at a cost that grows with
  every level of nesting. The tree keeps every size it measures during a
  layout.

### Chrome's boxes

Every box that the layout tests expect is one that headless Chrome gives the
same design written as HTML and CSS, by the mapping in
`docs/formats/layout.md`. `make chrome-boxes FILE=<design.json>` exports a
design that way (`render-sandbox/layout-export.mjs`), renders it in headless
Chrome at a device pixel ratio of 2 (`render-sandbox/chrome-boxes.mjs`), and
prints the box of every node as JSON, so a test's expected value is what the
tool prints, never a number worked out by hand:

```sh
make chrome-boxes FILE=card.json   # the board of the example in layout.md
```

The tool needs Node.js 22 or later and Google Chrome, which `make test` uses
too, for the tests of the tool. The expected boxes were measured with Chrome
155, at `/Applications/Google Chrome.app` on macOS. `CHROME` names the binary
elsewhere, and the tool says so when it runs another version of Chrome.

## Fonts

There are no system fonts for Skia in the browser, so omni-render compiles in
Inter Regular (`fonts/Inter-Regular.ttf`, from the
[Inter 4.1 release](https://github.com/rsms/inter/releases/tag/v4.1)), under
the SIL Open Font License (`fonts/LICENSE.txt`). The WebAssembly build loads it
through FreeType; macOS loads it through CoreText. Text in a font family that
the renderer has no font files for is set in Inter, so it measures the same
on every machine.
