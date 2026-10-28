//! US-009: renderer gate B, the layout parity check.
//!
//! `make check-layout` exports the stress file (US-004) as HTML with the export
//! of `make chrome-boxes` (`render-sandbox/layout-export.mjs`: line heights in
//! px, never `normal`, the bundled Inter loaded through `@font-face`), renders
//! it in headed Chrome through Playwright at the viewport and device pixel
//! ratio of gate A (US-008), reads every node's box with
//! `getBoundingClientRect`, compares it with the box that the WebAssembly
//! build lays out in the page (`_omni_render_layout_design`, never the native
//! build), and appends the result to `docs/renderer-gate.md` as the Gate B
//! section. The script, `render-sandbox/check-layout.mjs`, drives Chrome and
//! calls into `render-sandbox/check-layout-lib.mjs`, a plain ES module that
//! needs no browser and no Playwright, so these tests run its functions in
//! Node:
//!
//! - `config`: gate A's `viewport` and `deviceScaleFactor` (the same values,
//!   taken from `bench-render-lib.mjs`) and `tolerancePx: 2`.
//! - `prepareExport(design)`: `{ html }`, the HTML that Chrome renders for
//!   `design`, a page of boards or a board: the export of `layout-export.mjs`
//!   with the fonts of `bundledFonts()`.
//! - `compareBoxes(chrome, renderer, tolerancePx = config.tolerancePx)`: both
//!   arguments map node IDs to `{ x, y, width, height }`, as `boxesOf` of
//!   `layout-export.mjs` and `_omni_render_layout_design` give them. It throws
//!   an Error naming the IDs when the two do not hold the same nodes. It
//!   returns `{ compared, mismatches, largest, pass }`: `compared` is the
//!   number of nodes; `mismatches` has an entry `{ id, property, difference,
//!   chrome, renderer }` for each node whose largest difference of x, y,
//!   width or height is more than the tolerance (`property` is the property
//!   that differs most, `chrome` and `renderer` the two boxes), the largest
//!   difference first; `largest` is `{ id, property, difference }` for the
//!   node and property that differ most, within the tolerance or not (with a
//!   `difference` of 0 when every box is equal); `pass` is true when there is
//!   no mismatch. A difference of exactly the tolerance is within it.
//! - `renderReport({ machine, chrome, date, comparison })`: the Markdown of the
//!   Gate B section, headed `## Gate B`, for the `comparison` that
//!   `compareBoxes` returned. It reads `Result: **PASS**` or `Result:
//!   **FAIL**` (the word FAIL appears only when the gate failed), and gives
//!   the machine, the Chrome version, the date, the 1440x900 viewport and the
//!   device pixel ratio, `Nodes compared: <n>` and `Mismatches: <n>` as plain
//!   numbers, the 2 px tolerance, the largest difference with its node ID, and
//!   every mismatch with its node ID.
//! - `spliceReport(document, section)`: `document` with the section that
//!   starts at the same `## ` heading as `section` replaced by it, every other
//!   section left as it was, or `section` added after the others when
//!   `document` has none like it.
//!
//! What no test can see, the human check covers: that the boxes of Chrome and
//! of the renderer on the product owner's machine are within 2 px.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn sandbox_file(name: &str) -> PathBuf {
    common::repo_root().join("render-sandbox").join(name)
}

fn read_text(path: &PathBuf) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

/// Evaluates the JavaScript expression `expression` with the exports of
/// `render-sandbox/check-layout-lib.mjs` in scope as `lib`, and returns its
/// result as JSON.
fn lib(expression: &str) -> Value {
    let path = sandbox_file("check-layout-lib.mjs");
    assert!(
        path.is_file(),
        "render-sandbox/check-layout-lib.mjs must exist"
    );
    let program = format!(
        "import * as lib from {url};\n\
         const {{ config, prepareExport, compareBoxes, renderReport, spliceReport }} = lib;\n\
         const result = await (async () => ({expression}))();\n\
         console.log(JSON.stringify(result === undefined ? null : result));",
        url = serde_json::to_string(&format!("file://{}", path.display())).unwrap(),
    );
    let output = Command::new("node")
        .args(["--input-type=module", "-e", &program])
        .output()
        .expect("node can be started");
    assert!(
        output.status.success(),
        "evaluating `{expression}` in Node failed:\n{}",
        common::describe(&output)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "`{expression}` did not print JSON ({error}):\n{}",
            common::describe(&output)
        )
    })
}

/// A box, as JavaScript.
fn js_box(x: f64, y: f64, width: f64, height: f64) -> String {
    format!("{{ x: {x}, y: {y}, width: {width}, height: {height} }}")
}

/// A comparison that `compareBoxes` returns, written out as JavaScript for
/// `renderReport`.
fn comparison_js(
    compared: u32,
    mismatches: &[(&str, &str, f64)],
    largest: (&str, &str, f64),
) -> String {
    let entries: Vec<String> = mismatches
        .iter()
        .map(|(id, property, difference)| {
            format!(
                "{{ id: '{id}', property: '{property}', difference: {difference}, chrome: {}, renderer: {} }}",
                js_box(0.0, 0.0, 100.0, 50.0),
                js_box(0.0, 0.0, 100.0 + difference, 50.0)
            )
        })
        .collect();
    format!(
        "{{ compared: {compared}, mismatches: [{}], largest: {{ id: '{}', property: '{}', difference: {} }}, pass: {} }}",
        entries.join(", "),
        largest.0,
        largest.1,
        largest.2,
        mismatches.is_empty()
    )
}

fn report(comparison: &str) -> String {
    let js = format!(
        "renderReport({{ machine: 'MacBook Pro, Apple M4 Pro', chrome: '155.0.8059.41', date: '2026-10-14', comparison: {comparison} }})"
    );
    let report = lib(&js);
    report
        .as_str()
        .expect("renderReport returns Markdown text")
        .to_string()
}

// ---------------------------------------------------------------------------
// The make target and the script.

#[test]
fn make_check_layout_runs_the_playwright_script_after_building_its_inputs() {
    let commands = common::make_dry_run("check-layout");
    assert!(
        commands.contains("check-layout.mjs"),
        "`make check-layout` must run render-sandbox/check-layout.mjs:\n{commands}"
    );
    assert!(
        commands.contains("node"),
        "`make check-layout` must run the script with node:\n{commands}"
    );
    assert!(
        commands.contains("omni_render.wasm"),
        "`make check-layout` must build the WebAssembly renderer first (make render-wasm), since the renderer's boxes come from it:\n{commands}"
    );
    assert!(
        commands.contains("stress-file"),
        "`make check-layout` must write the stress file first (make stress-file):\n{commands}"
    );
    assert!(
        commands.contains("npm install"),
        "`make check-layout` must install Playwright, as `make bench-render` does:\n{commands}"
    );
}

#[test]
fn the_script_renders_the_export_in_headed_chrome_and_reads_getboundingclientrect() {
    let script = read_text(&sandbox_file("check-layout.mjs"));
    assert!(
        script.contains("playwright"),
        "check-layout.mjs must use Playwright"
    );
    assert!(
        script.contains("headless: false"),
        "check-layout.mjs must start Chrome headed (`headless: false`)"
    );
    assert!(
        script.contains("getBoundingClientRect"),
        "check-layout.mjs must read the box of every node with getBoundingClientRect"
    );
    assert!(
        script.contains("document.fonts"),
        "check-layout.mjs must wait for the bundled fonts to load before it measures"
    );
    assert!(
        script.contains("layout-export.mjs") || script.contains("check-layout-lib.mjs"),
        "check-layout.mjs must export the design with layout-export.mjs (`make chrome-boxes`' export), directly or through check-layout-lib.mjs"
    );
    assert!(
        script.contains("docs/renderer-gate.md"),
        "check-layout.mjs must append its result to docs/renderer-gate.md"
    );
    assert!(
        script.contains("exitCode"),
        "check-layout.mjs must exit with a failure status when a node is out of tolerance"
    );
}

#[test]
fn the_renderers_boxes_come_from_the_webassembly_build_running_in_the_page() {
    let script = read_text(&sandbox_file("check-layout.mjs"));
    let page_path = sandbox_file("check-layout.html");
    assert!(
        page_path.is_file(),
        "render-sandbox/check-layout.html must exist: the page that lays the stress file out with the WebAssembly build"
    );
    let page = read_text(&page_path);
    let module_name = page
        .split(['"', '\'', ' ', '>', '<'])
        .find(|token| token.ends_with(".js") && token.contains("check-layout"))
        .unwrap_or_else(|| {
            panic!("check-layout.html must load a script of its own (main-check-layout.js)")
        })
        .trim_start_matches("./")
        .to_string();
    let module = read_text(&sandbox_file(&module_name));
    assert!(
        module.contains("build/omni_render.js"),
        "{module_name} must load the WebAssembly module that `make render-wasm` builds"
    );
    assert!(
        module.contains("_omni_render_layout_design"),
        "{module_name} must lay the design out with the module's `_omni_render_layout_design` export"
    );
    assert!(
        module.contains("stress-file.json"),
        "{module_name} must load the stress file"
    );
    assert!(
        script.contains("check-layout.html"),
        "check-layout.mjs must open check-layout.html in Chrome to get the renderer's boxes"
    );
    for native in ["layout-boxes", "nodes-boxes", "cargo "] {
        assert!(
            !script.contains(native) && !module.contains(native),
            "the renderer's boxes must not come from the native build (found `{native}`)"
        );
    }
}

// ---------------------------------------------------------------------------
// The library: configuration and export.

#[test]
fn the_check_uses_the_viewport_and_pixel_ratio_of_gate_a_and_a_tolerance_of_2_px() {
    let config = lib("config");
    assert_eq!(config["viewport"]["width"], 1440, "{config}");
    assert_eq!(config["viewport"]["height"], 900, "{config}");
    assert_eq!(config["deviceScaleFactor"], 2, "{config}");
    assert_eq!(config["tolerancePx"], 2, "{config}");
    let gate_a = {
        let path = sandbox_file("bench-render-lib.mjs");
        let program = format!(
            "import {{ config }} from {url}; console.log(JSON.stringify(config));",
            url = serde_json::to_string(&format!("file://{}", path.display())).unwrap()
        );
        let output = Command::new("node")
            .args(["--input-type=module", "-e", &program])
            .output()
            .expect("node can be started");
        serde_json::from_slice::<Value>(&output.stdout).expect("gate A's config is JSON")
    };
    assert_eq!(
        config["viewport"], gate_a["viewport"],
        "gate B must use gate A's viewport"
    );
    assert_eq!(
        config["deviceScaleFactor"], gate_a["deviceScaleFactor"],
        "gate B must use gate A's device pixel ratio"
    );
}

#[test]
fn prepare_export_gives_the_chrome_boxes_export_with_the_renderers_bundled_inter() {
    let design = r#"{
        "id": "page0001", "type": "page", "name": "Page", "children": [
          { "id": "brd00001", "type": "board", "name": "Board", "width": { "fixed": 300 }, "height": "hug",
            "layout": { "mode": "flex", "direction": "column" },
            "children": [
              { "id": "txt00001", "type": "text", "name": "Label", "width": "hug", "height": "hug",
                "content": "Quarterly report", "fontFamily": "Inter", "fontSize": 16 }
            ] }
        ] }"#;
    let exported = lib(&format!("prepareExport(JSON.parse({design:?}))"));
    let html = exported["html"].as_str().unwrap_or_else(|| {
        panic!("prepareExport must return {{ html }}, but it returned {exported}")
    });
    assert!(
        html.contains("data-node=\"txt00001\""),
        "the export must hold an element for node txt00001"
    );
    assert!(
        html.contains("@font-face"),
        "the export must load the bundled font through @font-face"
    );
    let font = fs::read(common::repo_root().join("crates/omni-render/fonts/Inter-Regular.ttf"))
        .expect("the bundled Inter can be read");
    let encoded = base64_encode(&font);
    assert!(
        html.contains(&encoded),
        "the export must embed the very file the renderer bundles, crates/omni-render/fonts/Inter-Regular.ttf"
    );
}

/// Standard base64, which the export embeds the font as.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let triple = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                text.push(char::from(
                    ALPHABET[((triple >> (18 - 6 * index)) & 63) as usize],
                ));
            } else {
                text.push('=');
            }
        }
    }
    text
}

// ---------------------------------------------------------------------------
// The library: comparing boxes.

#[test]
fn a_difference_of_exactly_2_px_is_within_the_tolerance_and_more_is_a_mismatch() {
    let chrome = format!("{{ brd00001: {} }}", js_box(0.0, 0.0, 100.0, 50.0));
    let within = format!("{{ brd00001: {} }}", js_box(0.0, 0.0, 102.0, 48.0));
    let result = lib(&format!("compareBoxes({chrome}, {within})"));
    assert_eq!(result["pass"], true, "2 px off is within 2 px: {result}");
    assert_eq!(result["largest"]["difference"], 2, "{result}");
    assert_eq!(result["largest"]["id"], "brd00001", "{result}");

    let beyond = format!("{{ brd00001: {} }}", js_box(0.0, 0.0, 100.0, 52.5));
    let result = lib(&format!("compareBoxes({chrome}, {beyond})"));
    assert_eq!(
        result["pass"], false,
        "2.5 px off is not within 2 px: {result}"
    );
    assert_eq!(
        result["mismatches"].as_array().unwrap().len(),
        1,
        "{result}"
    );
}

#[test]
fn every_one_of_x_y_width_and_height_counts() {
    let chrome = format!("{{ a0000001: {} }}", js_box(10.0, 20.0, 30.0, 40.0));
    for (property, renderer) in [
        ("x", js_box(12.5, 20.0, 30.0, 40.0)),
        ("y", js_box(10.0, 17.0, 30.0, 40.0)),
        ("width", js_box(10.0, 20.0, 33.0, 40.0)),
        ("height", js_box(10.0, 20.0, 30.0, 36.0)),
    ] {
        let result = lib(&format!(
            "compareBoxes({chrome}, {{ a0000001: {renderer} }})"
        ));
        assert_eq!(result["pass"], false, "a {property} that differs: {result}");
        assert_eq!(
            result["mismatches"][0]["property"], property,
            "the mismatch must name the property that differs: {result}"
        );
        assert_eq!(result["mismatches"][0]["id"], "a0000001", "{result}");
    }
}

#[test]
fn a_mismatch_lists_the_node_id_both_boxes_and_the_property_that_differs_most() {
    let chrome = format!(
        "{{ brd00001: {}, a0000001: {}, b0000001: {} }}",
        js_box(0.0, 0.0, 200.0, 100.0),
        js_box(0.0, 0.0, 80.0, 50.0),
        js_box(80.0, 0.0, 40.0, 30.0)
    );
    let renderer = format!(
        "{{ brd00001: {}, a0000001: {}, b0000001: {} }}",
        js_box(0.0, 0.0, 200.0, 100.0),
        js_box(0.0, 0.0, 83.0, 51.0),
        js_box(80.0, 0.0, 40.0, 30.0)
    );
    let result = lib(&format!("compareBoxes({chrome}, {renderer})"));
    assert_eq!(result["compared"], 3, "{result}");
    assert_eq!(result["pass"], false, "{result}");
    let mismatches = result["mismatches"].as_array().unwrap();
    assert_eq!(
        mismatches.len(),
        1,
        "only the node that is out of tolerance is a mismatch: {result}"
    );
    let mismatch = &mismatches[0];
    assert_eq!(mismatch["id"], "a0000001", "{result}");
    assert_eq!(mismatch["property"], "width", "{result}");
    assert_eq!(mismatch["difference"], 3, "{result}");
    assert_eq!(mismatch["chrome"]["width"], 80, "{result}");
    assert_eq!(mismatch["renderer"]["width"], 83, "{result}");
    assert_eq!(result["largest"]["id"], "a0000001", "{result}");
    assert_eq!(result["largest"]["property"], "width", "{result}");
}

#[test]
fn boxes_for_different_nodes_cannot_be_compared() {
    for (chrome, renderer, missing) in [
        (
            format!(
                "{{ a0000001: {}, b0000001: {} }}",
                js_box(0.0, 0.0, 1.0, 1.0),
                js_box(0.0, 0.0, 1.0, 1.0)
            ),
            format!("{{ a0000001: {} }}", js_box(0.0, 0.0, 1.0, 1.0)),
            "b0000001",
        ),
        (
            format!("{{ a0000001: {} }}", js_box(0.0, 0.0, 1.0, 1.0)),
            format!(
                "{{ a0000001: {}, c0000001: {} }}",
                js_box(0.0, 0.0, 1.0, 1.0),
                js_box(0.0, 0.0, 1.0, 1.0)
            ),
            "c0000001",
        ),
    ] {
        let js = format!(
            "(() => {{ try {{ compareBoxes({chrome}, {renderer}); return 'no error'; }} catch (error) {{ return String(error.message); }} }})()"
        );
        let message = lib(&js);
        let message = message.as_str().unwrap();
        assert!(
            message.contains(missing),
            "compareBoxes must throw an error that names node {missing}, but it gave `{message}`"
        );
    }
}

// ---------------------------------------------------------------------------
// The library: the report.

#[test]
fn a_passing_comparison_reports_pass_with_the_counts_and_never_fail() {
    let report = report(&comparison_js(36629, &[], ("a1b2c3d4", "width", 0.5)));
    assert!(
        report.starts_with("## Gate B"),
        "the report must be the Gate B section:\n{report}"
    );
    assert!(report.contains("Result: **PASS**"), "{report}");
    assert!(report.contains("Nodes compared: 36629"), "{report}");
    assert!(report.contains("Mismatches: 0"), "{report}");
    assert!(
        !report.contains("FAIL"),
        "a passing gate must not say FAIL:\n{report}"
    );
}

#[test]
fn a_failed_comparison_reports_a_fail_and_lists_every_mismatch_with_its_node_id() {
    let report = report(&comparison_js(
        36629,
        &[
            ("zz000001", "height", 9.5),
            ("zz000002", "x", 3.25),
            ("zz000003", "width", 2.5),
        ],
        ("zz000001", "height", 9.5),
    ));
    assert!(report.contains("Result: **FAIL**"), "{report}");
    assert!(report.contains("Mismatches: 3"), "{report}");
    for id in ["zz000001", "zz000002", "zz000003"] {
        assert!(
            report.contains(id),
            "the report must list the mismatch of node {id}:\n{report}"
        );
    }
    assert!(report.contains("9.5"), "{report}");
    assert!(
        !report.contains("PASS"),
        "a failed gate must not say PASS:\n{report}"
    );
}

#[test]
fn the_report_is_a_section_that_an_existing_document_takes_after_gate_a() {
    let report = report(&comparison_js(10, &[], ("a0000001", "x", 0.0)));
    let gate_a = "# Renderer gate\n\nIntro.\n\n## Gate A: performance\n\nResult: **PASS**\n\n- Date: 2026-10-14\n";
    let js = format!("spliceReport({gate_a:?}, {report:?})");
    let document = lib(&js);
    let document = document.as_str().unwrap();
    assert!(
        document.starts_with("# Renderer gate\n\nIntro.\n\n## Gate A: performance\n\nResult: **PASS**\n\n- Date: 2026-10-14\n"),
        "Gate A must be left as it was:\n{document}"
    );
    let a = document.find("## Gate A").unwrap();
    let b = document.find("## Gate B").expect("Gate B is added");
    assert!(a < b, "Gate B comes after Gate A:\n{document}");
    assert_eq!(document.matches("## Gate B").count(), 1, "{document}");
}

#[test]
fn splicing_again_replaces_the_gate_b_section_and_keeps_the_sections_around_it() {
    let old = "# Renderer gate\n\n## Gate A: performance\n\nA body.\n\n## Gate B: layout parity\n\nOld result.\n\n## Gate C\n\nC body.\n";
    let section = "## Gate B: layout parity\n\nNew result.\n";
    let js = format!("spliceReport({old:?}, {section:?})");
    let document = lib(&js);
    let document = document.as_str().unwrap();
    assert!(document.contains("New result."), "{document}");
    assert!(!document.contains("Old result."), "{document}");
    assert!(
        document.contains("## Gate A: performance\n\nA body."),
        "{document}"
    );
    assert!(document.contains("## Gate C\n\nC body."), "{document}");
    assert_eq!(document.matches("## Gate B").count(), 1, "{document}");
    let order =
        ["## Gate A", "## Gate B", "## Gate C"].map(|heading| document.find(heading).unwrap());
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{document}");
}

#[test]
fn splicing_into_a_document_without_sections_keeps_it_and_adds_the_section() {
    let js =
        "spliceReport('# Renderer gate\\n\\nIntro.\\n', '## Gate B: layout parity\\n\\nBody.\\n')";
    let document = lib(js);
    let document = document.as_str().unwrap();
    assert!(document.contains("Intro."), "{document}");
    assert!(
        document.contains("## Gate B: layout parity\n\nBody."),
        "{document}"
    );
}

// ---------------------------------------------------------------------------
// The documents the run writes.

/// The text of the `## ` section of `document` whose heading starts with
/// `heading`.
fn section_of(document: &str, heading: &str) -> Option<String> {
    let start = document.find(heading)?;
    let rest = &document[start..];
    let end = rest[heading.len()..]
        .find("\n## ")
        .map_or(rest.len(), |offset| offset + heading.len());
    Some(rest[..end].to_string())
}

#[test]
fn docs_renderer_gate_md_holds_a_passing_run_of_gate_b_on_the_stress_file_after_gate_a() {
    let report = read_text(&common::repo_root().join("docs/renderer-gate.md"));
    let gate_a = report
        .find("## Gate A")
        .expect("Gate A must still be in docs/renderer-gate.md");
    let gate_b = report
        .find("## Gate B")
        .expect("docs/renderer-gate.md must have a Gate B section (`make check-layout` writes it)");
    assert!(gate_a < gate_b, "Gate B is appended after Gate A");
    let section = section_of(&report, "## Gate B").unwrap();
    assert!(
        section.contains("Result: **PASS**"),
        "every node of the stress file must be within 2 px, so the recorded result must be PASS:\n{section}"
    );
    assert!(
        !section.contains("FAIL"),
        "the recorded run must not say FAIL:\n{section}"
    );
    assert!(section.contains("Mismatches: 0"), "{section}");
    assert!(
        section.contains("2 px"),
        "it must state the tolerance:\n{section}"
    );
    assert!(
        section.contains("1440")
            && section.contains("900")
            && section.contains("device pixel ratio 2"),
        "it must record the viewport and the pixel ratio of gate A:\n{section}"
    );
    assert!(
        section.to_lowercase().contains("stress file"),
        "it must say that it compared the stress file:\n{section}"
    );
    let compared: u64 = section
        .lines()
        .find_map(|line| {
            line.trim()
                .trim_start_matches("- ")
                .strip_prefix("Nodes compared: ")
        })
        .unwrap_or_else(|| panic!("it must say `Nodes compared: <n>`:\n{section}"))
        .trim()
        .parse()
        .unwrap_or_else(|_| {
            panic!("the number of nodes compared must be a plain number:\n{section}")
        });
    assert!(
        compared > 30_000,
        "the stress file has about 36,000 nodes, but the run compared {compared}"
    );
    assert!(
        section.contains("Chrome"),
        "it must record the Chrome version:\n{section}"
    );
    let gate_a_section = section_of(&report, "## Gate A").unwrap();
    assert!(
        gate_a_section.contains("Result: **PASS**") && gate_a_section.contains("Median"),
        "the Gate A section must be left as it was:\n{gate_a_section}"
    );
}

/// The entries of the "Known differences" section of docs/formats/layout.md:
/// each bullet, with its continuation lines.
fn known_differences() -> (String, Vec<String>) {
    let layout = read_text(&common::repo_root().join("docs/formats/layout.md"));
    let section = section_of(&layout, "## Known differences")
        .expect("docs/formats/layout.md must keep its Known differences section");
    let mut entries: Vec<String> = Vec::new();
    for line in section.lines() {
        if line.starts_with("- ") {
            entries.push(line.to_string());
        } else if line.starts_with("  ") && !line.trim().is_empty() {
            if let Some(last) = entries.last_mut() {
                last.push('\n');
                last.push_str(line);
            }
        }
    }
    (section, entries)
}

#[test]
fn every_known_difference_left_in_the_layout_doc_is_confirmed_within_2_px_on_the_stress_file() {
    let (section, entries) = known_differences();
    assert!(
        !section.contains("US-009 resolves"),
        "US-009 has resolved the known differences, so the section must no longer say it will:\n{section}"
    );
    assert!(
        section.contains("renderer-gate.md"),
        "the section must point at docs/renderer-gate.md, where the stress file comparison is recorded:\n{section}"
    );
    // A difference that is fixed is taken out of the list, because the section
    // lists where the boxes still differ; one that stays says why that is fine.
    for entry in &entries {
        let lower = entry.to_lowercase();
        assert!(
            lower.contains("stress file") && lower.contains("2 px"),
            "this entry must say that it was confirmed to stay within 2 px on the stress file, or be taken out because it is fixed:\n{entry}"
        );
    }
}

#[test]
fn the_gate_b_section_accounts_for_the_known_differences() {
    let report = read_text(&common::repo_root().join("docs/renderer-gate.md"));
    let section =
        section_of(&report, "## Gate B").expect("docs/renderer-gate.md must have a Gate B section");
    let lower = section.to_lowercase();
    assert!(
        lower.contains("known differences"),
        "the Gate B section must say what became of the known differences of docs/formats/layout.md:\n{section}"
    );
    assert!(
        lower.contains("fixed") || lower.contains("within 2 px"),
        "it must say that each was fixed or confirmed to stay within 2 px:\n{section}"
    );
}

#[test]
fn the_readme_explains_how_to_run_gate_b() {
    let readme = read_text(&common::repo_root().join("crates/omni-render/README.md"));
    assert!(
        readme.contains("make check-layout"),
        "the README must show how to run the layout parity check: `make check-layout`"
    );
    assert!(
        readme.contains("Gate B") || readme.contains("gate B"),
        "the README must name the check as renderer gate B"
    );
}
