//! US-101: `omni_penpot::read_file` on fixtures/penpot/tirebud-ordrar.penpot,
//! the stress test of the reader: 5 pages and 111,008 shapes in a 114 MB
//! archive (247 MB of JSON). The reader must take all of it, with every shape
//! on its page and the pages in the order of their index (not the order of the
//! archive or of their ids).
//!
//! The tests skip, with a message, when fixtures/penpot/ is empty.

mod common;

use std::sync::OnceLock;

use omni_penpot::{PenpotFile, read_file};

use common::{fixture, page_shapes};

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
