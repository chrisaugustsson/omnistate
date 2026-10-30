//! Components and instances (US-029, Q27, FR-30, FR-31). The design system doc
//! holds the project's components next to its tokens: an agent replaces all of
//! them in one all-or-nothing import of `<Component>` tags, and a design doc
//! holds pages whose instances place them. The tags are those of
//! docs/formats/design-tags.md, checked by `omni_core::design::tags`.
//!
//! A design system's head is its token tree and its components as one chunk of
//! JSON, and a design doc's head a manifest of the chunks of its pages and
//! boards (src/boards.rs). Every write is a new head, one row of `doc_changes`
//! and a `doc.changed` event. An instance resolves against the design system's
//! head as it is when the design is read.

use crate::AppState;
use crate::boards;
use crate::chunks::{Snapshot, index_snapshot, read_chunk, store_text};
use crate::docs::{
    AgentIdentity, DocError, OpResultBody, Tx, log_change, log_change_of_boards, random_node_id,
    results_of,
};
use crate::live;
use crate::placeholders::{self, Sizing};
use crate::previews;
use crate::projects::{parse_body, require_project};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use omni_core::design::tags::{self, TagNode};
use omni_core::design::tokens::TokenTree;
use omni_core::design::{
    DesignDoc, Instance, Node, NodeData, NodeId, NodeKind, PropDef, PropDefs, PropValue,
};
use omni_core::ops::{ApplyError, Operation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use utoipa::ToSchema;

/// The node IDs in use, which a new ID must not be.
type UsedIds = Arc<Mutex<HashSet<NodeId>>>;

/// The key that holds the components next to the tokens in the JSON of a
/// design system's chunk. A token tree has no top-level `$` key of its own.
pub(crate) const COMPONENTS_KEY: &str = "$components";

// The content of heads ------------------------------------------------------------

pub(crate) fn nodes_json(nodes: &[Node]) -> Value {
    Value::Array(
        nodes
            .iter()
            .map(|node| serde_json::to_value(node).expect("a node serializes to JSON"))
            .collect(),
    )
}

pub(crate) fn nodes_from(value: &Value) -> Result<Vec<Node>, String> {
    let Value::Array(items) = value else {
        return Err("it is not a list of nodes".to_string());
    };
    items
        .iter()
        .map(|item| serde_json::from_value(item.clone()).map_err(|e| e.to_string()))
        .collect()
}

/// The chunk of a design system's head: its token tree (an empty one when it
/// has no tokens), and its components under `$components` when it has any.
pub(crate) fn design_system_text(tokens: Option<&TokenTree>, components: &[Node]) -> String {
    let mut json = tokens.map_or_else(|| json!({}), TokenTree::to_json);
    if !components.is_empty()
        && let Some(object) = json.as_object_mut()
    {
        object.insert(COMPONENTS_KEY.to_string(), nodes_json(components));
    }
    json.to_string()
}

/// The tokens and the components that a design system's head holds.
/// The tokens are `None` for a design system of components only.
pub(crate) fn read_design_system(
    text: &str,
    what: &str,
) -> Result<(Option<TokenTree>, Vec<Node>), ApiError> {
    let unavailable = |why: String| {
        eprintln!("omni-api: {what} holds no design system: {why}");
        ApiError::Unavailable
    };
    // A design system that has nothing yet (its head when it was made) is the
    // empty text.
    if text.is_empty() {
        return Ok((None, Vec::new()));
    }
    let mut json: Value = serde_json::from_str(text).map_err(|e| unavailable(e.to_string()))?;
    let components = match json
        .as_object_mut()
        .and_then(|object| object.remove(COMPONENTS_KEY))
    {
        Some(value) => nodes_from(&value).map_err(unavailable)?,
        None => Vec::new(),
    };
    if has_no_tokens(&json) {
        return Ok((None, components));
    }
    let tokens = TokenTree::from_json(&json).map_err(|e| unavailable(e.to_string()))?;
    Ok((Some(tokens), components))
}

/// Whether the token part of a design system's JSON is empty: a design
/// system of components only.
pub(crate) fn has_no_tokens(json: &Value) -> bool {
    json.as_object().is_some_and(serde_json::Map::is_empty)
}

/// The nodes as tags, one tree after the other.
pub(crate) fn tags_of(nodes: &[Node]) -> String {
    nodes.iter().map(tags::print).collect::<Vec<_>>().join("\n")
}

fn all_ids(nodes: &[Node]) -> HashSet<NodeId> {
    let mut ids = HashSet::new();
    let mut pending: Vec<&Node> = nodes.iter().collect();
    while let Some(node) = pending.pop() {
        ids.insert(*node.id());
        pending.extend(node.children());
    }
    ids
}

// The components of a design system ------------------------------------------------------

/// The components of a design system, in order.
#[derive(Clone, Default)]
pub(crate) struct Library(Vec<Node>);

impl Library {
    pub(crate) fn new(components: Vec<Node>) -> Self {
        Self(components)
    }

    pub(crate) fn into_components(self) -> Vec<Node> {
        self.0
    }

    fn get(&self, id: &NodeId) -> Option<&Node> {
        self.0.iter().find(|component| component.id() == id)
    }

    /// The components with the IDs in `ids`, in order.
    fn only(&self, ids: &BTreeSet<NodeId>) -> Vec<Node> {
        self.0
            .iter()
            .filter(|component| ids.contains(component.id()))
            .cloned()
            .collect()
    }
}

fn defs_of(component: &Node) -> PropDefs {
    match component.data() {
        NodeData::Component(component) => component.props.clone().unwrap_or_default(),
        _ => PropDefs::default(),
    }
}

fn quoted<'a>(words: impl IntoIterator<Item = &'a str>) -> String {
    let words: Vec<String> = words.into_iter().map(|word| format!("`{word}`")).collect();
    match words.as_slice() {
        [] => "nothing".to_string(),
        [first @ .., last] if !first.is_empty() => format!("{} or {last}", first.join(", ")),
        _ => words.join(""),
    }
}

/// The nodes of a component's tree, the component itself included, by ID.
fn tree_of(root: &Node) -> HashMap<NodeId, &Node> {
    let mut nodes = HashMap::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        nodes.insert(*node.id(), node);
        pending.extend(node.children());
    }
    nodes
}

/// Checks an instance against the component it places: the component is in
/// the library, each property it sets exists with a value of its kind, and
/// each override changes a node of the component with a field the node takes.
fn check_instance(name: &str, instance: &Instance, library: &Library) -> Result<(), String> {
    let Some(component) = library.get(&instance.component) else {
        return Err(format!(
            "`{}` is not a component of the design system, but the instance `{name}` places it",
            instance.component
        ));
    };
    let component_name = component.name();
    let defs = defs_of(component);
    if let Some(props) = &instance.props {
        for (prop, value) in props.iter() {
            let Some(def) = defs.get(prop) else {
                return Err(format!(
                    "`{prop}` is not a property of the component `{component_name}`, but the instance `{name}` gives it a value: the component defines {}",
                    quoted(defs.iter().map(|(name, _)| name.as_str()))
                ));
            };
            match (def, value) {
                (PropDef::Variant { values, .. }, PropValue::Text(text))
                    if !values.contains(text) =>
                {
                    return Err(format!(
                        "`{text}` is not a value of the variant `{prop}` of the component `{component_name}`: use one of {}",
                        quoted(values.iter().map(String::as_str))
                    ));
                }
                (PropDef::Variant { .. } | PropDef::Text { .. }, PropValue::Text(_))
                | (PropDef::Boolean { .. }, PropValue::Flag(_)) => {}
                (PropDef::Slot { .. }, PropValue::Text(text)) => {
                    let found = text
                        .parse::<NodeId>()
                        .ok()
                        .is_some_and(|id| library.get(&id).is_some());
                    if !found {
                        return Err(format!(
                            "`{text}` is not a component of the design system, but the slot `{prop}` of the component `{component_name}` takes one"
                        ));
                    }
                }
                (def, value) => {
                    let wanted = match def {
                        PropDef::Boolean { .. } => "`true` or `false`",
                        PropDef::Variant { .. } => "one of its values, as text",
                        PropDef::Text { .. } => "text",
                        PropDef::Slot { .. } => "the node ID of a component",
                    };
                    let given = match value {
                        PropValue::Text(_) => "text",
                        PropValue::Flag(_) => "true or false",
                    };
                    return Err(format!(
                        "`{prop}` is a {} property of the component `{component_name}`, but the instance `{name}` gives it {given}: write {wanted}",
                        def.kind()
                    ));
                }
            }
        }
    }
    if let Some(overrides) = &instance.overrides {
        let nodes = tree_of(component);
        for (key, change) in overrides.iter() {
            let Some(target) = nodes.get(key) else {
                return Err(format!(
                    "`{key}` is no node of the component `{component_name}`, but the instance `{name}` overrides it"
                ));
            };
            let kind = target.kind();
            if change.content.is_some() && kind != NodeKind::Text {
                return Err(format!(
                    "`content` is the text of a text node, but `{key}` is a {kind} node of the component `{component_name}`"
                ));
            }
            if change.fills.is_some()
                && !matches!(
                    kind,
                    NodeKind::Frame | NodeKind::Vector | NodeKind::Component
                )
            {
                return Err(format!(
                    "`fills` is for frames and vectors, but `{key}` is a {kind} node of the component `{component_name}`"
                ));
            }
            if let Some(swapped) = &change.component {
                if kind != NodeKind::Instance {
                    return Err(format!(
                        "`component` swaps the contents of a slot, but `{key}` is a {kind} node of the component `{component_name}`, not an instance"
                    ));
                }
                if library.get(swapped).is_none() {
                    return Err(format!(
                        "`{swapped}` is not a component of the design system, but the instance `{name}` swaps it into `{key}`"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Checks every instance below `nodes`, at any depth.
pub(crate) fn check_instances(nodes: &[Node], library: &Library) -> Result<(), String> {
    let mut pending: Vec<&Node> = nodes.iter().rev().collect();
    while let Some(node) = pending.pop() {
        if let NodeData::Instance(instance) = node.data() {
            check_instance(node.name(), instance, library)?;
        }
        pending.extend(node.children().iter().rev());
    }
    Ok(())
}

/// Checks the components of an import against each other: a slot default and
/// a nested instance name a component of the import, a nested instance fits
/// its component, and no component places itself, even through others.
pub(crate) fn check_components(components: &[Node]) -> Result<(), String> {
    let library = Library::new(components.to_vec());
    for component in components {
        for (prop, def) in defs_of(component).iter() {
            if let PropDef::Slot { default } = def
                && library.get(default).is_none()
            {
                return Err(format!(
                    "`{default}` is not a component of this import, but it is the default of the slot `{prop}` of the component `{}`: import the component with the others",
                    component.name()
                ));
            }
        }
        check_instances(component.children(), &library)?;
    }
    // A component places the components of the instances in its tree.
    let places = |component: &Node| -> Vec<NodeId> {
        let mut found = Vec::new();
        let mut pending: Vec<&Node> = component.children().iter().collect();
        while let Some(node) = pending.pop() {
            if let NodeData::Instance(instance) = node.data() {
                found.push(instance.component);
            }
            pending.extend(node.children());
        }
        found
    };
    let mut done: HashSet<NodeId> = HashSet::new();
    for start in components {
        let mut path: Vec<NodeId> = Vec::new();
        let mut stack: Vec<(NodeId, Vec<NodeId>)> = vec![(*start.id(), places(start))];
        path.push(*start.id());
        while let Some((_, remaining)) = stack.last_mut() {
            let Some(next) = remaining.pop() else {
                let (finished, _) = stack.pop().expect("the stack has a top");
                path.pop();
                done.insert(finished);
                continue;
            };
            if done.contains(&next) {
                continue;
            }
            if path.contains(&next) {
                let name = library.get(&next).map_or("", Node::name);
                return Err(format!(
                    "the component `{name}` places itself, through its own tree or through other components: a component cannot contain itself"
                ));
            }
            if let Some(component) = library.get(&next) {
                path.push(next);
                stack.push((next, places(component)));
            }
        }
    }
    Ok(())
}

/// The components that `nodes` use, with the components those place in their
/// trees and the ones their slots can take: instances anywhere in the nodes,
/// the values and overrides that fill a slot, and the defaults of slots.
fn used_by(nodes: &[Node], library: &Library) -> BTreeSet<NodeId> {
    fn walk(nodes: &[Node], library: &Library, used: &mut BTreeSet<NodeId>) {
        for node in nodes {
            if let NodeData::Instance(instance) = node.data() {
                place(&instance.component, library, used);
                if let (Some(props), Some(component)) =
                    (&instance.props, library.get(&instance.component))
                {
                    let defs = defs_of(component);
                    for (prop, value) in props.iter() {
                        if let (Some(PropDef::Slot { .. }), PropValue::Text(text)) =
                            (defs.get(prop), value)
                            && let Ok(id) = text.parse::<NodeId>()
                        {
                            place(&id, library, used);
                        }
                    }
                }
                if let Some(overrides) = &instance.overrides {
                    for (_, change) in overrides.iter() {
                        if let Some(id) = &change.component {
                            place(id, library, used);
                        }
                    }
                }
            }
            walk(node.children(), library, used);
        }
    }
    fn place(id: &NodeId, library: &Library, used: &mut BTreeSet<NodeId>) {
        if !used.insert(*id) {
            return;
        }
        if let Some(component) = library.get(id) {
            for (_, def) in defs_of(component).iter() {
                if let PropDef::Slot { default } = def {
                    place(default, library, used);
                }
            }
            walk(component.children(), library, used);
        }
    }
    let mut used = BTreeSet::new();
    walk(nodes, library, &mut used);
    used
}

pub(crate) fn nodes_values(nodes: &[Node]) -> Vec<Value> {
    match nodes_json(nodes) {
        Value::Array(items) => items,
        _ => Vec::new(),
    }
}

/// What the design viewer reads of a design's head first: its pages with their
/// boards as placeholders, and the design system's head.
pub(crate) struct HeadOutline {
    pub(crate) outline: Vec<Value>,
    /// Every component of the design system's head, so that a board read later
    /// resolves its instances against it.
    pub(crate) components: Vec<Value>,
    pub(crate) tokens: Option<Value>,
}

/// The outline of a design's head, and the head of the project's design system
/// as it is now.
pub(crate) async fn outline_of_head(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    head: Option<&str>,
) -> Result<HeadOutline, ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    let found = design_system_row(&mut tx, project_id, false).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let (system, key) = match found {
        Some((doc_id, system_head)) => (
            read_system_head(state, workspace_id, &doc_id, system_head.as_deref()).await?,
            system_head,
        ),
        None => (SystemHead::default(), None),
    };
    let components = nodes_values(&system.library.0);
    let tokens = system.tokens_json();
    // The placeholders are laid out against the design system head the viewer
    // resolves the boards against.
    let sizing = Sizing::new(
        state,
        workspace_id,
        key.as_deref(),
        tokens.as_ref(),
        &components,
    )
    .await;
    let outline = match head {
        Some(head) => boards::read_outline(state, workspace_id, head, "the head", &sizing).await?,
        None => Vec::new(),
    };
    Ok(HeadOutline {
        outline,
        components,
        tokens,
    })
}

/// The component that the instance `instance` places, looked for in every
/// node of the pages.
fn component_of_instance(nodes: &[Node], instance: &str) -> Option<NodeId> {
    nodes.iter().find_map(|node| {
        match node.data() {
            NodeData::Instance(placed) if node.id().as_str() == instance => {
                return Some(placed.component);
            }
            _ => {}
        }
        component_of_instance(node.children(), instance)
    })
}

/// The tags of the node `id` among `nodes` and their descendants.
fn tags_of_node(nodes: &[Node], id: &str) -> Option<String> {
    nodes.iter().find_map(|node| {
        if node.id().as_str() == id {
            return Some(tags::print(node));
        }
        tags_of_node(node.children(), id)
    })
}

/// The tags of the node `node` in `chunk`, a page or a board of a design's
/// head: the node found by its ID in the subtree that the chunk holds. A page
/// chunk has no children; its boards are chunks of their own.
pub(crate) async fn head_node_tags(
    state: &AppState,
    workspace_id: &str,
    chunk: &str,
    node: &str,
) -> Result<Option<String>, ApiError> {
    let text = read_chunk(state, workspace_id, chunk).await?;
    let unreadable = |why: String| {
        eprintln!("omni-api: the chunk {chunk} holds no board: {why}");
        ApiError::Unavailable
    };
    let mut value: Value = serde_json::from_str(&text).map_err(|e| unreadable(e.to_string()))?;
    let nodes = match nodes_from(&Value::Array(vec![value.clone()])) {
        Ok(nodes) => nodes,
        Err(_) => {
            if let Some(object) = value.as_object_mut() {
                object
                    .entry("children")
                    .or_insert_with(|| Value::Array(Vec::new()));
            }
            nodes_from(&Value::Array(vec![value])).map_err(unreadable)?
        }
    };
    Ok(tags_of_node(&nodes, node))
}

/// The tags of the node `inner` of the component that the instance `instance`
/// places, as the design system's head defines the component: the second half
/// of the node address `<instanceId>/<componentNodeId>` of a comment on a
/// design's head (US-106). `chunk` is the board chunk of the head that holds
/// the instance, so no other board is read. `None` when the instance does not
/// place a component that has the node.
pub(crate) async fn head_instance_node(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    chunk: &str,
    instance: &str,
    inner: &str,
) -> Result<Option<String>, ApiError> {
    let text = read_chunk(state, workspace_id, chunk).await?;
    let nodes = serde_json::from_str::<Value>(&text)
        .map_err(|e| e.to_string())
        .and_then(|board| nodes_from(&Value::Array(vec![board])))
        .map_err(|e| {
            eprintln!("omni-api: the chunk {chunk} holds no board: {e}");
            ApiError::Unavailable
        })?;
    let Some(placed) = component_of_instance(&nodes, instance) else {
        return Ok(None);
    };
    let system = system_head_of_project(state, workspace_id, project_id).await?;
    Ok(system.library.get(&placed).and_then(|component| {
        tree_of(component)
            .values()
            .find(|node| node.id().as_str() == inner)
            .map(|node| tags::print(node))
    }))
}

// The imports ------------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct ImportTagsRequest {
    /// The tags of docs/formats/design-tags.md: `<Component>` tags for the
    /// components of a design system, `<Page>` tags for a design.
    pub tags: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct InvalidDesignBody {
    /// `invalid_components` or `invalid_design`
    pub error: String,
    /// What is wrong; it names the line of the tags, the component, the
    /// property, the value or the node.
    pub message: String,
}

/// Why a components or design request failed: the usual API errors, or tags
/// that the rules refuse.
pub enum DesignError {
    Api(ApiError),
    Invalid { code: &'static str, message: String },
}

impl From<ApiError> for DesignError {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for DesignError {
    fn into_response(self) -> Response {
        match self {
            Self::Api(error) => error.into_response(),
            Self::Invalid { code, message } => (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": code, "message": message })),
            )
                .into_response(),
        }
    }
}

/// Reads the tags of an import, none or more, and checks that each is one of
/// `kind`.
fn parse_import(
    text: &str,
    code: &'static str,
    kind: NodeKind,
) -> Result<Vec<TagNode>, DesignError> {
    let invalid = |message: String| DesignError::Invalid { code, message };
    let roots = tags::parse_all(text).map_err(|error| invalid(error.to_string()))?;
    let wanted = match kind {
        NodeKind::Component => "`<Component>` tags, one after the other",
        _ => "`<Page>` tags, one after the other",
    };
    for root in &roots {
        if root.kind() != kind {
            let found = tags::all()
                .iter()
                .find(|tag| tag.name.eq_ignore_ascii_case(root.kind().as_str()))
                .map_or("a tag", |tag| tag.name);
            return Err(invalid(format!(
                "this import holds {wanted}, but it has a `<{found}>` named `{}`",
                root.name()
            )));
        }
    }
    Ok(roots)
}

/// Gives the new nodes of `roots` their IDs from the server, in document
/// order, and returns the nodes with the IDs it handed out. An `id` that is
/// written must be one of `known`.
fn assign(
    roots: Vec<TagNode>,
    known: &HashSet<NodeId>,
    used: &UsedIds,
    code: &'static str,
    what: &str,
) -> Result<(Vec<Node>, Vec<String>), DesignError> {
    let invalid = |message: String| DesignError::Invalid { code, message };
    for root in &roots {
        for id in root.ids() {
            if !known.contains(&id) {
                return Err(invalid(format!(
                    "`{id}` is no node of {what}, so the tag cannot name it: leave `id` out for a new node, the server assigns its ID"
                )));
            }
        }
    }
    let mut created = Vec::new();
    let mut next = || loop {
        let id = random_node_id();
        if used.lock().unwrap_or_else(|e| e.into_inner()).insert(id) {
            created.push(id.to_string());
            return id;
        }
    };
    let mut nodes = Vec::with_capacity(roots.len());
    for root in roots {
        nodes.push(root.try_assign_ids(&mut next).map_err(invalid)?);
    }
    Ok((nodes, created))
}

// The design system's head and the writes to heads ----------------------------------------

/// The project's design system doc and its head, `None` for a project that has
/// none. A project has at most one.
async fn design_system_row(
    tx: &mut Tx,
    project_id: &str,
    lock: bool,
) -> Result<Option<(String, Option<String>)>, ApiError> {
    let lock = if lock { " FOR UPDATE" } else { "" };
    sqlx::query_as(&format!(
        "SELECT id::text, head FROM docs \
         WHERE project_id = $1::uuid AND kind = 'design-system' AND deleted_at IS NULL{lock}"
    ))
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// What the head of a design system holds: its tokens, none for a design
/// system of components only, and its components.
#[derive(Clone, Default)]
pub(crate) struct SystemHead {
    pub(crate) tokens: Option<TokenTree>,
    pub(crate) library: Library,
}

impl SystemHead {
    /// The token tree in every mode, as the reads of a design answer it.
    pub(crate) fn tokens_json(&self) -> Option<Value> {
        self.tokens.as_ref().map(TokenTree::to_json)
    }

    /// Every component of the head.
    pub(crate) fn components_json(&self) -> Vec<Value> {
        nodes_values(&self.library.0)
    }

    /// The components that `pages` use, as the head defines them.
    pub(crate) fn components_used_by(&self, pages: &[Node]) -> Vec<Value> {
        let used = used_by(pages, &self.library);
        nodes_values(&self.library.only(&used))
    }
}

/// Reads the design system's head chunk; a design system with no head holds
/// nothing.
pub(crate) async fn read_system_head(
    state: &AppState,
    workspace_id: &str,
    doc_id: &str,
    head: Option<&str>,
) -> Result<SystemHead, ApiError> {
    let Some(head) = head else {
        return Ok(SystemHead::default());
    };
    let text = read_chunk(state, workspace_id, head).await?;
    let what = format!("the head of the design system {doc_id}");
    let (tokens, components) = read_design_system(&text, &what)?;
    Ok(SystemHead {
        tokens,
        library: Library::new(components),
    })
}

/// The head of the project's design system, which the designs of the project
/// resolve their instances and tokens against. A project without a design
/// system has an empty one.
pub(crate) async fn system_head_of_project(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
) -> Result<SystemHead, ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    let found = design_system_row(&mut tx, project_id, false).await?;
    tx.commit().await.map_err(ApiError::database)?;
    match found {
        Some((doc_id, head)) => {
            read_system_head(state, workspace_id, &doc_id, head.as_deref()).await
        }
        None => Ok(SystemHead::default()),
    }
}

/// The version of the project's design system that its designs resolve
/// against now: its head, `None` for a project without one or with an empty one.
pub(crate) async fn system_version_of_project(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
) -> Result<Option<String>, ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    let found = design_system_row(&mut tx, project_id, false).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(found.and_then(|(_, head)| head))
}

/// The project's design system doc with its row locked, so that its imports
/// run one after the other; 404 `no_design_system` for a project without one.
pub(crate) async fn locked_design_system(
    tx: &mut Tx,
    project_id: &str,
) -> Result<(String, Option<String>), ApiError> {
    design_system_row(tx, project_id, true)
        .await?
        .ok_or(ApiError::Refused(StatusCode::NOT_FOUND, "no_design_system"))
}

/// Makes `text` the head of the design system `doc_id` in the transaction of
/// an import (whose doc row is locked): stores the chunk, lists it in the
/// indexes, sets the head, adds the row of `doc_changes` and emits
/// `doc.changed`. Returns the new head.
pub(crate) async fn write_system_head(
    state: &AppState,
    tx: &mut Tx,
    target: &SystemTarget<'_>,
    operations: &Value,
    text: String,
) -> Result<String, ApiError> {
    let head = store_text(state, target.workspace_id, &text).await?;
    index_snapshot(
        tx,
        target.workspace_id,
        &head,
        &Snapshot::new(text, Vec::new()),
    )
    .await?;
    sqlx::query("UPDATE docs SET head = $2 WHERE id = $1::uuid")
        .bind(target.doc_id)
        .bind(&head)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::database)?;
    log_change(
        tx,
        target.workspace_id,
        target.doc_id,
        target.agent,
        operations,
        target.head_before,
        &head,
    )
    .await?;
    live::emit(
        tx,
        target.workspace_id,
        target.project_id,
        "doc.changed",
        &json!({ "doc_id": target.doc_id, "head": head }),
    )
    .await?;
    Ok(head)
}

/// The design system an import writes to.
pub(crate) struct SystemTarget<'a> {
    pub(crate) workspace_id: &'a str,
    pub(crate) project_id: &'a str,
    pub(crate) doc_id: &'a str,
    pub(crate) head_before: Option<&'a str>,
    pub(crate) agent: &'a Value,
}

/// Applies a batch of operations to the head of a design, all or nothing, in
/// the transaction of the request, whose doc row is locked. Instances are
/// checked against the design system's head. The new head lists the chunk of
/// every page and board: a board the batch did not change keeps its chunk, so
/// only the changed boards and the new manifest are stored. The write adds one
/// row of `doc_changes` that lists the changed boards, and emits
/// `doc.changed`. The caller commits. A batch that changes nothing writes
/// nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn edit_head(
    state: &AppState,
    tx: &mut Tx,
    workspace_id: &str,
    project_id: &str,
    doc_id: &str,
    head_before: Option<&str>,
    agent: &Value,
    sent: Vec<Value>,
    ops: Vec<Operation>,
    board: Option<String>,
) -> Result<Vec<OpResultBody>, DocError> {
    let head = match head_before {
        Some(chunk) => {
            let what = format!("the head of doc {doc_id}");
            boards::read_head_design(state, workspace_id, chunk, &what).await?
        }
        None => boards::HeadDesign {
            manifest: boards::Manifest { pages: Vec::new() },
            pages: Vec::new(),
            revisions: HashMap::new(),
        },
    };
    let (system, system_key) = match design_system_row(tx, project_id, false).await? {
        Some((system_id, system_head)) => (
            read_system_head(state, workspace_id, &system_id, system_head.as_deref()).await?,
            system_head,
        ),
        None => (SystemHead::default(), None),
    };

    let manifest = head.manifest;
    let mut doc = DesignDoc::new(head.pages, head.revisions);
    let scope = match board {
        None => None,
        Some(board) => Some(
            board
                .parse::<NodeId>()
                .ok()
                .filter(|board| doc.is_board(*board))
                .ok_or(ApiError::Refused(StatusCode::NOT_FOUND, "unknown_board"))?,
        ),
    };
    let mut used = all_ids(doc.pages());
    used.extend(all_ids(&system.library.0));
    let mut new_ids = || loop {
        let id = random_node_id();
        if used.insert(id) {
            return id;
        }
    };
    let check = |nodes: &[Node]| check_instances(nodes, &system.library);
    let applied = match doc.apply(&ops, scope, &mut new_ids, &check) {
        Ok(applied) => applied,
        Err(ApplyError::Conflict(conflict)) => return Err(DocError::Conflict(conflict)),
        Err(ApplyError::Invalid { index, message }) => {
            return Err(DocError::Invalid { index, message });
        }
    };
    let results = results_of(&applied);
    if results.is_empty() {
        return Ok(results);
    }

    let (snapshot, boards) =
        boards::plan_head_write(&manifest, doc.pages(), doc.revisions()).into_snapshot();
    // The changed chunks first, then the manifest that names them.
    let texts: Vec<String> = snapshot
        .parts
        .iter()
        .map(|part| part.text.clone())
        .collect();
    let (store, workspace) = (state.clone(), workspace_id.to_string());
    boards::each_concurrently(texts, move |text| {
        let (store, workspace) = (store.clone(), workspace.clone());
        async move { store_text(&store, &workspace, &text).await.map(|_| ()) }
    })
    .await?;
    let head_after = store_text(state, workspace_id, &snapshot.html).await?;
    index_snapshot(tx, workspace_id, &head_after, &snapshot).await?;
    // The boxes of the boards this write changed, for the outline to read.
    let changed: HashSet<&str> = boards
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|change| change["after"].as_str())
        .collect();
    let changed_boards: Vec<(String, String)> = snapshot
        .parts
        .iter()
        .filter(|part| changed.contains(part.hash.as_str()))
        .map(|part| (part.hash.clone(), part.text.clone()))
        .collect();
    if !changed_boards.is_empty() {
        let (tokens, components) = (system.tokens_json(), nodes_values(&system.library.0));
        previews::store(
            state,
            workspace_id,
            system_key.clone(),
            tokens.clone(),
            components.clone(),
            changed_boards.clone(),
        );
        placeholders::store_boxes(
            state,
            workspace_id,
            system_key,
            tokens,
            components,
            changed_boards,
        );
    }
    sqlx::query("UPDATE docs SET head = $2 WHERE id = $1::uuid")
        .bind(doc_id)
        .bind(&head_after)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::database)?;
    log_change_of_boards(
        tx,
        workspace_id,
        doc_id,
        agent,
        &Value::Array(sent),
        head_before,
        &head_after,
        Some(&boards),
        None,
    )
    .await?;
    live::emit(
        tx,
        workspace_id,
        project_id,
        "doc.changed",
        &json!({ "doc_id": doc_id, "head": head_after }),
    )
    .await?;
    Ok(results)
}

#[derive(Serialize, ToSchema)]
pub struct ImportedHeadComponents {
    /// The project's design system doc.
    pub doc_id: String,
    /// The design system's new head.
    pub head: String,
    /// The components as tags, with every ID, in the order written.
    pub tags: String,
    /// The components as JSON nodes of type `component`, in the order written.
    #[schema(value_type = Vec<Object>)]
    pub components: Vec<Value>,
}

/// Replaces the components of the project's design system head, all or
/// nothing; the tokens stay. The write makes a new head,
/// adds a row of `doc_changes` and emits `doc.changed`; tags that are refused
/// change, log and emit nothing. Designs resolve their instances against the
/// new head from their next read on.
#[utoipa::path(
    put,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/components",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = ImportTagsRequest,
    responses(
        (status = 200, description = "The components as stored, with the IDs the server assigned", body = ImportedHeadComponents),
        (status = 400, description = "`tags` is missing (`invalid_request`), or the tags are refused: they do not parse, a nested instance or a slot default names no component of the import, or a component places itself (`invalid_components`, with a message)", body = InvalidDesignBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such project in this workspace, or the project has no design system doc (`no_design_system`)", body = ErrorBody),
    )
)]
pub async fn import_head_components(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<ImportedHeadComponents>, DesignError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let workspace_id = request.workspace_id;
    let sent = parse_body::<ImportTagsRequest>(&body);
    let mut tx = state.workspace_tx(&workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let (doc_id, head_before) = locked_design_system(&mut tx, &project_id).await?;
    let tags = sent?
        .tags
        .ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let roots = parse_import(&tags, "invalid_components", NodeKind::Component)?;

    let current = read_system_head(&state, &workspace_id, &doc_id, head_before.as_deref()).await?;
    let known = all_ids(&current.library.0);
    let used: UsedIds = Arc::new(Mutex::new(known.clone()));
    let (components, _) = assign(
        roots,
        &known,
        &used,
        "invalid_components",
        "the design system's head",
    )?;
    check_components(&components).map_err(|message| DesignError::Invalid {
        code: "invalid_components",
        message,
    })?;
    let values = nodes_values(&components);
    let agent = identity.to_json();
    let target = SystemTarget {
        workspace_id: &workspace_id,
        project_id: &project_id,
        doc_id: &doc_id,
        head_before: head_before.as_deref(),
        agent: &agent,
    };
    let text = design_system_text(current.tokens.as_ref(), &components);
    let operations = json!([{ "op": "import_components", "components": values }]);
    let head = write_system_head(&state, &mut tx, &target, &operations, text).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&workspace_id, &project_id);
    Ok(Json(ImportedHeadComponents {
        doc_id,
        head,
        tags: tags_of(&components),
        components: values,
    }))
}

// Usage -----------------------------------------------------------------------------------------

/// A board that uses a component.
#[derive(Serialize, ToSchema)]
pub struct UsageBoard {
    pub doc_id: String,
    pub doc_title: String,
    pub page_id: String,
    pub page_name: String,
    pub board_id: String,
    pub board_name: String,
    /// The IDs of the components the board uses.
    pub components: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct UsageResponse {
    pub project_id: String,
    /// The component that was asked for; `null` for every component.
    pub component: Option<String>,
    pub boards: Vec<UsageBoard>,
}

/// Lists every board of the heads of the project's design docs that uses the
/// component: through an instance anywhere in the board, a slot value of an
/// instance, an instance in the tree of another component that the board uses,
/// or the default of a slot. Without `?component=`, every board that uses any
/// component is listed. Every head is read against the design system's head.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/components/usage",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("component" = Option<String>, Query, description = "The component's node ID; without it, every board that uses a component"),
    ),
    responses(
        (status = 200, description = "The boards that use the component", body = UsageResponse),
        (status = 400, description = "`component` is no node ID (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace, or the design system's head does not define the component (`unknown_component`)", body = ErrorBody),
    )
)]
pub async fn component_usage(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<UsageResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let component: Option<NodeId> = match query.get("component") {
        Some(text) => Some(
            text.parse()
                .map_err(|_| ApiError::bad_request("invalid_request"))?,
        ),
        None => None,
    };
    let system = design_system_row(&mut tx, &project_id, false).await?;
    let docs = head_designs(&mut tx, &project_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let system = match system {
        Some((doc_id, head)) => {
            read_system_head(&state, workspace_id, &doc_id, head.as_deref()).await?
        }
        None => SystemHead::default(),
    };
    if let Some(component) = &component
        && system.library.get(component).is_none()
    {
        return Err(ApiError::Refused(
            StatusCode::NOT_FOUND,
            "unknown_component",
        ));
    }

    let mut boards = Vec::new();
    for design in docs {
        let pages = boards::read_pages(
            &state,
            workspace_id,
            &design.chunk,
            &format!("the head of doc {}", design.doc_id),
        )
        .await?;
        for page in &pages {
            for board in page.children() {
                let used = used_by(std::slice::from_ref(board), &system.library);
                let listed = match &component {
                    Some(component) => used.contains(component),
                    None => !used.is_empty(),
                };
                if listed {
                    boards.push(UsageBoard {
                        doc_id: design.doc_id.clone(),
                        doc_title: design.title.clone(),
                        page_id: page.id().to_string(),
                        page_name: page.name().to_string(),
                        board_id: board.id().to_string(),
                        board_name: board.name().to_string(),
                        components: used.iter().map(NodeId::to_string).collect(),
                    });
                }
            }
        }
    }
    Ok(Json(UsageResponse {
        project_id,
        component: component.map(|component| component.to_string()),
        boards,
    }))
}

#[derive(sqlx::FromRow)]
struct HeadDoc {
    doc_id: String,
    title: String,
    chunk: String,
}

/// The design docs of the project that have a head, oldest doc first.
async fn head_designs(tx: &mut Tx, project_id: &str) -> Result<Vec<HeadDoc>, ApiError> {
    sqlx::query_as(
        "SELECT d.id::text AS doc_id, d.title, d.head AS chunk \
         FROM docs d \
         WHERE d.project_id = $1::uuid AND d.kind = 'design' AND d.head IS NOT NULL \
           AND d.deleted_at IS NULL \
         ORDER BY d.created_at, d.id",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}
