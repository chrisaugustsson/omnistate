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
//! The tests build their own export.

mod common;

use std::fs;
use std::path::Path;

use omni_core::design::Node;
use omni_penpot::convert_file;
use omni_render::paint::Assets;
use serde_json::{Value, json};

use common::convert::*;

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

    let conversion = convert_file(&file).expect("converts");

    let designs = files_with_extension(&out.join("designs"), "json");
    assert_eq!(
        designs.len(),
        3,
        "one file for each of the 3 boards: {designs:?}"
    );
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

    let entries = read_report(&out);
    assert_eq!(entries.len(), 1, "only the boolean shape: {entries:#?}");
    let entry = &entries[0];
    assert_eq!(entry["shapeId"], uid(3));
    assert_eq!(entry["name"], "Union");
    assert_eq!(entry["board"], "Same name");
    assert_eq!(entry["boardId"], uid(1));
    assert_eq!(conversion.report.entries.len(), 1);
    assert_eq!(entry["reason"], conversion.report.entries[0].reason);

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
