//! Reads Penpot's `.penpot` export files (docs/formats/penpot-export.md) and
//! converts their boards into OmniState designs and compares those with Penpot's
//! own rendering, and writes the page that sets the two side by side.

pub mod compare;
mod convert;
pub mod report;

pub use convert::{
    Attachment, Conversion, ConvertedBoard, ConvertedShape, Report, ReportEntry, convert_file,
};

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
    /// The prototype flows, read as they come: an object or a list of
    /// `{id, name, startingFrame}`.
    #[serde(default)]
    flows: serde_json::Value,
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

/// Opens the `.penpot` export at `path` as a zip archive.
fn open_archive(path: &Path) -> Result<ZipArchive<BufReader<File>>, ReadError> {
    let shown = path.display();
    let file =
        File::open(path).map_err(|error| ReadError(format!("cannot open {shown}: {error}")))?;
    ZipArchive::new(BufReader::new(file))
        .map_err(|error| ReadError(format!("{shown} is not a zip archive: {error}")))
}

/// Where a page is in the archive, and where it goes in the file.
struct PageHead {
    /// `files/<file id>/pages/<page id>`, the folder of the page's shapes.
    dir: String,
    id: String,
    name: String,
    index: u32,
    /// The page's prototype flows, as its file gives them.
    flows: serde_json::Value,
}

/// Reads the `.penpot` export at `path`.
pub fn read_file(path: &Path) -> Result<PenpotFile, ReadError> {
    let mut archive = open_archive(path)?;
    let mut reader = Reader {
        archive: &mut archive,
        shown: path.display().to_string(),
        buffer: Vec::new(),
    };
    let entry = reader.first_file()?;
    let mut pages = Vec::new();
    for head in reader.page_heads(&entry.id)? {
        pages.push(reader.page(&head)?);
    }
    Ok(PenpotFile {
        id: entry.id,
        name: entry.name,
        pages,
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

    /// The file that the manifest lists first, the one the crate reads.
    fn first_file(&mut self) -> Result<ManifestFile, ReadError> {
        let manifest: Manifest = self.json("manifest.json")?;
        manifest
            .files
            .into_iter()
            .next()
            .ok_or_else(|| ReadError(format!("manifest.json of {} lists no file", self.shown)))
    }

    /// The pages of the file `file_id`, in the order of their index.
    fn page_heads(&mut self, file_id: &str) -> Result<Vec<PageHead>, ReadError> {
        let mut heads = Vec::new();
        for page_id in self.page_ids(file_id) {
            let dir = format!("files/{file_id}/pages/{page_id}");
            let raw: RawPage = self.json(&format!("{dir}.json"))?;
            heads.push(PageHead {
                id: raw.id.unwrap_or(page_id),
                name: raw.name,
                index: raw.index.unwrap_or(u32::MAX),
                flows: raw.flows,
                dir,
            });
        }
        heads.sort_by(|a, b| (a.index, &a.id).cmp(&(b.index, &b.id)));
        Ok(heads)
    }

    /// The ids of the shapes directly on a page, bottom of the stacking order
    /// first: the list of its root frame.
    fn top_level_ids(&mut self, dir: &str) -> Result<Vec<String>, ReadError> {
        let root: RawShape = self.json(&format!("{dir}/{ROOT_ID}.json"))?;
        Ok(root.shapes)
    }

    fn page(&mut self, head: &PageHead) -> Result<Page, ReadError> {
        let mut seen = HashSet::from([ROOT_ID.to_string()]);
        let ids = self.top_level_ids(&head.dir)?;
        let mut shapes = Vec::with_capacity(ids.len());
        for id in &ids {
            shapes.push(self.shape(&head.dir, id, ROOT_ID, &mut seen)?);
        }
        Ok(Page {
            id: head.id.clone(),
            name: head.name.clone(),
            shapes,
        })
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
