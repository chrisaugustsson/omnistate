//! US-103: `omni-penpot compare <file.penpot> --out <dir>` on small exports and
//! PNGs that the tests build themselves. These tests do not need
//! fixtures/penpot/.
//!
//! The command converts the boards of the export (as `convert` does) and, for
//! each board, compares OmniState's rendering and layout with Penpot's. Penpot's
//! own PNG of a board is looked for in the folder beside the export that has the
//! export's name without `.penpot`: `fixtures/penpot/tirebud-idag/` for
//! `fixtures/penpot/tirebud-idag.penpot`. Penpot names a PNG after the board,
//! with each `/` written ` _ ` (the board "Idag / Dag" is `Idag _ Dag.png`), and
//! numbers the boards that share a name `-2`, `-3`, and so on, the board on top
//! of the stacking order keeping the bare name (as in tirebud-idag, where the
//! four texts called "Kolumnrubrik" give `Kolumnrubrik.png` for the one listed
//! last, then `-2` for the one before it). A board without a PNG, and a PNG
//! that no board has, such as one of a text on the page, leave the others alone.
//! Without the folder no board has a PNG.
//!
//! It writes into `<dir>` (creating it if needed) a `compare.json`, and the
//! images it refers to by paths relative to `<dir>`. `compare.json` is
//! `{ "tolerance": N, "boards": [..] }`, with `tolerance` the largest difference
//! in a channel (red, green, blue or alpha, each 0 to 255) of a pixel that still
//! counts as the same, and one entry per converted board, in the order of
//! `convert`, with:
//!
//! - `board` (the name), `boardId` (the Penpot id) and `page`;
//! - `layoutOnly`, true when the board has no Penpot PNG, and then a `note` that
//!   says "layout comparison only" and `pixel` null;
//! - `pixel`, for a board that has a PNG: `{ width, height, compared, masked,
//!   differing, share, render, diff }`. The board is drawn at 2x, with the
//!   board's top left corner at the image's, on a canvas of the size of Penpot's
//!   PNG (`width` by `height`, which may be a pixel more than twice the board's
//!   size where that has a fraction), and written as the PNG `render`. A pixel
//!   differs when its red, green, blue or alpha is more than `tolerance` away
//!   from Penpot's. `masked` counts the pixels left out, the boxes of the board's
//!   unconverted shapes; `compared` is every other pixel; `differing` is how
//!   many of those differ and `share` is `differing / compared`. `diff` is a PNG
//!   of the same size in which every differing pixel has one highlight color
//!   that no other pixel has, and masked pixels do not;
//! - `layout`: `{ compared, mismatches }`, for every converted shape of the board
//!   the board included: Penpot's stored box against the box that OmniState's
//!   layout computes, both in page coordinates. A mismatch is `{ shapeId, name,
//!   penpot: {x, y, width, height}, omnistate: {x, y, width, height}, offBy }`
//!   with `offBy` the largest of the four differences, and is listed when that is
//!   more than 2 px. These are the structural mismatches (US-054, Q49): inside
//!   the comparison only, each text is pinned to the box Penpot stored for it,
//!   so that Penpot's whole-pixel rounding of text boxes does not count;
//! - `textDrift`: `{ compared, mismatches }` in the same form, measured on the
//!   design as it is imported, with every text following Chrome. The converted
//!   design never holds the pin: its texts keep their hug sizing;
//! - `unconverted`: `[{ shapeId, name, reason }]`, the shapes of the board that
//!   the converter left out whole (a report entry with no property), whose
//!   boxes in Penpot's PNG are masked. A shape that was converted without one of
//!   its properties, such as a blur, is not left out and is not masked.
//!
//! The output is the same on every run.

mod common;

use std::fs;

use omni_penpot::convert_file;
use serde_json::{Value, json};

use common::compare::*;
use common::convert::*;
use common::repo_root;

/// A board with a rectangle, and the reference PNG: no pixel differs.
#[test]
fn a_board_is_rendered_at_2x_as_a_png_the_size_of_penpots() {
    let outcome = compare_synthetic(
        "render",
        vec![standard_board(1, vec![])],
        &[("Home.png", &standard_png())],
    );
    let board = outcome.board(&uid(1));
    assert_eq!(board["board"], "Home");
    assert_eq!(board["layoutOnly"], false);
    let pixel = &board["pixel"];
    assert_eq!(field(pixel, "width"), 400.0);
    assert_eq!(field(pixel, "height"), 200.0);

    let render = outcome.png(&pixel["render"]);
    assert_eq!((render.width, render.height), (400, 200));
    // The board is the whole image, at 2x: its square covers 200 by 100 pixels
    // from the top left corner, and the edges are crisp.
    assert_eq!(render.at(0, 0), RED);
    assert_eq!(render.at(199, 99), RED);
    assert_eq!(render.at(100, 50), RED);
    assert_eq!(render.at(200, 50), WHITE);
    assert_eq!(render.at(100, 100), WHITE);
    assert_eq!(render.at(399, 199), WHITE);
}

#[test]
fn identical_images_have_no_differing_pixels() {
    let outcome = compare_synthetic(
        "identical",
        vec![standard_board(1, vec![])],
        &[("Home.png", &standard_png())],
    );
    tolerance(&outcome.report);
    let board = outcome.board(&uid(1));
    let pixel = &board["pixel"];
    assert_eq!(field(pixel, "compared"), 80_000.0);
    assert_eq!(field(pixel, "masked"), 0.0);
    assert_eq!(field(pixel, "differing"), 0.0);
    assert_eq!(field(pixel, "share"), 0.0);
    assert_eq!(board["unconverted"], json!([]));
    assert!(board["note"].is_null() || board["note"] == "");
}

#[test]
fn the_share_of_differing_pixels_and_a_diff_image_that_highlights_them() {
    let block = [300, 20, 340, 40];
    let mut png = standard_png();
    png.fill(block, BLUE);
    let outcome = compare_synthetic(
        "share",
        vec![standard_board(1, vec![])],
        &[("Home.png", &png)],
    );
    let pixel = &outcome.board(&uid(1))["pixel"];
    assert_eq!(field(pixel, "compared"), 80_000.0);
    assert_eq!(field(pixel, "differing"), 800.0);
    assert!((field(pixel, "share") - 0.01).abs() < 1e-9, "{pixel}");

    let diff = outcome.png(&pixel["diff"]);
    assert_eq!((diff.width, diff.height), (400, 200));
    let highlight = diff.at(300, 20);
    for y in 0..200 {
        for x in 0..400 {
            let highlighted = diff.at(x, y) == highlight;
            assert_eq!(
                highlighted,
                Pixels::within(x, y, block),
                "pixel ({x}, {y}) of the diff image is {:?}, the highlight is {highlight:?}",
                diff.at(x, y)
            );
        }
    }
}

#[test]
fn a_difference_up_to_the_tolerance_is_the_same_and_one_more_differs() {
    let first = compare_synthetic(
        "tolerance-probe",
        vec![standard_board(1, vec![])],
        &[("Home.png", &standard_png())],
    );
    let tolerance = tolerance(&first.report);

    let mut png = standard_png();
    // Every channel the tolerance away, and then one channel a step further.
    let within = 255 - tolerance;
    png.fill([300, 20, 320, 40], [within, within, within, 255]);
    png.fill([340, 20, 360, 40], [254 - tolerance, 255, 255, 255]);
    png.fill([300, 60, 320, 80], [255, 255, 255, 255 - tolerance]);
    png.fill([340, 60, 360, 80], [255, 255, 255, 254 - tolerance]);
    let outcome = compare_synthetic(
        "tolerance",
        vec![standard_board(1, vec![])],
        &[("Home.png", &png)],
    );
    let pixel = &outcome.board(&uid(1))["pixel"];
    assert_eq!(
        field(pixel, "differing"),
        800.0,
        "the two blocks a step beyond the tolerance of {tolerance} differ: {pixel}"
    );
}

#[test]
fn unconverted_shapes_are_masked_out_of_the_pixel_comparison_and_listed() {
    // Penpot draws the union at (200, 0) to (280, 80) of its PNG; OmniState
    // draws nothing there. Elsewhere one block really differs.
    let mut png = standard_png();
    png.fill([200, 0, 280, 80], BLUE);
    png.fill([300, 20, 320, 40], BLUE);
    let outcome = compare_synthetic(
        "masked",
        vec![standard_board(
            1,
            vec![shape(3, "bool", "Union", [200.0, 50.0, 40.0, 40.0])],
        )],
        &[("Home.png", &png)],
    );
    let board = outcome.board(&uid(1));
    let pixel = &board["pixel"];
    assert_eq!(field(pixel, "masked"), 6_400.0);
    assert_eq!(field(pixel, "compared"), 73_600.0);
    assert_eq!(field(pixel, "differing"), 400.0, "{pixel}");
    assert!((field(pixel, "share") - 400.0 / 73_600.0).abs() < 1e-9);

    let diff = outcome.png(&pixel["diff"]);
    let highlight = diff.at(300, 20);
    for (x, y) in [(200, 0), (240, 40), (279, 79)] {
        assert_ne!(diff.at(x, y), highlight, "({x}, {y}) is masked, not wrong");
    }

    let listed = board["unconverted"].as_array().expect("a list");
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["shapeId"], uid(3));
    assert_eq!(listed[0]["name"], "Union");
    assert!(
        listed[0]["reason"].as_str().is_some_and(|r| !r.is_empty()),
        "{listed:?}"
    );
    // Nothing OmniState did not draw can be off in the layout comparison.
    let layout = &board["layout"];
    assert_eq!(field(layout, "compared"), 2.0, "the board and its square");
    assert_eq!(layout["mismatches"], json!([]));
}

#[test]
fn a_shape_converted_without_one_property_is_not_masked() {
    // The blur is left out of the rectangle, which is drawn red and sharp; in
    // Penpot's PNG it is blue.
    let mut png = standard_png();
    png.fill([200, 0, 280, 80], BLUE);
    let blurred = rect(3, "Blurred", [200.0, 50.0, 40.0, 40.0], "#ff0000").with(json!({
        "blur": { "id": uid(901), "type": "layer-blur", "value": 8, "hidden": false }
    }));
    let outcome = compare_synthetic(
        "blur",
        vec![standard_board(1, vec![blurred])],
        &[("Home.png", &png)],
    );
    let board = outcome.board(&uid(1));
    let pixel = &board["pixel"];
    assert_eq!(field(pixel, "masked"), 0.0);
    assert_eq!(field(pixel, "differing"), 6_400.0, "{pixel}");
    assert_eq!(board["unconverted"], json!([]));
}

#[test]
fn layout_lists_the_shapes_more_than_2_px_from_where_penpot_stored_them() {
    // A column of four, with no padding or gap, lays the squares out at y 50,
    // 90, 130 and 170, all at x 100. Penpot's stored boxes put B 3 px to the
    // right (off), C 2 px (within), and D 5 px down (off). The children of a
    // flex frame are listed last-appearing first.
    let square = |n, name: &str, x, y| shape(n, "rect", name, [x, y, 100.0, 40.0]);
    let outcome = compare_synthetic(
        "layout",
        vec![
            shape(1, "frame", "Home", [100.0, 50.0, 300.0, 200.0])
                .with(flex("column"))
                .kids(vec![
                    square(5, "D", 100.0, 175.0),
                    square(4, "C", 102.0, 130.0),
                    square(3, "B", 103.0, 90.0),
                    square(2, "A", 100.0, 50.0),
                ]),
        ],
        &[],
    );
    let board = outcome.board(&uid(1));
    let layout = &board["layout"];
    assert_eq!(field(layout, "compared"), 5.0, "the board and its four");
    let mismatches = layout["mismatches"].as_array().expect("a list");
    let mut ids: Vec<&str> = mismatches
        .iter()
        .map(|m| m["shapeId"].as_str().expect("an id"))
        .collect();
    ids.sort();
    assert_eq!(ids, [uid(3), uid(5)], "only B and D: {mismatches:#?}");

    let of = |name: &str| {
        mismatches
            .iter()
            .find(|m| m["name"] == name)
            .unwrap_or_else(|| panic!("no mismatch for {name}"))
    };
    let (b, d) = (of("B"), of("D"));
    for (found, penpot, omnistate, off_by) in [
        (
            b,
            [103.0, 90.0, 100.0, 40.0],
            [100.0, 90.0, 100.0, 40.0],
            3.0,
        ),
        (
            d,
            [100.0, 175.0, 100.0, 40.0],
            [100.0, 170.0, 100.0, 40.0],
            5.0,
        ),
    ] {
        for (side, expected) in [("penpot", penpot), ("omnistate", omnistate)] {
            for (key, want) in ["x", "y", "width", "height"].iter().zip(expected) {
                assert_near(
                    field(&found[side], key),
                    want,
                    0.01,
                    &format!("{key} of the {side} box of {}", found["name"]),
                );
            }
        }
        assert_near(field(found, "offBy"), off_by, 0.01, "offBy");
    }
}

/// The shape ids of the mismatches of a layout comparison, sorted.
fn mismatch_ids(comparison: &Value) -> Vec<String> {
    let mut ids: Vec<String> = comparison["mismatches"]
        .as_array()
        .unwrap_or_else(|| panic!("no list of mismatches in {comparison}"))
        .iter()
        .map(|m| m["shapeId"].as_str().expect("an id").to_string())
        .collect();
    ids.sort();
    ids
}

#[test]
fn structural_mismatches_pin_texts_to_penpots_boxes_and_text_drift_follows_chrome() {
    let file = write_export("compare-text-drift", vec![text_drift_board(1)]);
    let outcome = compare_ok(&file, &temp_dir("compare-text-drift"));
    let board = outcome.board(&uid(1));

    // With each text pinned to Penpot's box, every shape is where Penpot put it.
    let structural = &board["layout"];
    assert_eq!(
        field(structural, "compared"),
        6.0,
        "the board and its five shapes"
    );
    assert_eq!(
        structural["mismatches"],
        json!([]),
        "with the texts pinned to Penpot's boxes nothing is off"
    );

    // As imported, the texts follow Chrome and everything after them moves.
    let drift = &board["textDrift"];
    assert!(
        drift.is_object(),
        "compare.json has the text drift of the board: {board:#}"
    );
    assert_eq!(
        field(drift, "compared"),
        6.0,
        "the same shapes are compared"
    );
    assert_eq!(
        mismatch_ids(drift),
        [uid(2), uid(3), uid(4), uid(5), uid(6)],
        "the texts and the shapes after them drift: {drift:#}"
    );
    let of = |name: &str| {
        drift["mismatches"]
            .as_array()
            .expect("a list")
            .iter()
            .find(|m| m["name"] == name)
            .unwrap_or_else(|| panic!("no text drift for {name}: {drift:#}"))
            .clone()
    };
    let two_lines = of("Two lines");
    assert_near(
        field(&two_lines["penpot"], "height"),
        46.0,
        0.01,
        "Penpot's box",
    );
    assert_near(
        field(&two_lines["omnistate"], "height"),
        2.0 * 20.296875,
        0.01,
        "two line pitches, as Chrome lays the text out",
    );
    let below = of("Below");
    assert_near(field(&below["penpot"], "y"), 46.0, 0.01, "Penpot's box");
    assert_near(
        field(&below["omnistate"], "y"),
        2.0 * 20.296875,
        0.01,
        "below the text as Chrome lays it out",
    );
    let hi = of("Hi");
    assert!(
        field(&hi["omnistate"], "width") < 39.0,
        "the text is as wide as Chrome measures it, not Penpot's 40 px: {hi}"
    );

    // The pin is in the comparison only: the converted texts still hug.
    let conversion =
        convert_file(&file).unwrap_or_else(|error| panic!("cannot convert {file:?}: {error}"));
    let root = board_node(common::convert::board(&conversion, "Texts"));
    for (name, axis, min) in [
        ("Two lines", "height", "minHeight"),
        ("Hi", "width", "minWidth"),
        ("Hi", "height", "minHeight"),
    ] {
        let json = json_of(node(root, name));
        assert_sizing(&json[axis], Sz::Hug, &format!("{name} hugs its {axis}"));
        assert!(
            json.get(min).is_none(),
            "{name}: the converted design holds no pin, no {min}: {json:#}"
        );
    }
}

#[test]
fn boards_without_a_penpot_png_get_the_layout_comparison_only() {
    let outcome = compare_synthetic(
        "layout-only",
        vec![
            standard_board(1, vec![]),
            white_board(
                10,
                "No PNG",
                [500.0, 50.0, 200.0, 100.0],
                vec![rect(11, "Bar", [500.0, 50.0, 80.0, 20.0], "#00ff00")],
            ),
        ],
        &[("Home.png", &standard_png())],
    );
    let with_png = outcome.board(&uid(1));
    assert_eq!(with_png["layoutOnly"], false);
    assert!(with_png["pixel"].is_object());
    assert_eq!(field(&with_png["layout"], "compared"), 2.0);

    let without = outcome.board(&uid(10));
    assert_eq!(without["layoutOnly"], true);
    assert!(without["pixel"].is_null(), "{without}");
    let note = without["note"].as_str().expect("a note").to_lowercase();
    assert!(note.contains("layout comparison only"), "{note}");
    assert_eq!(field(&without["layout"], "compared"), 2.0);
    assert_eq!(without["layout"]["mismatches"], json!([]));
}

#[test]
fn without_a_png_folder_every_board_is_layout_only() {
    let outcome = compare_synthetic("no-folder", vec![standard_board(1, vec![])], &[]);
    let board = outcome.board(&uid(1));
    assert_eq!(board["layoutOnly"], true);
    assert!(board["pixel"].is_null());
    assert_eq!(field(&board["layout"], "compared"), 2.0);
}

#[test]
fn pngs_are_found_by_board_name_with_slashes_written_as_underscores_and_numbered_twins() {
    let red = Pixels::solid(200, 100, RED);
    let white = Pixels::solid(200, 100, WHITE);
    let board = |n, name: &str, x, hex: &str| {
        shape(n, "frame", name, [x, 0.0, 100.0, 50.0]).with(fill(hex))
    };
    let outcome = compare_synthetic(
        "names",
        vec![
            board(1, "Idag / Dag", 0.0, "#ffffff"),
            // Twin and Twin-2: the board listed last (on top) has the bare name.
            board(2, "Twin", 500.0, "#ffffff"),
            board(3, "Twin", 1000.0, "#ff0000"),
            // A text on the page has a PNG too, and no board.
            board(4, "Other", 1500.0, "#ffffff"),
        ],
        &[
            ("Idag _ Dag.png", &white),
            ("Twin.png", &red),
            ("Twin-2.png", &white),
            ("Label.png", &red),
        ],
    );
    for (id, differing) in [(1, 0.0), (2, 0.0), (3, 0.0)] {
        let board = outcome.board(&uid(id));
        assert_eq!(board["layoutOnly"], false, "{}", board["board"]);
        assert_eq!(
            field(&board["pixel"], "differing"),
            differing,
            "board {id} is compared with its own PNG: {board}"
        );
    }
    let other = outcome.board(&uid(4));
    assert_eq!(other["layoutOnly"], true, "{other}");
    assert_eq!(
        outcome.report["boards"].as_array().expect("a list").len(),
        4,
        "a PNG without a board adds nothing"
    );
}

#[test]
fn the_render_has_the_size_of_penpots_png_even_where_the_board_has_a_fraction() {
    // 100.25 by 50.25 px is 200.5 by 100.5 at 2x, which Penpot exports as 201
    // by 101.
    let png = Pixels::solid(201, 101, WHITE);
    let outcome = compare_synthetic(
        "fraction",
        vec![white_board(1, "Home", [10.0, 10.0, 100.25, 50.25], vec![])],
        &[("Home.png", &png)],
    );
    let pixel = &outcome.board(&uid(1))["pixel"];
    assert_eq!(field(pixel, "width"), 201.0);
    assert_eq!(field(pixel, "height"), 101.0);
    assert_eq!(field(pixel, "compared"), 201.0 * 101.0);
    for key in ["render", "diff"] {
        let image = outcome.png(&pixel[key]);
        assert_eq!((image.width, image.height), (201, 101), "the {key} image");
    }
}

#[test]
fn the_same_inputs_give_the_same_files() {
    let mut png = standard_png();
    png.fill([300, 20, 340, 40], BLUE);
    let top_level = || {
        vec![
            standard_board(
                1,
                vec![shape(3, "bool", "Union", [200.0, 50.0, 40.0, 40.0])],
            ),
            white_board(10, "No PNG", [500.0, 50.0, 200.0, 100.0], vec![]),
        ]
    };
    let first = compare_synthetic("repeat-1", top_level(), &[("Home.png", &png)]);
    let second = compare_synthetic("repeat-2", top_level(), &[("Home.png", &png)]);
    assert_eq!(first.report, second.report);
    let files = |outcome: &Outcome| {
        let mut files = std::collections::BTreeMap::new();
        let mut stack = vec![outcome.out.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("a listable directory") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let name = path.strip_prefix(&outcome.out).expect("below the output");
                    files.insert(
                        name.to_string_lossy().to_string(),
                        fs::read(&path).expect("a readable file"),
                    );
                }
            }
        }
        files
    };
    let (a, b) = (files(&first), files(&second));
    assert!(
        a.len() >= 3,
        "compare.json, a render and a diff: {:?}",
        a.keys()
    );
    assert_eq!(a, b);
}

#[test]
fn a_file_that_does_not_exist_fails_with_a_message() {
    let out = temp_dir("compare-missing");
    let missing = std::env::temp_dir().join("omni-penpot-no-such-file.penpot");
    let result = run_compare(&missing, &out);
    assert!(!result.status.success());
    let message = String::from_utf8_lossy(&result.stderr);
    assert!(
        message.contains("omni-penpot-no-such-file.penpot"),
        "the failure names the file: {message}"
    );
}

#[test]
fn the_tolerance_is_documented() {
    let path = repo_root().join("docs/penpot-compare.md");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("docs/penpot-compare.md is not there: {error}"))
        .to_lowercase();
    let outcome = compare_synthetic("doc", vec![standard_board(1, vec![])], &[]);
    let tolerance = tolerance(&outcome.report);
    assert!(
        text.contains("tolerance"),
        "the document explains the tolerance"
    );
    assert!(
        text.split(|c: char| !c.is_ascii_digit())
            .any(|word| word == tolerance.to_string()),
        "the document gives the tolerance, {tolerance}"
    );
    assert!(
        text.contains("layout comparison only"),
        "the document says what a board without a PNG gets"
    );
}

#[test]
fn the_structural_mismatches_and_the_text_drift_are_documented() {
    let text = fs::read_to_string(repo_root().join("docs/penpot-compare.md"))
        .unwrap_or_else(|error| panic!("docs/penpot-compare.md is not there: {error}"))
        .to_lowercase();
    for words in ["structural mismatches", "text drift", "textdrift", "pinned"] {
        assert!(
            text.contains(words),
            "the document explains the two layout numbers and says {words:?}"
        );
    }
}
