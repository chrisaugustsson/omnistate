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

use common::{fixture, page_shapes};

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
