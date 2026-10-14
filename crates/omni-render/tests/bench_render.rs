//! US-008: renderer gate A, the pass/fail performance test of the renderer.
//!
//! `make bench-render` runs `render-sandbox/bench-render.mjs`, which drives
//! headed Chrome through Playwright at 1440x900 and a device pixel ratio of 2,
//! loads the stress file (US-004) in the WebAssembly renderer, measures it 5
//! times from a cold page load, and writes the medians to
//! `docs/renderer-gate.md`. The gate's numbers and verdicts come from
//! `render-sandbox/bench-render-lib.mjs`, a plain ES module that needs no
//! browser and no Playwright, so these tests run its functions in Node:
//!
//! - `config`: `{ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2,
//!   runs: 5, limits: { boardMs: 500, fileMs: 2000, frameP95Ms: 16.7 } }`.
//! - `median(values)`: the middle value, or the mean of the two middle ones;
//!   the argument is left as it was.
//! - `percentile(values, p)`: the nearest-rank percentile, the value at
//!   position `ceil(p / 100 * n)` (1-based) of the sorted values.
//! - `frameP95(frameTimes)`: the 95th-percentile frame time.
//! - `isSoftwareRenderer(renderer)`: whether the WebGL renderer string names
//!   software rendering (SwiftShader, llvmpipe, and the like).
//! - `assertHardwareRenderer(renderer)`: throws an Error that quotes the
//!   renderer string when it is a software renderer.
//! - `evaluate(runs)`: `runs` has one `{ boardMs, fileMs, frameP95Ms }` per cold
//!   page load: boardMs is data loaded to the first complete frame of the
//!   2,000-node board, fileMs data loaded to the first frame showing every
//!   board of the 100-board file, frameP95Ms the 95th-percentile frame time of
//!   the scripted zoom and pan path. It throws unless there are exactly
//!   `config.runs` runs, and returns `{ medians, checks, pass }` where `checks`
//!   holds one `{ name, value, limit, pass }` per measurement, `value` being
//!   its median. boardMs and fileMs must be under their limits, frameP95Ms at
//!   or under its limit.
//! - `renderReport({ machine, renderer, runs, date })`: the Markdown that goes
//!   to `docs/renderer-gate.md`, headed "Gate A". It writes numbers plainly,
//!   without thousands separators (`1733`, not `1,733`), says "PASS" or
//!   "FAIL" for each limit and for the gate (never "FAIL" when all pass), and
//!   lists the machine, the WebGL renderer string, the date, the 1440x900
//!   viewport, "5 runs", every run's three measurements, their medians and
//!   the limits.
//!
//! What no test can see, the human check covers: that the numbers on the
//! product owner's machine really are under the limits, and that headed Chrome
//! uses the real GPU there.

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

/// Evaluates the JavaScript expression `expression` with the functions of the
/// benchmark library in scope, and returns its result as JSON.
fn lib(expression: &str) -> Value {
    let path = sandbox_file("bench-render-lib.mjs");
    assert!(
        path.is_file(),
        "render-sandbox/bench-render-lib.mjs must exist"
    );
    let program = format!(
        "import * as lib from {url};\n\
         const {{ config, median, percentile, frameP95, isSoftwareRenderer, \
         assertHardwareRenderer, evaluate, renderReport }} = lib;\n\
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

/// The runs `evaluate` takes: five cold page loads with these measurements.
fn runs_js(board: [u32; 5], file: [u32; 5], frame: [f64; 5]) -> String {
    let runs: Vec<String> = (0..5)
        .map(|i| {
            format!(
                "{{ boardMs: {}, fileMs: {}, frameP95Ms: {} }}",
                board[i], file[i], frame[i]
            )
        })
        .collect();
    format!("[{}]", runs.join(", "))
}

#[test]
fn make_bench_render_runs_the_playwright_script_after_building_its_inputs() {
    let commands = common::make_dry_run("bench-render");
    assert!(
        commands.contains("bench-render.mjs"),
        "`make bench-render` must run render-sandbox/bench-render.mjs:\n{commands}"
    );
    assert!(
        commands.contains("node"),
        "`make bench-render` must run the script with node:\n{commands}"
    );
    assert!(
        commands.contains("omni_render.wasm"),
        "`make bench-render` must build the WebAssembly renderer first (make render-wasm):\n{commands}"
    );
    assert!(
        commands.contains("stress-file"),
        "`make bench-render` must write the stress file first (make stress-file):\n{commands}"
    );
}

#[test]
fn the_bench_script_drives_headed_chrome_through_playwright_at_the_fixed_viewport() {
    let script = read_text(&sandbox_file("bench-render.mjs"));
    assert!(
        script.contains("playwright"),
        "bench-render.mjs must use Playwright"
    );
    assert!(
        script.contains("headless: false"),
        "bench-render.mjs must start Chrome headed (`headless: false`)"
    );
    assert!(
        script.contains("bench-render-lib.mjs"),
        "bench-render.mjs must take its viewport, pixel ratio, run count and verdicts from bench-render-lib.mjs"
    );
    for name in [
        "config",
        "evaluate",
        "renderReport",
        "assertHardwareRenderer",
    ] {
        assert!(
            script.contains(name),
            "bench-render.mjs must use `{name}` from bench-render-lib.mjs"
        );
    }
    assert!(
        script.contains("UNMASKED_RENDERER_WEBGL"),
        "bench-render.mjs must read the unmasked WebGL renderer string of the page's context"
    );
    assert!(
        script.contains("docs/renderer-gate.md"),
        "bench-render.mjs must write docs/renderer-gate.md"
    );
}

#[test]
fn playwright_is_declared_as_a_dependency_of_the_sandbox() {
    let manifest: Value = serde_json::from_str(&read_text(&sandbox_file("package.json")))
        .expect("render-sandbox/package.json is JSON");
    let declared = ["dependencies", "devDependencies"].iter().any(|section| {
        manifest[section]
            .as_object()
            .is_some_and(|deps| deps.keys().any(|name| name.contains("playwright")))
    });
    assert!(
        declared,
        "render-sandbox/package.json must declare Playwright in dependencies or devDependencies"
    );
}

#[test]
fn the_gate_runs_5_times_at_1440_by_900_and_a_pixel_ratio_of_2() {
    assert_eq!(lib("config.runs"), 5);
    assert_eq!(lib("config.viewport.width"), 1440);
    assert_eq!(lib("config.viewport.height"), 900);
    assert_eq!(lib("config.deviceScaleFactor"), 2);
}

#[test]
fn the_gate_limits_are_500_ms_2_s_and_16_7_ms() {
    assert_eq!(lib("config.limits.boardMs"), 500);
    assert_eq!(lib("config.limits.fileMs"), 2000);
    assert_eq!(lib("config.limits.frameP95Ms"), 16.7);
}

#[test]
fn software_rendering_is_recognised_by_the_webgl_renderer_string() {
    for renderer in [
        "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver)",
        "Google SwiftShader",
        "llvmpipe (LLVM 15.0.7, 256 bits)",
        "Mesa OffScreen",
        "Microsoft Basic Render Driver",
    ] {
        let js = format!(
            "isSoftwareRenderer({})",
            serde_json::to_string(renderer).unwrap()
        );
        assert_eq!(
            lib(&js),
            Value::Bool(true),
            "{renderer} is software rendering"
        );
    }
    for renderer in [
        "ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version)",
        "ANGLE (NVIDIA, NVIDIA GeForce RTX 4070 Direct3D11 vs_5_0 ps_5_0, D3D11)",
        "ANGLE (Intel, Mesa Intel(R) UHD Graphics 630 (CFL GT2), OpenGL 4.6)",
    ] {
        let js = format!(
            "isSoftwareRenderer({})",
            serde_json::to_string(renderer).unwrap()
        );
        assert_eq!(lib(&js), Value::Bool(false), "{renderer} is a GPU");
    }
}

#[test]
fn the_run_fails_when_the_renderer_string_shows_software_rendering() {
    let renderer =
        "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)";
    let js = format!(
        "(() => {{ try {{ assertHardwareRenderer({}); return null; }} catch (error) {{ return String(error.message); }} }})()",
        serde_json::to_string(renderer).unwrap()
    );
    let message = lib(&js);
    let message = message
        .as_str()
        .expect("assertHardwareRenderer must throw for a SwiftShader renderer");
    assert!(
        message.contains("SwiftShader"),
        "the error must quote the renderer string, got: {message}"
    );
}

#[test]
fn a_hardware_renderer_is_accepted() {
    let js = "(() => { assertHardwareRenderer('ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version)'); return 'ok'; })()";
    assert_eq!(lib(js), "ok");
}

#[test]
fn the_median_is_the_middle_measurement_of_an_unsorted_list() {
    assert_eq!(lib("median([431, 12, 980, 77, 300])"), 300);
    assert_eq!(lib("median([5, 1, 9])"), 5);
    assert_eq!(lib("median([7])"), 7);
}

#[test]
fn the_median_of_an_even_list_is_the_mean_of_the_middle_two() {
    assert_eq!(lib("median([10, 40, 20, 30])"), 25);
}

#[test]
fn the_median_does_not_reorder_the_measurements() {
    assert_eq!(
        lib("(() => { const values = [3, 1, 2]; median(values); return values; })()"),
        serde_json::json!([3, 1, 2])
    );
}

#[test]
fn a_percentile_is_the_nearest_rank_of_the_sorted_values() {
    assert_eq!(
        lib("percentile(Array.from({ length: 100 }, (_, i) => 100 - i), 95)"),
        95
    );
    assert_eq!(lib("percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 50)"), 5);
    assert_eq!(lib("percentile([4, 2, 9], 100)"), 9);
}

#[test]
fn the_frame_p95_ignores_the_slowest_five_percent_of_frames() {
    // 95 frames of 10 ms and 5 of 40 ms: the 95th percentile is the last fast one.
    assert_eq!(
        lib("frameP95([...Array(95).fill(10), ...Array(5).fill(40)])"),
        10
    );
    // 94 fast frames and 6 slow ones put the 95th percentile among the slow ones.
    assert_eq!(
        lib("frameP95([...Array(94).fill(10), ...Array(6).fill(40)])"),
        40
    );
}

#[test]
fn the_gate_reports_the_median_of_each_measurement_across_the_five_runs() {
    let runs = runs_js(
        [431, 120, 980, 77, 300],
        [1500, 1900, 800, 1700, 1600],
        [12.5, 15.0, 9.0, 16.0, 11.0],
    );
    let result = lib(&format!("evaluate({runs})"));
    assert_eq!(result["medians"]["boardMs"], 300);
    assert_eq!(result["medians"]["fileMs"], 1600);
    assert_eq!(result["medians"]["frameP95Ms"], 12.5);
}

#[test]
fn the_gate_passes_when_every_median_is_within_its_limit() {
    let runs = runs_js(
        [431, 450, 420, 440, 460],
        [1500, 1600, 1700, 1800, 1900],
        [12.5, 15.0, 9.0, 16.0, 11.0],
    );
    let result = lib(&format!("evaluate({runs})"));
    assert_eq!(result["pass"], true, "{result}");
    let checks = result["checks"].as_array().expect("checks is a list");
    assert_eq!(checks.len(), 3, "one check per measurement: {result}");
    for check in checks {
        assert_eq!(check["pass"], true, "{check}");
        assert!(check["limit"].is_number(), "{check}");
        assert!(check["name"].is_string(), "{check}");
    }
}

#[test]
fn a_median_over_500_ms_for_the_2000_node_board_fails_the_gate() {
    let runs = runs_js([520, 540, 510, 100, 90], [1500; 5], [10.0; 5]);
    let result = lib(&format!("evaluate({runs})"));
    assert_eq!(result["pass"], false, "{result}");
    let check = result["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "boardMs")
        .expect("a check named boardMs");
    assert_eq!(check["pass"], false);
    assert_eq!(check["value"], 510);
    assert_eq!(check["limit"], 500);
}

#[test]
fn a_board_median_of_exactly_500_ms_is_not_under_the_limit() {
    let runs = runs_js([500; 5], [1500; 5], [10.0; 5]);
    let result = lib(&format!("evaluate({runs})"));
    assert_eq!(result["pass"], false, "{result}");
}

#[test]
fn a_median_over_2_s_for_the_100_board_file_fails_the_gate() {
    let runs = runs_js([300; 5], [2100, 2200, 2050, 900, 800], [10.0; 5]);
    let result = lib(&format!("evaluate({runs})"));
    assert_eq!(result["pass"], false, "{result}");
    let check = result["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "fileMs")
        .expect("a check named fileMs");
    assert_eq!(check["pass"], false);
    assert_eq!(check["limit"], 2000);
}

#[test]
fn a_file_median_of_exactly_2_s_is_not_under_the_limit() {
    let runs = runs_js([300; 5], [2000; 5], [10.0; 5]);
    let result = lib(&format!("evaluate({runs})"));
    assert_eq!(result["pass"], false, "{result}");
}

#[test]
fn a_frame_p95_over_16_7_ms_fails_the_gate_and_exactly_16_7_ms_holds_60_fps() {
    let slow = runs_js([300; 5], [1500; 5], [16.8; 5]);
    let result = lib(&format!("evaluate({slow})"));
    assert_eq!(result["pass"], false, "{result}");
    let check = result["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "frameP95Ms")
        .expect("a check named frameP95Ms");
    assert_eq!(check["pass"], false);
    assert_eq!(check["limit"], 16.7);

    let exact = runs_js([300; 5], [1500; 5], [16.7; 5]);
    let result = lib(&format!("evaluate({exact})"));
    assert_eq!(result["pass"], true, "{result}");
}

#[test]
fn one_slow_cold_load_does_not_fail_the_gate_because_the_median_is_reported() {
    let runs = runs_js([300, 310, 4000, 290, 305], [1500; 5], [10.0; 5]);
    let result = lib(&format!("evaluate({runs})"));
    assert_eq!(result["pass"], true, "{result}");
}

#[test]
fn the_gate_needs_exactly_five_runs() {
    for count in [0, 1, 4, 6] {
        let js = format!(
            "(() => {{ try {{ evaluate(Array.from({{ length: {count} }}, () => ({{ boardMs: 1, fileMs: 1, frameP95Ms: 1 }}))); return 'no error'; }} catch (error) {{ return 'threw'; }} }})()"
        );
        assert_eq!(
            lib(&js),
            "threw",
            "evaluate must throw for {count} runs, the gate takes exactly 5"
        );
    }
}

#[test]
fn the_report_lists_the_renderer_machine_runs_medians_and_verdicts() {
    let runs = runs_js(
        [431, 450, 420, 440, 460],
        [1511, 1622, 1733, 1844, 1955],
        [12.5, 15.0, 9.0, 16.0, 11.0],
    );
    let js = format!(
        "renderReport({{ machine: 'MacBook Pro, Apple M4 Pro', renderer: 'ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version)', runs: {runs}, date: '2026-10-14' }})"
    );
    let report = lib(&js);
    let report = report.as_str().expect("renderReport returns Markdown text");
    for expected in [
        "Gate A",
        "MacBook Pro, Apple M4 Pro",
        "Apple M4 Pro, Unspecified Version",
        "2026-10-14",
        "1440",
        "900",
        "5 runs",
        "median",
        // the medians
        "440",
        "1733",
        "12.5",
        // individual runs
        "431",
        "460",
        "1511",
        "1955",
        "9.0",
        "16.0",
        // the limits
        "500",
        "2000",
        "16.7",
        "PASS",
    ] {
        assert!(
            report.to_lowercase().contains(&expected.to_lowercase()),
            "the report must contain `{expected}`:\n{report}"
        );
    }
    assert!(
        !report.contains("FAIL"),
        "a passing gate must not say FAIL:\n{report}"
    );
}

#[test]
fn the_report_says_fail_for_a_gate_that_missed_a_limit() {
    let runs = runs_js([900; 5], [1500; 5], [10.0; 5]);
    let js = format!(
        "renderReport({{ machine: 'test machine', renderer: 'ANGLE (Apple, ANGLE Metal Renderer: Apple M4 Pro, Unspecified Version)', runs: {runs}, date: '2026-10-14' }})"
    );
    let report = lib(&js);
    let report = report.as_str().expect("renderReport returns Markdown text");
    assert!(
        report.contains("FAIL"),
        "the report must say FAIL:\n{report}"
    );
    assert!(
        report.contains("900"),
        "the report must show the 900 ms median:\n{report}"
    );
}

#[test]
fn docs_renderer_gate_md_holds_a_run_of_gate_a_on_a_gpu() {
    let path = common::repo_root().join("docs/renderer-gate.md");
    let report = read_text(&path);
    let lower = report.to_lowercase();
    assert!(
        report.contains("Gate A"),
        "docs/renderer-gate.md must have a Gate A section (`make bench-render` writes it)"
    );
    assert!(lower.contains("median"), "it must report medians");
    assert!(lower.contains("5 runs"), "it must say it took 5 runs");
    assert!(
        report.contains("1440") && report.contains("900"),
        "it must record the 1440x900 viewport"
    );
    for software in ["swiftshader", "llvmpipe"] {
        assert!(
            !lower.contains(software),
            "the recorded renderer must be a GPU, not `{software}`"
        );
    }
    assert!(
        lower.contains("renderer"),
        "it must record the WebGL renderer string it measured on"
    );
}
