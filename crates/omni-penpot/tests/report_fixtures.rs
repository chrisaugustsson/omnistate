//! US-104: `omni-penpot report` on the real exports in fixtures/penpot/, which
//! is what `make penpot-compare` reads (see report_synthetic.rs for the page).
//! These tests skip, with a message, when the folder holds no export.
//!
//! - tirebud-idag.penpot has Penpot's PNGs for its 15 boards: each is shown with
//!   its three images.
//! - tirebud-ordrar.penpot has none, and 795 boards: each is layout comparison
//!   only.
//! - Each file has a board "Unplaced" on each page with shapes outside its
//!   boards (US-054): one in idag, two in ordrar.
//! - The structural mismatches (`data-layout-mismatches`, US-054, Q49) are
//!   measured with each text pinned to Penpot's stored box inside the
//!   comparison only; with the converter fixes of US-054 the two files have at
//!   most 1,000 of them between them (46,030 layout mismatches before).
//!
//! The report of the whole folder is made once. Nothing here asks for a pixel
//! score: the page shows how far OmniState is from Penpot, it does not pass or
//! fail.

mod common;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use common::convert::{run_cli, temp_dir};
use common::repo_root;
use common::report::*;

struct Run {
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
        Some(Run { report })
    })
    .as_ref()
}

#[test]
fn the_two_exports_have_at_most_1000_structural_mismatches() {
    let Some(run) = run() else { return };
    let folder = repo_root().join("fixtures/penpot");
    let files = ["tirebud-idag.penpot", "tirebud-ordrar.penpot"];
    if !files.iter().all(|file| folder.join(file).is_file()) {
        eprintln!("SKIPPED: the mismatch target needs both TireBud exports in fixtures/penpot/");
        return;
    }
    let mut per_file: BTreeMap<String, f64> = BTreeMap::new();
    for board in run.report.boards() {
        *per_file
            .entry(board.attr("data-file").expect("a file"))
            .or_default() += board.tag().number("data-layout-mismatches");
    }
    let (summary, _) = run.report.summary();
    let structural = summary.number("data-layout-mismatches");
    assert!(
        structural <= 1_000.0,
        "`make penpot-compare` finds {structural} structural mismatches over 2 px, with the \
         texts pinned to Penpot's boxes, more than 1,000: {per_file:?}"
    );
}
