//! US-101: `omni_penpot::read_file` on fixtures/penpot/tirebud-ordrar.penpot,
//! the stress test of the reader: 5 pages and 111,008 shapes in a 114 MB
//! archive (247 MB of JSON). The reader must take all of it, with every shape
//! on its page and the pages in the order of their index (not the order of the
//! archive or of their ids).
//!
//! The tests skip, with a message, when fixtures/penpot/ is empty.

mod common;

use std::sync::OnceLock;

use omni_penpot::{PenpotFile, Shape, ShapeKind, read_file};

use common::{assert_close, find, fixture, page_shapes};

const FILE: &str = "tirebud-ordrar.penpot";

fn ordrar() -> Option<&'static PenpotFile> {
    static FILE_CELL: OnceLock<Option<PenpotFile>> = OnceLock::new();
    FILE_CELL
        .get_or_init(|| {
            fixture(FILE).map(|path| {
                read_file(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
            })
        })
        .as_ref()
}

#[test]
fn the_file_is_named_and_its_pages_follow_their_index() {
    let Some(file) = ordrar() else { return };
    assert_eq!(file.id, "6b854330-2008-807c-8008-bc046d0255f4");
    assert_eq!(file.name, "TireBud - Ordrar");
    let pages: Vec<(&str, &str)> = file
        .pages
        .iter()
        .map(|page| (page.id.as_str(), page.name.as_str()))
        .collect();
    assert_eq!(
        pages,
        [
            ("1ad5cc28-5cf0-8030-8008-ba5de4a45275", "Omslag"),
            ("1ad5cc28-5cf0-8030-8008-ba504220099e", "Grunder"),
            ("676f9c00-643a-800e-8008-ba42bfc77467", "Komponenter"),
            ("1ad5cc28-5cf0-8030-8008-ba5de4f1c1f8", "Sidmallar"),
            ("1ad5cc28-5cf0-8030-8008-ba538ecc6452", "Ordrar"),
        ]
    );
}

#[test]
fn every_page_has_all_of_its_shapes() {
    let Some(file) = ordrar() else { return };
    // (top-level shapes, boards, all shapes) of each page; the page's root frame
    // is in none of them.
    let expected = [
        (5, 5, 109),
        (5, 5, 644),
        (14, 14, 3053),
        (4, 3, 387),
        (999, 768, 106815),
    ];
    assert_eq!(file.pages.len(), expected.len());
    for (page, (top_level, boards, shapes)) in file.pages.iter().zip(expected) {
        assert_eq!(
            page.shapes.len(),
            top_level,
            "top-level shapes of {}",
            page.name
        );
        assert_eq!(page.boards().count(), boards, "boards of {}", page.name);
        assert_eq!(page_shapes(page).len(), shapes, "shapes of {}", page.name);
    }
}

#[test]
fn all_111008_shapes_are_read_by_kind() {
    let Some(file) = ordrar() else { return };
    let shapes: Vec<&Shape> = file.pages.iter().flat_map(page_shapes).collect();
    assert_eq!(shapes.len(), 111_008);
    let count = |kind: ShapeKind| shapes.iter().filter(|shape| shape.kind == kind).count();
    assert_eq!(count(ShapeKind::Frame), 45_293);
    assert_eq!(count(ShapeKind::Text), 27_380);
    assert_eq!(count(ShapeKind::Path), 18_329);
    assert_eq!(count(ShapeKind::Rect), 11_811);
    assert_eq!(count(ShapeKind::Group), 6_900);
    assert_eq!(count(ShapeKind::Circle), 1_295);
}

#[test]
fn a_board_of_the_biggest_page_is_found_with_its_box() {
    let Some(file) = ordrar() else { return };
    let page = file
        .pages
        .iter()
        .find(|page| page.name == "Ordrar")
        .expect("the page Ordrar exists");
    let board =
        find(page, "02a69ced-fff2-8036-8008-bb46ec040d48").expect("the board is on the page");
    assert_eq!(board.kind, ShapeKind::Frame);
    assert_eq!(
        board.name,
        "Ordrar / Order \u{2013} Mekaniker, Redo, k\u{e4}rna / Desktop"
    );
    assert_eq!(board.children.len(), 1);
    let bounds = board.bounds.expect("a board has a box");
    assert_close(bounds.x, 1600.0, "x");
    assert_close(bounds.y, 176824.0, "y");
    assert_close(bounds.width, 1280.0, "width");
    assert_close(bounds.height, 163.0, "height");
}
