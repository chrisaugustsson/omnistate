//! US-104: `omni-penpot report <folder> --out <dir>` and `make penpot-compare`,
//! on small exports and PNGs that the tests build themselves. These tests do not
//! need fixtures/penpot/ (report_fixtures.rs runs the real exports).
//!
//! The command takes every `*.penpot` file directly in `<folder>` (other files
//! and folders are ignored, except that the folder of PNGs beside an export,
//! named like it without `.penpot`, is where Penpot's PNGs are looked for, as in
//! `omni-penpot compare`). It converts and compares each file as `compare`
//! does and writes one page, `<dir>/index.html`, with the images it shows below
//! `<dir>`: it must work when `<dir>` is opened from disk or moved, so every
//! `src` is a relative path inside `<dir>` and Penpot's PNGs are copied there
//! (fixtures/penpot/ is not in the repository). The same inputs give the same
//! files. A folder that does not exist, or has no `*.penpot` file, fails with a
//! message that names the folder.
//!
//! What the page holds, in the markup these tests read (all text is HTML
//! escaped; names of boards and shapes are the designer's, not ours):
//!
//! - a summary, the element with `id="summary"`, before the first board, with
//!   `data-boards` (every board of every file), `data-median-share` (the median
//!   of the pixel scores of the boards that have one, the mean of the two middle
//!   ones where there is an even number; empty or absent where no board has one),
//!   `data-layout-mismatches` (the total number of layout mismatches) and
//!   `data-unconverted` (the total number of unconverted shapes). Its text says
//!   the same: the three whole numbers, and the median as a percentage with one
//!   decimal (`1.0%` for 0.01);
//! - one element per board, with `data-file` (the export's file name),
//!   `data-board-id`, `data-board` (the name), `data-pixel-share` (the pixel
//!   score, empty or absent for a board without a PNG), `data-layout-mismatches`
//!   and `data-unconverted` (the counts for the board). Its text has the name,
//!   and the pixel score as a percentage with one decimal. The pixel score is
//!   `compare`'s `share`, the share of compared pixels that differ: 0 is a
//!   perfect match and lower is better;
//! - for a board that has a PNG, three `<img>` elements, in this order, which a
//!   stylesheet lays out side by side: `data-role="penpot"` (Penpot's PNG),
//!   `data-role="omnistate"` (the render) and `data-role="diff"` (the diff image),
//!   each with alt text. A board without a PNG has no images and its text says
//!   "layout comparison only";
//! - one element with `data-role="mismatch"` per layout mismatch, with
//!   `data-shape-id` and `data-off-by`, whose text has the shape's name; and one
//!   with `data-role="unconverted"` per unconverted shape, with `data-shape-id`,
//!   whose text has the name and the reason;
//! - the boards from worst to best over all files: first the boards that have a
//!   pixel score, the largest share first, then the boards without one, the most
//!   layout mismatches first.
//!
//! `make penpot-compare` runs the command on `fixtures/penpot` with the output
//! in `docs/penpot-compare`; `PENPOT_FIXTURES` and `PENPOT_REPORT` replace those.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use common::compare::*;
use common::convert::*;
use common::report::*;
use common::{repo_root, stored_zip};

/// A flex column board called `name` at `(x, 0)`, 300 by 200, of four squares
/// that the layout puts at `(x, 0)`, `(x, 40)`, `(x, 80)` and `(x, 120)`. Penpot's
/// stored box of each square in `shifted` is 5 px to the right of that, which the
/// layout comparison lists as a mismatch. The squares are called `<prefix> 0` to
/// `<prefix> 3`, and are numbered `n + 1` to `n + 4`.
fn column_board(n: u32, name: &str, prefix: &str, x: f64, shifted: &[usize]) -> S {
    let squares = (0..4)
        .map(|i| {
            let left = if shifted.contains(&i) { x + 5.0 } else { x };
            shape(
                n + 1 + i as u32,
                "rect",
                &format!("{prefix} {i}"),
                [left, 40.0 * i as f64, 100.0, 40.0],
            )
        })
        .rev()
        .collect();
    shape(n, "frame", name, [x, 0.0, 300.0, 200.0])
        .with(fill("#ffffff"))
        .with(flex("column"))
        .kids(squares)
}

/// A white board "Good" or "Mid" or the like with a red square in its top left
/// corner, 200 by 100 on its page, that `standard_png` is the export of.
fn square_board(n: u32, name: &str, x: f64, more: Vec<S>) -> S {
    let mut kids = vec![rect(
        n + 1,
        &format!("{name} square"),
        [x, 0.0, 100.0, 50.0],
        "#ff0000",
    )];
    kids.extend(more);
    white_board(n, name, [x, 0.0, 200.0, 100.0], kids)
}

fn union(n: u32, x: f64) -> S {
    shape(n, "bool", &format!("Union {n}"), [x, 0.0, 40.0, 40.0])
}

/// One export to write into a folder: its name without `.penpot`, the shapes on
/// its page and the PNGs in the folder beside it.
struct Input {
    name: &'static str,
    top_level: Vec<S>,
    pngs: Vec<(String, Pixels)>,
}

/// A folder of its own with the exports, their PNG folders, and files and
/// folders that the report has to ignore.
fn write_folder(test: &str, inputs: Vec<Input>) -> PathBuf {
    let folder = temp_dir(&format!("report-in-{test}"));
    fs::create_dir_all(&folder).expect("the folder can be created");
    for input in inputs {
        let entries = common::export_entries(FILE_ID, "Synthetic", &[page(input.top_level)]);
        fs::write(
            folder.join(format!("{}.penpot", input.name)),
            stored_zip(&entries),
        )
        .expect("an export can be written");
        if !input.pngs.is_empty() {
            let pngs = folder.join(input.name);
            fs::create_dir_all(&pngs).expect("the PNG folder can be created");
            for (name, pixels) in input.pngs {
                fs::write(pngs.join(name), pixels.to_png()).expect("a PNG can be written");
            }
        }
    }
    fs::write(folder.join("notes.txt"), "not an export").expect("a note can be written");
    fs::create_dir_all(folder.join("unrelated")).expect("a folder can be created");
    fs::write(
        folder.join("unrelated/Other.png"),
        Pixels::solid(4, 4, RED).to_png(),
    )
    .expect("a PNG can be written");
    folder
}

fn run_report(folder: &Path, out: &Path) -> Output {
    run_cli(&[
        "report",
        folder.to_str().expect("a UTF-8 path"),
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ])
}

fn report_ok(folder: &Path, out: &Path) -> Report {
    let result = run_report(folder, out);
    assert!(
        result.status.success(),
        "omni-penpot report failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Report::read(out)
}

/// Every file below `dir`, by path relative to it.
fn files_below(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in fs::read_dir(&next).expect("a listable directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let name = path.strip_prefix(dir).expect("below the folder");
                files.insert(
                    name.to_string_lossy().to_string(),
                    fs::read(&path).expect("a readable file"),
                );
            }
        }
    }
    files
}

/// The numbers of the shapes in a board's list of items, sorted.
fn shape_ids(items: &[(&common::report::Tag, String)]) -> Vec<String> {
    let mut ids: Vec<String> = items
        .iter()
        .map(|(tag, _)| tag.attr("data-shape-id").expect("a shape id"))
        .collect();
    ids.sort();
    ids
}

/// The scenario most tests read. Two exports, five boards:
///
/// | board | file | PNG | pixel share | mismatches | unconverted |
/// |-------|------|-----|-------------|------------|-------------|
/// | Good  | alpha | same | 0    | 0 | 0 |
/// | Bad   | alpha | all blue | 1 | 0 | 1 |
/// | Small | alpha | none | -    | 1 | 0 |
/// | Mid   | beta  | one 40 x 20 block wrong | 0.01 | 0 | 0 |
/// | Big   | beta  | none | -    | 2 | 3 |
///
/// Worst first: Bad, Mid, Good, Big, Small. The median share is 0.01; the
/// totals are 5 boards, 3 mismatches and 4 unconverted shapes.
struct Scenario {
    folder: PathBuf,
    out: PathBuf,
    report: Report,
    /// What `omni-penpot compare` makes of each export.
    alpha: Outcome,
    beta: Outcome,
}

/// The folder of the scenario and what `compare` makes of its two exports.
fn scenario_inputs() -> &'static (PathBuf, Outcome, Outcome) {
    static INPUTS: OnceLock<(PathBuf, Outcome, Outcome)> = OnceLock::new();
    INPUTS.get_or_init(|| {
        let mut mid_png = standard_png();
        mid_png.fill([300, 20, 340, 40], BLUE);
        let folder = write_folder(
            "scenario",
            vec![
                Input {
                    name: "alpha",
                    top_level: vec![
                        square_board(1, "Good", 0.0, vec![]),
                        white_board(
                            10,
                            "Bad",
                            [500.0, 0.0, 200.0, 100.0],
                            vec![union(11, 500.0)],
                        ),
                        column_board(20, "Small", "Small square", 1000.0, &[1]),
                    ],
                    pngs: vec![
                        ("Good.png".to_string(), standard_png()),
                        ("Bad.png".to_string(), Pixels::solid(400, 200, BLUE)),
                    ],
                },
                Input {
                    name: "beta",
                    top_level: vec![
                        square_board(30, "Mid", 0.0, vec![]),
                        column_board(40, "Big", "Big square", 500.0, &[0, 2]).kids_plus(vec![
                            union(45, 500.0),
                            union(46, 500.0),
                            union(47, 500.0),
                        ]),
                    ],
                    pngs: vec![("Mid.png".to_string(), mid_png)],
                },
            ],
        );
        let oracle = |name: &str| {
            compare_ok(
                &folder.join(format!("{name}.penpot")),
                &temp_dir(&format!("report-oracle-{name}")),
            )
        };
        let (alpha, beta) = (oracle("alpha"), oracle("beta"));
        (folder, alpha, beta)
    })
}

fn scenario() -> &'static Scenario {
    static SCENARIO: OnceLock<Scenario> = OnceLock::new();
    SCENARIO.get_or_init(|| {
        let (folder, alpha, beta) = scenario_inputs();
        let out = temp_dir("report-out-scenario");
        let report = report_ok(folder, &out);
        Scenario {
            folder: folder.clone(),
            out,
            report,
            alpha: Outcome {
                out: alpha.out.clone(),
                report: alpha.report.clone(),
            },
            beta: Outcome {
                out: beta.out.clone(),
                report: beta.report.clone(),
            },
        }
    })
}

trait KidsPlus {
    fn kids_plus(self, more: Vec<S>) -> S;
}

impl KidsPlus for S {
    /// Puts `more` inside the shape after the shapes it has, on top of them.
    fn kids_plus(mut self, more: Vec<S>) -> S {
        self.kids.extend(more);
        self
    }
}

fn names(report: &Report) -> Vec<String> {
    report
        .boards()
        .iter()
        .map(|board| board.attr("data-board").expect("a board name"))
        .collect()
}

#[test]
fn every_board_of_every_file_is_on_the_page_once() {
    let s = scenario();
    let boards = s.report.boards();
    assert_eq!(boards.len(), 5, "{:?}", names(&s.report));
    let mut found: Vec<(String, String, String)> = boards
        .iter()
        .map(|board| {
            (
                board.attr("data-file").expect("a file"),
                board.attr("data-board").expect("a name"),
                board.id(),
            )
        })
        .collect();
    found.sort();
    let expected = [
        ("alpha.penpot", "Good", 1),
        ("alpha.penpot", "Bad", 10),
        ("alpha.penpot", "Small", 20),
        ("beta.penpot", "Mid", 30),
        ("beta.penpot", "Big", 40),
    ];
    let mut expected: Vec<(String, String, String)> = expected
        .iter()
        .map(|(file, name, n)| (file.to_string(), name.to_string(), uid(*n)))
        .collect();
    expected.sort();
    assert_eq!(found, expected);
    for board in boards {
        assert!(
            board
                .text()
                .contains(&board.attr("data-board").expect("a name")),
            "the page shows the name of {}",
            board.id()
        );
    }
}

#[test]
fn a_board_with_a_png_shows_penpots_png_the_render_and_the_diff_side_by_side() {
    let s = scenario();
    let mut checked = 0;
    for (n, outcome) in [(1, &s.alpha), (10, &s.alpha), (30, &s.beta)] {
        let board = s.report.board(&uid(n));
        let name = board.attr("data-board").expect("a name");
        let images = board.images();
        let roles: Vec<String> = images
            .iter()
            .map(|image| image.attr("data-role").unwrap_or_default())
            .collect();
        assert_eq!(roles, ["penpot", "omnistate", "diff"], "images of {name}");

        let pixel = &outcome.board(&uid(n))["pixel"];
        let penpot = image_pixels(&s.out, images[0]);
        let render = image_pixels(&s.out, images[1]);
        let diff = image_pixels(&s.out, images[2]);
        assert_eq!(
            render,
            outcome.png(&pixel["render"]),
            "the render of {name}"
        );
        assert_eq!(diff, outcome.png(&pixel["diff"]), "the diff of {name}");
        assert_eq!((penpot.width, penpot.height), (render.width, render.height));
        assert_eq!((diff.width, diff.height), (render.width, render.height));
        checked += 1;
    }
    assert_eq!(checked, 3);

    // Penpot's own PNG is copied into the report, not linked to the fixtures.
    let mid = s.report.board(&uid(30));
    let mut expected = standard_png();
    expected.fill([300, 20, 340, 40], BLUE);
    assert_eq!(
        image_pixels(&s.out, mid.image("penpot").expect("the PNG")),
        expected
    );
    assert_eq!(
        image_pixels(
            &s.out,
            s.report.board(&uid(10)).image("penpot").expect("the PNG")
        ),
        Pixels::solid(400, 200, BLUE)
    );
}

#[test]
fn the_pixel_score_is_shown_for_every_board_that_has_a_png() {
    let s = scenario();
    for (n, share, text) in [(1, 0.0, "0.0%"), (10, 1.0, "100.0%"), (30, 0.01, "1.0%")] {
        let board = s.report.board(&uid(n));
        assert!(
            (board.tag().number("data-pixel-share") - share).abs() < 1e-9,
            "board {n}: {:?}",
            board.tag()
        );
        assert!(
            board.text().contains(text),
            "the text of board {n} has {text}: {}",
            board.text()
        );
    }
}

#[test]
fn a_board_without_a_png_says_layout_comparison_only_and_shows_no_images() {
    let s = scenario();
    for n in [20, 40] {
        let board = s.report.board(&uid(n));
        let name = board.attr("data-board").expect("a name");
        assert!(
            board.images().is_empty(),
            "{name} has nothing to show side by side"
        );
        assert!(
            board
                .text()
                .to_lowercase()
                .contains("layout comparison only"),
            "{name}: {}",
            board.text()
        );
        let share = board.attr("data-pixel-share").unwrap_or_default();
        assert!(
            share.trim().is_empty(),
            "{name} has no pixel score, not {share:?}"
        );
    }
}

#[test]
fn the_layout_mismatches_of_a_board_are_listed_with_their_shapes() {
    let s = scenario();
    let big = s.report.board(&uid(40));
    let items = big.items("mismatch");
    assert_eq!(shape_ids(&items), [uid(41), uid(43)], "{items:?}");
    assert_eq!(big.tag().number("data-layout-mismatches"), 2.0);
    for (tag, text) in &items {
        let id = tag.attr("data-shape-id").expect("an id");
        let name = if id == uid(41) {
            "Big square 0"
        } else {
            "Big square 2"
        };
        assert!(
            text.contains(name),
            "the item of {id} names the shape: {text}"
        );
        assert!((tag.number("data-off-by") - 5.0).abs() < 0.01, "{tag:?}");
    }

    let small = s.report.board(&uid(20));
    assert_eq!(shape_ids(&small.items("mismatch")), [uid(22)]);
    assert_eq!(small.tag().number("data-layout-mismatches"), 1.0);

    // A board that matches lists none and says 0.
    let good = s.report.board(&uid(1));
    assert!(good.items("mismatch").is_empty());
    assert_eq!(good.tag().number("data-layout-mismatches"), 0.0);
}

#[test]
fn the_unconverted_shapes_of_a_board_are_listed_with_their_reasons() {
    let s = scenario();
    let big = s.report.board(&uid(40));
    let items = big.items("unconverted");
    assert_eq!(shape_ids(&items), [uid(45), uid(46), uid(47)], "{items:?}");
    assert_eq!(big.tag().number("data-unconverted"), 3.0);

    let bad = s.report.board(&uid(10));
    let items = bad.items("unconverted");
    assert_eq!(shape_ids(&items), [uid(11)]);
    assert!(items[0].1.contains("Union 11"), "the name: {}", items[0].1);
    let compared_reason = s.alpha.board(&uid(10))["unconverted"][0]["reason"]
        .as_str()
        .expect("a reason");
    assert!(
        items[0].1.contains(compared_reason),
        "the reason `compare` gives, {compared_reason:?}, is shown: {}",
        items[0].1
    );
    assert_eq!(bad.tag().number("data-unconverted"), 1.0);
    assert_eq!(s.report.board(&uid(1)).items("unconverted").len(), 0);
}

#[test]
fn boards_are_sorted_worst_first_across_files() {
    let s = scenario();
    assert_eq!(
        names(&s.report),
        ["Bad", "Mid", "Good", "Big", "Small"],
        "boards with a pixel score by largest share, then boards without by mismatches"
    );
}

#[test]
fn the_summary_at_the_top_has_the_counts_the_median_and_the_totals() {
    let s = scenario();
    let (summary, text) = s.report.summary();
    assert_eq!(summary.number("data-boards"), 5.0);
    assert!(
        (summary.number("data-median-share") - 0.01).abs() < 1e-9,
        "{summary:?}"
    );
    assert_eq!(summary.number("data-layout-mismatches"), 3.0);
    assert_eq!(summary.number("data-unconverted"), 4.0);

    let numbers = number_tokens(&text);
    for expected in ["5", "3", "4"] {
        assert!(
            numbers.contains(&expected.to_string()),
            "{expected} in the summary: {text}"
        );
    }
    assert!(
        text.contains("1.0%"),
        "the median score in the summary: {text}"
    );
}

#[test]
fn the_summary_agrees_with_the_boards_below_it() {
    let s = scenario();
    let (summary, _) = s.report.summary();
    let boards = s.report.boards();
    let sum = |key: &str| boards.iter().map(|b| b.tag().number(key)).sum::<f64>();
    assert_eq!(summary.number("data-boards"), boards.len() as f64);
    assert_eq!(
        summary.number("data-layout-mismatches"),
        sum("data-layout-mismatches")
    );
    assert_eq!(summary.number("data-unconverted"), sum("data-unconverted"));
    let items = |role: &str| boards.iter().map(|b| b.items(role).len()).sum::<usize>();
    assert_eq!(
        items("mismatch") as f64,
        summary.number("data-layout-mismatches")
    );
    assert_eq!(
        items("unconverted") as f64,
        summary.number("data-unconverted")
    );
}

#[test]
fn the_median_of_an_even_number_of_scores_is_the_mean_of_the_middle_two() {
    let folder = write_folder(
        "even",
        vec![Input {
            name: "pair",
            top_level: vec![
                square_board(1, "Same", 0.0, vec![]),
                white_board(10, "Wrong", [500.0, 0.0, 200.0, 100.0], vec![]),
            ],
            pngs: vec![
                ("Same.png".to_string(), standard_png()),
                ("Wrong.png".to_string(), Pixels::solid(400, 200, BLUE)),
            ],
        }],
    );
    let report = report_ok(&folder, &temp_dir("report-out-even"));
    let (summary, text) = report.summary();
    assert_eq!(summary.number("data-boards"), 2.0);
    assert!(
        (summary.number("data-median-share") - 0.5).abs() < 1e-9,
        "{summary:?}"
    );
    assert!(text.contains("50.0%"), "{text}");
    assert_eq!(names(&report), ["Wrong", "Same"]);
}

#[test]
fn boards_without_pngs_have_no_median_and_are_sorted_by_mismatches() {
    let folder = write_folder(
        "layout-only",
        vec![Input {
            name: "plain",
            top_level: vec![
                column_board(1, "One", "a", 0.0, &[0]),
                column_board(10, "Three", "b", 500.0, &[0, 1, 2]),
                column_board(20, "None", "c", 1000.0, &[]),
                column_board(30, "Two", "d", 1500.0, &[0, 3]),
            ],
            pngs: vec![],
        }],
    );
    let report = report_ok(&folder, &temp_dir("report-out-layout-only"));
    assert_eq!(names(&report), ["Three", "Two", "One", "None"]);
    let (summary, _) = report.summary();
    assert_eq!(summary.number("data-boards"), 4.0);
    assert_eq!(summary.number("data-layout-mismatches"), 6.0);
    assert_eq!(summary.number("data-unconverted"), 0.0);
    let median = summary.attr("data-median-share").unwrap_or_default();
    assert!(
        median.trim().is_empty(),
        "no board has a score, so no median: {median:?}"
    );
}

#[test]
fn names_from_the_design_are_escaped() {
    let board_name = "Q&A \"one\" / <b>two";
    let shape_name = "<script>alert(1)</script>";
    let folder = write_folder(
        "escape",
        vec![Input {
            name: "weird",
            top_level: vec![
                white_board(1, board_name, [0.0, 0.0, 100.0, 50.0], vec![]),
                column_board(10, "Plain", shape_name, 500.0, &[0]),
            ],
            pngs: vec![(
                "Q&A \"one\" _ <b>two.png".to_string(),
                Pixels::solid(200, 100, WHITE),
            )],
        }],
    );
    let out = temp_dir("report-out-escape");
    let report = report_ok(&folder, &out);
    assert!(
        !report.html.contains("<script>alert(1)"),
        "a shape name is written as markup"
    );
    assert!(
        !report.html.contains("<b>two"),
        "a board name is written as markup"
    );

    let board = report.board(&uid(1));
    assert_eq!(board.attr("data-board").as_deref(), Some(board_name));
    assert!(board.text().contains(board_name), "{}", board.text());
    assert_eq!(
        board.images().len(),
        3,
        "the PNG of a board with such a name is found"
    );
    for image in board.images() {
        image_file(&out, image);
    }
    let items = report.board(&uid(10)).items("mismatch");
    assert_eq!(items.len(), 1);
    assert!(
        items[0].1.contains(&format!("{shape_name} 0")),
        "{}",
        items[0].1
    );
}

#[test]
fn the_same_inputs_give_the_same_files() {
    let s = scenario();
    let again = temp_dir("report-out-again");
    report_ok(&s.folder, &again);
    let (first, second) = (files_below(&s.out), files_below(&again));
    assert!(first.contains_key("index.html"));
    assert!(
        first.keys().filter(|name| name.ends_with(".png")).count() >= 9,
        "three images for each of the three boards with a PNG: {:?}",
        first.keys()
    );
    assert_eq!(
        first.keys().collect::<Vec<_>>(),
        second.keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &first {
        assert!(bytes == &second[name], "{name} differs between runs");
    }
}

#[test]
fn a_folder_that_does_not_exist_fails_with_a_message() {
    let missing = std::env::temp_dir().join("omni-penpot-no-such-folder");
    let _ = fs::remove_dir_all(&missing);
    let out = temp_dir("report-out-missing");
    let result = run_report(&missing, &out);
    assert!(!result.status.success());
    let message = String::from_utf8_lossy(&result.stderr);
    assert!(message.contains("omni-penpot-no-such-folder"), "{message}");
    assert!(!out.join("index.html").exists());
}

#[test]
fn a_folder_without_penpot_files_fails_with_a_message() {
    let empty = temp_dir("report-in-empty");
    fs::create_dir_all(&empty).expect("the folder can be created");
    fs::write(empty.join("notes.txt"), "no exports here").expect("a file can be written");
    let out = temp_dir("report-out-empty");
    let result = run_report(&empty, &out);
    assert!(!result.status.success());
    let message = String::from_utf8_lossy(&result.stderr).to_lowercase();
    assert!(message.contains(".penpot"), "{message}");
    assert!(
        message.contains("report-in-empty"),
        "the failure names the folder: {message}"
    );
    assert!(!out.join("index.html").exists());
}

fn make(args: &[&str]) -> Output {
    Command::new("make")
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("make can be started")
}

fn makefile() -> String {
    fs::read_to_string(repo_root().join("Makefile")).expect("the Makefile is there")
}

#[test]
fn make_has_a_phony_penpot_compare_target() {
    let phony: Vec<String> = makefile()
        .lines()
        .filter(|line| line.starts_with(".PHONY"))
        .map(str::to_string)
        .collect();
    assert!(
        phony
            .iter()
            .any(|line| line.split_whitespace().any(|word| word == "penpot-compare")),
        "penpot-compare is a .PHONY target: {phony:?}"
    );
}

#[test]
fn make_penpot_compare_reads_fixtures_penpot_and_writes_docs_penpot_compare() {
    let result = make(&["-n", "penpot-compare"]);
    assert!(
        result.status.success(),
        "make has the target: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let plan = String::from_utf8_lossy(&result.stdout);
    assert!(plan.contains("omni-penpot"), "{plan}");
    assert!(plan.contains("report"), "{plan}");
    assert!(plan.contains("fixtures/penpot"), "{plan}");
    assert!(plan.contains("docs/penpot-compare"), "{plan}");
}

#[test]
fn the_folders_of_make_penpot_compare_can_be_replaced() {
    let result = make(&[
        "-n",
        "penpot-compare",
        "PENPOT_FIXTURES=/some/exports",
        "PENPOT_REPORT=/some/report",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let plan = String::from_utf8_lossy(&result.stdout);
    assert!(plan.contains("/some/exports"), "{plan}");
    assert!(plan.contains("/some/report"), "{plan}");
}

#[test]
fn make_penpot_compare_writes_the_report_of_the_folder() {
    let s = scenario();
    let out = temp_dir("report-out-make");
    let result = make(&[
        "penpot-compare",
        &format!("PENPOT_FIXTURES={}", s.folder.display()),
        &format!("PENPOT_REPORT={}", out.display()),
    ]);
    assert!(
        result.status.success(),
        "make penpot-compare failed: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let from_make = files_below(&out);
    let from_command = files_below(&s.out);
    assert_eq!(
        from_make.keys().collect::<Vec<_>>(),
        from_command.keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &from_command {
        assert!(bytes == &from_make[name], "{name} differs");
    }
}

#[test]
fn the_report_is_documented() {
    let text = fs::read_to_string(repo_root().join("docs/penpot-compare.md"))
        .unwrap_or_else(|error| panic!("docs/penpot-compare.md is not there: {error}"))
        .to_lowercase();
    assert!(
        text.contains("make penpot-compare"),
        "the document says how to make the report"
    );
    assert!(
        text.contains("docs/penpot-compare/index.html"),
        "and where it is"
    );
    assert!(text.contains("worst"), "and how the boards are sorted");
}
