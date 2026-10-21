//! Imports a Penpot export into OmniState (US-051): its pages and boards become
//! one design doc, its components, design tokens, colors and typographies the
//! project's design system doc, and its images attachments. It reuses the
//! reader and the converter, and lists what it cannot import in its report.

use std::path::Path;

use omni_core::design::Node;
use serde_json::Value;

use crate::{Attachment, ReadError};

/// What importing one `.penpot` file gives.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// The pages of the design doc, in Penpot's page order, each a `page` node
    /// holding its boards.
    pub pages: Vec<Node>,
    /// The design system doc: a DTCG token tree (docs/formats/tokens.md) with
    /// the components under `$components`, as a design system version holds it.
    pub design_system: Value,
    /// The images that the pages and components refer to as `attachment:<id>`.
    pub attachments: Vec<Attachment>,
    /// Everything that could not be imported.
    pub report: Vec<ImportEntry>,
}

/// One thing that could not be imported.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportEntry {
    /// The name of the Penpot page, empty for what is on no page (a token).
    pub page: String,
    /// The name of the board the shape is in, empty for a shape outside boards.
    pub board: String,
    pub board_id: String,
    /// The Penpot id of the shape, empty for what is no shape.
    pub shape_id: String,
    /// The name of the shape, or of the token, color or typography.
    pub name: String,
    /// The property that was left out, or None if the whole thing was.
    pub property: Option<String>,
    pub reason: String,
}

/// Reads the `.penpot` export at `path` and imports it.
pub fn import_file(path: &Path) -> Result<Import, ReadError> {
    let _ = path;
    todo!("US-051: import a Penpot export")
}
