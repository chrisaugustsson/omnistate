//! US-102: the command `omni-penpot convert <file.penpot> --out <dir>`.
//!
//! It writes into `<dir>` (creating it if needed):
//!
//! - `designs/<name>.json`: one OmniState design per board, each the JSON of the
//!   page that `Conversion::boards[..].design` holds, under a file name of its
//!   own for every board, whatever the names of the boards;
//! - `report.json`: the conversion report, `{"entries": [...]}` with, for each
//!   entry, `board`, `boardId`, `shapeId`, `name`, `reason` and, for a property
//!   left out of a shape that was converted, `property`;
//! - `attachments/<id>.<extension>`: each image that the designs refer to as
//!   `attachment:<id>`.
//!
//! The files are the same on every run, and exit status says whether it worked.
//! Most tests build their own export; the last one needs fixtures/penpot/ and
//! skips, with a message, when it is empty.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use omni_core::design::{Node, NodeKind};
use omni_penpot::convert_file;
use omni_render::paint::Assets;
use serde_json::{Value, json};

use common::convert::*;
use common::fixture;

/// An export with two boards, an image, and a boolean shape that is not
/// converted.
fn export_with_everything(test: &str) -> std::path::PathBuf {
    let media = uid(100);
    write_export_with(
        test,
        vec![
            shape(1, "frame", "Same name", [0.0, 0.0, 400.0, 300.0]).kids(vec![
                shape(2, "rect", "Photo", [0.0, 0.0, 100.0, 100.0])
                    .with(json!({ "fills": [image_fill(&media)] })),
                shape(3, "bool", "Union", [120.0, 0.0, 40.0, 40.0]),
                text(4, "Title", [0.0, 120.0, 100.0, 24.0], "Hej"),
            ]),
            shape(5, "frame", "Same name", [500.0, 0.0, 200.0, 100.0]),
            shape(6, "frame", "Other / name", [800.0, 0.0, 200.0, 100.0]),
        ],
        media_entries(&media, &uid(200)),
    )
}

/// Every file under `dir`, by path relative to it, with its bytes.
fn read_tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, found: &mut BTreeMap<String, Vec<u8>>) {
        for entry in
            fs::read_dir(dir).unwrap_or_else(|error| panic!("cannot list {dir:?}: {error}"))
        {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                walk(root, &path, found);
            } else {
                let name = path.strip_prefix(root).expect("under the root");
                found.insert(
                    name.to_string_lossy().to_string(),
                    fs::read(&path).expect("a readable file"),
                );
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(dir, dir, &mut found);
    found
}

fn read_designs(out: &Path) -> Vec<Node> {
    files_with_extension(&out.join("designs"), "json")
        .iter()
        .map(|path| {
            let text = fs::read_to_string(path).expect("a readable design");
            serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("{path:?} is not a valid design: {error}"))
        })
        .collect()
}

fn read_report(out: &Path) -> Vec<Value> {
    let text = fs::read_to_string(out.join("report.json")).expect("report.json is written");
    let report: Value = serde_json::from_str(&text).expect("report.json is JSON");
    report["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("report.json has no list of entries: {report}"))
        .clone()
}

#[test]
fn convert_writes_one_design_per_board_the_report_and_the_images() {
    let file = export_with_everything("cli-all");
    let out = temp_dir("all");
    let result = run_convert(&file, &out);
    assert!(
        result.status.success(),
        "omni-penpot failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let designs = files_with_extension(&out.join("designs"), "json");
    assert_eq!(
        designs.len(),
        3,
        "one file for each of the 3 boards: {designs:?}"
    );
    for design in read_designs(&out) {
        assert_eq!(design.kind(), NodeKind::Page);
        assert_eq!(design.children().len(), 1);
        assert_eq!(design.children()[0].kind(), NodeKind::Board);
    }

    let entries = read_report(&out);
    assert_eq!(entries.len(), 1, "only the boolean shape: {entries:#?}");
    let entry = &entries[0];
    assert_eq!(entry["shapeId"], uid(3));
    assert_eq!(entry["name"], "Union");
    assert_eq!(entry["board"], "Same name");
    assert_eq!(entry["boardId"], uid(1));
    assert!(
        entry["reason"]
            .as_str()
            .is_some_and(|reason| !reason.trim().is_empty()),
        "{entry}"
    );

    let mut references = Vec::new();
    for design in read_designs(&out) {
        references.extend(attachment_refs(&json_of(&design)));
    }
    assert_eq!(references.len(), 1);
    let id = references[0].strip_prefix("attachment:").expect("a prefix");
    let images = fs::read_dir(out.join("attachments"))
        .expect("attachments/ is written")
        .map(|entry| entry.expect("an entry").path())
        .collect::<Vec<_>>();
    let image = images
        .iter()
        .find(|path| path.file_stem().is_some_and(|stem| stem == id))
        .unwrap_or_else(|| panic!("no attachment file for {id} in {images:?}"));
    let bytes = fs::read(image).expect("a readable image");
    assert_eq!(bytes, PNG);
    Assets::new()
        .add_image(&references[0].parse().expect("a reference"), &bytes)
        .unwrap_or_else(|error| panic!("the renderer cannot load {image:?}: {error}"));
}

#[test]
fn the_files_hold_what_convert_file_returns() {
    let file = export_with_everything("cli-same-as-api");
    let out = temp_dir("same-as-api");
    assert!(run_convert(&file, &out).status.success());
    let conversion = convert_file(&file).expect("converts");

    let mut from_files: Vec<String> = read_designs(&out)
        .iter()
        .map(|design| serde_json::to_string(design).unwrap())
        .collect();
    let mut from_api: Vec<String> = conversion
        .boards
        .iter()
        .map(|board| serde_json::to_string(&board.design).unwrap())
        .collect();
    from_files.sort();
    from_api.sort();
    assert_eq!(from_files, from_api);

    let mut listed: Vec<(String, String)> = read_report(&out)
        .iter()
        .map(|entry| {
            (
                entry["shapeId"].as_str().unwrap_or_default().to_string(),
                entry["reason"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let mut expected: Vec<(String, String)> = conversion
        .report
        .entries
        .iter()
        .map(|entry| (entry.shape_id.clone(), entry.reason.clone()))
        .collect();
    listed.sort();
    expected.sort();
    assert_eq!(listed, expected);
}

#[test]
fn running_it_twice_writes_identical_files() {
    let file = export_with_everything("cli-twice");
    let (first, second) = (temp_dir("twice-1"), temp_dir("twice-2"));
    assert!(run_convert(&file, &first).status.success());
    assert!(run_convert(&file, &second).status.success());
    let (first, second) = (read_tree(&first), read_tree(&second));
    assert!(
        first.len() >= 5,
        "designs, report and image: {:?}",
        first.keys()
    );
    assert_eq!(first, second);
}

#[test]
fn the_output_directory_is_created_with_its_parents() {
    let file = export_with_everything("cli-mkdir");
    let out = temp_dir("mkdir").join("not/yet/there");
    let result = run_convert(&file, &out);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(out.join("report.json").is_file());
}

#[test]
fn a_board_without_problems_gives_an_empty_report() {
    let file = write_export(
        "cli-clean",
        vec![shape(1, "frame", "Home", [0.0, 0.0, 100.0, 100.0])],
    );
    let out = temp_dir("clean");
    assert!(run_convert(&file, &out).status.success());
    assert_eq!(read_report(&out), Vec::<Value>::new());
    assert_eq!(read_designs(&out).len(), 1);
}

#[test]
fn bad_usage_and_unreadable_files_fail_with_a_message() {
    let file = export_with_everything("cli-usage");
    let out = temp_dir("usage");
    let out_arg = out.to_str().expect("a UTF-8 path");
    let file_arg = file.to_str().expect("a UTF-8 path");

    let no_arguments = run_cli(&[]);
    assert!(!no_arguments.status.success());
    assert!(
        String::from_utf8_lossy(&no_arguments.stderr)
            .to_lowercase()
            .contains("usage"),
        "no usage message: {}",
        String::from_utf8_lossy(&no_arguments.stderr)
    );

    let no_out = run_cli(&["convert", file_arg]);
    assert!(!no_out.status.success());
    assert!(
        String::from_utf8_lossy(&no_out.stderr).contains("--out"),
        "the message does not name --out: {}",
        String::from_utf8_lossy(&no_out.stderr)
    );

    let missing = out.join("nothing.penpot");
    let missing_arg = missing.to_str().expect("a UTF-8 path");
    let not_there = run_cli(&["convert", missing_arg, "--out", out_arg]);
    assert!(!not_there.status.success());
    assert!(
        String::from_utf8_lossy(&not_there.stderr).contains("nothing.penpot"),
        "the message does not name the file: {}",
        String::from_utf8_lossy(&not_there.stderr)
    );

    let not_penpot = write_temp_text("cli-not-an-export.penpot", "this is not a zip archive");
    let broken = run_cli(&[
        "convert",
        not_penpot.to_str().expect("a UTF-8 path"),
        "--out",
        out_arg,
    ]);
    assert!(!broken.status.success());
    assert!(!String::from_utf8_lossy(&broken.stderr).trim().is_empty());
}

fn write_temp_text(name: &str, text: &str) -> std::path::PathBuf {
    common::write_temp(name, text.as_bytes())
}

#[test]
fn idag_converts_to_15_designs_and_a_report() {
    let Some(file) = fixture("tirebud-idag.penpot") else {
        return;
    };
    let out = temp_dir("idag");
    let result = run_convert(&file, &out);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(read_designs(&out).len(), 15);
    assert_eq!(files_with_extension(&out.join("designs"), "json").len(), 15);
    let entries = read_report(&out);
    // At least the 8 texts that sit outside the boards.
    assert!(entries.len() >= 8, "{} entries", entries.len());
    for entry in &entries {
        for key in ["board", "boardId", "shapeId", "name", "reason"] {
            assert!(entry.get(key).is_some(), "{entry} has no `{key}`");
        }
    }
}
