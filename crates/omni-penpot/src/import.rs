//! Imports a Penpot export into OmniState (US-051): its pages and boards become
//! one design doc, its components, design tokens, colors and typographies the
//! project's design system doc, and its images attachments. It reuses the
//! reader and the converter, and lists what it cannot import in its report.
//!
//! Copies of components become instances (with the text, fills, visibility and
//! swapped-component overrides the designer made), and the shapes of the main
//! instances stay on their pages as the plain shapes they are. The rules of
//! the converter (docs/formats/penpot-export.md) apply to every shape.

pub(crate) mod tokens;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use omni_core::design::{Node, tags};
use serde_json::{Map, Value, json};

use crate::convert::{self, node_id};
use crate::{Attachment, ConvertedBoard, ReadError, ReportEntry};

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
    /// The mode of the theme that is active in Penpot, None if the export has
    /// no theme.
    pub mode: Option<String>,
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

/// An import with what the comparison with Penpot's PNGs needs.
#[cfg_attr(not(feature = "compare"), allow(dead_code))]
pub(crate) struct Built {
    pub(crate) import: Import,
    /// The converted boards, each a page that holds the board alone.
    pub(crate) boards: Vec<ConvertedBoard>,
    /// The entries of the converter, for the shapes it left out whole.
    pub(crate) entries: Vec<ReportEntry>,
}

/// Reads the `.penpot` export at `path` and imports it.
pub fn import_file(path: &Path) -> Result<Import, ReadError> {
    build(path).map(|built| built.import)
}

pub(crate) fn build(path: &Path) -> Result<Built, ReadError> {
    let read = convert::import_run(path)?;

    // One page per Penpot page, holding the boards the converter made.
    let mut next = read.ids;
    let mut pages: Vec<Value> = read
        .pages
        .iter()
        .map(|(_, name)| {
            next += 1;
            json!({ "id": node_id(next), "type": "page", "name": name, "children": [] })
        })
        .collect();
    let mut boards = Vec::with_capacity(read.boards.len());
    for (index, board) in read.boards {
        if let Some(node) = board.design.children().first()
            && let Ok(value) = serde_json::to_value(node)
            && let Some(children) = pages[index]["children"].as_array_mut()
        {
            children.push(value);
        }
        boards.push(board);
    }
    let pages = pages
        .into_iter()
        .map(|page| {
            let name = page["name"].as_str().unwrap_or_default().to_string();
            serde_json::from_value::<Node>(page).map_err(|error| {
                ReadError(format!(
                    "the page {name:?} does not pass validation: {error}"
                ))
            })
        })
        .collect::<Result<Vec<Node>, ReadError>>()?;

    let mut design_system = read.tokens.tree.clone();
    for component in &read.components {
        if let Err(error) = serde_json::from_value::<Node>(component.clone()) {
            return Err(ReadError(format!(
                "the component {:?} does not pass validation: {error}",
                component["name"].as_str().unwrap_or_default()
            )));
        }
    }
    if let Some(root) = design_system.as_object_mut() {
        root.insert("$components".into(), Value::Array(read.components));
    }

    let mut report = Vec::new();
    let mut seen = HashSet::new();
    for (page, entry) in &read.entries {
        // A shape is listed once for a reason, though a component and the
        // page it is drawn on both convert it.
        let key = (
            entry.shape_id.clone(),
            entry.property.clone(),
            entry.reason.clone(),
        );
        if !entry.shape_id.is_empty() && !seen.insert(key) {
            continue;
        }
        report.push(ImportEntry {
            page: page.clone(),
            board: entry.board.clone(),
            board_id: entry.board_id.clone(),
            shape_id: entry.shape_id.clone(),
            name: entry.name.clone(),
            property: entry.property.clone(),
            reason: entry.reason.clone(),
        });
    }
    for note in &read.tokens.notes {
        report.push(ImportEntry {
            page: String::new(),
            board: String::new(),
            board_id: String::new(),
            shape_id: String::new(),
            name: note.name.clone(),
            property: note.property.clone(),
            reason: note.reason.clone(),
        });
    }

    Ok(Built {
        import: Import {
            pages,
            design_system,
            attachments: read.attachments,
            report,
            mode: read.tokens.active,
        },
        boards,
        entries: read.entries.into_iter().map(|(_, entry)| entry).collect(),
    })
}

// Without a design system -----------------------------------------------------------------

/// The keys of a node that hold no token reference.
const NOT_TOKENS: &[&str] = &[
    "id",
    "type",
    "name",
    "content",
    "path",
    "image",
    "component",
    "props",
    "overrides",
    "bind",
];

/// A design that has no design system to refer to: the instances of the pages
/// become the frames they stand for, and the token references the colors they
/// have in the active mode.
struct Flat<'a> {
    components: HashMap<&'a str, &'a Value>,
    tree: &'a Value,
    mode: Option<&'a str>,
    next: u64,
}

impl Flat<'_> {
    /// The value of the color token `path` in the active mode.
    fn color(&self, path: &str) -> Option<String> {
        let mut at = self.tree;
        for part in path.split('.') {
            at = at.get(part)?;
        }
        let per_mode = self
            .mode
            .and_then(|mode| at["$extensions"]["omnistate"]["modes"].get(mode));
        per_mode
            .or_else(|| at.get("$value"))?
            .as_str()
            .filter(|text| text.starts_with('#'))
            .map(str::to_string)
    }

    fn id(&mut self) -> String {
        self.next += 1;
        node_id(self.next)
    }

    /// Writes the colors in place of the token references below `node`, and
    /// the frames in place of the instances.
    fn flatten(&mut self, node: &mut Value, depth: usize) {
        match node {
            Value::String(text) => {
                if let Some(color) = text.strip_prefix('$').and_then(|path| self.color(path)) {
                    *text = color;
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.flatten(item, depth)),
            Value::Object(object) => {
                if object.get("type").and_then(Value::as_str) == Some("instance") {
                    *node = self.frame(node, depth);
                    return;
                }
                for (key, value) in object.iter_mut() {
                    if key == "children" || !NOT_TOKENS.contains(&key.as_str()) {
                        self.flatten(value, depth);
                    }
                }
            }
            _ => {}
        }
    }

    /// The frame that `instance` stands for: its component's layers with the
    /// overrides applied, in the place and the size the instance gives.
    fn frame(&mut self, instance: &Value, depth: usize) -> Value {
        let component = instance["component"]
            .as_str()
            .and_then(|id| self.components.get(id).copied());
        let (Some(component), true) = (component, depth < 24) else {
            return json!({
                "id": self.id(), "type": "frame", "name": instance["name"],
                "width": { "fixed": 0 }, "height": { "fixed": 0 },
                "layout": { "mode": "none" }, "children": []
            });
        };
        let mut frame = component.clone();
        if let Some(overrides) = instance["overrides"].as_object() {
            override_in(&mut frame, overrides);
        }
        self.renumber(&mut frame);
        let object = frame.as_object_mut().expect("a component is an object");
        object.insert("type".into(), json!("frame"));
        object.remove("props");
        for key in [
            "name",
            "visible",
            "x",
            "y",
            "absolute",
            "margin",
            "gridColumn",
            "gridRow",
            "alignSelf",
            "justifySelf",
            "width",
            "height",
        ] {
            if let Some(value) = instance.get(key) {
                object.insert(key.to_string(), value.clone());
            }
        }
        // The layers, which may hold instances of their own.
        if let Some(Value::Array(children)) = object.get_mut("children") {
            for child in children {
                self.flatten_layer(child, depth + 1);
            }
        }
        // The frame's own style.
        for (key, value) in object.iter_mut() {
            if key != "children" && !NOT_TOKENS.contains(&key.as_str()) {
                self.flatten(value, depth);
            }
        }
        frame
    }

    /// A layer of a component: no `bind`, and any instance in it expanded.
    fn flatten_layer(&mut self, layer: &mut Value, depth: usize) {
        if let Some(object) = layer.as_object_mut() {
            object.remove("bind");
        }
        if layer["type"] == "instance" {
            *layer = self.frame(layer, depth);
            return;
        }
        if let Some(object) = layer.as_object_mut() {
            for (key, value) in object.iter_mut() {
                if key == "children" {
                    if let Value::Array(children) = value {
                        for child in children {
                            self.flatten_layer(child, depth);
                        }
                    }
                } else if !NOT_TOKENS.contains(&key.as_str()) {
                    self.flatten(value, depth);
                }
            }
        }
    }

    /// Gives every node of the tree an ID of its own.
    fn renumber(&mut self, node: &mut Value) {
        if let Some(object) = node.as_object_mut() {
            let id = self.id();
            object.insert("id".into(), json!(id));
            if let Some(Value::Array(children)) = object.get_mut("children") {
                for child in children {
                    self.renumber(child);
                }
            }
        }
    }
}

/// Applies the overrides of an instance, keyed by the old IDs of the nodes.
fn override_in(node: &mut Value, overrides: &Map<String, Value>) {
    let Some(object) = node.as_object_mut() else {
        return;
    };
    let change = object
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| overrides.get(id))
        .and_then(Value::as_object)
        .cloned();
    for (field, value) in change.iter().flatten() {
        match field.as_str() {
            "content" | "component" => {
                object.insert(field.clone(), value.clone());
            }
            "fills" => {
                object.insert("fills".into(), value.clone());
            }
            "visible" => {
                object.insert("visible".into(), value.clone());
            }
            _ => {}
        }
    }
    if let Some(Value::Array(children)) = object.get_mut("children") {
        for child in children {
            override_in(child, overrides);
        }
    }
}

impl Import {
    /// The import for a project that cannot take its design system: the
    /// design system is left out, the instances of the pages are the frames
    /// they stand for, and the token references the colors they have in
    /// Penpot's active mode. The report says why, with `reason`.
    pub fn without_design_system(&self, reason: &str) -> Result<Import, ReadError> {
        let components: Vec<&Value> = self.design_system["$components"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        let mut flat = Flat {
            components: components
                .iter()
                .filter_map(|component| Some((component["id"].as_str()?, *component)))
                .collect(),
            tree: &self.design_system,
            mode: self.mode.as_deref(),
            next: 1 << 32,
        };
        let mut pages = Vec::with_capacity(self.pages.len());
        for page in &self.pages {
            let mut value = serde_json::to_value(page)
                .map_err(|error| ReadError(format!("a page cannot be written: {error}")))?;
            flat.flatten(&mut value, 0);
            pages.push(serde_json::from_value::<Node>(value).map_err(|error| {
                ReadError(format!(
                    "the page {:?} does not pass validation without its design system: {error}",
                    page.name()
                ))
            })?);
        }
        let mut report = self.report.clone();
        report.push(ImportEntry {
            page: String::new(),
            board: String::new(),
            board_id: String::new(),
            shape_id: String::new(),
            name: "design system".to_string(),
            property: None,
            reason: reason.to_string(),
        });
        let mode = self.mode.clone();
        let tokens = json!({ "$extensions": { "omnistate": { "modes": [mode.clone().unwrap_or_else(|| "default".to_string())] } } });
        Ok(Import {
            pages,
            design_system: tokens,
            attachments: self.attachments.clone(),
            report,
            mode,
        })
    }
}

// Sending an import ---------------------------------------------------------------------

/// What the server named an import's parts, as it was sent: node IDs are
/// assigned by the server, and the instances, the overrides and the images of
/// an import have to name them. A node ID the server has not yet handed out
/// stands for itself and is left out of the tags, so the server gives it one.
#[derive(Debug, Default, Clone)]
pub struct Ids {
    nodes: HashMap<String, String>,
    attachments: HashMap<String, String>,
}

impl Ids {
    /// Says that the attachment `old`, as the import names it, is `new` on the
    /// server.
    pub fn attachment(&mut self, old: &str, new: &str) {
        self.attachments.insert(old.to_string(), new.to_string());
    }
}

/// Components or pages as tags, ready to be sent.
pub struct Rendered {
    pub tags: String,
    /// Whether the tags hold everything: no instance or override had to wait
    /// for an ID that the server has not handed out. When they do not, send
    /// them, [`Import::learn_components`] and render again.
    pub complete: bool,
    sent: Vec<Value>,
    unknown: HashSet<String>,
}

impl Import {
    /// The token tree of the design system, to put into the draft with `PUT
    /// .../tokens`.
    pub fn token_tree(&self) -> Value {
        let mut tree = self.design_system.clone();
        if let Some(root) = tree.as_object_mut() {
            root.remove("$components");
        }
        tree
    }

    /// The token tree as JSON text, the `tokens` of the request.
    pub fn token_tree_json(&self) -> String {
        self.token_tree().to_string()
    }

    /// Whether the design system has tokens at all.
    pub fn has_tokens(&self) -> bool {
        self.design_system
            .as_object()
            .is_some_and(|root| root.keys().any(|key| !key.starts_with('$')))
    }

    /// How many components the design system has.
    pub fn component_count(&self) -> usize {
        self.design_system["$components"]
            .as_array()
            .map_or(0, Vec::len)
    }

    /// The components as `<Component>` tags. The first time, the instances
    /// inside the components wait for the IDs of the components they place;
    /// see [`Rendered::complete`].
    pub fn component_tags(&self, ids: &Ids) -> Result<Rendered, ReadError> {
        let mut unknown = HashSet::new();
        let mut complete = true;
        let mut sent = Vec::new();
        for component in self.design_system["$components"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(node) = render(component, ids, &mut unknown, &mut complete) {
                sent.push(node);
            }
        }
        let tags = print(&sent, &unknown)?;
        Ok(Rendered {
            tags,
            complete,
            sent,
            unknown,
        })
    }

    /// Learns the IDs the server gave the new nodes of `rendered`, from its
    /// answer to the import of the components (the JSON text of the response,
    /// whose `components` are the ones it stored).
    pub fn learn_components(
        &self,
        ids: &mut Ids,
        rendered: &Rendered,
        answer: &str,
    ) -> Result<(), ReadError> {
        let answer: Value = serde_json::from_str(answer).map_err(|error| {
            ReadError(format!(
                "the server's answer about the components is not readable: {error}"
            ))
        })?;
        let stored = answer["components"].as_array().ok_or_else(|| {
            ReadError("the server's answer has no components; is it an OmniState API?".to_string())
        })?;
        for (sent, stored) in rendered.sent.iter().zip(stored) {
            learn(sent, stored, &rendered.unknown, ids);
        }
        Ok(())
    }

    /// The pages as `<Page>` tags, without IDs: the server gives every node
    /// one. `ids` has to know the components.
    pub fn design_tags(&self, ids: &Ids) -> Result<String, ReadError> {
        let mut unknown = HashSet::new();
        let mut complete = true;
        let mut sent = Vec::new();
        for page in &self.pages {
            let value = serde_json::to_value(page)
                .map_err(|error| ReadError(format!("a page cannot be written: {error}")))?;
            if let Some(node) = render(&value, ids, &mut unknown, &mut complete) {
                sent.push(node);
            }
        }
        // Every node of a page is new.
        let all: HashSet<String> = unknown;
        print(&sent, &all)
    }
}

/// `node` with the server's IDs: its own where the server gave one, the
/// import's where not (they are left out of the tags). An instance whose
/// component the server has not named is left out, and an override of a node
/// it has not named too. Images name the server's attachments.
fn render(
    node: &Value,
    ids: &Ids,
    unknown: &mut HashSet<String>,
    complete: &mut bool,
) -> Option<Value> {
    let mut node = node.clone();
    let object = node.as_object_mut()?;
    if let Some(old) = object.get("id").and_then(Value::as_str).map(str::to_string) {
        match ids.nodes.get(&old) {
            Some(new) => {
                object.insert("id".into(), json!(new));
            }
            None => {
                unknown.insert(old);
            }
        }
    }
    attachments_of(object, ids);
    if object.get("type").and_then(Value::as_str) == Some("instance") {
        let component = object.get("component").and_then(Value::as_str)?;
        match ids.nodes.get(component) {
            Some(new) => {
                object.insert("component".into(), json!(new));
            }
            None => {
                *complete = false;
                return None;
            }
        }
        if let Some(overrides) = object.remove("overrides") {
            let mut kept = Map::new();
            for (key, change) in overrides.as_object().into_iter().flatten() {
                let Some(new) = ids.nodes.get(key) else {
                    *complete = false;
                    continue;
                };
                let mut change = change.clone();
                attachments_of_value(&mut change, ids);
                if let Some(fields) = change.as_object_mut()
                    && let Some(swapped) = fields.get("component").and_then(Value::as_str)
                {
                    match ids.nodes.get(swapped) {
                        Some(swapped) => {
                            fields.insert("component".into(), json!(swapped));
                        }
                        None => {
                            *complete = false;
                            fields.remove("component");
                        }
                    }
                }
                if change.as_object().is_some_and(|fields| !fields.is_empty()) {
                    kept.insert(new.clone(), change);
                }
            }
            if !kept.is_empty() {
                object.insert("overrides".into(), Value::Object(kept));
            }
        }
    }
    if let Some(children) = object.get("children").and_then(Value::as_array) {
        let kept: Vec<Value> = children
            .iter()
            .filter_map(|child| render(child, ids, unknown, complete))
            .collect();
        object.insert("children".into(), Value::Array(kept));
    }
    Some(node)
}

/// Renames the attachments of the fills and the `image` of a node.
fn attachments_of(object: &mut Map<String, Value>, ids: &Ids) {
    if let Some(Value::String(image)) = object.get_mut("image") {
        rename(image, ids);
    }
    if let Some(fills) = object.get_mut("fills") {
        attachments_of_value(fills, ids);
    }
}

fn attachments_of_value(value: &mut Value, ids: &Ids) {
    match value {
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| attachments_of_value(item, ids)),
        Value::Object(object) => {
            if let Some(Value::String(image)) = object.get_mut("image") {
                rename(image, ids);
            }
            if let Some(fills) = object.get_mut("fills") {
                attachments_of_value(fills, ids);
            }
        }
        _ => {}
    }
}

fn rename(image: &mut String, ids: &Ids) {
    if let Some(new) = image
        .strip_prefix("attachment:")
        .and_then(|old| ids.attachments.get(old))
    {
        *image = format!("attachment:{new}");
    }
}

/// Learns the IDs of the nodes of `sent` that had none from the same nodes
/// as `stored`, in the same order.
fn learn(sent: &Value, stored: &Value, unknown: &HashSet<String>, ids: &mut Ids) {
    if let (Some(old), Some(new)) = (sent["id"].as_str(), stored["id"].as_str())
        && unknown.contains(old)
    {
        ids.nodes.insert(old.to_string(), new.to_string());
    }
    let none = Vec::new();
    let sent_children = sent["children"].as_array().unwrap_or(&none);
    let stored_children = stored["children"].as_array().unwrap_or(&none);
    for (sent, stored) in sent_children.iter().zip(stored_children) {
        learn(sent, stored, unknown, ids);
    }
}

/// The tags of `nodes`, one tree after the other, without the `id` of the
/// nodes in `unknown`.
fn print(nodes: &[Value], unknown: &HashSet<String>) -> Result<String, ReadError> {
    let mut text = Vec::with_capacity(nodes.len());
    for value in nodes {
        let node = serde_json::from_value::<Node>(value.clone()).map_err(|error| {
            ReadError(format!(
                "{:?} cannot be sent: {error}",
                value["name"].as_str().unwrap_or_default()
            ))
        })?;
        text.push(without_ids(&tags::print(&node), unknown));
    }
    Ok(text.join("\n"))
}

/// The tags with the `id` of the nodes in `unknown` taken out. The printer
/// writes the `id` first, right after the tag name.
fn without_ids(tags: &str, unknown: &HashSet<String>) -> String {
    let mut out = String::with_capacity(tags.len());
    for line in tags.lines() {
        let indent = line.len() - line.trim_start().len();
        let rest = &line[indent..];
        let named = rest.strip_prefix('<').and_then(|after| {
            let end = after.find(|c: char| !c.is_ascii_alphabetic())?;
            let attribute = after[end..].strip_prefix(" id=\"")?;
            let id = attribute.get(..8)?;
            attribute[8..].starts_with('"').then_some((end, id))
        });
        match named {
            Some((end, id)) if unknown.contains(id) => {
                out.push_str(&line[..indent + 1 + end]);
                out.push_str(&rest[1 + end + " id=\"".len() + 8 + 1..]);
            }
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    out.pop();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ids_of_new_nodes_are_taken_out_of_the_tags() {
        let tags =
            "<Frame id=\"00000001\" name=\"A\">\n  <Text id=\"00000002\" name=\"B\"/>\n</Frame>";
        let unknown: HashSet<String> = ["00000002".to_string()].into();
        assert_eq!(
            without_ids(tags, &unknown),
            "<Frame id=\"00000001\" name=\"A\">\n  <Text name=\"B\"/>\n</Frame>"
        );
    }
}
