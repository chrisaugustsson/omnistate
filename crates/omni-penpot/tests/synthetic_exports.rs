//! US-101: `omni_penpot::read_file` on small exports that the tests build
//! themselves, for the behaviour the real fixtures do not show: shapes of a type
//! the reader does not know and an archive whose order differs from the order of
//! the design. These tests do not need fixtures/penpot/.
//!
//! - A shape of a type other than frame, group, rect, circle, text or path is
//!   read as `ShapeKind::Unknown(type)` with its id (and name), and the shapes
//!   around it are read as usual; so is one that carries nothing but an id and a
//!   type.
//! - Fields the reader does not know do not fail a known shape.
//! - A path whose x, y, width and height are null gets its box from `selrect`.
//! - Pages come in the order of their `index`, shapes in the order of the root
//!   frame's and each shape's `shapes` list, whatever the order of the archive.

mod common;

use omni_penpot::{Page, PenpotFile, ShapeKind, read_file};
use serde_json::json;

use common::{ROOT_ID, SyntheticPage, assert_close, export, find, shape_json, write_temp};

const FILE_ID: &str = "f0000000-0000-4000-8000-000000000001";

fn page(
    id: &str,
    name: &str,
    index: u32,
    top_level: &[&str],
    shapes: Vec<serde_json::Value>,
) -> SyntheticPage {
    SyntheticPage {
        id: id.to_string(),
        name: name.to_string(),
        index,
        top_level: top_level.iter().map(|id| id.to_string()).collect(),
        shapes,
    }
}

fn read(test: &str, pages: &[SyntheticPage]) -> PenpotFile {
    let path = write_temp(
        &format!("{test}.penpot"),
        &export(FILE_ID, "Synthetic", pages),
    );
    read_file(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

fn only_page(file: &PenpotFile) -> &Page {
    assert_eq!(file.pages.len(), 1);
    &file.pages[0]
}

#[test]
fn a_shape_of_an_unknown_type_is_kept_with_its_type_and_id() {
    let shapes = vec![
        shape_json(
            "board-1",
            "frame",
            "Board",
            ROOT_ID,
            [0.0, 0.0, 200.0, 100.0],
            &["a", "union", "bare", "b"],
        ),
        json!({ "id": "bare", "type": "svg-raw" }),
        shape_json(
            "a",
            "rect",
            "Before",
            "board-1",
            [0.0, 0.0, 10.0, 10.0],
            &[],
        ),
        shape_json(
            "union",
            "bool",
            "Union",
            "board-1",
            [20.0, 0.0, 30.0, 30.0],
            &[],
        ),
        shape_json(
            "b",
            "text",
            "After",
            "board-1",
            [60.0, 0.0, 10.0, 10.0],
            &[],
        ),
    ];
    let file = read(
        "unknown_type",
        &[page("p1", "Page", 0, &["board-1"], shapes)],
    );
    let board = &only_page(&file).shapes[0];
    let kinds: Vec<(&str, &ShapeKind)> = board
        .children
        .iter()
        .map(|child| (child.id.as_str(), &child.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("a", &ShapeKind::Rect),
            ("union", &ShapeKind::Unknown("bool".to_string())),
            ("bare", &ShapeKind::Unknown("svg-raw".to_string())),
            ("b", &ShapeKind::Text),
        ]
    );
    assert_eq!(board.children[1].name, "Union");
}

#[test]
fn an_unknown_shape_at_the_top_of_a_page_does_not_fail_the_file() {
    let shapes = vec![
        shape_json(
            "image-1",
            "image",
            "Photo",
            ROOT_ID,
            [0.0, 0.0, 64.0, 64.0],
            &[],
        ),
        shape_json(
            "board-1",
            "frame",
            "Board",
            ROOT_ID,
            [100.0, 0.0, 200.0, 100.0],
            &[],
        ),
    ];
    let file = read(
        "unknown_top_level",
        &[page("p1", "Page", 0, &["image-1", "board-1"], shapes)],
    );
    let page = only_page(&file);
    assert_eq!(page.shapes.len(), 2);
    assert_eq!(page.shapes[0].id, "image-1");
    assert_eq!(page.shapes[0].kind, ShapeKind::Unknown("image".to_string()));
    assert_eq!(page.boards().count(), 1, "an unknown shape is not a board");
    assert_eq!(page.boards().next().unwrap().id, "board-1");
}

#[test]
fn a_path_with_null_geometry_gets_its_box_from_selrect() {
    let mut path = shape_json("p", "path", "Path", ROOT_ID, [10.5, 20.25, 3.0, 4.0], &[]);
    for field in ["x", "y", "width", "height"] {
        path[field] = json!(null);
    }
    let file = read(
        "null_geometry",
        &[page("p1", "Page", 0, &["p"], vec![path])],
    );
    let path = &only_page(&file).shapes[0];
    assert_eq!(path.kind, ShapeKind::Path);
    let bounds = path.bounds.expect("a path has a box");
    assert_close(bounds.x, 10.5, "x");
    assert_close(bounds.y, 20.25, "y");
    assert_close(bounds.width, 3.0, "width");
    assert_close(bounds.height, 4.0, "height");
}

#[test]
fn hidden_is_read_and_defaults_to_visible() {
    let mut hidden = shape_json("h", "rect", "Hidden", ROOT_ID, [0.0, 0.0, 1.0, 1.0], &[]);
    hidden["hidden"] = json!(true);
    let mut shown = shape_json("s", "rect", "Shown", ROOT_ID, [0.0, 0.0, 1.0, 1.0], &[]);
    shown["hidden"] = json!(false);
    let unmarked = shape_json("u", "rect", "Unmarked", ROOT_ID, [0.0, 0.0, 1.0, 1.0], &[]);
    let file = read(
        "hidden",
        &[page(
            "p1",
            "Page",
            0,
            &["h", "s", "u"],
            vec![hidden, shown, unmarked],
        )],
    );
    let page = only_page(&file);
    assert!(find(page, "h").unwrap().hidden);
    assert!(!find(page, "s").unwrap().hidden);
    assert!(!find(page, "u").unwrap().hidden);
}

#[test]
fn pages_come_in_index_order_whatever_the_order_of_the_archive() {
    // Neither the archive order (Third, First, Second) nor the id order
    // (p-a, p-b, p-c = Third, Second, First) is the index order.
    let pages = [
        page("p-a", "Third", 2, &[], vec![]),
        page("p-c", "First", 0, &[], vec![]),
        page("p-b", "Second", 1, &[], vec![]),
    ];
    let file = read("page_order", &pages);
    let names: Vec<&str> = file.pages.iter().map(|page| page.name.as_str()).collect();
    assert_eq!(names, ["First", "Second", "Third"]);
    assert_eq!(file.pages[0].id, "p-c");
    assert!(file.pages.iter().all(|page| page.shapes.is_empty()));
    assert_eq!(file.id, FILE_ID);
    assert_eq!(file.name, "Synthetic");
}

#[test]
fn shapes_follow_the_lists_of_the_design_not_the_order_of_the_archive() {
    // The archive holds the entries in the reverse of the design's order.
    let shapes = vec![
        shape_json("c2", "rect", "C2", "board-1", [0.0, 0.0, 1.0, 1.0], &[]),
        shape_json("c1", "rect", "C1", "board-1", [0.0, 0.0, 1.0, 1.0], &[]),
        shape_json(
            "board-1",
            "frame",
            "Board 1",
            ROOT_ID,
            [0.0, 0.0, 10.0, 10.0],
            &["c1", "c2"],
        ),
        shape_json(
            "board-2",
            "frame",
            "Board 2",
            ROOT_ID,
            [0.0, 20.0, 10.0, 10.0],
            &[],
        ),
    ];
    let file = read(
        "shape_order",
        &[page("p1", "Page", 0, &["board-2", "board-1"], shapes)],
    );
    let page = only_page(&file);
    let top: Vec<&str> = page.shapes.iter().map(|shape| shape.id.as_str()).collect();
    assert_eq!(top, ["board-2", "board-1"]);
    let inner: Vec<&str> = page.shapes[1]
        .children
        .iter()
        .map(|shape| shape.id.as_str())
        .collect();
    assert_eq!(inner, ["c1", "c2"]);
    assert_eq!(page.shapes[1].children[0].parent_id, "board-1");
    assert_eq!(page.shapes[1].parent_id, ROOT_ID);
}
