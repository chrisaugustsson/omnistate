//! US-103: `omni-penpot compare` on the real exports in fixtures/penpot/ (see
//! compare_synthetic.rs for what it writes). These tests skip, with a message,
//! when the exports are not there.
//!
//! - tirebud-idag.penpot has PNGs, in fixtures/penpot/tirebud-idag/, for its 15
//!   boards (and for 8 texts on the page, which are no boards): every board gets
//!   the pixel and the layout comparison, on a render the size of its PNG. The
//!   board "Unplaced", which holds those 8 texts (US-054), has no PNG and gets
//!   the layout comparison only.
//! - tirebud-ordrar.penpot has no PNGs and no folder: all of its 795 boards,
//!   and its two Unplaced boards, get the layout comparison only.
//!
//! Nothing here asks for a score: the project measures first and decides what to
//! fix after reading the report.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use omni_penpot::convert_file;
use serde_json::Value;

use common::compare::*;
use common::convert::{assert_near, temp_dir};
use common::{fixture, repo_root};

struct Run {
    out: PathBuf,
    report: Value,
}

fn run(name: &str, out: &str) -> Option<Run> {
    let file = fixture(name)?;
    let out = temp_dir(out);
    let outcome = compare_ok(&file, &out);
    Some(Run {
        out,
        report: outcome.report,
    })
}

/// The comparison of tirebud-idag, run once. None where the export, or the
/// folder of PNGs beside it, is missing.
fn idag() -> Option<&'static Run> {
    static RUN: OnceLock<Option<Run>> = OnceLock::new();
    RUN.get_or_init(|| {
        if !repo_root().join("fixtures/penpot/tirebud-idag").is_dir() {
            eprintln!("SKIPPED: fixtures/penpot/tirebud-idag/ (Penpot's PNGs) is not there");
            return None;
        }
        run("tirebud-idag.penpot", "compare-idag")
    })
    .as_ref()
}

fn ordrar() -> Option<&'static Run> {
    static RUN: OnceLock<Option<Run>> = OnceLock::new();
    RUN.get_or_init(|| run("tirebud-ordrar.penpot", "compare-ordrar"))
        .as_ref()
}

fn boards(run: &Run) -> &Vec<Value> {
    run.report["boards"].as_array().expect("a list of boards")
}

/// The boards of the file, without the Unplaced board that holds the shapes
/// outside them.
fn file_boards(run: &Run) -> Vec<&Value> {
    boards(run)
        .iter()
        .filter(|board| board["board"] != "Unplaced")
        .collect()
}

/// A path that compare.json names, below the output directory.
fn path_in(run: &Run, relative: &Value) -> PathBuf {
    let path = run.out.join(relative.as_str().expect("a path"));
    assert!(path.is_file(), "{path:?} is not written");
    path
}

#[test]
fn every_idag_board_is_rendered_at_the_size_of_its_penpot_png() {
    let Some(run) = idag() else { return };
    assert_eq!(boards(run).len(), 16);
    let unplaced: Vec<&Value> = boards(run)
        .iter()
        .filter(|board| board["board"] == "Unplaced")
        .collect();
    assert_eq!(unplaced.len(), 1);
    assert_eq!(unplaced[0]["layoutOnly"], true, "Penpot has no PNG of it");
    assert!(field(&unplaced[0]["layout"], "compared") >= 8.0);
    let folder = repo_root().join("fixtures/penpot/tirebud-idag");
    assert_eq!(file_boards(run).len(), 15);
    for board in file_boards(run) {
        let name = board["board"].as_str().expect("a name");
        assert_eq!(board["layoutOnly"], false, "{name} has a PNG");
        let png = folder.join(format!("{}.png", name.replace(" / ", " _ ")));
        let (width, height) = png_size(&png);
        let pixel = &board["pixel"];
        assert_eq!(field(pixel, "width"), f64::from(width), "{name}");
        assert_eq!(field(pixel, "height"), f64::from(height), "{name}");
        for key in ["render", "diff"] {
            assert_eq!(
                png_size(&path_in(run, &pixel[key])),
                (width, height),
                "the {key} image of {name}"
            );
        }
    }
}

#[test]
fn every_idag_board_has_a_consistent_pixel_score() {
    let Some(run) = idag() else { return };
    tolerance(&run.report);
    for board in file_boards(run) {
        let name = board["board"].as_str().expect("a name");
        let pixel = &board["pixel"];
        let (width, height) = (field(pixel, "width"), field(pixel, "height"));
        let (compared, masked, differing, share) = (
            field(pixel, "compared"),
            field(pixel, "masked"),
            field(pixel, "differing"),
            field(pixel, "share"),
        );
        assert_eq!(compared + masked, width * height, "{name}");
        assert!(differing <= compared, "{name}");
        assert!((0.0..=1.0).contains(&share), "{name}: share {share}");
        if compared > 0.0 {
            assert_near(share, differing / compared, 1e-9, name);
        }
    }
    // The comparison looks at pixels: the converted desktop board draws almost
    // as Penpot does, apart from the fonts.
    let desktop = boards(run)
        .iter()
        .find(|board| board["board"] == "Idag / Dag / Desktop")
        .expect("the desktop board");
    assert!(
        field(&desktop["pixel"], "share") < 0.5,
        "the desktop board is mostly the same: {}",
        desktop["pixel"]
    );
}

#[test]
fn idag_unconverted_shapes_are_the_report_entries_of_whole_shapes() {
    let Some(run) = idag() else { return };
    let conversion = convert_file(&fixture("tirebud-idag.penpot").expect("the fixture"))
        .expect("the export converts");
    for board in boards(run) {
        let id = board["boardId"].as_str().expect("an id");
        let listed: BTreeSet<&str> = board["unconverted"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|entry| entry["shapeId"].as_str().expect("an id"))
            .collect();
        let reported: BTreeSet<&str> = conversion
            .report
            .entries
            .iter()
            .filter(|entry| entry.board_id == id && entry.property.is_none())
            .map(|entry| entry.shape_id.as_str())
            .collect();
        assert_eq!(listed, reported, "board {}", board["board"]);
    }
}

#[test]
fn idag_layout_is_compared_for_every_board_and_lists_only_real_mismatches() {
    let Some(run) = idag() else { return };
    let mut compared = 0.0;
    for board in boards(run) {
        let layout = &board["layout"];
        compared += field(layout, "compared");
        for mismatch in layout["mismatches"].as_array().expect("a list") {
            assert!(
                !mismatch["shapeId"].as_str().unwrap_or("").is_empty(),
                "{mismatch}"
            );
            assert!(field(mismatch, "offBy") > 2.0, "{mismatch}");
            for side in ["penpot", "omnistate"] {
                for key in ["x", "y", "width", "height"] {
                    field(&mismatch[side], key);
                }
            }
        }
    }
    // The 15 boards hold about 2,400 shapes.
    assert!(compared > 1_000.0, "only {compared} shapes were compared");
}

#[test]
fn idag_gives_the_same_files_on_a_second_run() {
    let (Some(first), Some(file)) = (idag(), fixture("tirebud-idag.penpot")) else {
        return;
    };
    let out = temp_dir("compare-idag-again");
    let second = compare_ok(&file, &out);
    assert_eq!(first.report, second.report);
    let board = &file_boards(first)[0]["pixel"];
    for key in ["render", "diff"] {
        let relative = board[key].as_str().expect("a path");
        assert_eq!(
            fs::read(first.out.join(relative)).expect("first run"),
            fs::read(out.join(relative)).expect("second run"),
            "{relative}"
        );
    }
}

#[test]
fn every_ordrar_board_gets_the_layout_comparison_only() {
    let Some(run) = ordrar() else { return };
    assert_eq!(boards(run).len(), 797);
    assert_eq!(file_boards(run).len(), 795);
    let mut compared = 0.0;
    for board in boards(run) {
        let name = board["board"].as_str().expect("a name");
        assert_eq!(board["layoutOnly"], true, "{name}");
        assert!(board["pixel"].is_null(), "{name} has no PNG: {board}");
        let note = board["note"].as_str().expect("a note").to_lowercase();
        assert!(note.contains("layout comparison only"), "{name}: {note}");
        compared += field(&board["layout"], "compared");
        for mismatch in board["layout"]["mismatches"].as_array().expect("a list") {
            assert!(field(mismatch, "offBy") > 2.0, "{mismatch}");
        }
    }
    // 111,000 shapes, most of them inside boards.
    assert!(compared > 50_000.0, "only {compared} shapes were compared");
}
