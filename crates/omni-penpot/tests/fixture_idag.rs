//! US-101: `omni_penpot::read_file` on the real export fixtures/penpot/
//! tirebud-idag.penpot, a small file with one page of 23 top-level elements
//! and 2,425 shapes (2,426 shape files, the page's root frame among them).
//!
//! The values below were read off the export itself. `read_file(path)` returns
//! a `PenpotFile { id, name, pages }` with the pages ordered by their index; a
//! `Page { id, name, shapes }` holds the shapes directly on it (the children of
//! the root frame, bottom of the stacking order first) and `boards()` yields the
//! ones that are frames. A `Shape { id, name, kind, parent_id, bounds, hidden,
//! children }` is a tree: `children` follow the order of the parent's list in the
//! export, `bounds` is the box the export stores in `selrect`, in page
//! coordinates, and `parent_id` is the id of the parent, the root frame's
//! `00000000-0000-0000-0000-000000000000` for the top-level shapes.
//!
//! The tests skip, with a message, when fixtures/penpot/ is empty.

mod common;

use std::collections::HashSet;
use std::sync::OnceLock;

use omni_penpot::{PenpotFile, Shape, ShapeKind, read_file};

use common::{ROOT_ID, assert_close, find, fixture, page_shapes};

const FILE: &str = "tirebud-idag.penpot";

fn idag() -> Option<&'static PenpotFile> {
    static FILE_CELL: OnceLock<Option<PenpotFile>> = OnceLock::new();
    FILE_CELL
        .get_or_init(|| {
            fixture(FILE).map(|path| {
                read_file(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
            })
        })
        .as_ref()
}

fn count_kind(shapes: &[&Shape], kind: ShapeKind) -> usize {
    shapes.iter().filter(|shape| shape.kind == kind).count()
}

#[test]
fn the_file_and_its_one_page_are_named() {
    let Some(file) = idag() else { return };
    assert_eq!(file.id, "a3faf778-14f6-810a-8008-bbe77780d545");
    assert_eq!(file.name, "TireBud - Idag");
    assert_eq!(file.pages.len(), 1);
    assert_eq!(file.pages[0].id, "a3faf778-14f6-810a-8008-bbe77780d546");
    assert_eq!(file.pages[0].name, "Idag");
}

#[test]
fn the_page_lists_its_23_top_level_elements_in_stacking_order() {
    let Some(file) = idag() else { return };
    let names: Vec<&str> = file.pages[0]
        .shapes
        .iter()
        .map(|shape| shape.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "Idag / Dag / Desktop",
            "Idag / Tom dag / Desktop",
            "Idag / Bokning \u{f6}ppen / Desktop",
            "Idag / Dag / Surfplatta",
            "Idag / Dag / Telefon",
            "Idag / Tom dag / Telefon",
            "Idag / Om sidan",
            "Kolumnrubrik",
            "Kolumnrubrik",
            "Kolumnrubrik",
            "Kolumnrubrik",
            "Label Dag",
            "Idag / Dag / Nu",
            "Status Idag / Dag / Desktop",
            "Status Idag / Dag / Surfplatta",
            "Status Idag / Dag / Telefon",
            "Label Bokning \u{f6}ppen",
            "Status Idag / Bokning \u{f6}ppen / Desktop",
            "Label Tom dag",
            "Status Idag / Tom dag / Desktop",
            "Status Idag / Tom dag / Telefon",
            "Label Arbetstavla",
            "Idag / Arbetstavla / Nu",
        ]
    );
    for shape in &file.pages[0].shapes {
        assert_eq!(shape.parent_id, ROOT_ID, "{} sits on the page", shape.name);
    }
}

#[test]
fn boards_are_the_top_level_frames() {
    let Some(file) = idag() else { return };
    let page = &file.pages[0];
    let boards: Vec<&Shape> = page.boards().collect();
    assert_eq!(boards.len(), 15);
    assert!(boards.iter().all(|board| board.kind == ShapeKind::Frame));
    assert!(
        boards
            .iter()
            .all(|board| page.shapes.iter().any(|shape| shape.id == board.id))
    );
    let texts = page
        .shapes
        .iter()
        .filter(|shape| shape.kind == ShapeKind::Text)
        .count();
    assert_eq!(texts, 8, "the labels are top-level text shapes, not boards");
    assert_eq!(boards[0].name, "Idag / Dag / Desktop");
}

#[test]
fn a_board_has_its_box_its_kind_and_its_children_in_order() {
    let Some(file) = idag() else { return };
    let board = &file.pages[0].shapes[0];
    assert_eq!(board.id, "41f50750-8a4c-80d6-8008-bbf551b71f1d");
    assert_eq!(board.kind, ShapeKind::Frame);
    let bounds = board.bounds.expect("a board has a box");
    assert_close(bounds.x, 2345.0, "x");
    assert_close(bounds.y, 2422.0, "y");
    assert_close(bounds.width, 1440.0, "width");
    assert_close(bounds.height, 900.0, "height");
    let children: Vec<&str> = board
        .children
        .iter()
        .map(|child| child.id.as_str())
        .collect();
    assert_eq!(
        children,
        [
            "41f50750-8a4c-80d6-8008-bbf551b71f1e",
            "41f50750-8a4c-80d6-8008-bbf551b71f1f",
            "41f50750-8a4c-80d6-8008-bbf551b71f20",
        ]
    );
    for child in &board.children {
        assert_eq!(child.parent_id, board.id);
    }
}

#[test]
fn the_whole_tree_is_read_with_every_shape_in_it_once() {
    let Some(file) = idag() else { return };
    let page = &file.pages[0];
    let shapes = page_shapes(page);
    assert_eq!(shapes.len(), 2425);
    assert_eq!(count_kind(&shapes, ShapeKind::Frame), 981);
    assert_eq!(count_kind(&shapes, ShapeKind::Text), 798);
    assert_eq!(count_kind(&shapes, ShapeKind::Path), 294);
    assert_eq!(count_kind(&shapes, ShapeKind::Rect), 181);
    assert_eq!(count_kind(&shapes, ShapeKind::Group), 87);
    assert_eq!(count_kind(&shapes, ShapeKind::Circle), 84);
    let unknown: Vec<&Shape> = shapes
        .iter()
        .copied()
        .filter(|shape| matches!(shape.kind, ShapeKind::Unknown(_)))
        .collect();
    assert!(
        unknown.is_empty(),
        "every shape of this export is of a known type, found {unknown:?}"
    );
    let ids: HashSet<&str> = shapes.iter().map(|shape| shape.id.as_str()).collect();
    assert_eq!(ids.len(), shapes.len(), "a shape id appears twice");
    let board = &page.shapes[0];
    assert_eq!(
        common::flatten(board).len(),
        510,
        "the board and the 509 shapes inside it"
    );
}

#[test]
fn every_shape_names_its_parent() {
    let Some(file) = idag() else { return };
    for shape in page_shapes(&file.pages[0]) {
        for child in &shape.children {
            assert_eq!(
                child.parent_id, shape.id,
                "{} inside {}",
                child.name, shape.name
            );
        }
    }
}

#[test]
fn a_path_without_x_and_y_still_has_its_stored_box() {
    let Some(file) = idag() else { return };
    // The export stores null for the x, y, width and height of paths and keeps
    // their box in selrect only.
    let path = find(&file.pages[0], "41f50750-8a4c-80d6-8008-bbf551b771ba")
        .expect("the path is in the file");
    assert_eq!(path.kind, ShapeKind::Path);
    assert_eq!(path.name, "svg-path");
    assert_eq!(path.parent_id, "41f50750-8a4c-80d6-8008-bbf551b771b8");
    let bounds = path.bounds.expect("a path has a box");
    assert_close(bounds.x, 5217.666666666667, "x");
    assert_close(bounds.y, 3525.3332780202236, "y");
    assert_close(bounds.width, 1.3333333333330302, "width");
    assert_close(bounds.height, 1.3333333333321207, "height");
}

#[test]
fn the_other_shape_types_are_recognised() {
    let Some(file) = idag() else { return };
    let page = &file.pages[0];
    let expectations = [
        (
            "41f50750-8a4c-80d6-8008-bbf551b76f2f",
            ShapeKind::Text,
            "Text",
        ),
        (
            "41f50750-8a4c-80d6-8008-bbf551b7720a",
            ShapeKind::Rect,
            "Status bar",
        ),
        (
            "41f50750-8a4c-80d6-8008-bbf551b71fc6",
            ShapeKind::Circle,
            "Dot",
        ),
        (
            "41f50750-8a4c-80d6-8008-bbf551b76f95",
            ShapeKind::Group,
            "Glyph",
        ),
    ];
    for (id, kind, name) in expectations {
        let shape = find(page, id).unwrap_or_else(|| panic!("{name} is in the file"));
        assert_eq!(shape.kind, kind, "{name}");
        assert_eq!(shape.name, name);
    }
    let rect = find(page, "41f50750-8a4c-80d6-8008-bbf551b7720a").unwrap();
    let bounds = rect.bounds.expect("a rect has a box");
    assert_close(bounds.x, 4895.0, "x");
    assert_close(bounds.y, 2939.999807357788, "y");
    assert_close(bounds.width, 3.0, "width");
    assert_close(bounds.height, 40.0, "height");
    let group = find(page, "41f50750-8a4c-80d6-8008-bbf551b76f95").unwrap();
    assert_eq!(group.children.len(), 4);
}

#[test]
fn hidden_shapes_are_marked_hidden() {
    let Some(file) = idag() else { return };
    let page = &file.pages[0];
    let shapes = page_shapes(page);
    assert_eq!(shapes.iter().filter(|shape| shape.hidden).count(), 101);
    let hidden =
        find(page, "41f50750-8a4c-80d6-8008-bbf551b71f91").expect("the text is in the file");
    assert!(hidden.hidden);
    assert!(
        !find(page, "41f50750-8a4c-80d6-8008-bbf551b76f2f")
            .unwrap()
            .hidden
    );
    assert!(!page.shapes[0].hidden);
}
