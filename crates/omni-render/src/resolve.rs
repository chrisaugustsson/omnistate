//! Resolving a page against a design system (US-032): every instance becomes
//! a frame holding its component's layers, and every token reference becomes
//! the token's value in one mode. What comes out is an ordinary page, which
//! layout and painting handle like any other, and the places where something
//! was missing.
//!
//! Instances expand like this:
//!
//! - The component that the instance names is replaced by the variant of its
//!   set that shows the instance's variant values (`DesignSystem::variant`).
//! - The instance becomes a `frame` with the instance's ID, placement and
//!   sizes, and with the component's size (on each axis the instance leaves
//!   out), layout and style. Its children are the component's layers.
//! - Each layer takes the component's property values, the instance's where it
//!   sets them and the defaults where it does not (`bind`), and then the
//!   instance's overrides, which win. A layer that is an instance expands the
//!   same way, with the overrides of the outer instance applying to the nodes
//!   of the inner component too.
//! - A node inside an instance gets an ID of its own, because several
//!   instances hold the same component node. `Resolved::addresses` says which
//!   address (`<instanceId>/<componentNodeId>`) each one stands for. A node
//!   inside an instance inside an instance is addressed from the instance of
//!   the page.
//!
//! An instance of a component that the design system does not have, a node
//! with a token reference the design system does not have, and a node with a
//! token whose value does not fit where it is used (a color for a padding) are
//! drawn as an error placeholder, a tinted box with a magenta edge, and listed
//! in `Resolved::errors`. The rest of the board resolves as usual.

mod details;

use crate::inspect::{InstanceInspection, OverrideInspection, PropValue};
use crate::nesting::MAX_NESTING;
use crate::system::{ComponentDef, DesignSystem, Missing, RenderError};
use omni_core::design::tokens::ModeChoice;
use omni_core::design::{Node, NodeAddress, NodeId, PropDef, TokenRef};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub(crate) use details::{Details, Facts, UsedToken};

/// How many instances may be inside one another before an instance is taken
/// for a component that holds itself and drawn as a placeholder.
const MAX_DEPTH: usize = 24;

/// The color of the error placeholder's edge, and of the text of a node that
/// is missing a token.
const ERROR_COLOR: &str = "#ff00ff";
/// The tint inside an error placeholder.
const ERROR_TINT: &str = "#ff00ff66";
/// The size of the placeholder of a missing component on an axis the instance
/// leaves to its component.
const ERROR_SIZE: f64 = 24.0;

/// The keys of a node that hold no token reference: text and names, which may
/// well start with `$`, and the parts that are resolved on their own.
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
    "children",
];

const PLACEMENT: &[&str] = &[
    "absolute",
    "x",
    "y",
    "gridColumn",
    "gridRow",
    "margin",
    "justifySelf",
    "alignSelf",
];

const SIZES: &[&str] = &[
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
];

/// A page with its instances expanded and its tokens resolved.
pub(crate) struct Resolved {
    pub(crate) page: Node,
    /// The address that each node inside an instance stands for, by the ID
    /// the node has in `page`. Other nodes keep their IDs.
    pub(crate) addresses: HashMap<NodeId, NodeAddress>,
    /// The nodes of `page` that were instances, by their IDs in `page`, where
    /// they are frames.
    pub(crate) instances: HashSet<NodeId>,
    /// What the inspect panel shows of the nodes, by their IDs in `page`.
    pub(crate) details: Details,
    pub(crate) errors: Vec<RenderError>,
    /// The IDs of `page` and the IDs made up for it, and how many were made
    /// up, so that boards resolved later (`resolve_board`) make up others.
    pub(crate) taken: HashSet<NodeId>,
    pub(crate) next: u64,
}

/// A board with its instances expanded and its tokens resolved.
pub(crate) struct ResolvedBoard {
    pub(crate) board: Node,
    pub(crate) addresses: HashMap<NodeId, NodeAddress>,
    pub(crate) instances: HashSet<NodeId>,
    pub(crate) details: Details,
    pub(crate) errors: Vec<RenderError>,
}

/// A property value of an instance.
#[derive(Clone)]
enum Prop {
    Text(String),
    Flag(bool),
}

/// What the layers of a component inside an instance take.
struct Inside {
    /// The instance of the page that the layers are inside.
    instance: NodeId,
    props: HashMap<String, Prop>,
    /// The overrides in force, by the ID of the node they change.
    overrides: Map<String, Value>,
    depth: usize,
}

/// Expands the instances of `page` and resolves its token references against
/// `system` in `choice`.
pub(crate) fn resolve(page: &Node, system: &DesignSystem, choice: &ModeChoice) -> Resolved {
    let source = serde_json::to_value(page).unwrap_or(Value::Null);
    let mut resolver = Resolver {
        system,
        choice: choice.clone(),
        used: ids_of(page),
        next: 0,
        addresses: HashMap::new(),
        instances: HashSet::new(),
        details: Details::default(),
        errors: Vec::new(),
    };
    let resolved = resolver.node(source.clone(), None, 0);
    let page = match serde_json::from_value::<Node>(resolved.clone()) {
        Ok(page) => page,
        // Each node's tokens are checked as they are resolved, so this is a
        // last resort: a board that still does not read is drawn unresolved
        // rather than leaving the page unread.
        Err(_) => keep_readable(&source, resolved).unwrap_or_else(|| page.clone()),
    };
    Resolved {
        page,
        addresses: resolver.addresses,
        instances: resolver.instances,
        details: resolver.details,
        errors: resolver.errors,
        taken: resolver.used,
        next: resolver.next,
    }
}

/// A board being resolved a few nodes at a time (`begin_board`, then
/// `BoardResolving::step` until it gives the board), so that resolving a big
/// board does not hold the page for as long as the whole board takes.
pub(crate) struct BoardResolving {
    board: Node,
    /// The board as JSON, until `step` opens it.
    root: Option<Value>,
    /// The containers being resolved, outermost first.
    open: Vec<Partial>,
    finished: Option<Value>,
    used: HashSet<NodeId>,
    next: u64,
    addresses: HashMap<NodeId, NodeAddress>,
    instances: HashSet<NodeId>,
    details: Details,
    errors: Vec<RenderError>,
}

/// A container of a board whose children are being resolved one by one.
struct Partial {
    /// The container with its token references resolved and no children yet.
    shell: Value,
    rest: std::vec::IntoIter<Value>,
    done: Vec<Value>,
    /// How many levels below the page the container is.
    level: usize,
    /// The modes the container's children are resolved in.
    choice: ModeChoice,
}

/// Starts resolving `board`, a board of a page that `resolve` resolved before,
/// as `resolve` does for the whole page. `taken` and `next` are the IDs the
/// page has and how many were made up (`Resolved::taken`); they go into the
/// resolving until it is finished and are given back with the board (the
/// caller takes the IDs of the board that this one replaces out of `taken`
/// first).
pub(crate) fn begin_board(
    board: Node,
    taken: &mut HashSet<NodeId>,
    next: &mut u64,
) -> BoardResolving {
    taken.extend(ids_of(&board));
    BoardResolving {
        root: Some(serde_json::to_value(&board).unwrap_or(Value::Null)),
        board,
        open: Vec::new(),
        finished: None,
        used: std::mem::take(taken),
        next: *next,
        addresses: HashMap::new(),
        instances: HashSet::new(),
        details: Details::default(),
        errors: Vec::new(),
    }
}

impl BoardResolving {
    /// Resolves nodes until `budget` has passed or the board is done, against
    /// `system` in `choice`. Gives the board, and puts the IDs back in `taken`
    /// and `next`, when it is done.
    pub(crate) fn step(
        &mut self,
        system: &DesignSystem,
        choice: &ModeChoice,
        budget: Duration,
        taken: &mut HashSet<NodeId>,
        next: &mut u64,
    ) -> Option<ResolvedBoard> {
        let started = Instant::now();
        let mut resolver = Resolver {
            system,
            choice: choice.clone(),
            used: std::mem::take(&mut self.used),
            next: self.next,
            addresses: std::mem::take(&mut self.addresses),
            instances: std::mem::take(&mut self.instances),
            details: std::mem::take(&mut self.details),
            errors: std::mem::take(&mut self.errors),
        };
        if let Some(root) = self.root.take() {
            // A board is one level below its page.
            match resolver.open(root, 1) {
                Opened::Whole(value) => self.finished = Some(value),
                Opened::Partial(partial) => self.open.push(partial),
            }
        }
        while self.finished.is_none() && started.elapsed() < budget {
            let Some(top) = self.open.last_mut() else {
                break;
            };
            match top.rest.next() {
                Some(child) => {
                    resolver.choice = top.choice.clone();
                    match resolver.open(child, top.level + 1) {
                        Opened::Whole(value) => top.done.push(value),
                        Opened::Partial(partial) => self.open.push(partial),
                    }
                }
                None => {
                    let Some(mut partial) = self.open.pop() else {
                        break;
                    };
                    partial.shell["children"] = Value::Array(partial.done);
                    match self.open.last_mut() {
                        Some(parent) => parent.done.push(partial.shell),
                        None => self.finished = Some(partial.shell),
                    }
                }
            }
        }
        self.used = resolver.used;
        self.next = resolver.next;
        self.addresses = resolver.addresses;
        self.instances = resolver.instances;
        self.details = resolver.details;
        self.errors = resolver.errors;
        let resolved = self.finished.take()?;
        let node = serde_json::from_value::<Node>(resolved)
            .or_else(|_| serde_json::from_value::<Node>(serde_json::to_value(&self.board)?))
            .unwrap_or_else(|_| self.board.clone());
        *taken = std::mem::take(&mut self.used);
        *next = self.next;
        Some(ResolvedBoard {
            board: node,
            addresses: std::mem::take(&mut self.addresses),
            instances: std::mem::take(&mut self.instances),
            details: std::mem::take(&mut self.details),
            errors: std::mem::take(&mut self.errors),
        })
    }
}

/// A node being resolved outside of any instance: all of it, or a container
/// whose children are still to be.
enum Opened {
    Whole(Value),
    Partial(Partial),
}

/// The IDs of `root` and everything below it.
pub(crate) fn ids_of(root: &Node) -> HashSet<NodeId> {
    let mut ids = HashSet::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        ids.insert(*node.id());
        pending.extend(node.children());
    }
    ids
}

/// `resolved` with each board that does not read as a design replaced by the
/// board of `source`.
fn keep_readable(source: &Value, mut resolved: Value) -> Option<Node> {
    let boards = resolved.get_mut("children")?.as_array_mut()?;
    let originals = source.get("children")?.as_array()?;
    for (board, original) in boards.iter_mut().zip(originals) {
        if serde_json::from_value::<Node>(board.clone()).is_err() {
            *board = original.clone();
        }
    }
    serde_json::from_value(resolved).ok()
}

struct Resolver<'a> {
    system: &'a DesignSystem,
    /// The modes the node being resolved is in: the page's choice, overlaid
    /// with the `modes` of each board that holds the node, the innermost last.
    choice: ModeChoice,
    /// The IDs of the page, which no ID made up for a node inside an instance
    /// may repeat.
    used: HashSet<NodeId>,
    next: u64,
    addresses: HashMap<NodeId, NodeAddress>,
    instances: HashSet<NodeId>,
    details: Details,
    errors: Vec<RenderError>,
}

impl Resolver<'_> {
    /// An ID that no node of the page has.
    fn fresh(&mut self) -> NodeId {
        const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
        loop {
            let mut text = [b'0'; 8];
            let mut rest = self.next;
            for digit in text.iter_mut().rev() {
                *digit = ALPHABET[(rest % 32) as usize];
                rest /= 32;
            }
            self.next += 1;
            let id: NodeId = std::str::from_utf8(&text)
                .ok()
                .and_then(|text| text.parse().ok())
                .expect("made-up node IDs are in the alphabet");
            if self.used.insert(id) {
                return id;
            }
        }
    }

    /// Resolves the node `value` and everything below it. `inside` is what
    /// the node takes from the instance that it is a layer of, if it is one,
    /// and `level` is how many levels below the page the node is (the page is
    /// level 0). Nodes below level `MAX_NESTING` are left out, and the node
    /// that held them is drawn as an error placeholder (`too_deep`).
    fn node(&mut self, value: Value, inside: Option<&Inside>, level: usize) -> Value {
        let outer = self.choice.clone();
        self.choice = self.overlaid(&value);
        let resolved = self.node_in(value, inside, level);
        self.choice = outer;
        resolved
    }

    /// The modes that `value` is resolved in: those of the node that holds
    /// it, with the explicit `modes` of a board on top. A mode that the design
    /// system no longer has is left out, so that the board is still drawn.
    fn overlaid(&self, value: &Value) -> ModeChoice {
        let mut choice = self.choice.clone();
        let named = value
            .get("modes")
            .and_then(Value::as_object)
            .filter(|_| kind_of(value) == "board");
        if let Some(named) = named {
            let collections = self.system.collections();
            for (name, mode) in named {
                let Some(mode) = mode.as_str() else { continue };
                if collections
                    .iter()
                    .any(|c| c.name == *name && c.modes.iter().any(|listed| listed == mode))
                {
                    choice.0.insert(name.clone(), mode.to_string());
                }
            }
        }
        choice
    }

    fn node_in(&mut self, mut value: Value, inside: Option<&Inside>, level: usize) -> Value {
        let Some(id) = value
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<NodeId>().ok())
        else {
            return value;
        };
        let kind = kind_of(&value);
        let address = NodeAddress::new(inside.map(|inside| inside.instance), id);
        if let Some(inside) = inside {
            bind(&mut value, &kind, inside);
            apply_overrides(&mut value, &kind, &id, inside);
        }
        let out_id = if inside.is_some() {
            let fresh = self.fresh();
            self.addresses.insert(fresh, address.clone());
            fresh
        } else {
            id
        };
        if kind == "instance" {
            self.instances.insert(out_id);
            return self.instance(value, out_id, address, inside, level);
        }
        let children = value
            .as_object_mut()
            .and_then(|node| node.remove("children"));
        if let Some(node) = value.as_object_mut() {
            node.remove("bind");
            node.insert("id".to_string(), json!(out_id));
        }
        self.tokens(&mut value, &address);
        if let Some(Value::Array(children)) = children {
            let children = if children.is_empty() || level < MAX_NESTING {
                children
                    .into_iter()
                    .map(|child| self.node(child, inside, level + 1))
                    .collect()
            } else {
                self.too_deep(&mut value, address);
                Vec::new()
            };
            value["children"] = Value::Array(children);
        }
        value
    }

    /// Resolves the node `value`, which is not inside an instance, as `node`
    /// does, except that a container gives back its children to be resolved
    /// one by one (`BoardResolving`). An instance is resolved whole.
    fn open(&mut self, mut value: Value, level: usize) -> Opened {
        let Some(id) = value
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<NodeId>().ok())
        else {
            return Opened::Whole(value);
        };
        if kind_of(&value) == "instance" {
            return Opened::Whole(self.node(value, None, level));
        }
        let address = NodeAddress::new(None, id);
        self.choice = self.overlaid(&value);
        let children = value
            .as_object_mut()
            .and_then(|node| node.remove("children"));
        if let Some(node) = value.as_object_mut() {
            node.remove("bind");
            node.insert("id".to_string(), json!(id));
        }
        self.tokens(&mut value, &address);
        let Some(Value::Array(children)) = children else {
            return Opened::Whole(value);
        };
        if children.is_empty() || level < MAX_NESTING {
            Opened::Partial(Partial {
                shell: value,
                rest: children.into_iter(),
                done: Vec::new(),
                level,
                choice: self.choice.clone(),
            })
        } else {
            self.too_deep(&mut value, address);
            value["children"] = Value::Array(Vec::new());
            Opened::Whole(value)
        }
    }

    /// Draws `node`, which holds nodes deeper than `MAX_NESTING` levels, as
    /// an error placeholder, and notes that what it holds is left out.
    fn too_deep(&mut self, node: &mut Value, address: NodeAddress) {
        mark_error(node);
        self.errors.push(RenderError {
            node: address,
            missing: Missing::Nesting,
        });
    }

    /// Expands the instance `value` into a frame with the ID `out_id`.
    fn instance(
        &mut self,
        value: Value,
        out_id: NodeId,
        address: NodeAddress,
        inside: Option<&Inside>,
        level: usize,
    ) -> Value {
        let own = value
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<NodeId>().ok())
            .unwrap_or(out_id);
        let component = value
            .get("component")
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<NodeId>().ok());
        let depth = inside.map_or(0, |inside| inside.depth);
        let base = component.and_then(|id| self.system.component(&id));
        let (Some(_), Some(base), true) = (component, base, depth < MAX_DEPTH) else {
            if let Some(component) = component {
                self.errors.push(RenderError {
                    node: address,
                    missing: Missing::Component(component),
                });
            }
            return placeholder(&value, out_id);
        };

        let given = value.get("props").and_then(Value::as_object);
        let wanted: HashMap<String, String> = given
            .into_iter()
            .flatten()
            .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_string())))
            .collect();
        let chosen = self.system.variant(base, &wanted);
        let props = property_values(chosen, given);
        let stored = value.get("overrides").and_then(Value::as_object);
        self.details.instances.insert(
            out_id,
            instance_inspection(base, chosen, &props, given, stored),
        );

        let mut overrides = value
            .get("overrides")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(inside) = inside {
            for (node, fields) in &inside.overrides {
                let (Some(fields), Some(slot)) = (
                    fields.as_object(),
                    overrides
                        .entry(node.clone())
                        .or_insert_with(|| json!({}))
                        .as_object_mut(),
                ) else {
                    continue;
                };
                for (field, setting) in fields {
                    slot.insert(field.clone(), setting.clone());
                }
            }
        }
        let layers_inside = Inside {
            instance: inside.map_or(own, |inside| inside.instance),
            props,
            overrides,
            depth: depth + 1,
        };

        let mut frame = chosen.json.clone();
        let layers = frame
            .as_object_mut()
            .and_then(|frame| frame.remove("children"));
        if let Some(frame) = frame.as_object_mut() {
            for key in PLACEMENT.iter().chain(["props", "bind"].iter()) {
                frame.remove(*key);
            }
            frame.insert("type".to_string(), json!("frame"));
            frame.insert("id".to_string(), json!(out_id));
            for key in ["name", "visible"] {
                if let Some(setting) = value.get(key) {
                    frame.insert(key.to_string(), setting.clone());
                }
            }
            for key in PLACEMENT.iter().chain(SIZES.iter()) {
                if let Some(setting) = value.get(*key) {
                    frame.insert((*key).to_string(), setting.clone());
                }
            }
        }
        // The component's own node can be overridden too.
        for key in [base.id, chosen.id] {
            apply_overrides(&mut frame, "frame", &key, &layers_inside);
        }
        self.tokens(&mut frame, &address);
        let layers = layers
            .and_then(|layers| match layers {
                Value::Array(layers) => Some(layers),
                _ => None,
            })
            .unwrap_or_default();
        let layers = if layers.is_empty() || level < MAX_NESTING {
            layers
                .into_iter()
                .map(|layer| self.node(layer, Some(&layers_inside), level + 1))
                .collect()
        } else {
            self.too_deep(&mut frame, address);
            Vec::new()
        };
        frame["children"] = Value::Array(layers);
        frame
    }

    /// Replaces the token references in the properties of `node` (not its
    /// children) with the tokens' values in the mode. A node with a reference
    /// that has no token, or with a token whose value does not fit where it is
    /// used (a color for a padding), is drawn as an error placeholder, and the
    /// reference is left in place.
    fn tokens(&mut self, node: &mut Value, address: &NodeAddress) {
        // Most nodes refer to no token, and they have nothing to resolve.
        if !node.as_object().is_some_and(|properties| {
            properties
                .iter()
                .any(|(key, setting)| !NOT_TOKENS.contains(&key.as_str()) && refers(setting))
        }) {
            return;
        }
        let mut missing = Vec::new();
        let mut resolved = Vec::new();
        if let Some(properties) = node.as_object_mut() {
            let mut pointer = String::new();
            for (key, setting) in properties.iter_mut() {
                if !NOT_TOKENS.contains(&key.as_str()) {
                    pointer.clear();
                    pointer.push('/');
                    push_escaped(&mut pointer, key);
                    self.resolve_in(setting, key, &mut pointer, &mut missing, &mut resolved);
                }
            }
        }
        let used: Vec<UsedToken> = resolved
            .iter()
            .map(|replaced| UsedToken {
                pointer: replaced.pointer.clone(),
                reference: replaced.reference.clone(),
                value: replaced.token.clone(),
            })
            .collect();
        let mut errors: Vec<Missing> = missing.into_iter().map(Missing::Token).collect();
        if !resolved.is_empty() && !reads(node) {
            for (reference, property) in unfit(node, resolved) {
                let error = Missing::Unfit {
                    token: reference,
                    property,
                };
                if !errors.contains(&error) {
                    errors.push(error);
                }
            }
        }
        if errors.is_empty() {
            let id = node
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| id.parse::<NodeId>().ok());
            if let (Some(id), false) = (id, used.is_empty()) {
                self.details.tokens.insert(id, used);
            }
            return;
        }
        mark_error(node);
        for missing in errors {
            self.errors.push(RenderError {
                node: address.clone(),
                missing,
            });
        }
    }

    /// Resolves the token references in `value`, the setting `key` of a node
    /// at `pointer` (a JSON pointer into the node), and notes each one it
    /// replaced in `resolved`.
    fn resolve_in(
        &self,
        value: &mut Value,
        key: &str,
        pointer: &mut String,
        missing: &mut Vec<TokenRef>,
        resolved: &mut Vec<Replaced>,
    ) {
        match value {
            Value::String(text) if text.starts_with('$') => {
                let Ok(reference) = text.parse::<TokenRef>() else {
                    return;
                };
                match self.system.token(&reference, &self.choice) {
                    Some(token) => {
                        let token = design_value(token, key);
                        let text = std::mem::replace(value, token.clone());
                        resolved.push(Replaced {
                            pointer: pointer.to_string(),
                            reference,
                            text,
                            token,
                        });
                    }
                    None if !missing.contains(&reference) => missing.push(reference),
                    None => {}
                }
            }
            Value::Array(items) => {
                let outer = pointer.len();
                for (at, item) in items.iter_mut().enumerate() {
                    pointer.truncate(outer);
                    pointer.push('/');
                    pointer.push_str(&at.to_string());
                    self.resolve_in(item, key, pointer, missing, resolved);
                }
                pointer.truncate(outer);
            }
            Value::Object(properties) => {
                let outer = pointer.len();
                for (key, setting) in properties.iter_mut() {
                    if !NOT_TOKENS.contains(&key.as_str()) {
                        pointer.truncate(outer);
                        pointer.push('/');
                        push_escaped(pointer, key);
                        self.resolve_in(setting, key, pointer, missing, resolved);
                    }
                }
                pointer.truncate(outer);
            }
            _ => {}
        }
    }
}

/// Whether `setting`, a setting of a node, holds a token reference: what
/// `Resolver::resolve_in` replaces.
fn refers(setting: &Value) -> bool {
    match setting {
        Value::String(text) => text.starts_with('$'),
        Value::Array(items) => items.iter().any(refers),
        Value::Object(properties) => properties
            .iter()
            .any(|(key, setting)| !NOT_TOKENS.contains(&key.as_str()) && refers(setting)),
        _ => false,
    }
}

/// A token reference in a node that was replaced by the token's value.
struct Replaced {
    /// Where in the node, as a JSON pointer.
    pointer: String,
    reference: TokenRef,
    /// The reference as the node wrote it.
    text: Value,
    /// The token's value as the node takes it.
    token: Value,
}

/// Whether `node`, without its children, reads as a design node.
fn reads(node: &Value) -> bool {
    serde_json::from_value::<Node>(node.clone()).is_ok()
}

/// Puts back the references of `resolved` in `node` whose token values do not
/// fit where they are used, and gives each with the property of the node it
/// is in. When `node` does not read even with all its references in place,
/// it is left as it is and nothing is given.
fn unfit(node: &mut Value, resolved: Vec<Replaced>) -> Vec<(TokenRef, String)> {
    let put = |node: &mut Value, pointer: &str, setting: Value| {
        node.pointer_mut(pointer)
            .map(|slot| std::mem::replace(slot, setting))
    };
    for replaced in &resolved {
        put(node, &replaced.pointer, replaced.text.clone());
    }
    if !reads(node) {
        for replaced in resolved {
            put(node, &replaced.pointer, replaced.token);
        }
        return Vec::new();
    }
    let mut unfit = Vec::new();
    for replaced in resolved {
        let Some(text) = put(node, &replaced.pointer, replaced.token) else {
            continue;
        };
        if !reads(node) {
            put(node, &replaced.pointer, text);
            let property = replaced
                .pointer
                .split('/')
                .nth(1)
                .unwrap_or_default()
                .replace("~1", "/")
                .replace("~0", "~");
            unfit.push((replaced.reference, property));
        }
    }
    unfit
}

/// Appends `key` as a step of a JSON pointer to `pointer`.
fn push_escaped(pointer: &mut String, key: &str) {
    for character in key.chars() {
        match character {
            '~' => pointer.push_str("~0"),
            '/' => pointer.push_str("~1"),
            other => pointer.push(other),
        }
    }
}

fn kind_of(node: &Value) -> String {
    node.get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The values of the properties of `component` for an instance that gives
/// `given`: the instance's value where it fits the property, and the
/// property's default where not.
fn property_values(
    component: &ComponentDef,
    given: Option<&Map<String, Value>>,
) -> HashMap<String, Prop> {
    component
        .props
        .iter()
        .map(|(name, def)| {
            let setting = given.and_then(|given| given.get(name));
            let value = match def {
                PropDef::Variant { values, default } => Prop::Text(
                    setting
                        .and_then(Value::as_str)
                        .filter(|text| values.iter().any(|value| value == text))
                        .unwrap_or(default.as_str())
                        .to_string(),
                ),
                PropDef::Boolean { default } => {
                    Prop::Flag(setting.and_then(Value::as_bool).unwrap_or(*default))
                }
                PropDef::Text { default } => Prop::Text(
                    setting
                        .and_then(Value::as_str)
                        .unwrap_or(default.as_str())
                        .to_string(),
                ),
                PropDef::Slot { default } => Prop::Text(
                    setting
                        .and_then(Value::as_str)
                        .filter(|id| id.parse::<NodeId>().is_ok())
                        .map_or_else(|| default.to_string(), str::to_string),
                ),
            };
            (name.clone(), value)
        })
        .collect()
}

/// What the instance that shows `chosen`, a variant of `base`, stores: the
/// value of each property of the component (`props` are the ones the
/// instance gets) and the instance's own `overrides`.
fn instance_inspection(
    base: &ComponentDef,
    chosen: &ComponentDef,
    props: &HashMap<String, Prop>,
    given: Option<&Map<String, Value>>,
    overrides: Option<&Map<String, Value>>,
) -> InstanceInspection {
    let props = chosen
        .props
        .iter()
        .filter_map(|(name, def)| {
            let value = match props.get(name)? {
                Prop::Text(text) => json!(text),
                Prop::Flag(flag) => json!(flag),
            };
            let kind = match def {
                PropDef::Variant { .. } => "variant",
                PropDef::Boolean { .. } => "boolean",
                PropDef::Text { .. } => "text",
                PropDef::Slot { .. } => "slot",
            };
            Some(PropValue {
                name: name.clone(),
                kind: kind.to_string(),
                set: given.and_then(|given| given.get(name)) == Some(&value),
                value,
            })
        })
        .collect();
    let overrides = overrides
        .into_iter()
        .flatten()
        .filter_map(|(node, fields)| {
            Some(OverrideInspection {
                name: layer_name(&chosen.json, node)
                    .or_else(|| layer_name(&base.json, node))
                    .unwrap_or_default(),
                node: node.parse().ok()?,
                fields: fields.as_object()?.clone(),
            })
        })
        .collect();
    InstanceInspection {
        component: base.id,
        component_name: base.name.clone(),
        props,
        overrides,
    }
}

/// The name of the node `id` in the component `layer`, or of a node below it.
fn layer_name(layer: &Value, id: &str) -> Option<String> {
    let name = || {
        layer
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    if layer.get("id").and_then(Value::as_str) == Some(id) {
        return name();
    }
    layer
        .get("children")?
        .as_array()?
        .iter()
        .find_map(|child| layer_name(child, id))
}

/// Gives the layer `node` of kind `kind` the property values it is bound to.
fn bind(node: &mut Value, kind: &str, inside: &Inside) {
    let Some(bound) = node.get("bind").and_then(Value::as_object).cloned() else {
        return;
    };
    for (field, property) in &bound {
        let Some(value) = property.as_str().and_then(|name| inside.props.get(name)) else {
            continue;
        };
        match (field.as_str(), value, kind) {
            ("visible", Prop::Flag(shown), _) => node["visible"] = json!(shown),
            ("content", Prop::Text(text), "text") => node["content"] = json!(text),
            ("component", Prop::Text(component), "instance") => {
                node["component"] = json!(component);
            }
            _ => {}
        }
    }
}

/// Applies the override of the instance for the component node `id`, if it
/// has one, to `node` of kind `kind`.
fn apply_overrides(node: &mut Value, kind: &str, id: &NodeId, inside: &Inside) {
    let Some(set) = inside.overrides.get(id.as_str()).and_then(Value::as_object) else {
        return;
    };
    if let Some(shown) = set.get("visible").filter(|shown| shown.is_boolean()) {
        node["visible"] = shown.clone();
    }
    if let (Some(text), "text") = (set.get("content").filter(|text| text.is_string()), kind) {
        node["content"] = text.clone();
    }
    if let (Some(component), "instance") = (
        set.get("component")
            .filter(|component| component.is_string()),
        kind,
    ) {
        node["component"] = component.clone();
    }
    if let Some(fills) = set.get("fills").and_then(Value::as_array) {
        match kind {
            "frame" | "board" | "vector" => node["fills"] = Value::Array(fills.clone()),
            // A text takes the color of the first solid fill.
            "text" => {
                let solid = fills.iter().find_map(|fill| {
                    (fill.get("type")? == "solid").then(|| fill.get("color").cloned())?
                });
                if let Some(color) = solid {
                    node["color"] = color;
                }
            }
            _ => {}
        }
    }
}

/// What `component` instance `value` is drawn as when its component is
/// missing: an empty frame in the error look, in the instance's box.
fn placeholder(value: &Value, out_id: NodeId) -> Value {
    let mut frame = json!({
        "id": out_id,
        "type": "frame",
        "name": value.get("name").cloned().unwrap_or(json!("")),
        "visible": value.get("visible").cloned().unwrap_or(json!(true)),
        "width": {"fixed": ERROR_SIZE},
        "height": {"fixed": ERROR_SIZE},
        "layout": {"mode": "none"},
        "children": [],
    });
    for key in PLACEMENT.iter().chain(SIZES.iter()) {
        if let Some(setting) = value.get(*key) {
            frame[*key] = setting.clone();
        }
    }
    mark_error(&mut frame);
    frame
}

/// Gives `node` the error look: a tinted box with a magenta edge, or magenta
/// text.
fn mark_error(node: &mut Value) {
    let edge = json!([{"color": ERROR_COLOR, "width": 2}]);
    match kind_of(node).as_str() {
        "text" => node["color"] = json!(ERROR_COLOR),
        "frame" | "board" | "vector" => {
            node["fills"] = json!([{"type": "solid", "color": ERROR_TINT}]);
            node["strokes"] = edge;
        }
        "image" => node["strokes"] = edge,
        _ => {}
    }
}

/// The value of a token as a design writes it: a dimension in pixels is a
/// number, a shadow token is a drop shadow, and a typography token used for a
/// font family is its family.
fn design_value(token: Value, key: &str) -> Value {
    match token {
        Value::String(text) => match pixels(&text) {
            Some(number) => json!(number),
            None => Value::String(text),
        },
        Value::Object(mut fields) if fields.contains_key("offsetX") => {
            fields.insert("type".to_string(), json!("drop"));
            for field in ["offsetX", "offsetY", "blur", "spread"] {
                if let Some(Value::String(text)) = fields.get(field)
                    && let Some(number) = pixels(text)
                {
                    fields.insert(field.to_string(), json!(number));
                }
            }
            Value::Object(fields)
        }
        Value::Object(mut fields) if key == "fontFamily" => {
            fields.remove("fontFamily").unwrap_or(Value::Null)
        }
        other => other,
    }
}

/// The number in a dimension such as `16px`.
fn pixels(text: &str) -> Option<f64> {
    text.strip_suffix("px")?.trim().parse().ok()
}
