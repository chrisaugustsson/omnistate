CARGO ?= cargo

# The Emscripten SDK that links the WebAssembly renderer; crates/omni-render/
# README.md explains the pinned version. The first `make render-wasm` installs
# it into .emsdk/<version>/, so no emsdk_env.sh needs to be sourced.
EMSDK_VERSION := 3.1.74
EMSDK := $(CURDIR)/.emsdk/$(EMSDK_VERSION)
EMCC := $(EMSDK)/upstream/emscripten/emcc
# emcc runs on the Python that emsdk bundles on macOS, as emsdk_env.sh sets up.
EMSDK_PYTHON = $(firstword $(wildcard $(EMSDK)/python/*/bin/python3))

WASM_TARGET := wasm32-unknown-emscripten
WASM_OUT := $(or $(CARGO_TARGET_DIR),target)/$(WASM_TARGET)/release
SANDBOX_BUILD := render-sandbox/build

.PHONY: lint test render-wasm chrome-boxes stress-file nodes-boxes bench-render check-layout penpot-compare

# The last clippy run covers the WebAssembly-only code of omni-render.
lint:
	$(CARGO) fmt --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) clippy -p omni-render --target $(WASM_TARGET) --lib --bins -- -D warnings

test:
	$(CARGO) test --workspace

# Builds omni-render for the browser into render-sandbox/build/.
render-wasm: $(EMCC)
	EMSDK="$(EMSDK)" EMSDK_PYTHON="$(EMSDK_PYTHON)" CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_LINKER="$(EMCC)" \
		$(CARGO) build --release --target $(WASM_TARGET) -p omni-render --bin omni-render
	mkdir -p $(SANDBOX_BUILD)
	cp $(WASM_OUT)/omni-render.js $(SANDBOX_BUILD)/omni_render.js
	cp $(WASM_OUT)/omni_render.wasm $(SANDBOX_BUILD)/omni_render.wasm

# Prints the box of every node of the design in FILE as headless Chrome lays it
# out at a device pixel ratio of 2: one JSON object on standard output, the boxes
# the layout tests expect (render-sandbox/chrome-boxes.mjs).
chrome-boxes:
	@test -n "$(FILE)" || { echo "usage: make chrome-boxes FILE=<design.json>" >&2; exit 2; }
	@node render-sandbox/chrome-boxes.mjs "$(FILE)"

# Writes the stress file, a large design file with 100 boards that every
# renderer story is tested on (crates/omni-core/src/bin/stress-file.rs). SEED
# picks the file, the same seed always giving the same bytes, and OUT where it
# goes, relative to the repository root.
stress-file:
	$(CARGO) run --quiet -p omni-core --bin stress-file -- $(if $(SEED),--seed $(SEED)) --out $(or $(OUT),$(SANDBOX_BUILD)/stress-file.json)

# Writes the boxes that the native build lays out the nodes test page's design
# in (render-sandbox/nodes.json), which render-sandbox/nodes.html compares with
# the boxes of the WebAssembly build.
nodes-boxes:
	mkdir -p $(SANDBOX_BUILD)
	$(CARGO) run --quiet -p omni-render --example layout-boxes -- render-sandbox/nodes.json > $(SANDBOX_BUILD)/nodes-boxes.json

# Renderer gate A (US-008): measures the stress file in headed Chrome through
# Playwright and writes docs/renderer-gate.md. It needs a Mac with a GPU and
# Chrome, and fails if the WebGL renderer is software rendering or a limit is
# missed. The first run installs Playwright (without a browser) with npm.
bench-render: render-wasm stress-file
	npm install --prefix render-sandbox --no-audit --no-fund --loglevel=error
	node render-sandbox/bench-render.mjs

# Renderer gate B (US-009): renders the stress file in headed Chrome through
# Playwright and compares every node's box with the one that the WebAssembly
# build of the renderer lays out, then appends the result to
# docs/renderer-gate.md. Like `make bench-render` it needs a Mac with Chrome,
# and the first run installs Playwright (without a browser) with npm. It fails
# if a node is more than 2 px from Chrome's box.
check-layout: render-wasm stress-file
	npm install --prefix render-sandbox --no-audit --no-fund --loglevel=error
	node render-sandbox/check-layout.mjs

# Penpot comparison (US-104): converts every .penpot export in PENPOT_FIXTURES,
# renders and compares each board with Penpot's own PNG, and writes the report
# page, PENPOT_REPORT/index.html, with its images (docs/penpot-compare.md).
PENPOT_FIXTURES ?= fixtures/penpot
PENPOT_REPORT ?= docs/penpot-compare

penpot-compare:
	$(CARGO) run --quiet -p omni-penpot -- report "$(PENPOT_FIXTURES)" --out "$(PENPOT_REPORT)"

$(EMCC):
	test -d "$(EMSDK)" || git clone --depth 1 --branch $(EMSDK_VERSION) https://github.com/emscripten-core/emsdk.git "$(EMSDK)"
	"$(EMSDK)/emsdk" install $(EMSDK_VERSION)
	"$(EMSDK)/emsdk" activate $(EMSDK_VERSION)
