//! Components and instances (US-029, Q27, FR-30, FR-31). The design system doc
//! holds the project's components next to its tokens: an agent replaces all of
//! them in one all-or-nothing import of `<Component>` tags, and a design doc
//! holds pages whose instances place them, imported as `<Page>` tags. The tags
//! are those of docs/formats/design-tags.md, checked by
//! `omni_core::design::tags`.
//!
//! Every import is one row of the draft's log (an `import_components` or an
//! `import_design` operation that holds the nodes as JSON, with the IDs the
//! server assigned, and names the agent) and one `components.imported` or
//! `design.imported` event. The draft in memory keeps the last import, and a
//! draft that is not in memory is rebuilt from its base version and its log.
//! A version of a design system is its token tree and its components as one
//! chunk of JSON, and a version of a design doc its pages.
//!
//! An instance resolves against the change set's draft of the design system,
//! unsaved changes included, and a change set without one resolves against the
//! mainline design system. A design version records the design system version
//! it was saved against, and every view of it uses that version.

use crate::AppState;
use crate::changesets::{parse_body, require_project};
use crate::docs::{
    AgentIdentity, Tx, current_draft, ensure_draft, ensure_loaded, find_doc, load_state,
    random_node_id, require_changeset,
};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::tokens::design_system_of;
use crate::versions::{chunk_of_version, read_chunk, store_version};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use omni_core::design::tags::{self, TagNode};
use omni_core::design::tokens::TokenTree;
use omni_core::design::{Instance, Node, NodeData, NodeId, NodeKind, PropDef, PropDefs, PropValue};
use omni_core::ops::IndexedNode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use utoipa::ToSchema;

/// The key that holds the components next to the tokens in the JSON of a
/// design system version. A token tree has no top-level `$` key of its own.
pub(crate) const COMPONENTS_KEY: &str = "$components";

// The content of versions and drafts ---------------------------------------------------

fn nodes_json(nodes: &[Node]) -> Value {
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

/// The chunk of a design system version: its token tree (an empty one when it
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

/// The tokens and the components that a saved design system version holds.
/// The tokens are `None` for a design system of components only.
pub(crate) fn read_design_system(
    text: &str,
    what: &str,
) -> Result<(Option<TokenTree>, Vec<Node>), ApiError> {
    let unavailable = |why: String| {
        eprintln!("omni-api: {what} holds no design system: {why}");
        ApiError::Unavailable
    };
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

/// The chunk of a design version: its pages.
pub(crate) fn pages_text(pages: &[Node]) -> String {
    json!({ "pages": nodes_json(pages) }).to_string()
}

/// The pages that a saved design version holds.
pub(crate) fn read_pages(text: &str, what: &str) -> Result<Vec<Node>, ApiError> {
    let unavailable = |why: String| {
        eprintln!("omni-api: {what} holds no design: {why}");
        ApiError::Unavailable
    };
    let json: Value = serde_json::from_str(text).map_err(|e| unavailable(e.to_string()))?;
    let pages = json
        .get("pages")
        .ok_or_else(|| unavailable("it has no pages".to_string()))?;
    nodes_from(pages).map_err(unavailable)
}

/// The components of an `import_components` operation of the log, `None` for
/// any other operation.
pub(crate) fn imported_components(op: &str) -> Option<Result<Vec<Node>, String>> {
    let mut op: Value = serde_json::from_str(op).ok()?;
    if op.get("op")?.as_str()? != "import_components" {
        return None;
    }
    Some(nodes_from(&op.get_mut("components")?.take()))
}

/// The pages of an `import_design` operation of the log, `None` for any other
/// operation.
pub(crate) fn imported_pages(op: &str) -> Option<Result<Vec<Node>, String>> {
    let mut op: Value = serde_json::from_str(op).ok()?;
    if op.get("op")?.as_str()? != "import_design" {
        return None;
    }
    Some(nodes_from(&op.get_mut("pages")?.take()))
}

/// The node index of a design version: every node of its pages with its ID,
/// type, parent and own content (everything but its children).
pub(crate) fn index_pages(pages: &[Node]) -> Vec<IndexedNode> {
    fn walk(node: &Node, parent: Option<NodeId>, out: &mut Vec<IndexedNode>) {
        let mut own = serde_json::to_value(node).expect("a node serializes to JSON");
        if let Some(object) = own.as_object_mut() {
            object.remove("children");
        }
        out.push(IndexedNode {
            id: *node.id(),
            kind: node.kind().as_str(),
            parent,
            content: own.to_string(),
        });
        for child in node.children() {
            walk(child, Some(*node.id()), out);
        }
    }
    let mut out = Vec::new();
    for page in pages {
        walk(page, None, &mut out);
    }
    out
}

/// The nodes as tags, one tree after the other.
fn tags_of(nodes: &[Node]) -> String {
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

// The design system a draft or a version resolves against --------------------------------

/// Where the components of a resolution come from.
pub(crate) enum Source {
    /// The change set's draft of the design system.
    Draft { doc_id: String, draft_id: String },
    /// The design system's mainline version.
    Mainline { doc_id: String, version_id: String },
    /// No draft and no mainline version: no components.
    Nothing { doc_id: Option<String> },
}

pub(crate) struct Resolved {
    pub(crate) source: Source,
    pub(crate) library: Library,
}

impl Resolved {
    fn to_json(&self) -> Value {
        match &self.source {
            Source::Draft { doc_id, draft_id } => json!({
                "doc_id": doc_id,
                "source": "draft",
                "draft_id": draft_id,
                "version_id": null,
            }),
            Source::Mainline { doc_id, version_id } => json!({
                "doc_id": doc_id,
                "source": "mainline",
                "draft_id": null,
                "version_id": version_id,
            }),
            Source::Nothing { doc_id } => json!({
                "doc_id": doc_id,
                "source": null,
                "draft_id": null,
                "version_id": null,
            }),
        }
    }
}

/// The components of the design system version `version_id`; none for a
/// version without content.
pub(crate) async fn components_of(
    state: &AppState,
    workspace_id: &str,
    version_id: &str,
) -> Result<Library, ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    let found: Option<(Option<String>,)> =
        sqlx::query_as("SELECT chunk FROM versions WHERE id = $1::uuid")
            .bind(version_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let Some((Some(chunk),)) = found else {
        return Ok(Library::default());
    };
    let text = read_chunk(state, workspace_id, &chunk).await?;
    let (_, components) = read_design_system(&text, &format!("version {version_id}"))?;
    Ok(Library::new(components))
}

/// The design system that the change set's design resolves against: its
/// draft of the design system, unsaved changes included, else the mainline
/// design system.
pub(crate) async fn resolve(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    changeset_id: &str,
) -> Result<Resolved, ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    let found: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT id::text, mainline_version_id::text FROM docs \
         WHERE project_id = $1::uuid AND kind = 'design-system'",
    )
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let Some((doc_id, mainline)) = found else {
        tx.commit().await.map_err(ApiError::database)?;
        return Ok(Resolved {
            source: Source::Nothing { doc_id: None },
            library: Library::default(),
        });
    };
    let draft_id: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM drafts WHERE doc_id = $1::uuid AND changeset_id = $2::uuid",
    )
    .bind(&doc_id)
    .bind(changeset_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;

    if let Some(draft_id) = draft_id {
        // The lock is held for the read only, and never while another draft's
        // lock is.
        let slot = state.drafts.slot(workspace_id, &draft_id);
        let mut held = slot.lock().await;
        let mut tx = state.workspace_tx(workspace_id).await?;
        let draft = current_draft(&mut tx, &draft_id).await?;
        tx.commit().await.map_err(ApiError::database)?;
        ensure_loaded(state, workspace_id, &draft, &mut held).await?;
        let components = held
            .as_ref()
            .map(|loaded| loaded.components.clone())
            .unwrap_or_default();
        return Ok(Resolved {
            source: Source::Draft { doc_id, draft_id },
            library: Library::new(components),
        });
    }
    match mainline {
        Some(version_id) => {
            let library = components_of(state, workspace_id, &version_id).await?;
            Ok(Resolved {
                source: Source::Mainline { doc_id, version_id },
                library,
            })
        }
        None => Ok(Resolved {
            source: Source::Nothing {
                doc_id: Some(doc_id),
            },
            library: Library::default(),
        }),
    }
}

/// What a design doc's draft adds to the read of a draft: its pages as tags
/// and JSON, the design system it resolves against and the components its
/// instances use, as resolved.
pub(crate) struct DraftView {
    pub(crate) tags: String,
    pub(crate) pages: Vec<Value>,
    pub(crate) design_system: Value,
    pub(crate) components: Vec<Value>,
}

pub(crate) async fn view_of_draft(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    changeset_id: &str,
    pages: &[Node],
) -> Result<DraftView, ApiError> {
    let resolved = resolve(state, workspace_id, project_id, changeset_id).await?;
    let used = used_by(pages, &resolved.library);
    Ok(DraftView {
        tags: tags_of(pages),
        pages: nodes_values(pages),
        design_system: resolved.to_json(),
        components: nodes_values(&resolved.library.only(&used)),
    })
}

fn nodes_values(nodes: &[Node]) -> Vec<Value> {
    match nodes_json(nodes) {
        Value::Array(items) => items,
        _ => Vec::new(),
    }
}

/// What a saved version shows: for a design version, its pages and the
/// components its instances use from the design system version it recorded,
/// for a design system version, its components.
pub(crate) struct VersionView {
    pub(crate) tags: Option<String>,
    pub(crate) pages: Option<Vec<Value>>,
    pub(crate) components: Vec<Value>,
}

pub(crate) async fn view_of_version(
    state: &AppState,
    workspace_id: &str,
    kind: &str,
    version_id: &str,
    chunk: Option<&str>,
    design_system_version_id: Option<&str>,
) -> Result<Option<VersionView>, ApiError> {
    match kind {
        "design" => {
            let pages = match chunk {
                Some(chunk) => {
                    let text = read_chunk(state, workspace_id, chunk).await?;
                    read_pages(&text, &format!("version {version_id}"))?
                }
                None => Vec::new(),
            };
            let library = match design_system_version_id {
                Some(recorded) => components_of(state, workspace_id, recorded).await?,
                None => Library::default(),
            };
            let used = used_by(&pages, &library);
            Ok(Some(VersionView {
                tags: Some(tags_of(&pages)),
                pages: Some(nodes_values(&pages)),
                components: nodes_values(&library.only(&used)),
            }))
        }
        "design-system" => {
            let library = components_of(state, workspace_id, version_id).await?;
            Ok(Some(VersionView {
                tags: None,
                pages: None,
                components: nodes_values(&library.0),
            }))
        }
        _ => Ok(None),
    }
}

/// Saves the change set's draft of the design system first when it has
/// changes that no version holds, and returns the design system version that
/// a design version of the change set records: the version that holds the
/// draft as it is, or, without a draft or with an empty one, the mainline
/// version (`None` when the project has no design system or it has no
/// mainline version yet).
pub(crate) async fn settle_design_system(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    changeset_id: &str,
    agent: &Value,
) -> Result<Option<String>, ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    let found: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT id::text, mainline_version_id::text FROM docs \
         WHERE project_id = $1::uuid AND kind = 'design-system'",
    )
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let Some((doc_id, mainline)) = found else {
        tx.commit().await.map_err(ApiError::database)?;
        return Ok(None);
    };
    let draft_id: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM drafts WHERE doc_id = $1::uuid AND changeset_id = $2::uuid",
    )
    .bind(&doc_id)
    .bind(changeset_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let Some(draft_id) = draft_id else {
        return Ok(mainline);
    };

    // Under the draft's lock, as a save is, so that the version holds whole
    // batches only and a rebase cannot run in between.
    let slot = state.drafts.slot(workspace_id, &draft_id);
    let mut held = slot.lock().await;
    let mut tx = state.workspace_tx(workspace_id).await?;
    let draft = current_draft(&mut tx, &draft_id).await?;
    // The versions that may already hold the draft: the change set's latest
    // version of the design system, and the version the draft started from.
    let latest: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT id::text, chunk FROM versions \
         WHERE doc_id = $1::uuid AND changeset_id = $2::uuid AND NOT removed \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(&doc_id)
    .bind(changeset_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let start = draft
        .content_version_id
        .clone()
        .or_else(|| draft.base_version_id.clone());
    let start_chunk = match &start {
        Some(version) => chunk_of_version(&mut tx, &doc_id, version).await?,
        None => None,
    };
    tx.commit().await.map_err(ApiError::database)?;
    ensure_loaded(state, workspace_id, &draft, &mut held).await?;
    let loaded = held.as_ref().expect("the draft is loaded");
    // A draft with neither tokens nor components has nothing to save, so the
    // design records the mainline version, as it would without a draft.
    let Some(snapshot) = loaded.snapshot() else {
        return Ok(mainline);
    };
    let candidates = latest
        .into_iter()
        .chain(start.map(|version| (version, start_chunk)));
    for (version, chunk) in candidates {
        let Some(chunk) = chunk else { continue };
        if read_chunk(state, workspace_id, &chunk).await? == snapshot.html {
            return Ok(Some(version));
        }
    }
    let version = store_version(
        state,
        workspace_id,
        project_id,
        &draft,
        agent,
        Some(snapshot),
        None,
    )
    .await?;
    Ok(Some(version.id))
}

// The imports ------------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct ImportTagsRequest {
    /// The tags of docs/formats/design-tags.md: `<Component>` tags for the
    /// components of a design system, `<Page>` tags for a design.
    pub tags: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ImportedComponents {
    /// The project's design system doc.
    pub doc_id: String,
    pub changeset_id: String,
    /// The change set's draft of the design system.
    pub draft_id: String,
    /// The components as tags, with every ID, in the order written.
    pub tags: String,
    /// The components as JSON nodes of type `component`, in the order written.
    #[schema(value_type = Vec<Object>)]
    pub components: Vec<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct ImportedDesign {
    pub doc_id: String,
    pub changeset_id: String,
    /// The change set's draft of the design doc.
    pub draft_id: String,
    /// The pages as tags, with every ID.
    pub tags: String,
    /// The pages as JSON nodes, in the order written.
    #[schema(value_type = Vec<Object>)]
    pub pages: Vec<Value>,
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
    used: &crate::docs::UsedIds,
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

/// Replaces the components of the change set's draft of the project's design
/// system doc, all or nothing. A change set without a draft of the doc gets
/// one, based on the doc's mainline version. The import is logged with the
/// agent and emits `components.imported`; tags that are refused change, log
/// and emit nothing.
#[utoipa::path(
    put,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/components",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = ImportTagsRequest,
    responses(
        (status = 200, description = "The components as stored, with the IDs the server assigned", body = ImportedComponents),
        (status = 400, description = "`tags` is missing (`invalid_request`), or the tags are refused: they do not parse, a nested instance or a slot default names no component of the import, or a component places itself (`invalid_components`, with a message)", body = InvalidDesignBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace, or the project has no design system doc (`no_design_system`)", body = ErrorBody),
        (status = 409, description = "The design system has no mainline version, so only its own change set has a draft of it (`no_mainline_version`)", body = ErrorBody),
    )
)]
pub async fn import_components(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<ImportedComponents>, DesignError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let sent = parse_body::<ImportTagsRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_changeset(&mut tx, &project_id, &changeset_id).await?;
    let (doc_id, mainline) = design_system_of(&mut tx, &project_id).await?;
    let tags = sent?
        .tags
        .ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let roots = parse_import(&tags, "invalid_components", NodeKind::Component)?;
    let draft_id = ensure_draft(
        &mut tx,
        &request.workspace_id,
        &project_id,
        &doc_id,
        &changeset_id,
        mainline.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;

    let import = Import {
        workspace_id: request.workspace_id,
        project_id,
        doc_id,
        changeset_id,
        draft_id,
        agent: identity.to_json(),
        roots,
        library: Library::default(),
    };
    // A client that gives up must not leave the draft in memory behind its
    // log, so the import runs to its end whatever happens to this request.
    let answer = tokio::spawn(async move { apply_components(&state, import).await })
        .await
        .map_err(|e| {
            eprintln!("omni-api: a components import failed: {e}");
            ApiError::Unavailable
        })??;
    Ok(Json(answer))
}

/// What an import needs once the tags are read and the draft exists.
struct Import {
    workspace_id: String,
    project_id: String,
    doc_id: String,
    changeset_id: String,
    draft_id: String,
    agent: Value,
    roots: Vec<TagNode>,
    /// The components that a design import resolves against.
    library: Library,
}

async fn apply_components(
    state: &AppState,
    import: Import,
) -> Result<ImportedComponents, DesignError> {
    let Import {
        workspace_id,
        project_id,
        doc_id,
        changeset_id,
        draft_id,
        agent,
        roots,
        ..
    } = import;
    let slot = state.drafts.slot(&workspace_id, &draft_id);
    let mut held = slot.lock().await;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    let draft = current_draft(&mut tx, &draft_id).await?;
    if held.is_none() {
        *held = Some(load_state(state, &mut tx, &workspace_id, &draft).await?);
    }
    let used = state
        .drafts
        .used_ids(&mut tx, &workspace_id, &project_id)
        .await?;
    let loaded = held.as_ref().expect("the draft was just loaded");
    let known = all_ids(&loaded.components);
    let (components, created) = assign(
        roots,
        &known,
        &used,
        "invalid_components",
        "this change set's draft of the design system",
    )?;
    check_components(&components).map_err(|message| DesignError::Invalid {
        code: "invalid_components",
        message,
    })?;
    let seq = loaded.next_seq;
    let values = nodes_values(&components);
    let persisted = async {
        sqlx::query(
            "INSERT INTO draft_ops (workspace_id, draft_id, seq, op, agent, created) \
             VALUES ($1::uuid, $2::uuid, $3, $4::text::jsonb, $5::text::jsonb, $6)",
        )
        .bind(&workspace_id)
        .bind(&draft_id)
        .bind(seq)
        .bind(json!({ "op": "import_components", "components": values }).to_string())
        .bind(agent.to_string())
        .bind(&created)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        live::emit(
            &mut tx,
            &workspace_id,
            &project_id,
            "components.imported",
            &json!({
                "doc_id": doc_id,
                "changeset_id": changeset_id,
                "draft_id": draft_id,
            }),
        )
        .await?;
        tx.commit().await.map_err(ApiError::database)?;
        Ok::<(), ApiError>(())
    }
    .await;
    match persisted {
        Ok(()) => {
            let loaded = held.as_mut().expect("the draft is loaded");
            loaded.components = components;
            loaded.next_seq = seq + 1;
        }
        Err(error) => {
            // The next request loads the draft again from its log.
            *held = None;
            return Err(error.into());
        }
    }
    let tags = tags_of(&held.as_ref().expect("the draft is loaded").components);
    drop(held);
    state.live.announce(&workspace_id, &project_id);
    Ok(ImportedComponents {
        doc_id,
        changeset_id,
        draft_id,
        tags,
        components: values,
    })
}

/// Replaces the pages of the change set's draft of a design doc, all or
/// nothing, assigning IDs to new nodes. Every instance is checked against the
/// components that the draft resolves to: the change set's draft of the design
/// system, or the mainline design system. The import is logged with the agent
/// and emits `design.imported`; tags that are refused change, log and emit
/// nothing.
#[utoipa::path(
    put,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/design",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("doc_id" = String, Path, description = "The design doc's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = ImportTagsRequest,
    responses(
        (status = 200, description = "The pages as stored, with the IDs the server assigned", body = ImportedDesign),
        (status = 400, description = "The doc is no design (`not_a_design`), `tags` is missing (`invalid_request`), or the tags are refused: they do not parse, or an instance does not fit its component (`invalid_design`, with a message)", body = InvalidDesignBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set or doc in this project and workspace", body = ErrorBody),
        (status = 409, description = "The doc has no mainline version, so only its own change set has a draft of it (`no_mainline_version`)", body = ErrorBody),
    )
)]
pub async fn import_design(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id, doc_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<ImportedDesign>, DesignError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let sent = parse_body::<ImportTagsRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_changeset(&mut tx, &project_id, &changeset_id).await?;
    let doc = find_doc(&mut tx, &project_id, &doc_id).await?;
    if doc.kind != "design" {
        return Err(ApiError::bad_request("not_a_design").into());
    }
    let tags = sent?
        .tags
        .ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let roots = parse_import(&tags, "invalid_design", NodeKind::Page)?;
    if roots.is_empty() {
        return Err(DesignError::Invalid {
            code: "invalid_design",
            message: "this import has no tag: write one or more `<Page>` tags".to_string(),
        });
    }
    let draft_id = ensure_draft(
        &mut tx,
        &request.workspace_id,
        &project_id,
        &doc_id,
        &changeset_id,
        doc.mainline_version_id.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    // The components come first, under no lock of the design draft, so that
    // no two draft locks are ever held together.
    let resolved = resolve(&state, &request.workspace_id, &project_id, &changeset_id).await?;

    let import = Import {
        workspace_id: request.workspace_id,
        project_id,
        doc_id,
        changeset_id,
        draft_id,
        agent: identity.to_json(),
        roots,
        library: resolved.library,
    };
    let answer = tokio::spawn(async move { apply_design(&state, import).await })
        .await
        .map_err(|e| {
            eprintln!("omni-api: a design import failed: {e}");
            ApiError::Unavailable
        })??;
    Ok(Json(answer))
}

async fn apply_design(state: &AppState, import: Import) -> Result<ImportedDesign, DesignError> {
    let Import {
        workspace_id,
        project_id,
        doc_id,
        changeset_id,
        draft_id,
        agent,
        roots,
        library,
    } = import;
    let slot = state.drafts.slot(&workspace_id, &draft_id);
    let mut held = slot.lock().await;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    let draft = current_draft(&mut tx, &draft_id).await?;
    if held.is_none() {
        *held = Some(load_state(state, &mut tx, &workspace_id, &draft).await?);
    }
    let used = state
        .drafts
        .used_ids(&mut tx, &workspace_id, &project_id)
        .await?;
    let loaded = held.as_ref().expect("the draft was just loaded");
    let known = all_ids(&loaded.pages);
    let (pages, created) = assign(
        roots,
        &known,
        &used,
        "invalid_design",
        "this change set's draft of the design",
    )?;
    check_instances(&pages, &library).map_err(|message| DesignError::Invalid {
        code: "invalid_design",
        message,
    })?;
    let seq = loaded.next_seq;
    let values = nodes_values(&pages);
    let persisted = async {
        sqlx::query(
            "INSERT INTO draft_ops (workspace_id, draft_id, seq, op, agent, created) \
             VALUES ($1::uuid, $2::uuid, $3, $4::text::jsonb, $5::text::jsonb, $6)",
        )
        .bind(&workspace_id)
        .bind(&draft_id)
        .bind(seq)
        .bind(json!({ "op": "import_design", "pages": values }).to_string())
        .bind(agent.to_string())
        .bind(&created)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        live::emit(
            &mut tx,
            &workspace_id,
            &project_id,
            "design.imported",
            &json!({
                "doc_id": doc_id,
                "changeset_id": changeset_id,
                "draft_id": draft_id,
            }),
        )
        .await?;
        tx.commit().await.map_err(ApiError::database)?;
        Ok::<(), ApiError>(())
    }
    .await;
    match persisted {
        Ok(()) => {
            let loaded = held.as_mut().expect("the draft is loaded");
            loaded.pages = pages;
            loaded.next_seq = seq + 1;
        }
        Err(error) => {
            *held = None;
            return Err(error.into());
        }
    }
    let tags = tags_of(&held.as_ref().expect("the draft is loaded").pages);
    drop(held);
    state.live.announce(&workspace_id, &project_id);
    Ok(ImportedDesign {
        doc_id,
        changeset_id,
        draft_id,
        tags,
        pages: values,
    })
}

// Usage -----------------------------------------------------------------------------------------

/// A board that uses a component.
#[derive(Serialize, ToSchema)]
pub struct UsageBoard {
    pub doc_id: String,
    pub doc_title: String,
    /// The doc's mainline version that was read.
    pub version_id: String,
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

/// Lists every board of the mainline versions of the project's design docs
/// that uses the component: through an instance anywhere in the board, a slot
/// value of an instance, an instance in the tree of another component that the
/// board uses, or the default of a slot. Without `?component=`, every board
/// that uses any component is listed. Drafts are not counted; each version is
/// read against the design system version it recorded.
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
        (status = 404, description = "No such project in this workspace, or the mainline design system does not define the component (`unknown_component`)", body = ErrorBody),
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
    let mainline: Option<String> = sqlx::query_scalar(
        "SELECT mainline_version_id::text FROM docs \
         WHERE project_id = $1::uuid AND kind = 'design-system'",
    )
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .flatten();
    let docs = mainline_designs(&mut tx, &project_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    if let Some(component) = &component {
        let defined = match &mainline {
            Some(version_id) => components_of(&state, workspace_id, version_id)
                .await?
                .get(component)
                .is_some(),
            None => false,
        };
        if !defined {
            return Err(ApiError::Refused(
                StatusCode::NOT_FOUND,
                "unknown_component",
            ));
        }
    }

    let mut libraries: HashMap<String, Library> = HashMap::new();
    let mut boards = Vec::new();
    for design in docs {
        let text = read_chunk(&state, workspace_id, &design.chunk).await?;
        let pages = read_pages(&text, &format!("version {}", design.version_id))?;
        let library = match &design.design_system_version_id {
            Some(recorded) => {
                if !libraries.contains_key(recorded) {
                    let library = components_of(&state, workspace_id, recorded).await?;
                    libraries.insert(recorded.clone(), library);
                }
                libraries[recorded].clone()
            }
            None => Library::default(),
        };
        for page in &pages {
            for board in page.children() {
                let used = used_by(std::slice::from_ref(board), &library);
                let listed = match &component {
                    Some(component) => used.contains(component),
                    None => !used.is_empty(),
                };
                if listed {
                    boards.push(UsageBoard {
                        doc_id: design.doc_id.clone(),
                        doc_title: design.title.clone(),
                        version_id: design.version_id.clone(),
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
struct MainlineDesign {
    doc_id: String,
    title: String,
    version_id: String,
    chunk: String,
    design_system_version_id: Option<String>,
}

/// The design docs of the project that have a mainline version with content,
/// oldest doc first.
async fn mainline_designs(tx: &mut Tx, project_id: &str) -> Result<Vec<MainlineDesign>, ApiError> {
    sqlx::query_as(
        "SELECT d.id::text AS doc_id, d.title, v.id::text AS version_id, v.chunk AS chunk, \
                v.design_system_version_id::text AS design_system_version_id \
         FROM docs d \
         JOIN versions v ON v.workspace_id = d.workspace_id AND v.id = d.mainline_version_id \
         WHERE d.project_id = $1::uuid AND d.kind = 'design' AND v.chunk IS NOT NULL \
         ORDER BY d.created_at, d.id",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}
