//! US-101: docs/formats/penpot-export.md describes the parts of the `.penpot`
//! format that omni-penpot reads, written from observation of exported files and
//! Penpot's public user documentation, never from Penpot's source code (the
//! licence rule of the project). A test can check that the document exists and
//! covers the parts the crate reads; whether it is in the author's own words is a
//! review check.

mod common;

use std::fs;

fn document() -> String {
    let path = common::repo_root().join("docs/formats/penpot-export.md");
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

#[test]
fn the_document_covers_the_archive_layout() {
    let text = document();
    for part in ["manifest.json", "files/", "pages/", "index", "shapes"] {
        assert!(
            text.contains(part),
            "the format document does not mention `{part}`"
        );
    }
    assert!(
        text.contains(common::ROOT_ID),
        "the format document does not explain the root frame"
    );
}

#[test]
fn the_document_covers_what_the_reader_takes_from_a_shape() {
    let text = document();
    for field in ["selrect", "parentId", "hidden", "type"] {
        assert!(
            text.contains(field),
            "the format document does not mention the field `{field}`"
        );
    }
}

#[test]
fn the_document_names_the_shape_types_and_what_happens_to_the_others() {
    let text = document();
    for kind in ["frame", "group", "rect", "circle", "text", "path"] {
        assert!(
            text.contains(&format!("`{kind}`")),
            "the format document does not describe the shape type `{kind}`"
        );
    }
    assert!(
        text.to_lowercase().contains("unknown"),
        "the format document does not say what becomes of unknown shape types"
    );
}

#[test]
fn the_document_says_it_comes_from_observation_and_links_no_penpot_source() {
    let text = document();
    assert!(
        text.to_lowercase().contains("observ"),
        "the format document does not say that it is written from observation"
    );
    let lower = text.to_lowercase();
    assert!(
        !lower.contains("github.com/penpot"),
        "the format document links Penpot's source code"
    );
    assert!(
        !lower.contains("gitlab.com/penpot"),
        "the format document links Penpot's source code"
    );
}

#[test]
fn the_document_says_how_the_converter_maps_penpot_as_penpot_means_it() {
    // US-054: the line pitch of texts, the Unplaced board, and what is listed
    // in the report instead of converted.
    let text = document();
    for part in [
        "positionData",
        "1/128",
        "Unplaced",
        "interactions",
        "flows",
        "constraintsH",
        "constraintsV",
        "minWidth",
        "justifySelf",
    ] {
        assert!(
            text.contains(part),
            "the format document does not mention `{part}`"
        );
    }
}
