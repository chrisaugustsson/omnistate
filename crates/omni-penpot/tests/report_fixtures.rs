//! US-104: `omni-penpot report` on the real exports in fixtures/penpot/, which
//! is what `make penpot-compare` reads (see report_synthetic.rs for the page).
//! These tests skip, with a message, when the folder holds no export.
//!
//! - tirebud-idag.penpot has Penpot's PNGs for its 15 boards: each is shown with
//!   its three images.
//! - tirebud-ordrar.penpot has none, and 795 boards: each is layout comparison
//!   only.
//!
//! The report of the whole folder is made once. Nothing here asks for a score:
//! the page shows how far OmniState is from Penpot, it does not pass or fail.

mod common;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use common::convert::{run_cli, temp_dir};
use common::repo_root;
use common::report::*;

struct Run {
    out: std::path::PathBuf,
    report: Report,
}

fn run() -> Option<&'static Run> {
    static RUN: OnceLock<Option<Run>> = OnceLock::new();
    RUN.get_or_init(|| {
        let folder = repo_root().join("fixtures/penpot");
        let has_export = std::fs::read_dir(&folder).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().extension().is_some_and(|e| e == "penpot"))
        });
        if !has_export {
            eprintln!("SKIPPED: fixtures/penpot/ holds no .penpot export");
            return None;
        }
        let out = temp_dir("report-out-fixtures");
        let result = run_cli(&[
            "report",
            folder.to_str().expect("a UTF-8 path"),
            "--out",
            out.to_str().expect("a UTF-8 path"),
        ]);
        assert!(
            result.status.success(),
            "omni-penpot report failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report = Report::read(&out);
        Some(Run { out, report })
    })
    .as_ref()
}

/// The boards of each file, by file name.
fn per_file(run: &Run) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for board in run.report.boards() {
        *counts
            .entry(board.attr("data-file").expect("a file"))
            .or_default() += 1;
    }
    counts
}

#[test]
fn every_board_of_every_export_in_the_folder_is_on_the_page() {
    let Some(run) = run() else { return };
    let counts = per_file(run);
    let folder = repo_root().join("fixtures/penpot");
    for (file, boards) in [("tirebud-idag.penpot", 15), ("tirebud-ordrar.penpot", 795)] {
        if folder.join(file).is_file() {
            assert_eq!(
                counts.get(file),
                Some(&boards),
                "boards of {file}: {counts:?}"
            );
        }
    }
    let (summary, _) = run.report.summary();
    assert_eq!(
        summary.number("data-boards"),
        counts.values().sum::<usize>() as f64
    );
}

#[test]
fn boards_with_a_png_show_three_images_and_the_others_say_layout_comparison_only() {
    let Some(run) = run() else { return };
    let mut with_png = 0;
    for board in run.report.boards() {
        let name = board.attr("data-board").unwrap_or_default();
        let scored = !board
            .attr("data-pixel-share")
            .unwrap_or_default()
            .trim()
            .is_empty();
        if scored {
            with_png += 1;
            let roles: Vec<String> = board
                .images()
                .iter()
                .map(|image| image.attr("data-role").unwrap_or_default())
                .collect();
            assert_eq!(roles, ["penpot", "omnistate", "diff"], "{name}");
            for image in board.images() {
                image_file(&run.out, image);
            }
        } else {
            assert!(board.images().is_empty(), "{name}");
            assert!(
                board
                    .text()
                    .to_lowercase()
                    .contains("layout comparison only"),
                "{name}"
            );
        }
    }
    if repo_root().join("fixtures/penpot/tirebud-idag").is_dir() {
        assert!(
            with_png >= 15,
            "the 15 boards of tirebud-idag have PNGs: {with_png}"
        );
    }
}

#[test]
fn boards_are_sorted_worst_first() {
    let Some(run) = run() else { return };
    let mut scored = Vec::new();
    let mut unscored = Vec::new();
    let mut seen_unscored = false;
    for board in run.report.boards() {
        let name = board.attr("data-board").unwrap_or_default();
        let share = board.attr("data-pixel-share").unwrap_or_default();
        if share.trim().is_empty() {
            seen_unscored = true;
            unscored.push(board.tag().number("data-layout-mismatches"));
        } else {
            assert!(
                !seen_unscored,
                "{name} has a score and comes after a board without"
            );
            scored.push(board.tag().number("data-pixel-share"));
        }
    }
    assert!(
        scored.windows(2).all(|pair| pair[0] >= pair[1]),
        "{scored:?}"
    );
    assert!(
        unscored.windows(2).all(|pair| pair[0] >= pair[1]),
        "by mismatches"
    );
}

#[test]
fn the_summary_is_the_median_and_the_totals_of_the_boards() {
    let Some(run) = run() else { return };
    let (summary, text) = run.report.summary();
    let boards = run.report.boards();
    let mut shares: Vec<f64> = boards
        .iter()
        .filter(|board| {
            !board
                .attr("data-pixel-share")
                .unwrap_or_default()
                .trim()
                .is_empty()
        })
        .map(|board| board.tag().number("data-pixel-share"))
        .collect();
    shares.sort_by(f64::total_cmp);
    if !shares.is_empty() {
        let middle = shares.len() / 2;
        let median = if shares.len() % 2 == 1 {
            shares[middle]
        } else {
            (shares[middle - 1] + shares[middle]) / 2.0
        };
        assert!((summary.number("data-median-share") - median).abs() < 1e-9);
        assert!(text.contains(&format!("{:.1}%", median * 100.0)), "{text}");
    }
    let sum = |key: &str| {
        boards
            .iter()
            .map(|board| board.tag().number(key))
            .sum::<f64>()
    };
    assert_eq!(
        summary.number("data-layout-mismatches"),
        sum("data-layout-mismatches")
    );
    assert_eq!(summary.number("data-unconverted"), sum("data-unconverted"));
}

#[test]
fn each_board_lists_as_many_mismatches_and_unconverted_shapes_as_it_counts() {
    let Some(run) = run() else { return };
    for board in run.report.boards() {
        let name = board.attr("data-board").unwrap_or_default();
        assert_eq!(
            board.items("mismatch").len() as f64,
            board.tag().number("data-layout-mismatches"),
            "{name}"
        );
        assert_eq!(
            board.items("unconverted").len() as f64,
            board.tag().number("data-unconverted"),
            "{name}"
        );
    }
}
