//! US-102: `omni_penpot::convert_file` on the real exports fixtures/penpot/
//! tirebud-idag.penpot and tirebud-ordrar.penpot. The numbers below were counted
//! from the exports' own JSON, outside the crate.
//!
//! - Every top-level frame is a board and gets a design; the top-level shapes
//!   that are not frames (texts, in both files) cannot be boards, so each is
//!   listed in the report.
//! - Every shape inside a board becomes exactly one node: boards and nested
//!   frames, groups and rects as boards and frames, circles and paths as
//!   vectors, texts as texts. Neither file holds a shape type that is not
//!   converted.
//! - Every frame's flex or grid layout comes through.
//! - Every design passes OmniState's design validation, and converting twice
//!   gives the same result.
//!
//! The tests skip, with a message, when fixtures/penpot/ is empty.

mod common;

use std::sync::OnceLock;

use omni_core::design::{Node, NodeKind};
use omni_penpot::{Conversion, PenpotFile, Shape, read_file};

use common::convert::{assert_validates, board_node, json_of, nodes};
use common::{ROOT_ID, find, fixture, flatten, page_shapes};

fn read(name: &str) -> Option<PenpotFile> {
    fixture(name).map(|path| {
        read_file(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
    })
}

fn convert(name: &str) -> Option<Conversion> {
    fixture(name).map(|path| {
        omni_penpot::convert_file(&path)
            .unwrap_or_else(|error| panic!("cannot convert {path:?}: {error}"))
    })
}

fn idag() -> Option<&'static Conversion> {
    static CELL: OnceLock<Option<Conversion>> = OnceLock::new();
    CELL.get_or_init(|| convert("tirebud-idag.penpot")).as_ref()
}

fn idag_file() -> Option<&'static PenpotFile> {
    static CELL: OnceLock<Option<PenpotFile>> = OnceLock::new();
    CELL.get_or_init(|| read("tirebud-idag.penpot")).as_ref()
}

fn ordrar() -> Option<&'static Conversion> {
    static CELL: OnceLock<Option<Conversion>> = OnceLock::new();
    CELL.get_or_init(|| convert("tirebud-ordrar.penpot"))
        .as_ref()
}

fn ordrar_file() -> Option<&'static PenpotFile> {
    static CELL: OnceLock<Option<PenpotFile>> = OnceLock::new();
    CELL.get_or_init(|| read("tirebud-ordrar.penpot")).as_ref()
}

/// How many nodes of each kind the designs of `conversion` hold: boards,
/// frames, vectors and texts, in that order, and how many frame-like nodes
/// have a flex layout and how many a grid layout.
struct Counts {
    boards: usize,
    frames: usize,
    vectors: usize,
    texts: usize,
    images: usize,
    flex: usize,
    grid: usize,
}

fn count(conversion: &Conversion) -> Counts {
    let mut counts = Counts {
        boards: 0,
        frames: 0,
        vectors: 0,
        texts: 0,
        images: 0,
        flex: 0,
        grid: 0,
    };
    for board in &conversion.boards {
        for node in nodes(&board.design) {
            match node.kind() {
                NodeKind::Page => {}
                NodeKind::Board => counts.boards += 1,
                NodeKind::Frame => counts.frames += 1,
                NodeKind::Vector => counts.vectors += 1,
                NodeKind::Text => counts.texts += 1,
                NodeKind::Image => counts.images += 1,
                NodeKind::Instance => panic!("a converted design has an instance"),
            }
            match json_of(node)["layout"]["mode"].as_str() {
                Some("flex") => counts.flex += 1,
                Some("grid") => counts.grid += 1,
                _ => {}
            }
        }
    }
    counts
}

fn node_count(design: &Node) -> usize {
    nodes(design).len() - 1
}

/// Checks the things that hold for the designs of any export: one design per
/// board in the order the reader gives them, each a page that holds its board
/// at the board's own place, each valid, and one node per shape.
fn assert_boards_follow_the_file(conversion: &Conversion, file: &PenpotFile) {
    let boards: Vec<(&str, &Shape)> = file
        .pages
        .iter()
        .flat_map(|page| page.boards().map(move |board| (page.name.as_str(), board)))
        .collect();
    assert_eq!(conversion.boards.len(), boards.len());
    for (converted, (page, shape)) in conversion.boards.iter().zip(&boards) {
        assert_eq!(converted.id, shape.id);
        assert_eq!(converted.name, shape.name);
        assert_eq!(converted.page, *page);
        let root = board_node(&converted.design);
        assert_eq!(root.name(), shape.name);
        let bounds = shape.bounds.expect("a board has a box");
        let json = json_of(root);
        common::assert_close(
            json["x"].as_f64().expect("a board has an x"),
            bounds.x,
            "x of the board",
        );
        common::assert_close(
            json["y"].as_f64().expect("a board has a y"),
            bounds.y,
            "y of the board",
        );
        assert_validates(&converted.design, &converted.name);
        assert_eq!(
            node_count(&converted.design),
            flatten(shape).len(),
            "the nodes of board {:?} are its shapes",
            shape.name
        );
    }
}

#[test]
fn idag_has_a_design_for_each_of_its_15_boards() {
    let (Some(conversion), Some(file)) = (idag(), idag_file()) else {
        return;
    };
    assert_eq!(conversion.boards.len(), 15);
    assert_eq!(conversion.boards[0].name, "Idag / Dag / Desktop");
    assert_eq!(conversion.boards[0].page, "Idag");
    assert_boards_follow_the_file(conversion, file);
}

#[test]
fn idag_boards_hold_the_node_types_of_their_shapes() {
    let Some(conversion) = idag() else { return };
    let counts = count(conversion);
    assert_eq!(counts.boards, 15);
    // 981 frames, 87 groups and 181 rects, less the 15 boards, are frames.
    assert_eq!(counts.frames, 1234);
    // 84 circles and 294 paths.
    assert_eq!(counts.vectors, 378);
    // 798 texts, less the 8 that sit outside the boards.
    assert_eq!(counts.texts, 790);
    assert_eq!(counts.images, 0);
}

#[test]
fn idag_flex_layouts_come_through() {
    let Some(conversion) = idag() else { return };
    let counts = count(conversion);
    assert_eq!(counts.flex, 894, "the frames with a flex layout");
    assert_eq!(counts.grid, 0);
}

#[test]
fn idag_lists_the_texts_outside_boards_in_the_report() {
    let (Some(conversion), Some(file)) = (idag(), idag_file()) else {
        return;
    };
    let loose: Vec<&Shape> = file.pages[0]
        .shapes
        .iter()
        .filter(|shape| shape.kind != omni_penpot::ShapeKind::Frame)
        .collect();
    assert_eq!(loose.len(), 8);
    for shape in loose {
        let entries: Vec<_> = conversion
            .report
            .entries
            .iter()
            .filter(|entry| entry.shape_id == shape.id)
            .collect();
        assert_eq!(entries.len(), 1, "{:?} is listed once", shape.name);
        assert_eq!(entries[0].name, shape.name);
        assert!(entries[0].property.is_none());
        assert!(!entries[0].reason.trim().is_empty());
    }
}

#[test]
fn idag_report_entries_are_complete_and_none_is_a_dropped_shape_of_a_board() {
    let (Some(conversion), Some(file)) = (idag(), idag_file()) else {
        return;
    };
    for entry in &conversion.report.entries {
        assert!(!entry.shape_id.is_empty() && !entry.reason.trim().is_empty());
        assert!(
            find(&file.pages[0], &entry.shape_id).is_some(),
            "{entry:?} names a shape that is not in the file"
        );
        if !entry.board_id.is_empty() {
            assert!(
                entry.property.is_some(),
                "{entry:?} drops a whole shape of a board, which has only convertible types"
            );
            let board = conversion
                .boards
                .iter()
                .find(|board| board.id == entry.board_id)
                .expect("an entry names the board it is in");
            assert_eq!(board.name, entry.board);
        }
    }
}

#[test]
fn idag_converts_the_same_way_every_time() {
    let (Some(first), Some(again)) = (idag(), convert("tirebud-idag.penpot")) else {
        return;
    };
    assert_eq!(first, &again);
}

#[test]
fn ordrar_has_a_valid_design_for_each_of_its_795_boards() {
    let (Some(conversion), Some(file)) = (ordrar(), ordrar_file()) else {
        return;
    };
    assert_eq!(file.pages.len(), 5);
    assert_eq!(conversion.boards.len(), 795);
    assert_boards_follow_the_file(conversion, file);
}

#[test]
fn ordrar_boards_hold_the_node_types_and_layouts_of_their_shapes() {
    let Some(conversion) = ordrar() else { return };
    let counts = count(conversion);
    assert_eq!(counts.boards, 795);
    // Frames, groups and rects, less the boards.
    assert_eq!(counts.frames, 63_209);
    assert_eq!(counts.vectors, 19_624);
    assert_eq!(counts.texts, 27_148);
    assert_eq!(counts.images, 0);
    assert_eq!(counts.flex, 37_933);
    assert_eq!(counts.grid, 40);
}

#[test]
fn ordrar_lists_what_it_leaves_out_and_nothing_else() {
    let (Some(conversion), Some(file)) = (ordrar(), ordrar_file()) else {
        return;
    };
    let ids: std::collections::HashSet<&str> = file
        .pages
        .iter()
        .flat_map(page_shapes)
        .map(|shape| shape.id.as_str())
        .collect();
    assert!(!ids.contains(ROOT_ID));
    let mut loose = 0;
    for entry in &conversion.report.entries {
        assert!(!entry.reason.trim().is_empty(), "{entry:?} has no reason");
        assert!(
            ids.contains(entry.shape_id.as_str()),
            "{entry:?} names a shape that is not in the file"
        );
        if entry.board_id.is_empty() {
            loose += 1;
            assert!(entry.property.is_none());
        } else {
            assert!(
                entry.property.is_some(),
                "{entry:?} drops a whole shape of a board, which has only convertible types"
            );
        }
    }
    // The 232 texts that sit directly on the pages.
    assert_eq!(loose, 232);
}
