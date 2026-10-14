//! Converts the boards of a Penpot export into OmniState designs.

use std::path::Path;

use omni_core::design::Node;

use crate::ReadError;

/// What converting one `.penpot` file gives.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversion {
    /// One per board, in page order and then stacking order.
    pub boards: Vec<ConvertedBoard>,
    /// The images that the designs refer to as `attachment:<id>`.
    pub attachments: Vec<Attachment>,
    /// Everything that could not be converted.
    pub report: Report,
}

/// A board as an OmniState design: a page that holds the board alone.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvertedBoard {
    /// The name of the Penpot page the board is on.
    pub page: String,
    /// The Penpot id of the board.
    pub id: String,
    /// The name of the board.
    pub name: String,
    pub design: Node,
}

/// An image file of the export, written as `<id>.<extension>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub extension: String,
    pub bytes: Vec<u8>,
}

/// The shapes and properties that were not converted.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub entries: Vec<ReportEntry>,
}

/// One shape or property that was not converted.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportEntry {
    /// The name of the board the shape is in, empty for a shape outside boards.
    pub board: String,
    pub board_id: String,
    pub shape_id: String,
    pub name: String,
    /// The property that was left out, or None if the whole shape was.
    pub property: Option<String>,
    pub reason: String,
}

/// Reads the `.penpot` export at `path` and converts its boards.
pub fn convert_file(path: &Path) -> Result<Conversion, ReadError> {
    let _ = path;
    Err(ReadError("converting is not implemented".to_string()))
}
