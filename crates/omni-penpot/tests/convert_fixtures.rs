//! US-102: `omni_penpot::convert_file` on the real exports fixtures/penpot/
//! tirebud-idag.penpot and tirebud-ordrar.penpot. The numbers below were counted
//! from the exports' own JSON, outside the crate.
//!
//! - Every top-level frame is a board and gets a design; the top-level shapes
//!   that are not frames (texts, in both files) go into a board called
//!   "Unplaced" on their page (US-054).
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
use common::{fixture, flatten};

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
                other => panic!("a converted design has a node of type {other}"),
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

/// The converted boards that are boards of the file, without the Unplaced
/// boards that hold the shapes outside them.
fn file_boards(conversion: &Conversion) -> Vec<&omni_penpot::ConvertedBoard> {
    conversion
        .boards
        .iter()
        .filter(|board| board.name != "Unplaced")
        .collect()
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
    let converted_boards = file_boards(conversion);
    assert_eq!(converted_boards.len(), boards.len());
    for board in &conversion.boards {
        assert_validates(&board.design, &board.name);
    }
    for (converted, (page, shape)) in converted_boards.into_iter().zip(&boards) {
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
fn ordrar_has_a_valid_design_for_each_of_its_795_boards() {
    let (Some(conversion), Some(file)) = (ordrar(), ordrar_file()) else {
        return;
    };
    assert_eq!(file.pages.len(), 5);
    // And an Unplaced board on each of the two pages with loose texts (US-054).
    assert_eq!(conversion.boards.len(), 797);
    assert_eq!(file_boards(conversion).len(), 795);
    assert_boards_follow_the_file(conversion, file);
}

#[test]
fn ordrar_boards_hold_the_node_types_and_layouts_of_their_shapes() {
    let Some(conversion) = ordrar() else { return };
    let counts = count(conversion);
    // The 795 boards of the file and two Unplaced boards (US-054).
    assert_eq!(counts.boards, 797);
    // Frames, groups and rects, less the boards.
    assert_eq!(counts.frames, 63_209);
    assert_eq!(counts.vectors, 19_624);
    // With the 232 texts outside the boards, in the Unplaced boards.
    assert_eq!(counts.texts, 27_380);
    assert_eq!(counts.images, 0);
    assert_eq!(counts.flex, 37_933);
    assert_eq!(counts.grid, 40);
}
