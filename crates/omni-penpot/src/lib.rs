//! Reads Penpot's `.penpot` export files (docs/formats/penpot-export.md).

use std::fmt;
use std::path::Path;

/// A Penpot file: the pages of one `.penpot` export, ordered by their index.
#[derive(Debug, Clone, PartialEq)]
pub struct PenpotFile {
    pub id: String,
    pub name: String,
    pub pages: Vec<Page>,
}

/// One page of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub id: String,
    pub name: String,
    /// The shapes directly on the page, bottom of the stacking order first.
    pub shapes: Vec<Shape>,
}

impl Page {
    /// The boards of the page: its top-level shapes that are frames.
    pub fn boards(&self) -> impl Iterator<Item = &Shape> {
        std::iter::empty()
    }
}

/// A shape and everything inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub id: String,
    pub name: String,
    pub kind: ShapeKind,
    pub parent_id: String,
    /// The box Penpot stores for the shape, in page coordinates.
    pub bounds: Option<Rect>,
    pub hidden: bool,
    pub children: Vec<Shape>,
}

/// What a shape is. A type the reader does not know is kept as `Unknown`
/// with the type name the file gives it.
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeKind {
    Frame,
    Group,
    Rect,
    Circle,
    Text,
    Path,
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Why a `.penpot` file could not be read.
#[derive(Debug)]
pub struct ReadError(String);

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ReadError {}

/// Reads the `.penpot` export at `path`.
pub fn read_file(_path: &Path) -> Result<PenpotFile, ReadError> {
    Err(ReadError("not implemented".to_string()))
}
