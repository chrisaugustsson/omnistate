//! The import's share of the converter (US-051): the components of the export,
//! the copies of them that become instances with their overrides, and the
//! design tokens that fills, strokes and text colors stay linked to.
//!
//! A component is converted from its main instance the first time something
//! asks for it, so a copy never waits for a page that comes later. Its nodes
//! are remembered by the Penpot id of the main's shape they came from: a copy
//! names the main's shape each of its own shapes copies (`shapeRef`), so the
//! changes the designer made to the copy (`touched`) can be keyed by the nodes
//! of the component.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::*;
use crate::import::tokens::{self, Note, Tokens};

/// What the converter needs to know in an import.
pub(crate) struct Linked {
    components: HashMap<String, RawComponent>,
    done: HashMap<String, Option<Rc<Comp>>>,
    /// The components being converted, which a component cannot place.
    progress: HashSet<String>,
    /// The folder and the name of each page, by the page's id.
    pages: HashMap<String, (String, String)>,
    /// The token path a Penpot token name links to, written `$color.surface`.
    links: HashMap<String, String>,
}

/// A component of the export, as its file says.
#[derive(Clone)]
pub(crate) struct RawComponent {
    pub(crate) id: String,
    /// The name the component gets: Penpot's name, with its path in front.
    pub(crate) name: String,
    pub(crate) main_id: String,
    pub(crate) main_page: String,
    /// The `props` of the component: the variant properties of its set.
    pub(crate) props: Option<Value>,
}

/// A component as a node, with where its parts came from.
pub(crate) struct Comp {
    node_id: String,
    /// The box Penpot stores for the main instance.
    bounds: Rect,
    /// The node and its type for each shape of the main instance that became
    /// one, by the Penpot id of the shape.
    nodes: HashMap<String, (String, String)>,
    json: Value,
}

/// What an import reads from an export before it is put together.
pub(crate) struct ReadLinks {
    /// The boards with the index of the page each is on.
    pub(crate) boards: Vec<(usize, ConvertedBoard)>,
    /// The id and the name of each page, in page order.
    pub(crate) pages: Vec<(String, String)>,
    pub(crate) attachments: Vec<Attachment>,
    /// Everything left out, with the name of the page it is on.
    pub(crate) entries: Vec<(String, ReportEntry)>,
    /// The components as JSON nodes, in order.
    pub(crate) components: Vec<Value>,
    pub(crate) tokens: Tokens,
    /// How many node IDs the conversion has handed out.
    pub(crate) ids: u64,
}

/// The JSON entries of the archive under `prefix` that end in `.json`, by name.
/// An entry that cannot be read is left out and noted.
fn json_entries<R: Read + Seek>(
    reader: &mut Reader<'_, R>,
    prefix: &str,
    notes: &mut Vec<Note>,
) -> Result<Vec<Value>, ReadError> {
    let mut names: Vec<String> = reader
        .archive
        .file_names()
        .filter_map(|name| name.ok().map(|name| name.to_string()))
        .filter(|name| {
            name.strip_prefix(prefix)
                .is_some_and(|rest| rest.ends_with(".json") && !rest.contains('/'))
        })
        .collect();
    names.sort();
    let mut found = Vec::new();
    for name in names {
        if reader.load(&name)?.is_none() {
            continue;
        }
        match serde_json::from_slice::<Value>(&reader.buffer) {
            Ok(value) => found.push(value),
            Err(error) => notes.push(Note {
                name: name.clone(),
                property: None,
                reason: format!("the file {name} could not be read: {error}"),
            }),
        }
    }
    Ok(found)
}

/// The text of `key` in a JSON object.
fn text_at(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The components of the files, in order, with the properties of their variant
/// sets: the components with one `variantId` are the variants of one set, and
/// each one's properties list every value of the set with its own as default.
fn raw_components(files: &[Value], notes: &mut Vec<Note>) -> Vec<RawComponent> {
    struct Found<'a> {
        file: &'a Value,
        id: String,
        name: String,
        path: String,
    }
    let mut found: Vec<Found> = Vec::new();
    for file in files {
        let id = text_at(file, "id");
        let name = text_at(file, "name");
        if id.is_empty() || name.is_empty() {
            notes.push(Note {
                name,
                property: None,
                reason: "a component file has no id or no name; the component is not imported"
                    .to_string(),
            });
            continue;
        }
        found.push(Found {
            file,
            id,
            name,
            path: text_at(file, "path"),
        });
    }
    let properties = |file: &Value| -> Vec<(String, String)> {
        file.get("variantProperties")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|property| {
                Some((
                    property.get("name")?.as_str()?.to_string(),
                    property.get("value")?.as_str()?.to_string(),
                ))
            })
            .collect()
    };
    found.sort_by_cached_key(|f| {
        (
            f.path.clone(),
            f.name.clone(),
            properties(f.file),
            f.id.clone(),
        )
    });
    // The values of each property in each variant set, in the order of the
    // components.
    let mut sets: HashMap<String, Vec<(String, Vec<String>)>> = HashMap::new();
    for f in &found {
        let Some(set) = f.file.get("variantId").and_then(Value::as_str) else {
            continue;
        };
        let entry = sets.entry(set.to_string()).or_default();
        for (property, value) in properties(f.file) {
            match entry.iter_mut().find(|(known, _)| *known == property) {
                Some((_, values)) => {
                    if !values.contains(&value) {
                        values.push(value);
                    }
                }
                None => entry.push((property, vec![value])),
            }
        }
    }
    found
        .iter()
        .map(|f| {
            let own = properties(f.file);
            let props = f
                .file
                .get("variantId")
                .and_then(Value::as_str)
                .and_then(|set| sets.get(set))
                .filter(|_| !own.is_empty())
                .map(|set| {
                    let mut props = Map::new();
                    for (property, default) in &own {
                        let values = set
                            .iter()
                            .find(|(known, _)| known == property)
                            .map(|(_, values)| values.clone())
                            .unwrap_or_default();
                        props.insert(
                            property.clone(),
                            json!({ "type": "variant", "values": values, "default": default }),
                        );
                    }
                    Value::Object(props)
                });
            RawComponent {
                id: f.id.clone(),
                name: if f.path.is_empty() {
                    f.name.clone()
                } else {
                    format!("{} / {}", f.path, f.name)
                },
                main_id: text_at(f.file, "mainInstanceId"),
                main_page: text_at(f.file, "mainInstancePage"),
                props,
            }
        })
        .collect()
}

/// Reads the `.penpot` export at `path` for an import: its boards, with the
/// component copies as instances, its components, and its design tokens.
pub(crate) fn run(path: &Path) -> Result<ReadLinks, ReadError> {
    let mut archive = open_archive(path)?;
    let objects = object_files(&archive);
    let mut reader = Reader {
        archive: &mut archive,
        shown: path.display().to_string(),
        buffer: Vec::new(),
    };
    let file = reader.first_file()?;
    let heads = reader.page_heads(&file.id)?;

    let mut notes = Vec::new();
    let root = format!("files/{}/", file.id);
    let tokens_file = format!("{root}tokens.json");
    let tokens_json = match reader.load(&tokens_file)? {
        None => None,
        Some(()) => match serde_json::from_slice::<Value>(&reader.buffer) {
            Ok(value) => Some(value),
            Err(error) => {
                notes.push(Note {
                    name: tokens_file.clone(),
                    property: None,
                    reason: format!("the design tokens could not be read: {error}"),
                });
                None
            }
        },
    };
    let colors = json_entries(&mut reader, &format!("{root}colors/"), &mut notes)?;
    let typographies = json_entries(&mut reader, &format!("{root}typographies/"), &mut notes)?;
    let files = json_entries(&mut reader, &format!("{root}components/"), &mut notes)?;
    let raw = raw_components(&files, &mut notes);
    let order: Vec<String> = raw.iter().map(|component| component.id.clone()).collect();

    let mut tokens = tokens::build(tokens_json.as_ref(), &colors, &typographies);
    tokens.notes.splice(0..0, notes);
    let linked = Linked {
        components: raw.into_iter().map(|c| (c.id.clone(), c)).collect(),
        done: HashMap::new(),
        progress: HashSet::new(),
        pages: heads
            .iter()
            .map(|head| (head.id.clone(), (head.dir.clone(), head.name.clone())))
            .collect(),
        links: tokens.links.clone(),
    };
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
        linked: Some(linked),
        page_name: String::new(),
        entry_pages: Vec::new(),
        refs: None,
        ids: HashMap::new(),
        pending: Vec::new(),
    };
    let mut boards = Vec::new();
    let mut ranges = Vec::new();
    for head in &heads {
        let first = boards.len();
        converter.page(head, &mut boards)?;
        ranges.push((first, boards.len()));
    }
    converter.resolve_interactions(&mut boards);
    let mut components = Vec::new();
    for id in &order {
        if let Some(comp) = converter.component(id)? {
            components.push(comp.json.clone());
        }
    }
    let pages_of = ranges
        .iter()
        .enumerate()
        .flat_map(|(index, (first, last))| std::iter::repeat_n(index, last - first));
    Ok(ReadLinks {
        boards: pages_of.zip(boards).collect(),
        pages: heads
            .iter()
            .map(|head| (head.id.clone(), head.name.clone()))
            .collect(),
        attachments: converter.attachments,
        entries: converter
            .entry_pages
            .into_iter()
            .zip(converter.entries)
            .collect(),
        components,
        tokens,
        ids: converter.next_id,
    })
}

/// Sets `field` of the override of `key`.
fn put(out: &mut Map<String, Value>, key: &str, field: &str, value: Value) {
    let slot = out.entry(key.to_string()).or_insert_with(|| json!({}));
    slot[field] = value;
}

/// What a group of changes Penpot lists in `touched` is, for a report.
fn describe(group: &str) -> String {
    let known = match group {
        "geometry-group" => "its position or size",
        "name-group" => "its name",
        "stroke-group" => "its strokes",
        "content-group" => "its vector content",
        "text-content-attribute" => "its text style",
        "text-font-group" => "its font",
        "layer-effects-group" | "shadow-group" => "its shadows or effects",
        "show-content" => "whether it clips",
        "constraints-group" => "its constraints",
        "fill-group" => "its fills",
        "visibility-group" => "its visibility",
        "text-content-text" => "its text",
        _ if group.starts_with("layout-") => "its layout",
        _ => return format!("`{group}`"),
    };
    known.to_string()
}

impl<R: Read + Seek> Converter<'_, R> {
    /// Whether the export defines the component that `pen` is a part of.
    pub(super) fn links_component(&self, pen: &Pen) -> bool {
        match (&self.linked, pen.component_id.0.as_deref()) {
            (Some(linked), Some(id)) => linked.components.contains_key(id),
            _ => false,
        }
    }

    /// The token that the shape applies under `key`, written as a reference.
    pub(super) fn token_link(&self, pen: &Pen, key: &str) -> Option<String> {
        let name = pen.applied_tokens.0.as_ref()?.get(key)?.as_str()?;
        self.linked.as_ref()?.links.get(name).cloned()
    }

    /// Makes the first fill a reference to the token the shape applies to its
    /// fill, when that fill is a solid color.
    pub(super) fn link_fills(&self, pen: &Pen, fills: &mut [Value]) {
        if let Some(link) = self.token_link(pen, "fill")
            && let Some(first) = fills.first_mut()
            && first.get("type") == Some(&json!("solid"))
        {
            first["color"] = json!(link);
        }
    }

    /// Makes the color of every stroke a reference to the token the shape
    /// applies to its stroke color.
    pub(super) fn link_strokes(&self, pen: &Pen, strokes: &mut [Value]) {
        if let Some(link) = self.token_link(pen, "strokeColor") {
            for stroke in strokes {
                stroke["color"] = json!(link);
            }
        }
    }

    /// The component `id` of the export as a node, converted from its main
    /// instance the first time. None if it is not defined, cannot be
    /// converted, or is the component being converted.
    fn component(&mut self, id: &str) -> Result<Option<Rc<Comp>>, ReadError> {
        let Some(linked) = self.linked.as_mut() else {
            return Ok(None);
        };
        if let Some(done) = linked.done.get(id) {
            return Ok(done.clone());
        }
        if linked.progress.contains(id) {
            return Ok(None);
        }
        let Some(raw) = linked.components.get(id).cloned() else {
            return Ok(None);
        };
        linked.progress.insert(id.to_string());
        let converted = self.convert_component(&raw);
        let linked = self.linked.as_mut().expect("this is an import");
        linked.progress.remove(id);
        let comp = converted?.map(Rc::new);
        linked.done.insert(id.to_string(), comp.clone());
        Ok(comp)
    }

    /// Lists a component that is not imported.
    fn skip_component(&mut self, raw: &RawComponent, reason: &str) {
        let board = Board {
            id: String::new(),
            name: String::new(),
        };
        let who = Who {
            board: &board,
            id: "",
            name: &raw.name,
        };
        self.report(
            &who,
            None,
            format!("the component is not imported: {reason}"),
        );
    }

    fn convert_component(&mut self, raw: &RawComponent) -> Result<Option<Comp>, ReadError> {
        let Some((dir, page)) = self
            .linked
            .as_ref()
            .and_then(|linked| linked.pages.get(&raw.main_page))
            .cloned()
        else {
            self.skip_component(
                raw,
                "its main instance is on a page the export does not have",
            );
            return Ok(None);
        };
        // The component's shapes are not the shapes of the board being
        // converted.
        let shapes = std::mem::take(&mut self.shapes);
        let left_out = std::mem::take(&mut self.left_out);
        let refs = self.refs.replace(HashMap::new());
        let outer = std::mem::replace(&mut self.page_name, page);
        let body = self.component_body(raw, &dir);
        self.page_name = outer;
        self.refs = refs;
        self.left_out = left_out;
        self.shapes = shapes;
        body
    }

    /// The board a shape is in: the top-level frame above it, or the shape
    /// itself if it is one.
    fn top_board(&mut self, dir: &str, id: &str, pen: &Pen) -> Result<Board, ReadError> {
        let mut board = Board {
            id: id.to_string(),
            name: pen.name.0.clone().unwrap_or_default(),
        };
        let mut parent = pen.parent_id.0.clone();
        // A page is only so deep.
        for _ in 0..64 {
            let Some(next) = parent.filter(|parent| parent != ROOT_ID) else {
                break;
            };
            if self.reader.load(&format!("{dir}/{next}.json"))?.is_none() {
                break;
            }
            let Ok(above) = serde_json::from_slice::<Pen>(&self.reader.buffer) else {
                break;
            };
            board = Board {
                id: next,
                name: above.name.0.clone().unwrap_or_default(),
            };
            parent = above.parent_id.0.clone();
        }
        Ok(board)
    }

    fn component_body(&mut self, raw: &RawComponent, dir: &str) -> Result<Option<Comp>, ReadError> {
        let mut seen = HashSet::from([ROOT_ID.to_string()]);
        let pen = match self.load(dir, &raw.main_id, &mut seen)? {
            Loaded::Shape(pen) => pen,
            Loaded::Skipped(reason) => {
                self.skip_component(raw, &format!("its main instance: {reason}"));
                return Ok(None);
            }
        };
        if pen.kind.0.as_deref() != Some("frame") {
            self.skip_component(raw, "its main instance is not a board or frame");
            return Ok(None);
        }
        let board = self.top_board(dir, &raw.main_id, &pen)?;
        let found = bounds(&pen);
        let common = Common {
            id: raw.main_id.clone(),
            name: raw.name.clone(),
            bounds: found.unwrap_or(Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            }),
            boxed: found.is_some(),
            hidden: false,
        };
        let who = Who {
            board: &board,
            id: &common.id,
            name: &common.name,
        };
        self.checks(&who, &pen, "frame");
        // The component's box is its own: it sits nowhere.
        let parent = Parent {
            flow: Flow::None,
            x: 0.0,
            y: 0.0,
            page: true,
            cells: Vec::new(),
            align_items: None,
            justify_items: None,
        };
        let mut value = self.frame(dir, &common, &pen, "frame", &parent, &board, &mut seen)?;
        let node = value.as_object_mut().expect("a frame is an object");
        for key in [
            "x",
            "y",
            "absolute",
            "margin",
            "gridColumn",
            "gridRow",
            "alignSelf",
            "justifySelf",
        ] {
            node.remove(key);
        }
        node.insert("type".into(), json!("component"));
        node.insert("name".into(), json!(raw.name));
        if let Some(props) = &raw.props {
            node.insert("props".into(), props.clone());
        }
        let node_id = node
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(Some(Comp {
            node_id,
            bounds: common.bounds,
            nodes: self.refs.take().unwrap_or_default(),
            json: value,
        }))
    }

    /// A copy of a component as an instance of it, with the overrides that
    /// the designer made. None if the component cannot be had, and then the
    /// copy is converted as the plain shapes it holds.
    pub(super) fn instance(
        &mut self,
        dir: &str,
        common: &Common,
        pen: &Pen,
        parent: &Parent,
        board: &Board,
        seen: &mut HashSet<String>,
    ) -> Result<Option<Value>, ReadError> {
        let Some(id) = pen.component_id.0.as_deref().filter(|id| {
            self.linked
                .as_ref()
                .is_some_and(|linked| linked.components.contains_key(*id))
        }) else {
            return Ok(None);
        };
        let Some(comp) = self.component(id)? else {
            let who = Who {
                board,
                id: &common.id,
                name: &common.name,
            };
            self.report(
                &who,
                Some("componentId"),
                "the component of this copy cannot be imported, so the copy is drawn as the plain shapes it holds",
            );
            return Ok(None);
        };
        let mut node = self.base("instance", common, pen, parent);
        let by_item = Self::sized_by_item(parent, pen);
        for (axis, item, own, theirs) in [
            (
                "width",
                &pen.layout_item_h_sizing.0,
                common.bounds.width,
                comp.bounds.width,
            ),
            (
                "height",
                &pen.layout_item_v_sizing.0,
                common.bounds.height,
                comp.bounds.height,
            ),
        ] {
            let item = if by_item { item.as_deref() } else { None };
            match item {
                Some("fill") if !parent.page => {
                    node.insert(axis.into(), json!("fill"));
                }
                Some("auto") => {
                    node.insert(axis.into(), json!("hug"));
                }
                _ if common.boxed && (own - theirs).abs() > 0.5 => {
                    node.insert(axis.into(), json!({ "fixed": n(own.max(0.0)) }));
                }
                _ => {}
            }
        }
        node.insert("component".into(), json!(comp.node_id));
        let overrides = self.overrides(dir, pen, &comp, board, seen)?;
        if !overrides.is_empty() {
            node.insert("overrides".into(), Value::Object(overrides));
        }
        Ok(Some(Value::Object(node)))
    }

    /// What the designer changed in a copy: the changes of its shapes that an
    /// instance can hold, by the node of the component each shape copies.
    fn overrides(
        &mut self,
        dir: &str,
        root: &Pen,
        comp: &Comp,
        board: &Board,
        seen: &mut HashSet<String>,
    ) -> Result<Map<String, Value>, ReadError> {
        let mut out = Map::new();
        let children = |pen: &Pen| -> Vec<String> {
            pen.shapes
                .0
                .as_deref()
                .unwrap_or_default()
                .iter()
                .rev()
                .cloned()
                .collect()
        };
        let mut pending = Vec::new();
        if self.scan(root, true, comp, board, &mut out)? {
            pending.extend(children(root));
        }
        while let Some(id) = pending.pop() {
            match self.load(dir, &id, seen)? {
                Loaded::Skipped(reason) => self.skip(board, &id, "", reason),
                Loaded::Shape(pen) => {
                    if self.scan(&pen, false, comp, board, &mut out)? {
                        pending.extend(children(&pen));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Reads the changes one shape of a copy lists in `touched`. Returns
    /// whether the shapes inside it are looked at too.
    fn scan(
        &mut self,
        pen: &Pen,
        is_root: bool,
        comp: &Comp,
        board: &Board,
        out: &mut Map<String, Value>,
    ) -> Result<bool, ReadError> {
        let id = pen.id.0.clone().unwrap_or_default();
        let name = pen.name.0.clone().unwrap_or_default();
        let who = Who {
            board,
            id: &id,
            name: &name,
        };
        let target: Option<(String, String)> = if is_root {
            Some((comp.node_id.clone(), "board".to_string()))
        } else {
            pen.shape_ref
                .0
                .as_ref()
                .and_then(|reference| comp.nodes.get(reference))
                .cloned()
        };
        let mut left: Vec<String> = Vec::new();
        let mut descend = true;
        for group in pen.touched.0.as_deref().unwrap_or_default() {
            if let Some(slot) = group.strip_prefix("swap-slot-") {
                let swapped = match (comp.nodes.get(slot), pen.component_id.0.as_deref()) {
                    (Some((key, kind)), Some(component)) if kind.as_str() == "instance" => {
                        self.component(component)?.map(|inner| (key, inner))
                    }
                    _ => None,
                };
                match swapped {
                    Some((key, inner)) => {
                        put(out, key, "component", json!(inner.node_id));
                        // What is inside belongs to the other component.
                        descend = false;
                    }
                    None => left.push("the component swapped in".to_string()),
                }
                continue;
            }
            match (group.as_str(), &target) {
                ("text-content-text", Some((key, kind))) if kind.as_str() == "text" => {
                    match text_of(pen) {
                        Some(text) => put(out, key, "content", json!(text)),
                        None => left.push(describe(group)),
                    }
                }
                ("fill-group", Some((key, kind)))
                    if matches!(kind.as_str(), "board" | "frame" | "vector") =>
                {
                    let found = bounds(pen).unwrap_or(Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    });
                    let mut fills = self.fills(&who, pen, found);
                    self.link_fills(pen, &mut fills);
                    put(out, key, "fills", Value::Array(fills));
                }
                ("visibility-group", Some((key, _))) if !is_root => {
                    put(out, key, "visible", json!(!pen.hidden.0.unwrap_or(false)));
                }
                // The copy's own place, size, name and visibility are the
                // instance's.
                ("visibility-group" | "geometry-group" | "name-group", _) if is_root => {}
                (other, _) if is_root && other.starts_with("layout-item-") => {}
                (other, _) => {
                    let what = describe(other);
                    if !left.contains(&what) {
                        left.push(what);
                    }
                }
            }
        }
        if !left.is_empty() {
            self.report(
                &who,
                Some("overrides"),
                format!(
                    "the copy changes {} of this layer; an instance holds only text, fills, visibility and swapped components, so the change is not imported",
                    left.join(", ")
                ),
            );
        }
        Ok(descend)
    }
}
