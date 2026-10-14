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

.PHONY: lint test render-wasm chrome-boxes stress-file

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

$(EMCC):
	test -d "$(EMSDK)" || git clone --depth 1 --branch $(EMSDK_VERSION) https://github.com/emscripten-core/emsdk.git "$(EMSDK)"
	"$(EMSDK)/emsdk" install $(EMSDK_VERSION)
	"$(EMSDK)/emsdk" activate $(EMSDK_VERSION)
