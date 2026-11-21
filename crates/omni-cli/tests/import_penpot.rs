//! US-051, US-105: `omni import penpot <file.penpot> --project <id> [--track
//! <name>]`, tested as the built binary against a mock HTTP server. The import
//! against the real API is in tests/stack_import_penpot.rs, and the rules of
//! the import in crates/omni-penpot/tests/import_synthetic.rs.
//!
//! The contract these tests pin down:
//!
//! - The export is read before anything is sent: a file that is missing, or
//!   that is not a Penpot export, is a one-line error that names the file and
//!   sends nothing.
//! - Errors are one line on standard error, as in US-015
//!   (tests/common/mod.rs).
//! - A copy of a library component that the project's design system already
//!   has from Penpot (its `origin`) is sent as an instance of it, even when
//!   the export brings no tokens or components of its own, and nothing is
//!   written to the design system then.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";

fn mock() -> MockServer {
    MockServer::start(|_| Reply::json(500, &json!({ "error": "unexpected" })))
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--project", PROJECT, "--as", "run-51/importer"])
        .run()
}

/// An error as US-015 defines it: a documented non-zero exit code, nothing on
/// standard output and exactly one line on standard error that has every
/// needle in it.
fn assert_one_line_error(out: &Out, needles: &[&str]) {
    assert_ne!(out.code, 0, "expected a failure; stdout: {:?}", out.stdout);
    assert!(
        documented_exit_codes().contains(&out.code),
        "exit code {} is not documented under \"Exit codes\" in omni --help",
        out.code
    );
    assert_eq!(out.stdout, "", "an error prints nothing on standard output");
    let lines = out.stderr_lines();
    assert_eq!(
        lines.len(),
        1,
        "one line on standard error: {:?}",
        out.stderr
    );
    let line = lines[0].to_lowercase();
    for needle in needles {
        assert!(
            line.contains(&needle.to_lowercase()),
            "the error line should contain {needle:?}: {:?}",
            lines[0]
        );
    }
}

fn assert_nothing_sent(server: &MockServer) {
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

#[test]
fn a_missing_file_is_a_one_line_error_that_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let missing = home.join("nirgendwo.penpot");
    let out = run(&home, &["import", "penpot", missing.to_str().unwrap()]);
    assert_one_line_error(&out, &["nirgendwo.penpot"]);
    assert_nothing_sent(&server);
}

// A file that uses a library imported before ------------------------------------------------

const LIBRARY: &str = "f0000000-0000-4000-8000-0000000000b1";
const AREA: &str = "f0000000-0000-4000-8000-0000000000b2";
const AREA_PAGE: &str = "a0000000-0000-4000-8000-0000000000b2";
const BUTTON: &str = "c0000000-0000-4000-8000-0000000000b1";
const MAIN: &str = "00000000-0000-4000-8000-0000000000b1";
const MAIN_LABEL: &str = "00000000-0000-4000-8000-0000000000b2";
const ROOT: &str = "00000000-0000-0000-0000-000000000000";

/// A shape of the area file, with the fields the reader needs.
fn area_shape(id: &str, kind: &str, name: &str, parent: &str, extra: Value) -> Value {
    let mut shape = json!({
        "id": id, "name": name, "type": kind, "parentId": parent, "frameId": parent,
        "x": 0, "y": 0, "width": 160, "height": 40, "rotation": 0,
        "selrect": { "x": 0, "y": 0, "width": 160, "height": 40, "x1": 0, "y1": 0, "x2": 160, "y2": 40 },
        "strokes": [], "fills": []
    });
    for (key, value) in extra.as_object().unwrap() {
        shape[key] = value.clone();
    }
    shape
}

/// An export of the file Kassa alone, whose board holds a copy of the
/// library's Button with its Label changed to "Betala". It has no tokens and
/// no components of its own.
fn area_export(home: &Scratch) -> std::path::PathBuf {
    let board = "00000000-0000-4000-8000-0000000000c1";
    let copy = "00000000-0000-4000-8000-0000000000c2";
    let label = "00000000-0000-4000-8000-0000000000c3";
    let run = json!({
        "fontId": "gfont-inter", "fontFamily": "Inter", "fontSize": "14", "fontWeight": "400",
        "fontVariantId": "regular", "fontStyle": "normal", "lineHeight": "1.5",
        "letterSpacing": "0", "textAlign": "left", "textDecoration": "none",
        "textTransform": "none", "textDirection": "ltr",
        "fills": [{ "fillColor": "#ffffff", "fillOpacity": 1 }], "text": "Betala"
    });
    let mut paragraph = run.clone();
    paragraph.as_object_mut().unwrap().remove("text");
    paragraph["type"] = json!("paragraph");
    paragraph["children"] = json!([run]);
    let dir = format!("files/{AREA}/pages/{AREA_PAGE}");
    let shapes = [
        area_shape(
            ROOT,
            "frame",
            "Root Frame",
            ROOT,
            json!({ "shapes": [board] }),
        ),
        area_shape(board, "frame", "Kassa", ROOT, json!({ "shapes": [copy] })),
        area_shape(
            copy,
            "frame",
            "Köp",
            board,
            json!({
                "shapes": [label], "componentRoot": true, "componentId": BUTTON,
                "componentFile": LIBRARY, "shapeRef": MAIN, "touched": []
            }),
        ),
        area_shape(
            label,
            "text",
            "Label",
            copy,
            json!({
                "shapeRef": MAIN_LABEL, "growType": "fixed", "touched": ["text-content-text"],
                "content": { "type": "root", "children": [{ "type": "paragraph-set", "children": [paragraph] }] }
            }),
        ),
    ];
    let mut entries = vec![
        (
            "manifest.json".to_string(),
            json!({
                "type": "penpot/export-files", "version": 1,
                "files": [{ "id": AREA, "name": "TireBud - Kassa", "features": [] }],
                "relations": []
            })
            .to_string()
            .into_bytes(),
        ),
        (
            format!("files/{AREA}.json"),
            json!({ "id": AREA, "name": "TireBud - Kassa" })
                .to_string()
                .into_bytes(),
        ),
        (
            format!("{dir}.json"),
            json!({ "id": AREA_PAGE, "name": "Kassa", "index": 0 })
                .to_string()
                .into_bytes(),
        ),
    ];
    for shape in &shapes {
        entries.push((
            format!("{dir}/{}.json", shape["id"].as_str().unwrap()),
            shape.to_string().into_bytes(),
        ));
    }
    let path = home.join("kassa.penpot");
    std::fs::write(&path, stored_zip(&entries)).expect("write the export");
    path
}

#[test]
fn a_copy_of_a_library_the_design_system_has_is_sent_as_an_instance_of_it() {
    let base = format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}");
    // The design system holds the library's Button, imported before.
    let system = json!({
        "$components": [{
            "id": "btn00001", "type": "component", "name": "Button",
            "width": { "fixed": 160 }, "height": { "fixed": 40 }, "layout": { "mode": "none" },
            "origin": {
                "tool": "penpot", "file": LIBRARY, "component": BUTTON,
                "layers": { MAIN: "btn00001", MAIN_LABEL: "txt00001" }
            },
            "children": [{
                "id": "txt00001", "type": "text", "name": "Label", "width": "hug",
                "height": "hug", "content": "Köp", "fontFamily": "Inter", "fontSize": 14
            }]
        }]
    });
    let server = MockServer::start(move |request| {
        let path = request.path.split('?').next().unwrap_or_default();
        let answer = match (request.method.as_str(), path.strip_prefix(&base)) {
            ("GET", Some("/tokens")) => json!({ "doc_id": "system-doc", "tokens": {} }),
            ("GET", Some("/docs/system-doc/html")) => json!({ "html": system.to_string() }),
            ("POST", Some("/docs")) => json!({ "doc": { "id": "design-doc" } }),
            ("GET", Some("/docs/design-doc/html")) => json!({ "pages": [{ "id": "pg000001" }] }),
            ("POST", Some("/docs/design-doc/ops")) => {
                json!({ "results": [{ "node": "nd000001", "created": [] }] })
            }
            _ => return Reply::json(500, &json!({ "error": "unexpected" })),
        };
        Reply::json(200, &answer)
    });
    let home = logged_in(&server.url);
    let file = area_export(&home);
    let out = run(
        &home,
        &["import", "penpot", file.to_str().unwrap(), "--json"],
    );
    assert_eq!(
        out.code, 0,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );

    let requests = server.requests();
    assert!(
        requests.iter().all(|request| request.method != "PUT"),
        "nothing is written to the design system: {:?}",
        requests
            .iter()
            .map(|r| format!("{} {}", r.method, r.path))
            .collect::<Vec<_>>()
    );
    let sent: String = requests
        .iter()
        .filter(|request| request.path.ends_with("/ops"))
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect();
    assert!(
        sent.contains(r#"component=\"btn00001\""#),
        "the copy places the design system's Button: {sent}"
    );
    assert!(
        sent.contains(r#"txt00001: {content: \"Betala\"}"#),
        "the override names the Button's Label: {sent}"
    );
}

/// A zip archive of `entries` with every file stored uncompressed.
fn stored_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in entries {
        let offset = archive.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        // Local file header.
        archive.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        archive.extend_from_slice(&20u16.to_le_bytes());
        archive.extend_from_slice(&[0; 2 + 2 + 2]);
        archive.extend_from_slice(&33u16.to_le_bytes());
        archive.extend_from_slice(&crc.to_le_bytes());
        archive.extend_from_slice(&size.to_le_bytes());
        archive.extend_from_slice(&size.to_le_bytes());
        archive.extend_from_slice(&(name.len() as u16).to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(data);
        // Central directory header.
        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&[0; 2 + 2 + 2]);
        directory.extend_from_slice(&33u16.to_le_bytes());
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&[0; 2 + 2 + 2 + 2 + 4]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let directory_offset = archive.len() as u32;
    let directory_size = directory.len() as u32;
    archive.extend_from_slice(&directory);
    // End of central directory record.
    archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    archive.extend_from_slice(&[0; 2 + 2]);
    archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    archive.extend_from_slice(&directory_size.to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
