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
# Where the web app's design viewer loads the renderer from (served as /render/).
WEB_RENDER := web/public/render

.PHONY: lint test web-deps web-api render-wasm chrome-boxes stress-file nodes-boxes bench-render check-layout penpot-compare bench-ops bench-ordrar install-agent-tools

# The last clippy run covers the WebAssembly-only code of omni-render.
lint: web-deps
	$(CARGO) fmt --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	$(CARGO) clippy -p omni-render --target $(WASM_TARGET) --lib --bins -- -D warnings
	pnpm -r typecheck
	pnpm -r lint

# Installs the web app's dependencies from pnpm-lock.yaml (a no-op when they are current).
web-deps:
	pnpm install --frozen-lockfile

# Writes the OpenAPI document the API serves (web/openapi.json) and the
# TypeScript types and Zod schemas that the web app generates from it
# (web/src/api/generated.ts). Run it after changing a route or a body; the API's
# openapi tests and the web app's tests fail while either file is out of date.
web-api: web-deps
	$(CARGO) run --quiet -p omni-api -- openapi > web/openapi.json.tmp
	mv web/openapi.json.tmp web/openapi.json
	pnpm --filter omni-web generate

# The integration tests run in the test container inside the Compose network:
# the stack is (re)built and started first, and left running afterwards. The
# operation engine's tests (crates/omni-core/tests/ops.rs) run in WebAssembly
# too, in Node; that needs wasm-pack (cargo install wasm-pack) and the
# wasm32-unknown-unknown target that rust-toolchain.toml lists.
test: web-deps
	$(CARGO) test --workspace
	cd crates/omni-core && wasm-pack test --node -- --test ops
	pnpm -r test
	node --test workflows/*.test.mjs
	docker compose up --build --wait --detach proxy
	docker compose run --rm test

# Builds omni-render for the browser into render-sandbox/build/, and copies it
# to web/public/render/ for the web app's design viewer.
render-wasm: $(EMCC)
	EMSDK="$(EMSDK)" EMSDK_PYTHON="$(EMSDK_PYTHON)" CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_LINKER="$(EMCC)" \
		$(CARGO) build --release --target $(WASM_TARGET) -p omni-render --bin omni-render
	mkdir -p $(SANDBOX_BUILD)
	cp $(WASM_OUT)/omni-render.js $(SANDBOX_BUILD)/omni_render.js
	cp $(WASM_OUT)/omni_render.wasm $(SANDBOX_BUILD)/omni_render.wasm
	mkdir -p $(WEB_RENDER)
	cp $(SANDBOX_BUILD)/omni_render.js $(SANDBOX_BUILD)/omni_render.wasm $(WEB_RENDER)/

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

# One operation at the command line (US-022, Q31): builds a doc of 1,000 blocks
# in a draft, runs 500 single `set` operations on it one after the other, each
# with its own `omni doc edit`, and prints the 95th percentile of the time as
# the CLI measures it. It fails if that is 50 ms or more. It runs in the test
# container against the warm local stack (Postgres and RustFS), in release mode
# like the stack's own API, and on its own, so nothing else skews the times.
bench-ops:
	docker compose up --build --wait --detach proxy
	docker compose run --rm test cargo test --release -p omni-cli --features stack-tests --test stack_bench_ops --test stack_bench_design -- --nocapture

# How Ordrar loads and moves (US-117): starts a stack of its own (Compose project
# omnistate-bench, a release API and a production web app, fresh volumes), imports
# fixtures/penpot/tirebud-ordrar.penpot into a new project, and five times opens
# the design in a new headed Chrome with an empty cache after restarting the API.
# It prints the cold time to first board, the median and 95th-percentile frame
# time of a scripted 10 s pan and zoom, and the API's peak memory, and writes
# docs/bench-ordrar.md. Like `make bench-render` it needs a Mac with a GPU and
# Chrome, and the first run installs Playwright (without a browser) with npm.
# It reports the numbers against the limits of FR-28 and does not fail on them.
bench-ordrar: render-wasm
	$(CARGO) build --release --quiet -p omni-cli
	npm install --prefix render-sandbox --no-audit --no-fund --loglevel=error
	node render-sandbox/bench-ordrar.mjs

# Penpot comparison (US-104): converts every .penpot export in PENPOT_FIXTURES,
# renders and compares each board with Penpot's own PNG, and writes the report
# page, PENPOT_REPORT/index.html, with its images (docs/penpot-compare.md).
PENPOT_FIXTURES ?= fixtures/penpot
PENPOT_REPORT ?= docs/penpot-compare

penpot-compare:
	$(CARGO) run --quiet -p omni-penpot -- report "$(PENPOT_FIXTURES)" --out "$(PENPOT_REPORT)"

# Links the agent skills (agent-skills/*) into ~/.claude/skills/ and the
# workflows (workflows/*.js) into ~/.claude/workflows/ as absolute symlinks.
# Running it again changes nothing; a link to somewhere else is replaced, and a
# real file or directory in the way is reported and left untouched.
CLAUDE_HOME ?= $(HOME)/.claude

install-agent-tools:
	@mkdir -p "$(CLAUDE_HOME)/skills" "$(CLAUDE_HOME)/workflows"
	@for src in $(CURDIR)/agent-skills/*/; do \
		[ -d "$$src" ] || continue; src=$${src%/}; dst="$(CLAUDE_HOME)/skills/$$(basename "$$src")"; \
		if [ -e "$$dst" ] && [ ! -L "$$dst" ]; then echo "skipped $$dst: exists and is not a link"; \
		else ln -sfn "$$src" "$$dst"; fi; \
	done
	@for src in $(CURDIR)/workflows/*.js; do \
		[ -f "$$src" ] || continue; dst="$(CLAUDE_HOME)/workflows/$$(basename "$$src")"; \
		if [ -e "$$dst" ] && [ ! -L "$$dst" ]; then echo "skipped $$dst: exists and is not a link"; \
		else ln -sfn "$$src" "$$dst"; fi; \
	done

$(EMCC):
	test -d "$(EMSDK)" || git clone --depth 1 --branch $(EMSDK_VERSION) https://github.com/emscripten-core/emsdk.git "$(EMSDK)"
	"$(EMSDK)/emsdk" install $(EMSDK_VERSION)
	"$(EMSDK)/emsdk" activate $(EMSDK_VERSION)
