//! Converts the boards of a Penpot export into OmniState designs.
//!
//! The converter walks each page from its root frame, reads one shape file at a
//! time and builds the design of a board as JSON, which OmniState's own design
//! validation then reads. What cannot be converted goes into the report, and
//! the shape around it is converted as far as possible.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};
use std::path::Path;

use omni_core::design::{Node, PathData};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};
use zip::ZipArchive;

use crate::{PageHead, ROOT_ID, RawBox, ReadError, Reader, Rect, open_archive};

mod linked;

use linked::Linked;
pub(crate) use linked::run as import_run;

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
    /// The box Penpot stores for the board, in page coordinates. None when the
    /// file has none, and then the board sits at 0, 0 with no size.
    pub bounds: Option<Rect>,
    pub design: Node,
    /// Every shape of the board that became a node, the board included, with
    /// the box Penpot stores for it.
    pub shapes: Vec<ConvertedShape>,
    /// The box Penpot stores for each shape left out whole (a report entry with
    /// no property), by Penpot id. A shape the file gives no box is not here.
    pub left_out: HashMap<String, Rect>,
}

/// A Penpot shape that became a node of a design.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvertedShape {
    /// The Penpot id of the shape.
    pub id: String,
    pub name: String,
    /// The id of the node it became, as the design writes it.
    pub node_id: String,
    /// The box Penpot stores for the shape, in page coordinates.
    pub bounds: Rect,
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

/// The alphabet of node ids, as omni-core defines it.
const ID_ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// The node ID that is the number `number` in the alphabet of node IDs.
pub(crate) fn node_id(number: u64) -> String {
    let mut left = number;
    let mut id = [b'0'; 8];
    for slot in id.iter_mut().rev() {
        *slot = ID_ALPHABET[(left % 32) as usize];
        left /= 32;
    }
    String::from_utf8_lossy(&id).to_string()
}

/// What a shape file may say that the converter uses. Every field reads
/// leniently: a field of the wrong type counts as left out.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Pen {
    id: L<String>,
    name: L<String>,
    #[serde(rename = "type")]
    kind: L<String>,
    hidden: L<bool>,
    shapes: L<Vec<String>>,
    selrect: L<RawBox>,
    x: L<f64>,
    y: L<f64>,
    width: L<f64>,
    height: L<f64>,
    rotation: L<f64>,
    flip_x: L<bool>,
    flip_y: L<bool>,
    fills: L<Vec<Value>>,
    strokes: L<Vec<Value>>,
    shadow: L<Vec<Value>>,
    blur: L<Value>,
    opacity: L<f64>,
    r1: L<f64>,
    r2: L<f64>,
    r3: L<f64>,
    r4: L<f64>,
    show_content: L<bool>,
    masked_group: L<bool>,
    layout: L<String>,
    layout_flex_dir: L<String>,
    layout_wrap_type: L<String>,
    layout_align_items: L<String>,
    layout_align_content: L<String>,
    layout_justify_content: L<String>,
    layout_justify_items: L<String>,
    layout_gap: L<Value>,
    layout_padding: L<Value>,
    layout_grid_columns: L<Vec<Value>>,
    layout_grid_rows: L<Vec<Value>>,
    layout_grid_cells: L<Value>,
    layout_item_absolute: L<bool>,
    layout_item_h_sizing: L<String>,
    layout_item_v_sizing: L<String>,
    layout_item_min_w: L<f64>,
    layout_item_max_w: L<f64>,
    layout_item_min_h: L<f64>,
    layout_item_max_h: L<f64>,
    layout_item_margin: L<Value>,
    interactions: L<Vec<Value>>,
    constraints_h: L<String>,
    constraints_v: L<String>,
    position_data: L<Vec<Value>>,
    grow_type: L<String>,
    content: L<Value>,
    metadata: L<Value>,
    component_id: L<String>,
    component_root: L<bool>,
    main_instance: L<bool>,
    parent_id: L<String>,
    shape_ref: L<String>,
    touched: L<Vec<String>>,
    applied_tokens: L<Value>,
}

/// A value that reads as `None` when it is not a `T`.
struct L<T>(Option<T>);

impl<T> Default for L<T> {
    fn default() -> Self {
        L(None)
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for L<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(L(serde_json::from_value(value).ok()))
    }
}

/// How a frame lays out its children.
#[derive(Clone, Copy, PartialEq)]
enum Flow {
    None,
    Flex,
    Grid,
}

/// One cell of a grid layout: where it is, and which shapes sit in it.
struct Cell {
    shapes: Vec<String>,
    column: i64,
    row: i64,
    column_span: i64,
    row_span: i64,
    /// The cell's own alignment of its items, down and across, when it is not
    /// `auto`.
    align_self: Option<String>,
    justify_self: Option<String>,
}

/// What a shape needs to know about the frame it is in.
struct Parent {
    flow: Flow,
    /// The top left corner of the parent's box, in page coordinates.
    x: f64,
    y: f64,
    /// Whether the parent is the page itself, which the boards sit on.
    page: bool,
    cells: Vec<Cell>,
    /// How a grid aligns its items down and across (`layoutAlignItems`,
    /// `layoutJustifyItems`).
    align_items: Option<String>,
    justify_items: Option<String>,
}

/// The top-level board that holds a shape, which the report names.
struct Board {
    id: String,
    name: String,
}

/// The shape a report entry is about.
struct Who<'a> {
    board: &'a Board,
    id: &'a str,
    name: &'a str,
}

/// The parts of a node that every kind of shape has.
struct Common {
    id: String,
    name: String,
    bounds: Rect,
    /// Whether `bounds` is the box of the file, not a stand-in for a missing one.
    boxed: bool,
    hidden: bool,
}

/// What a shape file turned out to be.
enum Loaded {
    Shape(Box<Pen>),
    /// The shape cannot be read, for this reason.
    Skipped(&'static str),
}

/// What kind of node the style properties go on, which decides how strokes are
/// held to what the renderer draws.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Frame,
    Vector,
    Image,
}

struct Converter<'a, R: Read + Seek> {
    reader: Reader<'a, R>,
    file_id: String,
    /// The image files of the archive: stored file id to entry name.
    objects: HashMap<String, String>,
    attachments: Vec<Attachment>,
    /// What came of each image looked up: its attachment reference, or why not.
    attached: HashMap<String, Result<String, String>>,
    entries: Vec<ReportEntry>,
    next_id: u64,
    /// The shapes converted so far in the board being converted.
    shapes: Vec<ConvertedShape>,
    /// The boxes of the shapes left out whole so far in that board.
    left_out: HashMap<String, Rect>,
    /// The components, token links and nodes of an import (US-051); None for a
    /// plain conversion, which draws component links as the plain shapes they
    /// are.
    linked: Option<Linked>,
    /// The name of the page being converted, and the page each entry of
    /// `entries` is about.
    page_name: String,
    entry_pages: Vec<String>,
    /// The node each shape of the component being converted became, by the
    /// Penpot id of the shape, with the type of the node.
    refs: Option<HashMap<String, (String, String)>>,
}

/// Reads the `.penpot` export at `path` and converts its boards.
pub fn convert_file(path: &Path) -> Result<Conversion, ReadError> {
    let mut archive = open_archive(path)?;
    let objects = object_files(&archive);
    let mut reader = Reader {
        archive: &mut archive,
        shown: path.display().to_string(),
        buffer: Vec::new(),
    };
    let file = reader.first_file()?;
    let heads = reader.page_heads(&file.id)?;
    let mut converter = Converter {
        reader,
        file_id: file.id,
        objects,
        attachments: Vec::new(),
        attached: HashMap::new(),
        entries: Vec::new(),
        next_id: 0,
        shapes: Vec::new(),
        left_out: HashMap::new(),
        linked: None,
        page_name: String::new(),
        entry_pages: Vec::new(),
        refs: None,
    };
    let mut boards = Vec::new();
    for head in &heads {
        converter.page(head, &mut boards)?;
    }
    Ok(Conversion {
        boards,
        attachments: converter.attachments,
        report: Report {
            entries: converter.entries,
        },
    })
}

/// The image files of the archive, `objects/<id>.<extension>` but not the
/// `.json` records beside them. Where two share an id, the first by name wins.
fn object_files<R: Read + Seek>(archive: &ZipArchive<R>) -> HashMap<String, String> {
    let mut objects: HashMap<String, String> = HashMap::new();
    for name in archive.file_names() {
        let Ok(name) = name else { continue };
        let Some(rest) = name.strip_prefix("objects/") else {
            continue;
        };
        if rest.contains('/') || rest.ends_with(".json") {
            continue;
        }
        let Some((id, _)) = rest.rsplit_once('.') else {
            continue;
        };
        match objects.get(id) {
            Some(known) if known.as_str() <= name.as_ref() => {}
            _ => {
                objects.insert(id.to_string(), name.to_string());
            }
        }
    }
    objects
}

const REPEATED: &str =
    "the shape is listed a second time, a loop in the file; it is not converted again";
const MISSING: &str = "the shape is listed but the export has no file for it";
const UNREADABLE: &str = "the shape's file could not be read";
/// The name of the board that holds the shapes outside every board of a page.
const UNPLACED: &str = "Unplaced";

impl<R: Read + Seek> Converter<'_, R> {
    fn new_id(&mut self) -> String {
        self.next_id += 1;
        node_id(self.next_id)
    }

    fn report(&mut self, who: &Who, property: Option<&str>, reason: impl Into<String>) {
        self.entry_pages.push(self.page_name.clone());
        self.entries.push(ReportEntry {
            board: who.board.name.clone(),
            board_id: who.board.id.clone(),
            shape_id: who.id.to_string(),
            name: who.name.to_string(),
            property: property.map(str::to_string),
            reason: reason.into(),
        });
    }

    /// Lists a whole shape that was left out.
    fn skip(&mut self, board: &Board, id: &str, name: &str, reason: &str) {
        let who = Who { board, id, name };
        self.report(&who, None, reason);
    }

    /// Reads the shape `id` of the page in `dir`.
    fn load(
        &mut self,
        dir: &str,
        id: &str,
        seen: &mut HashSet<String>,
    ) -> Result<Loaded, ReadError> {
        if !seen.insert(id.to_string()) {
            return Ok(Loaded::Skipped(REPEATED));
        }
        if self.reader.load(&format!("{dir}/{id}.json"))?.is_none() {
            return Ok(Loaded::Skipped(MISSING));
        }
        Ok(match serde_json::from_slice::<Pen>(&self.reader.buffer) {
            Ok(pen) => Loaded::Shape(Box::new(pen)),
            Err(_) => Loaded::Skipped(UNREADABLE),
        })
    }

    fn page(&mut self, head: &PageHead, boards: &mut Vec<ConvertedBoard>) -> Result<(), ReadError> {
        self.page_name = head.name.clone();
        let mut seen = HashSet::from([ROOT_ID.to_string()]);
        let none = Board {
            id: String::new(),
            name: String::new(),
        };
        let page = Parent {
            flow: Flow::None,
            x: 0.0,
            y: 0.0,
            page: true,
            cells: Vec::new(),
            align_items: None,
            justify_items: None,
        };
        let first = boards.len();
        let mut loose = Vec::new();
        for id in self.reader.top_level_ids(&head.dir)? {
            let pen = match self.load(&head.dir, &id, &mut seen)? {
                Loaded::Shape(pen) => pen,
                Loaded::Skipped(reason) => {
                    self.skip(&none, &id, "", reason);
                    continue;
                }
            };
            let shape_id = pen.id.0.clone().unwrap_or(id);
            let name = pen.name.0.clone().unwrap_or_default();
            if pen.kind.0.as_deref() != Some("frame") {
                loose.push((shape_id, pen));
                continue;
            }
            let board = Board {
                id: shape_id.clone(),
                name: name.clone(),
            };
            let board_bounds = bounds(&pen);
            let Some(node) = self.convert(&head.dir, &shape_id, *pen, &page, &board, &mut seen)?
            else {
                continue;
            };
            let design = self.page_design(head, node, &name)?;
            boards.push(ConvertedBoard {
                page: head.name.clone(),
                id: shape_id,
                name,
                bounds: board_bounds,
                design,
                shapes: std::mem::take(&mut self.shapes),
                left_out: std::mem::take(&mut self.left_out),
            });
        }
        if let Some(unplaced) = self.unplaced(head, loose, &mut seen)? {
            boards.push(unplaced);
        }
        self.flows(head, &boards[first..]);
        Ok(())
    }

    /// The design of a page that holds the board `node` alone.
    fn page_design(&mut self, head: &PageHead, node: Value, name: &str) -> Result<Node, ReadError> {
        let design = json!({
            "id": self.new_id(),
            "type": "page",
            "name": head.name,
            "children": [node],
        });
        serde_json::from_value::<Node>(design).map_err(|error| {
            ReadError(format!(
                "the design of the board {name:?} does not pass validation: {error}"
            ))
        })
    }

    /// The board "Unplaced" that holds the shapes of a page that sit outside
    /// every board, each at the place Penpot gives it. The board is as large as
    /// their boxes together and sits at their top left corner. None if the page
    /// has no such shape that converts.
    fn unplaced(
        &mut self,
        head: &PageHead,
        loose: Vec<(String, Box<Pen>)>,
        seen: &mut HashSet<String>,
    ) -> Result<Option<ConvertedBoard>, ReadError> {
        if loose.is_empty() {
            return Ok(None);
        }
        let boxes: Vec<Rect> = loose.iter().filter_map(|(_, pen)| bounds(pen)).collect();
        let edge = |pick: fn(f64, f64) -> f64, side: fn(&Rect) -> f64| {
            boxes.iter().map(side).reduce(pick).unwrap_or(0.0)
        };
        let (left, top) = (edge(f64::min, |r| r.x), edge(f64::min, |r| r.y));
        let (right, bottom) = (
            edge(f64::max, |r| r.x + r.width),
            edge(f64::max, |r| r.y + r.height),
        );
        let area = Rect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        };
        let board = Board {
            id: head.id.clone(),
            name: UNPLACED.to_string(),
        };
        let parent = Parent {
            flow: Flow::None,
            x: left,
            y: top,
            page: false,
            cells: Vec::new(),
            align_items: None,
            justify_items: None,
        };
        let mut children = Vec::new();
        for (id, pen) in loose {
            if let Some(value) = self.convert(&head.dir, &id, *pen, &parent, &board, seen)? {
                children.push(value);
            }
        }
        let shapes = std::mem::take(&mut self.shapes);
        let left_out = std::mem::take(&mut self.left_out);
        if children.is_empty() {
            return Ok(None);
        }
        let node = json!({
            "id": self.new_id(),
            "type": "board",
            "name": UNPLACED,
            "x": area.x,
            "y": area.y,
            "width": { "fixed": n(area.width.max(0.0)) },
            "height": { "fixed": n(area.height.max(0.0)) },
            "layout": { "mode": "none" },
            "children": children,
        });
        Ok(Some(ConvertedBoard {
            page: head.name.clone(),
            id: board.id,
            name: board.name,
            bounds: Some(area),
            design: self.page_design(head, node, UNPLACED)?,
            shapes,
            left_out,
        }))
    }

    /// Lists the prototype flows of a page, each for its starting board.
    fn flows(&mut self, head: &PageHead, boards: &[ConvertedBoard]) {
        let flows: Vec<&Value> = match &head.flows {
            Value::Object(found) => found.values().collect(),
            Value::Array(found) => found.iter().collect(),
            _ => Vec::new(),
        };
        for flow in flows {
            let Some(start) = flow.get("startingFrame").and_then(Value::as_str) else {
                continue;
            };
            let name = flow.get("name").and_then(Value::as_str).unwrap_or_default();
            let board = boards
                .iter()
                .find(|board| board.id == start)
                .map(|board| board.name.clone())
                .unwrap_or_default();
            self.entry_pages.push(self.page_name.clone());
            self.entries.push(ReportEntry {
                board: board.clone(),
                board_id: start.to_string(),
                shape_id: start.to_string(),
                name: board,
                property: Some("flows".to_string()),
                reason: format!(
                    "the prototype flow \"{name}\" starts at this board; flows are not converted"
                ),
            });
        }
    }

    /// Converts one shape and, below it, the shapes inside it. Returns None
    /// for a shape of a type that is not converted.
    fn convert(
        &mut self,
        dir: &str,
        id: &str,
        pen: Pen,
        parent: &Parent,
        board: &Board,
        seen: &mut HashSet<String>,
    ) -> Result<Option<Value>, ReadError> {
        let kind = pen.kind.0.clone().unwrap_or_default();
        let name = pen.name.0.clone().unwrap_or_default();
        let id = pen.id.0.clone().unwrap_or_else(|| id.to_string());
        let who = Who {
            board,
            id: &id,
            name: &name,
        };
        let known = matches!(
            kind.as_str(),
            "frame" | "group" | "rect" | "circle" | "text" | "path" | "image"
        );
        if !known {
            let reason = if kind == "bool" {
                "boolean shape (type `bool`) is not converted, nor are the shapes inside it"
                    .to_string()
            } else {
                format!("unknown shape type `{kind}` is not converted")
            };
            self.report(&who, None, reason);
            if let Some(bounds) = bounds(&pen) {
                self.left_out.insert(id.clone(), bounds);
            }
            return Ok(None);
        }
        let boxed = bounds(&pen).is_some();
        let bounds = match bounds(&pen) {
            Some(bounds) => bounds,
            None => {
                self.report(
                    &who,
                    Some("selrect"),
                    "the shape has no box; it is placed at 0, 0 with no size",
                );
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                }
            }
        };
        let common = Common {
            id: id.clone(),
            name: name.clone(),
            bounds,
            boxed,
            hidden: pen.hidden.0.unwrap_or(false),
        };
        self.checks(&who, &pen, &kind);
        if self.linked.is_some()
            && pen.component_id.0.is_some()
            && pen.main_instance.0 != Some(true)
        {
            if parent.page {
                // A page holds boards only, so a board that is a copy stays
                // the plain board it is, and its own copies are instances.
                if self.links_component(&pen) {
                    let component = pen.component_id.0.as_deref().unwrap_or_default();
                    self.report(
                        &who,
                        Some("componentId"),
                        format!("the board is a copy of component {component}; a board cannot be an instance, so it is drawn as the plain shapes it holds"),
                    );
                }
            } else if let Some(instance) = self.instance(dir, &common, &pen, parent, board, seen)? {
                return Ok(Some(instance));
            }
        }
        let value = match kind.as_str() {
            "frame" | "group" | "rect" => {
                self.frame(dir, &common, &pen, &kind, parent, board, seen)?
            }
            "circle" => {
                let (w, h) = (bounds.width.max(0.0), bounds.height.max(0.0));
                let path = format!(
                    "M0 {cy} A{rx} {ry} 0 1 0 {w} {cy} A{rx} {ry} 0 1 0 0 {cy} Z",
                    cy = f(h / 2.0),
                    rx = f(w / 2.0),
                    ry = f(h / 2.0),
                    w = f(w),
                );
                self.vector(&common, &pen, &who, parent, path)
            }
            "path" => {
                let path = match pen.content.0.as_ref().and_then(Value::as_str) {
                    Some(content) => shift_path(content, bounds.x, bounds.y),
                    None => None,
                };
                let path = path.unwrap_or_else(|| {
                    self.report(
                        &who,
                        Some("content"),
                        "the path data could not be read; the vector has an empty path",
                    );
                    "M0 0".to_string()
                });
                self.vector(&common, &pen, &who, parent, path)
            }
            "text" => self.text(&common, &pen, &who, parent),
            _ => self.image(&common, &pen, &who, parent),
        };
        Ok(Some(value))
    }

    /// Reports what no node can show: rotation, flips, blur, masks, component
    /// links, prototype interactions and constraints.
    fn checks(&mut self, who: &Who, pen: &Pen, kind: &str) {
        if let Some(rotation) = pen.rotation.0.filter(|rotation| rotation.abs() > 0.01) {
            self.report(
                who,
                Some("rotation"),
                format!("the shape is rotated by {rotation} degrees; rotation is not converted, so it is drawn upright"),
            );
        }
        if pen.flip_x.0 == Some(true) || pen.flip_y.0 == Some(true) {
            self.report(
                who,
                Some("flip"),
                "the shape is flipped; flipping is not converted",
            );
        }
        if let Some(blur) = &pen.blur.0 {
            if blur.get("hidden").and_then(Value::as_bool) != Some(true) {
                let style = blur
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("layer-blur");
                self.report(
                    who,
                    Some("blur"),
                    format!("blur ({style}) is not supported; the shape is drawn sharp"),
                );
            }
        }
        if pen.component_root.0 == Some(true) && !self.links_component(pen) {
            let component = pen.component_id.0.as_deref().unwrap_or("with no id");
            let role = if pen.main_instance.0 == Some(true) {
                "main instance"
            } else {
                "copy"
            };
            self.report(
                who,
                Some("componentId"),
                format!("the shape is the root of a {role} of component {component}; the component link is not converted, so it is drawn as the plain shapes it holds"),
            );
        }
        if kind == "group" && pen.masked_group.0 == Some(true) {
            self.report(
                who,
                Some("maskedGroup"),
                "masks are not supported; the masked group is drawn without its mask",
            );
        }
        if let Some(count) = pen
            .interactions
            .0
            .as_ref()
            .map(Vec::len)
            .filter(|count| *count > 0)
        {
            self.report(
                who,
                Some("interactions"),
                format!("the shape has {count} prototype interaction(s), links to other boards; prototype links are not converted"),
            );
        }
        let horizontal = pen.constraints_h.0.as_deref().unwrap_or("left");
        let vertical = pen.constraints_v.0.as_deref().unwrap_or("top");
        if horizontal != "left" || vertical != "top" {
            self.report(
                who,
                Some("constraints"),
                format!("the constraints of the shape (horizontal {horizontal}, vertical {vertical}) are not converted; the shape keeps its place and size"),
            );
        }
    }

    /// Sets what every node has: id, type, name, visibility and placement.
    fn base(
        &mut self,
        kind: &str,
        common: &Common,
        pen: &Pen,
        parent: &Parent,
    ) -> Map<String, Value> {
        let mut node = Map::new();
        let node_id = self.new_id();
        if common.boxed {
            self.shapes.push(ConvertedShape {
                id: common.id.clone(),
                name: common.name.clone(),
                node_id: node_id.clone(),
                bounds: common.bounds,
            });
        }
        if let Some(refs) = self.refs.as_mut() {
            refs.insert(common.id.clone(), (node_id.clone(), kind.to_string()));
        }
        node.insert("id".into(), json!(node_id));
        node.insert("type".into(), json!(kind));
        node.insert("name".into(), json!(common.name));
        if common.hidden {
            node.insert("visible".into(), json!(false));
        }
        let absolute = parent.flow != Flow::None && pen.layout_item_absolute.0 == Some(true);
        if parent.flow == Flow::None || absolute {
            // A board sits at the exact place Penpot stores for it, so that
            // the comparison can match the two.
            let place = |value: f64| {
                if parent.page {
                    json!(value)
                } else {
                    num(value)
                }
            };
            node.insert("x".into(), place(common.bounds.x - parent.x));
            node.insert("y".into(), place(common.bounds.y - parent.y));
        }
        if absolute {
            node.insert("absolute".into(), json!(true));
        } else if parent.flow != Flow::None {
            if let Some(margin) = margin(pen) {
                node.insert("margin".into(), margin);
            }
        }
        if !absolute && parent.flow == Flow::Grid {
            let cell = parent
                .cells
                .iter()
                .find(|cell| cell.shapes.contains(&common.id));
            if let Some(cell) = cell {
                node.insert(
                    "gridColumn".into(),
                    json!({ "start": cell.column, "span": cell.column_span }),
                );
                node.insert(
                    "gridRow".into(),
                    json!({ "start": cell.row, "span": cell.row_span }),
                );
            }
            // A cell that aligns its own item wins over the grid.
            let own = |key: fn(&Cell) -> &Option<String>| cell.and_then(|cell| key(cell).clone());
            let down = own(|cell| &cell.align_self).or_else(|| parent.align_items.clone());
            let across = own(|cell| &cell.justify_self).or_else(|| parent.justify_items.clone());
            for (key, value) in [("alignSelf", down), ("justifySelf", across)] {
                if let Some(value @ ("start" | "center" | "end" | "stretch")) = value.as_deref() {
                    node.insert(key.into(), json!(value));
                }
            }
        }
        node
    }

    /// Whether a shape's own sizing rules apply: it flows in a layout, or it
    /// is a board.
    fn sized_by_item(parent: &Parent, pen: &Pen) -> bool {
        (parent.flow != Flow::None && pen.layout_item_absolute.0 != Some(true)) || parent.page
    }

    /// Sets `width`, `height` and the bounds of the size.
    fn size(
        node: &mut Map<String, Value>,
        common: &Common,
        pen: &Pen,
        parent: &Parent,
        base: [Option<&str>; 2],
    ) {
        let by_item = Self::sized_by_item(parent, pen);
        let axis = |item: &Option<String>, base: Option<&str>, length: f64| {
            let item = if by_item { item.as_deref() } else { None };
            match (item, base) {
                (Some("fill"), _) if !parent.page => json!("fill"),
                (Some("auto"), _) | (_, Some("hug")) => json!("hug"),
                _ => json!({ "fixed": n(length.max(0.0)) }),
            }
        };
        node.insert(
            "width".into(),
            axis(&pen.layout_item_h_sizing.0, base[0], common.bounds.width),
        );
        node.insert(
            "height".into(),
            axis(&pen.layout_item_v_sizing.0, base[1], common.bounds.height),
        );
        for (key, value) in [
            ("minWidth", pen.layout_item_min_w.0),
            ("maxWidth", pen.layout_item_max_w.0),
            ("minHeight", pen.layout_item_min_h.0),
            ("maxHeight", pen.layout_item_max_h.0),
        ] {
            if let Some(value) = value.filter(|value| *value >= 0.0) {
                node.insert(key.into(), num(value));
            }
        }
        // A child that fills an axis may shrink below its content, as in
        // Penpot, unless Penpot gives it a min of its own.
        for (axis, min) in [("width", "minWidth"), ("height", "minHeight")] {
            if node.get(axis) == Some(&json!("fill")) && !node.contains_key(min) {
                node.insert(min.into(), json!(0));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn frame(
        &mut self,
        dir: &str,
        common: &Common,
        pen: &Pen,
        kind: &str,
        parent: &Parent,
        board: &Board,
        seen: &mut HashSet<String>,
    ) -> Result<Value, ReadError> {
        let who = Who {
            board,
            id: &common.id,
            name: &common.name,
        };
        let flow = match (kind, pen.layout.0.as_deref()) {
            ("frame", Some("flex")) => Flow::Flex,
            ("frame", Some("grid")) => Flow::Grid,
            _ => Flow::None,
        };
        let cells = if flow == Flow::Grid {
            grid_cells(pen)
        } else {
            Vec::new()
        };
        let own = Parent {
            flow,
            x: common.bounds.x,
            y: common.bounds.y,
            page: false,
            cells,
            align_items: pen.layout_align_items.0.clone(),
            justify_items: pen.layout_justify_items.0.clone(),
        };
        let mut children = Vec::new();
        for child in pen.shapes.0.as_deref().unwrap_or_default() {
            match self.load(dir, child, seen)? {
                Loaded::Skipped(reason) => self.skip(board, child, "", reason),
                Loaded::Shape(shape) => {
                    if let Some(value) = self.convert(dir, child, *shape, &own, board, seen)? {
                        children.push(value);
                    }
                }
            }
        }
        if flow == Flow::Flex {
            // Penpot lists the children of a flex layout last-appearing first.
            // Children out of the flow come after the others, on top.
            let (mut flowing, absolute): (Vec<Value>, Vec<Value>) = children
                .into_iter()
                .partition(|child| child.get("absolute") != Some(&json!(true)));
            flowing.reverse();
            flowing.extend(absolute);
            children = flowing;
        }

        let mut node = self.base(
            if parent.page { "board" } else { "frame" },
            common,
            pen,
            parent,
        );
        Self::size(&mut node, common, pen, parent, [None, None]);
        let layout = match flow {
            Flow::None => json!({ "mode": "none" }),
            Flow::Flex => self.flex_layout(&who, pen),
            Flow::Grid => self.grid_layout(&who, pen),
        };
        node.insert("layout".into(), layout);
        let mut fills = self.fills(&who, pen, common.bounds);
        self.link_fills(pen, &mut fills);
        insert_list(&mut node, "fills", fills);
        let mut strokes = self.strokes(&who, pen, Look::Frame);
        self.link_strokes(pen, &mut strokes);
        insert_list(&mut node, "strokes", strokes);
        if let Some(radius) = radius(pen) {
            node.insert("radius".into(), radius);
        }
        let shadows = self.shadows(&who, pen);
        insert_list(&mut node, "shadows", shadows);
        if let Some(opacity) = opacity(pen) {
            node.insert("opacity".into(), opacity);
        }
        if pen.show_content.0 == Some(false) {
            node.insert("clip".into(), json!(true));
        }
        node.insert("children".into(), Value::Array(children));
        Ok(Value::Object(node))
    }

    fn flex_layout(&mut self, who: &Who, pen: &Pen) -> Value {
        let mut layout = Map::new();
        layout.insert("mode".into(), json!("flex"));
        let direction = pen
            .layout_flex_dir
            .0
            .as_deref()
            .filter(|direction| matches!(*direction, "column" | "row-reverse" | "column-reverse"))
            .unwrap_or("row");
        layout.insert("direction".into(), json!(direction));
        let wrap = match pen.layout_wrap_type.0.as_deref() {
            Some("wrap") => "wrap",
            Some("wrap-reverse") => "wrap-reverse",
            _ => "nowrap",
        };
        layout.insert("wrap".into(), json!(wrap));
        spacing(&mut layout, pen);
        // Penpot's stretch does not stretch a child that hugs its content, so
        // it is start; a child that fills the cross axis stretches anyway.
        match pen.layout_align_items.0.as_deref() {
            Some("stretch") => {
                layout.insert("alignItems".into(), json!("start"));
            }
            Some(align @ ("start" | "center" | "end" | "baseline")) => {
                layout.insert("alignItems".into(), json!(align));
            }
            _ => {}
        }
        if let Some(
            justify @ ("start" | "center" | "end" | "space-between" | "space-around"
            | "space-evenly"),
        ) = pen.layout_justify_content.0.as_deref()
        {
            layout.insert("justifyContent".into(), json!(justify));
        }
        if wrap != "nowrap" {
            if let Some(content) = pen
                .layout_align_content
                .0
                .as_deref()
                .filter(|content| *content != "stretch")
            {
                self.report(
                    who,
                    Some("layoutAlignContent"),
                    format!(
                        "aligning the lines of a wrapping layout (`{content}`) is not supported"
                    ),
                );
            }
        }
        Value::Object(layout)
    }

    fn grid_layout(&mut self, who: &Who, pen: &Pen) -> Value {
        let mut layout = Map::new();
        layout.insert("mode".into(), json!("grid"));
        for (key, tracks) in [
            ("columns", &pen.layout_grid_columns.0),
            ("rows", &pen.layout_grid_rows.0),
        ] {
            let tracks: Vec<Value> = tracks
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|track| self.track(who, track))
                .collect();
            if !tracks.is_empty() {
                layout.insert(key.into(), Value::Array(tracks));
            }
        }
        spacing(&mut layout, pen);
        Value::Object(layout)
    }

    fn track(&mut self, who: &Who, track: &Value) -> Value {
        let value = track.get("value").and_then(Value::as_f64);
        match track.get("type").and_then(Value::as_str) {
            Some("flex") => json!({ "fr": n(value.unwrap_or(1.0).max(0.0)) }),
            Some("fixed") => json!({ "fixed": n(value.unwrap_or(0.0).max(0.0)) }),
            Some("auto") | None => json!("auto"),
            Some(other) => {
                self.report(
                    who,
                    Some("layoutGridColumns"),
                    format!("grid tracks of type `{other}` are not supported; the track is sized by its content"),
                );
                json!("auto")
            }
        }
    }

    fn vector(
        &mut self,
        common: &Common,
        pen: &Pen,
        who: &Who,
        parent: &Parent,
        path: String,
    ) -> Value {
        let mut node = self.base("vector", common, pen, parent);
        Self::size(&mut node, common, pen, parent, [None, None]);
        node.insert("path".into(), json!(path));
        let mut fills = self.fills(who, pen, common.bounds);
        self.link_fills(pen, &mut fills);
        insert_list(&mut node, "fills", fills);
        let mut strokes = self.strokes(who, pen, Look::Vector);
        self.link_strokes(pen, &mut strokes);
        insert_list(&mut node, "strokes", strokes);
        let shadows = self.shadows(who, pen);
        insert_list(&mut node, "shadows", shadows);
        if let Some(opacity) = opacity(pen) {
            node.insert("opacity".into(), opacity);
        }
        Value::Object(node)
    }

    fn image(&mut self, common: &Common, pen: &Pen, who: &Who, parent: &Parent) -> Value {
        let media = pen
            .metadata
            .0
            .as_ref()
            .and_then(|metadata| metadata.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let reference = match self.attach(&media) {
            Ok(reference) => reference,
            Err(reason) => {
                // Without its image the shape is an empty frame of the same box.
                self.report(who, Some("image"), reason);
                let mut node = self.base("frame", common, pen, parent);
                Self::size(&mut node, common, pen, parent, [None, None]);
                node.insert("layout".into(), json!({ "mode": "none" }));
                node.insert("children".into(), json!([]));
                return Value::Object(node);
            }
        };
        let mut node = self.base("image", common, pen, parent);
        Self::size(&mut node, common, pen, parent, [None, None]);
        node.insert("image".into(), json!(reference));
        node.insert("fit".into(), json!("cover"));
        let mut strokes = self.strokes(who, pen, Look::Image);
        self.link_strokes(pen, &mut strokes);
        insert_list(&mut node, "strokes", strokes);
        if let Some(radius) = radius(pen) {
            node.insert("radius".into(), radius);
        }
        let shadows = self.shadows(who, pen);
        insert_list(&mut node, "shadows", shadows);
        if let Some(opacity) = opacity(pen) {
            node.insert("opacity".into(), opacity);
        }
        Value::Object(node)
    }

    /// The `attachment:<id>` reference of the image with the media id
    /// `media`, or why it cannot be had. The image's file is put among the
    /// attachments the first time.
    fn attach(&mut self, media: &str) -> Result<String, String> {
        if let Some(known) = self.attached.get(media) {
            return known.clone();
        }
        let outcome = self.load_image(media);
        self.attached.insert(media.to_string(), outcome.clone());
        outcome
    }

    fn load_image(&mut self, media: &str) -> Result<String, String> {
        if !is_uuid(media) {
            return Err(format!("the image id `{media}` is not a UUID"));
        }
        let record = format!("files/{}/media/{media}.json", self.file_id);
        match self.reader.load(&record) {
            Ok(Some(())) => {}
            Ok(None) => {
                return Err(format!(
                    "the export has no media record for the image {media}"
                ));
            }
            Err(error) => return Err(error.to_string()),
        }
        let record: Value = serde_json::from_slice(&self.reader.buffer)
            .map_err(|_| format!("the media record of the image {media} could not be read"))?;
        let stored = record
            .get("mediaId")
            .and_then(Value::as_str)
            .unwrap_or(media)
            .to_string();
        let entry = self
            .objects
            .get(&stored)
            .cloned()
            .ok_or_else(|| format!("the export has no image file for the image {media}"))?;
        match self.reader.load(&entry) {
            Ok(Some(())) => {}
            Ok(None) => return Err(format!("the image file {entry} is not in the export")),
            Err(error) => return Err(error.to_string()),
        }
        let extension = entry
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_lowercase())
            .unwrap_or_default();
        self.attachments.push(Attachment {
            id: media.to_string(),
            extension,
            bytes: self.reader.buffer.clone(),
        });
        Ok(format!("attachment:{media}"))
    }

    fn text(&mut self, common: &Common, pen: &Pen, who: &Who, parent: &Parent) -> Value {
        let mut paragraphs = Vec::new();
        if let Some(content) = &pen.content.0 {
            collect_paragraphs(content, &mut paragraphs);
        }
        // The runs of every paragraph, each with the paragraph it is in.
        let runs: Vec<(&Value, &Value)> = paragraphs
            .iter()
            .flat_map(|paragraph| {
                paragraph
                    .get("children")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(move |run| (run, *paragraph))
            })
            .collect();

        let mut node = self.base("text", common, pen, parent);
        let grow = pen.grow_type.0.as_deref().unwrap_or("fixed");
        let base = match grow {
            "auto-width" => [Some("hug"), Some("hug")],
            "auto-height" => [None, Some("hug")],
            _ => [None, None],
        };
        Self::size(&mut node, common, pen, parent, base);
        // A text that hugs its content follows Chrome, not the whole-pixel box
        // Penpot stored for it: it gets no min size from that box, in a layout
        // or not. The comparison pins texts to Penpot's boxes on its own
        // (docs/penpot-compare.md).

        let Some((first, first_paragraph)) = runs.first().copied() else {
            self.report(who, Some("content"), "the text has no characters or style");
            node.insert("content".into(), json!(""));
            node.insert("fontFamily".into(), json!("Inter"));
            node.insert("fontSize".into(), json!(14));
            return Value::Object(node);
        };
        let attr = |run: &Value, paragraph: &Value, key: &str| -> Option<Value> {
            run.get(key)
                .or_else(|| paragraph.get(key))
                .filter(|value| !value.is_null())
                .cloned()
        };
        let style = |run: &Value, paragraph: &Value| -> String {
            [
                "fontFamily",
                "fontSize",
                "fontWeight",
                "fontStyle",
                "lineHeight",
                "letterSpacing",
                "textAlign",
                "textDecoration",
                "textTransform",
                "fills",
            ]
            .iter()
            .map(|key| format!("{key}={:?};", attr(run, paragraph, key)))
            .collect()
        };
        let first_style = style(first, first_paragraph);
        if runs
            .iter()
            .any(|(run, paragraph)| style(run, paragraph) != first_style)
        {
            self.report(
                who,
                Some("content"),
                "the text has mixed styles (runs differ in font, size, weight, color or alignment); the whole text takes the style of its first run",
            );
        }

        let content = text_string(&paragraphs, first, first_paragraph);
        let get = |key: &str| attr(first, first_paragraph, key);
        node.insert("content".into(), json!(content));

        let family = get("fontFamily")
            .and_then(|value| value.as_str().map(str::to_string))
            .filter(|family| !family.is_empty() && family.trim() == family);
        let family = family.unwrap_or_else(|| {
            self.report(
                who,
                Some("fontFamily"),
                "the text names no usable font family; Inter is used",
            );
            "Inter".to_string()
        });
        node.insert("fontFamily".into(), json!(family));
        let size = get("fontSize")
            .as_ref()
            .and_then(as_number)
            .filter(|size| *size >= 0.0);
        let size = size.unwrap_or_else(|| {
            self.report(
                who,
                Some("fontSize"),
                "the text has no usable font size; 14 is used",
            );
            14.0
        });
        node.insert("fontSize".into(), num(size));
        if let Some(weight) = get("fontWeight").as_ref().and_then(as_number) {
            if (1.0..=1000.0).contains(&weight) {
                node.insert("fontWeight".into(), num(weight));
            } else {
                self.report(
                    who,
                    Some("fontWeight"),
                    format!("the font weight {weight} is out of range"),
                );
            }
        }
        if let Some(style @ ("normal" | "italic")) =
            get("fontStyle").as_ref().and_then(Value::as_str)
        {
            node.insert("fontStyle".into(), json!(style));
        }
        if let Some(pitch) = line_pitch(pen, common.bounds, grow).filter(|_| size > 0.0) {
            // Exactly, so that layout snaps the line to the pitch.
            node.insert("lineHeight".into(), json!((pitch + 1.0 / 128.0) / size));
        } else if let Some(height) = get("lineHeight")
            .as_ref()
            .and_then(as_number)
            .filter(|height| *height >= 0.0)
        {
            node.insert("lineHeight".into(), num(height));
        }
        if let Some(spacing) = get("letterSpacing").as_ref().and_then(as_number) {
            node.insert("letterSpacing".into(), num(spacing));
        }
        if let Some(align @ ("left" | "center" | "right" | "justify")) =
            get("textAlign").as_ref().and_then(Value::as_str)
        {
            node.insert("textAlign".into(), json!(align));
        }
        match get("textDecoration").as_ref().and_then(Value::as_str) {
            Some(decoration @ ("none" | "underline" | "line-through")) => {
                node.insert("textDecoration".into(), json!(decoration));
            }
            Some(other) => self.report(
                who,
                Some("textDecoration"),
                format!("the text decoration `{other}` is not supported"),
            ),
            None => {}
        }
        if let Some(fill) = get("fills")
            .as_ref()
            .and_then(Value::as_array)
            .and_then(|fills| fills.first())
        {
            if let Some(color) = self.text_color(who, fill) {
                node.insert("color".into(), json!(color));
            }
        }
        if let Some(link) = self.token_link(pen, "fill") {
            node.insert("color".into(), json!(link));
        }
        if let Some(opacity) = opacity(pen) {
            node.insert("opacity".into(), opacity);
        }
        if pen
            .strokes
            .0
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|stroke| {
                stroke
                    .get("strokeWidth")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    > 0.0
            })
        {
            self.report(who, Some("strokes"), "strokes on text are not supported");
        }
        if pen
            .shadow
            .0
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|shadow| shadow.get("hidden").and_then(Value::as_bool) != Some(true))
        {
            self.report(who, Some("shadow"), "shadows on text are not supported");
        }
        Value::Object(node)
    }

    /// The color of a text run's fill.
    fn text_color(&mut self, who: &Who, fill: &Value) -> Option<String> {
        let opacity = fill
            .get("fillOpacity")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        if let Some(gradient) = fill.get("fillColorGradient") {
            self.report(
                who,
                Some("fills"),
                "gradient fills on text are not supported; the text takes the color of the first stop",
            );
            let stop = gradient.get("stops")?.as_array()?.first()?;
            let stop_opacity = stop.get("opacity").and_then(Value::as_f64).unwrap_or(1.0);
            return color(stop.get("color")?.as_str()?, opacity * stop_opacity);
        }
        color(fill.get("fillColor")?.as_str()?, opacity)
    }

    fn fills(&mut self, who: &Who, pen: &Pen, bounds: Rect) -> Vec<Value> {
        let mut fills = Vec::new();
        for fill in pen.fills.0.as_deref().unwrap_or_default() {
            if let Some(fill) = self.fill(who, fill, bounds) {
                fills.push(fill);
            }
        }
        fills
    }

    fn fill(&mut self, who: &Who, fill: &Value, bounds: Rect) -> Option<Value> {
        let opacity = fill
            .get("fillOpacity")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        if let Some(image) = fill.get("fillImage") {
            let media = image.get("id").and_then(Value::as_str).unwrap_or_default();
            if image.get("keepAspectRatio").and_then(Value::as_bool) == Some(false) {
                self.report(
                    who,
                    Some("fills"),
                    "an image fill that stretches instead of keeping its aspect ratio is drawn as cover",
                );
            }
            return match self.attach(media) {
                Ok(reference) => {
                    Some(json!({ "type": "image", "image": reference, "fit": "cover" }))
                }
                Err(reason) => {
                    self.report(who, Some("fills"), reason);
                    None
                }
            };
        }
        if let Some(gradient) = fill.get("fillColorGradient") {
            return self.gradient(who, gradient, bounds, opacity);
        }
        let text = fill.get("fillColor").and_then(Value::as_str)?;
        match color(text, opacity) {
            Some(color) => Some(json!({ "type": "solid", "color": color })),
            None => {
                self.report(
                    who,
                    Some("fills"),
                    format!("`{text}` is not a color; the fill is left out"),
                );
                None
            }
        }
    }

    /// A linear gradient as a CSS one. Penpot gives the line the gradient runs
    /// along as two points in fractions of the box; CSS gives an angle and
    /// runs the line through the center of the box and out to its corners, so
    /// the stops are moved onto that line.
    fn gradient(
        &mut self,
        who: &Who,
        gradient: &Value,
        bounds: Rect,
        opacity: f64,
    ) -> Option<Value> {
        let kind = gradient
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("linear");
        if kind != "linear" {
            self.report(
                who,
                Some("fills"),
                format!("{kind} gradient fills are not supported; the fill is left out"),
            );
            return None;
        }
        let point =
            |key: &str, default: f64| gradient.get(key).and_then(Value::as_f64).unwrap_or(default);
        let (sx, sy) = (point("startX", 0.5), point("startY", 0.0));
        let (ex, ey) = (point("endX", 0.5), point("endY", 1.0));
        let (w, h) = (bounds.width, bounds.height);
        let (dx, dy) = ((ex - sx) * w, (ey - sy) * h);
        let length = dx.hypot(dy);
        let (ux, uy) = if length > 1e-9 {
            (dx / length, dy / length)
        } else {
            (0.0, 1.0)
        };
        let mut angle = ux.atan2(-uy).to_degrees();
        if angle < 0.0 {
            angle += 360.0;
        }
        let line = (w * ux).abs() + (h * uy).abs();
        let from_center = (sx - 0.5) * w * ux + (sy - 0.5) * h * uy;
        let mut stops = Vec::new();
        for stop in gradient
            .get("stops")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let offset = stop.get("offset").and_then(Value::as_f64).unwrap_or(0.0);
            let stop_opacity = stop.get("opacity").and_then(Value::as_f64).unwrap_or(1.0);
            let Some(stop_color) = stop
                .get("color")
                .and_then(Value::as_str)
                .and_then(|text| color(text, opacity * stop_opacity))
            else {
                continue;
            };
            let position = if line > 1e-9 {
                (from_center + offset * length) / line + 0.5
            } else {
                offset
            };
            stops.push(json!({ "position": n(position), "color": stop_color }));
        }
        if stops.len() < 2 {
            self.report(
                who,
                Some("fills"),
                "the gradient has fewer than two usable stops; the fill is left out",
            );
            return None;
        }
        Some(json!({ "type": "linear-gradient", "angle": n(angle), "stops": stops }))
    }

    fn strokes(&mut self, who: &Who, pen: &Pen, look: Look) -> Vec<Value> {
        let mut strokes = Vec::new();
        for stroke in pen.strokes.0.as_deref().unwrap_or_default() {
            let width = stroke
                .get("strokeWidth")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            if width <= 0.0 {
                continue;
            }
            let style = stroke
                .get("strokeStyle")
                .and_then(Value::as_str)
                .unwrap_or("solid");
            if !matches!(style, "solid" | "dashed") {
                self.report(
                    who,
                    Some("strokeStyle"),
                    format!(
                        "the stroke style `{style}` is not supported; the stroke is drawn solid"
                    ),
                );
            }
            let align = stroke
                .get("strokeAlignment")
                .and_then(Value::as_str)
                .unwrap_or("inner");
            // A stroke of an image lies inside its edge.
            let align = match (look, align) {
                (Look::Vector, "inner") => Some("inside"),
                (Look::Frame | Look::Vector, "outer") => Some("outside"),
                (Look::Frame | Look::Vector, "center") => Some("center"),
                (Look::Image, "outer" | "center") => {
                    self.report(
                        who,
                        Some("strokeAlignment"),
                        format!("the stroke alignment `{align}` is not supported on images; the stroke is drawn inside the image"),
                    );
                    None
                }
                _ => None,
            };
            if stroke.get("strokeColorGradient").is_some() || stroke.get("strokeImage").is_some() {
                self.report(
                    who,
                    Some("strokes"),
                    "gradient and image strokes are not supported; the stroke is left out",
                );
                continue;
            }
            let opacity = stroke
                .get("strokeOpacity")
                .and_then(Value::as_f64)
                .unwrap_or(1.0);
            match stroke
                .get("strokeColor")
                .and_then(Value::as_str)
                .and_then(|text| color(text, opacity))
            {
                Some(color) => {
                    let mut line = json!({ "color": color, "width": num(width) });
                    if let Some(align) = align {
                        line["align"] = json!(align);
                    }
                    if style == "dashed" {
                        // Penpot draws dashes and gaps six stroke widths long
                        // (observed at width 2).
                        line["dash"] =
                            json!({ "length": num(width * 6.0), "gap": num(width * 6.0) });
                    }
                    strokes.push(line);
                }
                None => self.report(
                    who,
                    Some("strokes"),
                    "the stroke has no usable color; it is left out",
                ),
            }
        }
        strokes
    }

    fn shadows(&mut self, who: &Who, pen: &Pen) -> Vec<Value> {
        let mut shadows = Vec::new();
        for shadow in pen.shadow.0.as_deref().unwrap_or_default() {
            if shadow.get("hidden").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let kind = match shadow.get("style").and_then(Value::as_str) {
                Some("drop-shadow") => "drop",
                Some("inner-shadow") => "inner",
                other => {
                    self.report(
                        who,
                        Some("shadow"),
                        format!(
                            "the shadow style `{}` is not supported",
                            other.unwrap_or("")
                        ),
                    );
                    continue;
                }
            };
            let paint = shadow.get("color");
            let opacity = paint
                .and_then(|paint| paint.get("opacity"))
                .and_then(Value::as_f64)
                .unwrap_or(1.0);
            let Some(shadow_color) = paint
                .and_then(|paint| paint.get("color"))
                .and_then(Value::as_str)
                .and_then(|text| color(text, opacity))
            else {
                self.report(
                    who,
                    Some("shadow"),
                    "the shadow has no usable color; it is left out",
                );
                continue;
            };
            let field = |key: &str| shadow.get(key).and_then(Value::as_f64).unwrap_or(0.0);
            shadows.push(json!({
                "type": kind,
                "color": shadow_color,
                "offsetX": num(field("offsetX")),
                "offsetY": num(field("offsetY")),
                "blur": num(field("blur").max(0.0)),
                "spread": num(field("spread")),
            }));
        }
        shadows
    }
}

/// The box of a shape: its selrect, or else its own x, y, width and height.
fn bounds(pen: &Pen) -> Option<Rect> {
    pen.selrect.0.as_ref().and_then(RawBox::rect).or_else(|| {
        Some(Rect {
            x: pen.x.0?,
            y: pen.y.0?,
            width: pen.width.0?,
            height: pen.height.0?,
        })
    })
}

/// The cells of a grid layout, in reading order.
fn grid_cells(pen: &Pen) -> Vec<Cell> {
    let Some(cells) = pen.layout_grid_cells.0.as_ref().and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut found: Vec<Cell> = cells
        .values()
        .map(|cell| {
            let int = |key: &str, default: i64| {
                cell.get(key)
                    .and_then(Value::as_f64)
                    .map_or(default, |value| value as i64)
            };
            Cell {
                shapes: cell
                    .get("shapes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|shape| shape.as_str().map(str::to_string))
                    .collect(),
                column: int("column", 1).max(1),
                row: int("row", 1).max(1),
                column_span: int("columnSpan", 1).max(1),
                row_span: int("rowSpan", 1).max(1),
                align_self: own_alignment(cell, "alignSelf"),
                justify_self: own_alignment(cell, "justifySelf"),
            }
        })
        .collect();
    found.sort_by_key(|cell| (cell.row, cell.column));
    found
}

/// How a grid cell aligns its own item, or None for `auto` or nothing.
fn own_alignment(cell: &Value, key: &str) -> Option<String> {
    cell.get(key)
        .and_then(Value::as_str)
        .filter(|value| *value != "auto")
        .map(str::to_string)
}

/// The margin of a shape in a layout (`layoutItemMargin`: `m1` top, `m2` right,
/// `m3` bottom, `m4` left), or None if all four sides are 0.
fn margin(pen: &Pen) -> Option<Value> {
    let margin = pen.layout_item_margin.0.as_ref()?;
    let mut sides = Map::new();
    for (key, side) in [
        ("m1", "top"),
        ("m2", "right"),
        ("m3", "bottom"),
        ("m4", "left"),
    ] {
        let value = margin.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        if n(value) != 0.0 {
            sides.insert(side.into(), num(value));
        }
    }
    (!sides.is_empty()).then_some(Value::Object(sides))
}

/// Sets the gap and padding of a flex or grid layout, as far as they are not 0.
fn spacing(layout: &mut Map<String, Value>, pen: &Pen) {
    if let Some(gap) = &pen.layout_gap.0 {
        let part = |key: &str| gap.get(key).and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
        let (row, column) = (part("rowGap"), part("columnGap"));
        if row != 0.0 || column != 0.0 {
            layout.insert("gap".into(), json!({ "row": n(row), "column": n(column) }));
        }
    }
    if let Some(padding) = &pen.layout_padding.0 {
        let part = |key: &str| {
            padding
                .get(key)
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .max(0.0)
        };
        let (top, right, bottom, left) = (part("p1"), part("p2"), part("p3"), part("p4"));
        if top != 0.0 || right != 0.0 || bottom != 0.0 || left != 0.0 {
            layout.insert(
                "padding".into(),
                json!({ "top": n(top), "right": n(right), "bottom": n(bottom), "left": n(left) }),
            );
        }
    }
}

/// The corner radii of a shape, or None if all four corners are square.
fn radius(pen: &Pen) -> Option<Value> {
    let corners =
        [pen.r1.0, pen.r2.0, pen.r3.0, pen.r4.0].map(|corner| corner.unwrap_or(0.0).max(0.0));
    if corners.iter().all(|corner| *corner == 0.0) {
        return None;
    }
    Some(json!({
        "topLeft": n(corners[0]),
        "topRight": n(corners[1]),
        "bottomRight": n(corners[2]),
        "bottomLeft": n(corners[3]),
    }))
}

/// The opacity of a shape, or None if it is fully opaque.
fn opacity(pen: &Pen) -> Option<Value> {
    pen.opacity
        .0
        .filter(|opacity| *opacity < 1.0)
        .map(|opacity| num(opacity.clamp(0.0, 1.0)))
}

/// Sets `key` to `list` when the list has anything in it.
fn insert_list(node: &mut Map<String, Value>, key: &str, list: Vec<Value>) {
    if !list.is_empty() {
        node.insert(key.into(), Value::Array(list));
    }
}

/// The distance between the lines of a text as Penpot's browser laid it out:
/// the gap between the y of the first two lines of `positionData`, or for a
/// text of one line whose box hugs it the height of the box. None when the file
/// has no lines to go by.
fn line_pitch(pen: &Pen, bounds: Rect, grow: &str) -> Option<f64> {
    let mut lines: Vec<f64> = Vec::new();
    for item in pen.position_data.0.as_deref().unwrap_or_default() {
        let y = item.get("y").and_then(Value::as_f64)?;
        if !lines.contains(&y) {
            lines.push(y);
        }
    }
    match lines.as_slice() {
        [first, second, ..] => Some(second - first).filter(|pitch| *pitch > 0.0),
        [_] if grow != "fixed" => Some(bounds.height).filter(|pitch| *pitch > 0.0),
        _ => None,
    }
}

/// The characters of a text: its paragraphs joined with a newline, with the
/// `textTransform` of the first run applied.
fn text_string(paragraphs: &[&Value], first: &Value, first_paragraph: &Value) -> String {
    let lines: Vec<String> = paragraphs
        .iter()
        .map(|paragraph| {
            paragraph
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect()
        })
        .collect();
    let content = lines.join("\n");
    let transform = first
        .get("textTransform")
        .or_else(|| first_paragraph.get("textTransform"))
        .filter(|value| !value.is_null())
        .and_then(Value::as_str);
    match transform {
        Some("uppercase") => content.to_uppercase(),
        Some("lowercase") => content.to_lowercase(),
        Some("capitalize") => capitalize(&content),
        _ => content,
    }
}

/// The characters of the text shape `pen`, None when it has no runs.
fn text_of(pen: &Pen) -> Option<String> {
    let mut paragraphs = Vec::new();
    if let Some(content) = &pen.content.0 {
        collect_paragraphs(content, &mut paragraphs);
    }
    let (first, first_paragraph) = paragraphs.iter().find_map(|paragraph| {
        let run = paragraph
            .get("children")
            .and_then(Value::as_array)?
            .first()?;
        Some((run, *paragraph))
    })?;
    Some(text_string(&paragraphs, first, first_paragraph))
}

/// The paragraphs of a text's content: the `paragraph` nodes of its tree.
fn collect_paragraphs<'a>(content: &'a Value, found: &mut Vec<&'a Value>) {
    if content.get("type").and_then(Value::as_str) == Some("paragraph") {
        found.push(content);
        return;
    }
    for child in content
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        collect_paragraphs(child, found);
    }
}

fn capitalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for character in text.chars() {
        if start {
            out.extend(character.to_uppercase());
        } else {
            out.push(character);
        }
        start = character.is_whitespace();
    }
    out
}

/// A number that a text style writes as a number or as a string.
fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// A color as `#rrggbb`, with the alpha byte added when `alpha` is below 1.
fn color(text: &str, alpha: f64) -> Option<String> {
    let hex = text.strip_prefix('#')?.to_lowercase();
    let hex = match hex.len() {
        3 => hex.chars().flat_map(|digit| [digit, digit]).collect(),
        6 => hex,
        _ => return None,
    };
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha >= 1.0 {
        Some(format!("#{hex}"))
    } else {
        Some(format!("#{hex}{:02x}", (alpha * 255.0).round() as u8))
    }
}

/// Whether `id` is a UUID in its hyphenated form, as attachment ids must be.
fn is_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// Rounds to a thousandth, which hides the noise in Penpot's floating-point
/// boxes (a width of 76.99999809265137) and costs nothing a pixel could show.
fn n(value: f64) -> f64 {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded.is_finite() {
        rounded + 0.0
    } else {
        0.0
    }
}

fn num(value: f64) -> Value {
    json!(n(value))
}

/// A number in path data.
fn f(value: f64) -> String {
    n(value).to_string()
}

/// SVG path data moved by (`-dx`, `-dy`): the absolute coordinates of the
/// commands that have them, written with spaces. None if the data is not
/// path data that OmniState accepts.
fn shift_path(data: &str, dx: f64, dy: f64) -> Option<String> {
    let bytes = data.as_bytes();
    let mut at = 0;
    let mut out = String::new();
    let mut command = b' ';
    let mut index = 0usize;
    let mut first = true;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte.is_ascii_whitespace() || byte == b',' {
            at += 1;
            continue;
        }
        if byte.is_ascii_alphabetic() {
            command = byte;
            index = 0;
            at += 1;
            if !out.is_empty() {
                out.push(' ');
            }
            out.push(command as char);
            continue;
        }
        let arc = command.eq_ignore_ascii_case(&b'a');
        let number = if arc && matches!(index % 7, 3 | 4) {
            // An arc flag is one digit, whatever follows.
            if !matches!(byte, b'0' | b'1') {
                return None;
            }
            at += 1;
            f64::from(byte - b'0')
        } else {
            let start = at;
            if matches!(bytes[at], b'+' | b'-') {
                at += 1;
            }
            while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'.') {
                at += 1;
            }
            if at < bytes.len() && matches!(bytes[at], b'e' | b'E') {
                at += 1;
                if at < bytes.len() && matches!(bytes[at], b'+' | b'-') {
                    at += 1;
                }
                while at < bytes.len() && bytes[at].is_ascii_digit() {
                    at += 1;
                }
            }
            data[start..at].parse::<f64>().ok()?
        };
        // Which coordinate this argument is, if any: 0 for x, 1 for y.
        let axis = match command.to_ascii_lowercase() {
            b'm' | b'l' | b't' | b'c' | b's' | b'q' => Some(index % 2),
            b'h' => Some(0),
            b'v' => Some(1),
            b'a' => match index % 7 {
                5 => Some(0),
                6 => Some(1),
                _ => None,
            },
            _ => None,
        };
        // A path starts with an absolute moveto even when it is written `m`.
        let absolute = command.is_ascii_uppercase() || (command == b'm' && first && index < 2);
        let value = match axis {
            Some(0) if absolute => number - dx,
            Some(1) if absolute => number - dy,
            _ => number,
        };
        out.push(' ');
        out.push_str(&f(value));
        index += 1;
        if command.eq_ignore_ascii_case(&b'm') && index >= 2 {
            first = false;
        }
    }
    let data = out.trim().to_string();
    data.parse::<PathData>().ok().map(|_| data)
}
