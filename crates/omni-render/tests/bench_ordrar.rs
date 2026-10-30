//! US-117: `make bench-ordrar`. What its numbers mean, run in Node on the real
//! page script and module through `tests/common/bench-ordrar-harness.mjs`; the
//! path's wheel events are replayed on the real view controls. The Chrome and
//! Docker side only `make bench-ordrar` itself can show.

mod common;

use std::process::Command;

use omni_render::layout::LayoutBox;
use omni_render::view::{Controls, MIN_ZOOM, Viewport, Wheel};
use serde_json::{Value, json};

/// Runs `body`, the body of an async JavaScript function, with the functions of
/// `render-sandbox/bench-ordrar-lib.mjs` and the harness in scope, and returns
/// what it returns as JSON.
fn node(body: &str) -> Value {
    let root = common::repo_root();
    let url = |path: &str| serde_json::to_string(&format!("file://{}/{path}", root.display()));
    let program = format!(
        "import {{ config, splitLoad, evaluate, memUsageMiB }} from {lib};\n\
         import {{ openPage, firstBoardShown, frame, framesOf, run, report }} from {harness};\n\
         const result = await (async () => {{ {body} }})();\n\
         console.log(JSON.stringify(result === undefined ? null : result));",
        lib = url("render-sandbox/bench-ordrar-lib.mjs").unwrap(),
        harness = url("crates/omni-render/tests/common/bench-ordrar-harness.mjs").unwrap(),
    );
    let output = Command::new("node")
        .args(["--input-type=module", "-e", &program])
        .output()
        .expect("node can be started");
    assert!(
        output.status.success(),
        "`{body}` failed in Node:\n{}",
        common::describe(&output)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("`{body}` did not print JSON ({error})"))
}

/// A stalled frame as the viewer makes it: `renderer.show` lays the page out
/// twice in 1132 ms, the page's other work takes 200 ms more, then the draw
/// takes 30 ms, 4 of them submitting to the GPU, and `gl.finish()` 6 ms.
const STALLED_FRAME: &str = "
    const page = openPage();
    page.startPath();
    page.draw({ ms: 12 });
    page.setSystem({ ms: 575, layoutMs: 550 });
    page.sceneLoad({ ms: 557, layoutMs: 500 });
    page.stall(200);
    page.draw({ ms: 30, submitMs: 4, finish: 6 });
    const { frames } = await page.endPath();
    return frames[1];
";

#[test]
fn a_frame_is_charged_with_all_the_time_since_the_previous_frame() {
    assert_eq!(node(STALLED_FRAME)["ms"], 1368);
}

#[test]
fn laying_the_scene_out_for_a_design_system_is_charged_to_parsing_and_layout() {
    let frame = node(STALLED_FRAME);
    assert_eq!(frame["parse"], 25 + 57, "{frame}");
    assert_eq!(frame["layout"], 550 + 500, "{frame}");
}

#[test]
fn time_no_phase_holds_is_the_frames_other_so_its_phases_add_up_to_it() {
    let frame = node(STALLED_FRAME);
    assert_eq!(frame["paint"], 26, "{frame}");
    assert_eq!(frame["gpu"], 10, "{frame}");
    assert_eq!(frame["other"], 200, "{frame}");
}

/// When the clock stops on a page that draws `frame`, and 60 ms later the first
/// board complete, which is done 147 ms in, and the first board once more.
fn clock_stop_after(frame: &str) -> Value {
    node(&format!(
        "const page = openPage();
         page.draw({{ ms: 40, drawn: {frame} }});
         page.stall(60);
         page.draw({{ ms: 40, finish: 7, drawn: firstBoardShown() }});
         page.draw({{ ms: 40, finish: 7, drawn: firstBoardShown() }});
         return page.bench.firstBoard;"
    ))
}

#[test]
fn the_clock_stops_when_the_first_board_stands_complete_and_not_at_its_placeholder() {
    assert_eq!(clock_stop_after("firstBoardShown({ nodes: 1 })"), 147);
}

#[test]
fn the_reduced_snapshot_of_the_first_board_does_not_stop_the_clock() {
    let frame = "{ ...firstBoardShown(), boardsDrawn: 5, nodesPainted: 107, detail: 'reduced', zoom: 0.0134 }";
    assert_eq!(clock_stop_after(frame), 147);
}

#[test]
fn a_frame_of_another_board_does_not_stop_the_clock() {
    assert_eq!(clock_stop_after("firstBoardShown({ id: 'nb68dacy' })"), 147);
}

#[test]
fn the_first_board_with_an_image_missing_does_not_stop_the_clock() {
    assert_eq!(
        clock_stop_after("firstBoardShown({ missingImages: 1 })"),
        147
    );
}

#[test]
fn the_first_board_cut_off_by_the_view_does_not_stop_the_clock() {
    assert_eq!(clock_stop_after("firstBoardShown({ whole: false })"), 147);
}

/// The view after one wheel event of the path, `at` ms into the page's clock.
struct View {
    at: f64,
    zoom: f32,
    rect: LayoutBox,
}

/// Plays the path over the Ordrar page, 8118 by 329640 px, on a 1440x900 canvas
/// that shows the first board at zoom 1, and replays its wheel events on the
/// real controls.
fn fly(tick_ms: u32) -> Vec<View> {
    let wheels = node(&format!("return await openPage().flyPath({tick_ms});"));
    let mut controls = Controls::new(Viewport::at(0.0, 0.0, 1.0, 1440.0, 900.0));
    wheels
        .as_array()
        .expect("the page's wheel events")
        .iter()
        .map(|event| {
            controls.wheel(Wheel {
                delta_x: event["deltaX"].as_f64().unwrap() as f32,
                delta_y: event["deltaY"].as_f64().unwrap() as f32,
                x: event["clientX"].as_f64().unwrap() as f32,
                y: event["clientY"].as_f64().unwrap() as f32,
                ctrl: event["ctrlKey"].as_bool().unwrap(),
                meta: false,
            });
            let viewport = controls.viewport();
            View {
                at: event["at"].as_f64().unwrap(),
                zoom: viewport.zoom(),
                rect: viewport.visible_rect(),
            }
        })
        .collect()
}

fn lowest_zoom(path: &[View]) -> f32 {
    path.iter().map(|view| view.zoom).fold(f32::MAX, f32::min)
}

/// The view when the path's clock reached `at`.
fn view_at(path: &[View], at: f64) -> &View {
    path.iter()
        .rfind(|view| view.at == at)
        .expect("the path moved at that time")
}

fn assert_near(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} is not within {tolerance} of {expected}"
    );
}

#[test]
fn the_path_goes_out_to_the_viewers_minimum_zoom_and_back_to_the_opening_board() {
    let path = fly(16);
    assert_near(lowest_zoom(&path), MIN_ZOOM, MIN_ZOOM * 0.01);
    let end = path.last().expect("the path moved the view");
    assert_near(end.zoom, 1.0, 0.1);
    assert_near(end.rect.x, 0.0, 144.0);
    assert_near(end.rect.y, 0.0, 90.0);
}

#[test]
fn the_pan_at_the_viewers_minimum_zoom_passes_over_the_whole_page() {
    let path = fly(16);
    let far_out: Vec<&LayoutBox> = path
        .iter()
        .filter(|view| view.zoom <= MIN_ZOOM * 1.01)
        .map(|view| &view.rect)
        .collect();
    let left = far_out.iter().map(|rect| rect.x).fold(f32::MAX, f32::min);
    let top = far_out.iter().map(|rect| rect.y).fold(f32::MAX, f32::min);
    let right = far_out
        .iter()
        .map(|rect| rect.x + rect.width)
        .fold(f32::MIN, f32::max);
    let bottom = far_out
        .iter()
        .map(|rect| rect.y + rect.height)
        .fold(f32::MIN, f32::max);
    assert!(left <= 0.0 && right >= 8118.0, "{left} to {right}");
    assert!(top <= 1000.0 && bottom >= 328_640.0, "{top} to {bottom}");
}

#[test]
fn the_path_is_the_same_at_any_frame_rate_and_takes_10_s() {
    let smooth = fly(16);
    let stuttering = fly(40);
    let (a, b) = (view_at(&smooth, 4000.0), view_at(&stuttering, 4000.0));
    assert_near(a.zoom, b.zoom, MIN_ZOOM * 0.01);
    assert_near(a.rect.y, b.rect.y, 100.0);
    assert_eq!(smooth.last().unwrap().at, 10_000.0);
    assert_eq!(stuttering.last().unwrap().at, 10_000.0);
}

#[test]
fn a_recorded_frame_holds_the_boards_in_view_and_the_zoom() {
    let frame = node(
        "const page = openPage();
         page.startPath();
         page.draw({ ms: 12, drawn: { boardsDrawn: 347, zoom: 0.0103 } });
         const { frames } = await page.endPath();
         return frames[0];",
    );
    assert_eq!(frame["boardsDrawn"], 347);
    assert_eq!(frame["zoom"], 0.0103);
}

#[test]
fn the_report_states_how_many_boards_are_in_view_at_the_viewers_limit() {
    let report = node(
        "const boards = (boardsDrawn) => run({ frames: [frame(12, { boardsDrawn: 20, zoom: 1 }), frame(12, { boardsDrawn, zoom: 0.01 })] });
         return report({
           boards: 797,
           boardsOnPage: 769,
           runs: [boards(341), boards(350), boards(345), boards(348), boards(344)],
         });",
    );
    let report = report.as_str().expect("the report is text");
    assert!(
        report.lines().any(|line| line.contains("in view")
            && line.contains("350")
            && line.contains("769")
            && line.contains("0.010")),
        "the report must state the 350 of 769 boards in view at zoom 0.010:\n{report}"
    );
}

#[test]
fn the_bench_prints_the_medians_of_the_runs_and_the_highest_memory_peak() {
    let result = node(
        "return evaluate([
           run({ firstBoardMs: 1300, frames: framesOf(12, 1300), peakMemoryMiB: 250 }),
           run({ firstBoardMs: 900, frames: framesOf(10, 1400), peakMemoryMiB: 280 }),
           run({ firstBoardMs: 2190, frames: framesOf(16, 1250), peakMemoryMiB: 260 }),
           run({ firstBoardMs: 1100, frames: framesOf(11, 1350), peakMemoryMiB: 240 }),
           run({ firstBoardMs: 1000, frames: framesOf(13, 1500), peakMemoryMiB: 270 }),
         ]);",
    );
    assert_eq!(result["medians"]["firstBoardMs"], 1100);
    assert_eq!(result["medians"]["frameMedianMs"], 12);
    assert_eq!(result["medians"]["frameP95Ms"], 1350);
    assert_eq!(result["medians"]["peakMemoryMiB"], 280);
}

#[test]
fn every_instant_of_the_load_counts_for_one_phase_up_to_the_first_board() {
    let split = node(
        "return splitLoad(
           100,
           [
             { phase: 'parse', start: 10, end: 20 },
             { phase: 'layout', start: 20, end: 40 },
             { phase: 'paint', start: 40, end: 50 },
             { phase: 'gpu', start: 50, end: 60 },
             { phase: 'parse', start: 90, end: 130 },
           ],
           [[0, 15], [60, 80]],
         );",
    );
    assert_eq!(
        split,
        json!({ "network": 30, "parsing": 20, "layout": 20, "painting": 10, "gpu": 10, "other": 10 })
    );
}

#[test]
fn the_page_tells_the_phases_of_the_load_apart() {
    let split = node(
        "const page = openPage();
         page.stall(200);
         page.sceneLoad({ ms: 100, layoutMs: 40 });
         page.draw({ ms: 30, submitMs: 4, finish: 6, drawn: firstBoardShown() });
         return splitLoad(page.bench.firstBoard, page.bench.spans, [[0, 200]]);",
    );
    assert_eq!(
        split,
        json!({ "network": 200, "parsing": 60, "layout": 40, "painting": 26, "gpu": 10, "other": 0 })
    );
}

/// The cells after the first, of the first row that starts with `title`, after
/// the heading that starts with `heading`.
fn row(report: &str, heading: &str, title: &str) -> Vec<String> {
    let line = report
        .lines()
        .skip_while(|line| !line.starts_with(heading))
        .find(|line| line.starts_with(&format!("| {title}")))
        .unwrap_or_else(|| panic!("no row {title} under {heading}:\n{report}"));
    line.split('|')
        .skip(2)
        .map(|cell| cell.trim().to_string())
        .filter(|cell| !cell.is_empty())
        .collect()
}

#[test]
fn the_report_profiles_the_load_and_the_mean_and_slowest_frames_phase_by_phase() {
    let report = node(
        "const frames = [
           ...Array(18).fill(frame(100, { parse: 10, layout: 60, paint: 20, gpu: 5, other: 5 })),
           ...Array(2).fill(frame(1000, { parse: 100, layout: 800, paint: 50, gpu: 10, other: 40 })),
         ];
         const cold = { network: 800, parsing: 60, layout: 40, painting: 20, gpu: 20, other: 60 };
         return report({ runs: Array(5).fill(run({ firstBoardMs: 1000, frames, cold })) });",
    );
    let report = report.as_str().expect("the report is text");

    let load = "### Cold load";
    assert_eq!(row(report, load, "Network"), ["800.0", "80.0%"]);
    assert_eq!(row(report, load, "Parsing"), ["60.0", "6.0%"]);
    assert_eq!(row(report, load, "Layout"), ["40.0", "4.0%"]);
    assert_eq!(row(report, load, "Painting"), ["20.0", "2.0%"]);
    assert_eq!(row(report, load, "Uploading to the GPU"), ["20.0", "2.0%"]);
    assert_eq!(row(report, load, "Other"), ["60.0", "6.0%"]);

    let moving = "### Pan and zoom";
    assert_eq!(
        row(report, moving, "Network"),
        [
            "not applicable",
            "not applicable",
            "not applicable",
            "not applicable"
        ]
    );
    assert_eq!(
        row(report, moving, "Parsing"),
        ["19.0", "10.0%", "100.0", "10.0%"]
    );
    assert_eq!(
        row(report, moving, "Layout"),
        ["134.0", "70.5%", "800.0", "80.0%"]
    );
    assert_eq!(
        row(report, moving, "Painting"),
        ["23.0", "12.1%", "50.0", "5.0%"]
    );
    assert_eq!(
        row(report, moving, "Uploading to the GPU"),
        ["5.5", "2.9%", "10.0", "1.0%"]
    );
    assert_eq!(
        row(report, moving, "Other"),
        ["8.5", "4.5%", "40.0", "4.0%"]
    );
}

#[test]
fn the_bench_measures_as_gate_a_does_over_a_10_s_path() {
    let config = node("return config;");
    assert_eq!(config["viewport"], json!({ "width": 1440, "height": 900 }));
    assert_eq!(config["deviceScaleFactor"], 2);
    assert_eq!(config["runs"], 5);
    assert_eq!(config["pathMs"], 10_000);
}

#[test]
fn the_memory_of_a_line_of_docker_stats_is_its_usage_in_mib() {
    let usage = node(
        "return ['127.4MiB / 7.653GiB', '1.5GiB / 7.653GiB', '512KiB / 7.653GiB', '\\u001b[2J\\u001b[H'].map(memUsageMiB);",
    );
    assert_eq!(usage, json!([127.4, 1536, 0.5, null]));
}

/// Whether `run_js` five times over meets the limit the check `name` holds.
fn within_limit(name: &str, run_js: &str) -> Value {
    node(&format!(
        "return evaluate(Array(5).fill({run_js})).checks.find((check) => check.name === '{name}').pass;"
    ))
}

#[test]
fn the_verdicts_hold_the_limits_of_fr_28_at_their_edges() {
    assert_eq!(
        within_limit("firstBoardMs", "run({ firstBoardMs: 1999 })"),
        true
    );
    assert_eq!(
        within_limit("firstBoardMs", "run({ firstBoardMs: 2000 })"),
        false
    );
    assert_eq!(
        within_limit("frameP95Ms", "run({ frames: framesOf(1, 16.7) })"),
        true
    );
    assert_eq!(
        within_limit("frameP95Ms", "run({ frames: framesOf(1, 16.8) })"),
        false
    );
    assert_eq!(
        within_limit("peakMemoryMiB", "run({ peakMemoryMiB: 511 })"),
        true
    );
    assert_eq!(
        within_limit("peakMemoryMiB", "run({ peakMemoryMiB: 512 })"),
        false
    );
}

#[test]
fn make_bench_ordrar_runs_the_bench_after_building_the_renderer_and_the_cli() {
    let commands = common::make_dry_run("bench-ordrar");
    assert!(
        commands.contains("node render-sandbox/bench-ordrar.mjs"),
        "`make bench-ordrar` must run the bench:\n{commands}"
    );
    assert!(
        commands.contains("omni_render.wasm"),
        "`make bench-ordrar` must build the WebAssembly renderer first:\n{commands}"
    );
    assert!(
        commands.contains("--release") && commands.contains("-p omni-cli"),
        "`make bench-ordrar` must build the release CLI that imports Ordrar:\n{commands}"
    );
}
