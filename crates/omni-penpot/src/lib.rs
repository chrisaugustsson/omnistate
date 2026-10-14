//! Reads Penpot's `.penpot` export files (docs/formats/penpot-export.md) and
//! converts their boards into OmniState designs.

mod convert;

pub use convert::{Attachment, Conversion, ConvertedBoard, Report, ReportEntry, convert_file};

use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use serde::Deserialize;
use zip::ZipArchive;
use zip::result::ZipError;

/// The id of the root frame that every page has, the parent of the shapes that
/// sit directly on the page.
const ROOT_ID: &str = "00000000-0000-0000-0000-000000000000";

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
        self.shapes
            .iter()
            .filter(|shape| shape.kind == ShapeKind::Frame)
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

/// The part of manifest.json the reader uses.
#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct ManifestFile {
    id: String,
    #[serde(default)]
    name: String,
}

/// The part of a page's own JSON the reader uses.
#[derive(Deserialize)]
struct RawPage {
    id: Option<String>,
    #[serde(default)]
    name: String,
    index: Option<u32>,
}

/// The part of a shape's JSON the reader uses. Every other field is skipped.
#[derive(Deserialize)]
struct RawShape {
    id: Option<String>,
    name: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "parentId")]
    parent_id: Option<String>,
    selrect: Option<RawBox>,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    hidden: Option<bool>,
    #[serde(default)]
    shapes: Vec<String>,
}

#[derive(Deserialize)]
struct RawBox {
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
}

impl RawBox {
    fn rect(&self) -> Option<Rect> {
        Some(Rect {
            x: self.x?,
            y: self.y?,
            width: self.width?,
            height: self.height?,
        })
    }
}

/// Reads the `.penpot` export at `path`.
pub fn read_file(path: &Path) -> Result<PenpotFile, ReadError> {
    let shown = path.display();
    let file =
        File::open(path).map_err(|error| ReadError(format!("cannot open {shown}: {error}")))?;
    let mut archive = ZipArchive::new(BufReader::new(file))
        .map_err(|error| ReadError(format!("{shown} is not a zip archive: {error}")))?;
    let mut reader = Reader {
        archive: &mut archive,
        shown: shown.to_string(),
        buffer: Vec::new(),
    };
    let manifest: Manifest = reader.json("manifest.json")?;
    let entry = manifest
        .files
        .into_iter()
        .next()
        .ok_or_else(|| ReadError(format!("manifest.json of {shown} lists no file")))?;
    let mut pages = Vec::new();
    for page_id in reader.page_ids(&entry.id) {
        let dir = format!("files/{}/pages/{page_id}", entry.id);
        pages.push((reader.page(&dir, &page_id)?, dir));
    }
    pages.sort_by(|(a, _), (b, _)| (a.0, &a.1.id).cmp(&(b.0, &b.1.id)));
    Ok(PenpotFile {
        id: entry.id,
        name: entry.name,
        pages: pages.into_iter().map(|(page, _)| page.1).collect(),
    })
}

struct Reader<'a, R: Read + Seek> {
    archive: &'a mut ZipArchive<R>,
    /// The path of the export, for messages.
    shown: String,
    /// The bytes of the entry read last, kept to avoid allocating for every one.
    buffer: Vec<u8>,
}

impl<R: Read + Seek> Reader<'_, R> {
    /// The bytes of the entry `name` in `self.buffer`, or None if the archive
    /// has no such entry.
    fn load(&mut self, name: &str) -> Result<Option<()>, ReadError> {
        let mut entry = match self.archive.by_name(name) {
            Ok(entry) => entry,
            Err(ZipError::FileNotFound) => return Ok(None),
            Err(error) => {
                return Err(ReadError(format!(
                    "cannot read {name} in {}: {error}",
                    self.shown
                )));
            }
        };
        self.buffer.clear();
        entry
            .read_to_end(&mut self.buffer)
            .map_err(|error| ReadError(format!("cannot read {name} in {}: {error}", self.shown)))?;
        Ok(Some(()))
    }

    /// The JSON entry `name`, which must be there and well formed.
    fn json<T: for<'de> Deserialize<'de>>(&mut self, name: &str) -> Result<T, ReadError> {
        if self.load(name)?.is_none() {
            return Err(ReadError(format!("{} has no {name}", self.shown)));
        }
        serde_json::from_slice(&self.buffer).map_err(|error| {
            ReadError(format!(
                "{name} in {} is not what it should be: {error}",
                self.shown
            ))
        })
    }

    /// The ids of the pages of the file `file_id`, from the entries named
    /// `files/<file id>/pages/<page id>.json`.
    fn page_ids(&self, file_id: &str) -> Vec<String> {
        let prefix = format!("files/{file_id}/pages/");
        self.archive
            .file_names()
            .filter_map(|name| {
                let name = name.ok()?;
                let id = name.strip_prefix(&prefix)?.strip_suffix(".json")?;
                (!id.contains('/')).then(|| id.to_string())
            })
            .collect()
    }

    /// The page in `dir` (`files/<file id>/pages/<page id>`) with its index.
    fn page(&mut self, dir: &str, page_id: &str) -> Result<(u32, Page), ReadError> {
        let raw: RawPage = self.json(&format!("{dir}.json"))?;
        let root: RawShape = self.json(&format!("{dir}/{ROOT_ID}.json"))?;
        let mut seen = HashSet::from([ROOT_ID.to_string()]);
        let mut shapes = Vec::with_capacity(root.shapes.len());
        for id in &root.shapes {
            shapes.push(self.shape(dir, id, ROOT_ID, &mut seen)?);
        }
        let page = Page {
            id: raw.id.unwrap_or_else(|| page_id.to_string()),
            name: raw.name,
            shapes,
        };
        Ok((raw.index.unwrap_or(u32::MAX), page))
    }

    /// The shape `id` of the page in `dir`, with the shapes inside it. A shape
    /// that is listed but has no entry, cannot be understood, or was already
    /// read (a list that loops) becomes an `Unknown` shape without children.
    fn shape(
        &mut self,
        dir: &str,
        id: &str,
        parent: &str,
        seen: &mut HashSet<String>,
    ) -> Result<Shape, ReadError> {
        let unknown = |kind: &str| Shape {
            id: id.to_string(),
            name: String::new(),
            kind: ShapeKind::Unknown(kind.to_string()),
            parent_id: parent.to_string(),
            bounds: None,
            hidden: false,
            children: Vec::new(),
        };
        if !seen.insert(id.to_string()) {
            return Ok(unknown("repeated"));
        }
        if self.load(&format!("{dir}/{id}.json"))?.is_none() {
            return Ok(unknown("missing"));
        }
        let Ok(raw) = serde_json::from_slice::<RawShape>(&self.buffer) else {
            return Ok(unknown("unreadable"));
        };
        let shape_id = raw.id.unwrap_or_else(|| id.to_string());
        let mut children = Vec::with_capacity(raw.shapes.len());
        for child in &raw.shapes {
            children.push(self.shape(dir, child, &shape_id, seen)?);
        }
        let bounds = raw.selrect.as_ref().and_then(RawBox::rect).or_else(|| {
            RawBox {
                x: raw.x,
                y: raw.y,
                width: raw.width,
                height: raw.height,
            }
            .rect()
        });
        Ok(Shape {
            kind: match raw.kind.as_deref().unwrap_or_default() {
                "frame" => ShapeKind::Frame,
                "group" => ShapeKind::Group,
                "rect" => ShapeKind::Rect,
                "circle" => ShapeKind::Circle,
                "text" => ShapeKind::Text,
                "path" => ShapeKind::Path,
                other => ShapeKind::Unknown(other.to_string()),
            },
            name: raw.name.unwrap_or_default(),
            parent_id: raw.parent_id.unwrap_or_else(|| parent.to_string()),
            id: shape_id,
            bounds,
            hidden: raw.hidden.unwrap_or(false),
            children,
        })
    }
}
