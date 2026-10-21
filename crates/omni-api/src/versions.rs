//! Versions: the unchangeable snapshots that reviewers comment on and that get
//! merged. An agent saves a change set's draft of a doc as a version, and
//! removes a doc by saving a version that marks it removed; nothing is deleted,
//! so the doc and every earlier version stay readable.
//!
//! A version records the doc's content once, as a chunk of RustFS named by the
//! BLAKE3 hash of its bytes (`ws/{workspace_id}/chunks/{chunk}`), and indexes
//! its nodes in the tenant table `node_index`: the ID, type, parent and
//! own-content fingerprint of each, in document order. The fingerprint hashes
//! what the node itself holds, so a change to a child or a move leaves it
//! alone, and comparing two versions is a comparison of two indexes. Every
//! chunk has a row in the chunk index, the tenant table `chunks`, keyed by the
//! workspace and the hash. All three tables are append-only: the API role has
//! no UPDATE privilege on them.
//!
//! A design version is a manifest instead (src/boards.rs): its own chunk lists
//! the chunks of its pages and boards, each stored and indexed like any other
//! chunk, so that a page or a board that a later version does not change is
//! shared with it.

use crate::AppState;
use crate::boards::{self, Manifest};
use crate::changesets::{clear_approval, parse_body};
use crate::comments;
use crate::design;
use crate::docs::{
    AgentIdentity, DraftRow, Tx, current_draft, ensure_loaded, find_doc, find_draft,
};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid, sha256_hex};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use omni_core::hash::blake3_hex;
use omni_core::ops::{IndexedNode, longest_increasing};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

/// The columns of a version; `created_at` is ISO 8601 text in UTC.
const VERSION_COLUMNS: &str = "v.id::text AS id, v.doc_id::text AS doc_id, \
     v.changeset_id::text AS changeset_id, v.draft_id::text AS draft_id, v.removed, v.chunk, \
     v.agent::text AS agent, v.design_system_version_id::text AS design_system_version_id, \
     v.manifest::text AS manifest, d.kind AS doc_kind, \
     to_char(v.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at";

/// One node of a version's index.
#[derive(Clone, Serialize, ToSchema, sqlx::FromRow)]
pub struct NodeEntry {
    pub id: String,
    /// The block's type: `section`, `heading`, `paragraph` and so on.
    #[serde(rename = "type")]
    #[sqlx(rename = "kind")]
    pub kind: String,
    /// The parent's node ID; `null` at the top level.
    pub parent: Option<String>,
    /// A hash of the node's own content: its type, properties and text, and
    /// nothing else. It does not change when a child or the node's place does.
    pub fingerprint: String,
}

#[derive(Serialize, ToSchema)]
pub struct VersionBody {
    pub id: String,
    pub doc_id: String,
    pub changeset_id: String,
    /// The draft that was saved.
    pub draft_id: Option<String>,
    /// Whether this version marks the doc as removed; it then has no nodes.
    pub removed: bool,
    pub created_at: String,
    /// `{"run", "role", "model", "harness"}`, as the agent declared them.
    #[schema(value_type = Object)]
    pub agent: Value,
    /// For a design version, the version of the design system it was saved
    /// against, which every view of it uses; `null` for other versions.
    pub design_system_version_id: Option<String>,
    /// For a design version, the chunk of each of its pages and boards, so
    /// that a viewer can read one board and an unchanged board is the same
    /// chunk in every version that has it; `null` for other versions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<Manifest>,
    /// The version's nodes in document order.
    pub nodes: Vec<NodeEntry>,
    /// A design version's pages as tags; only when the version is read by ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    /// A design version's pages as JSON nodes; only when read by ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub pages: Option<Vec<Value>>,
    /// The components that a design version's instances use, as the recorded
    /// design system version defines them; for a design system version, all of
    /// its components. Only when the version is read by ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub components: Option<Vec<Value>>,
    /// For a design version, the whole token tree of the design system version
    /// it recorded, in every mode (`null` for a design system of components
    /// only); the design viewer hands it to the renderer with `components`.
    /// Only when the version is read by ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub tokens: Option<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct VersionResponse {
    pub version: VersionBody,
}

/// A version in a doc's list: the version without its node index.
#[derive(Serialize, ToSchema)]
pub struct VersionItem {
    pub id: String,
    pub doc_id: String,
    pub changeset_id: String,
    /// The draft that was saved.
    pub draft_id: Option<String>,
    /// Whether this version marks the doc as removed.
    pub removed: bool,
    pub created_at: String,
    /// `{"run", "role", "model", "harness"}`, as the agent declared them.
    #[schema(value_type = Object)]
    pub agent: Value,
}

#[derive(Serialize, ToSchema)]
pub struct VersionListResponse {
    pub doc_id: String,
    /// Every saved version of the doc in every change set, oldest first.
    pub versions: Vec<VersionItem>,
}

/// A node whose place differs: it moved to another parent, or its order among
/// the siblings that keep their parent changed.
#[derive(Serialize, ToSchema)]
pub struct MovedNode {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub from_parent: Option<String>,
    pub to_parent: Option<String>,
    /// The sibling the node follows in the first version; `null` for a first
    /// child.
    pub from_after: Option<String>,
    /// The sibling the node follows in the second version; `null` for a first
    /// child.
    pub to_after: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DiffBody {
    pub doc_id: String,
    pub from: String,
    pub to: String,
    /// The design system version that the first version recorded; `null` for
    /// versions of other docs.
    pub from_design_system_version_id: Option<String>,
    /// The design system version that the second version recorded.
    pub to_design_system_version_id: Option<String>,
    /// The nodes only the second version has.
    pub added: Vec<NodeEntry>,
    /// The nodes only the first version has.
    pub removed: Vec<NodeEntry>,
    /// The nodes of both whose own content differs, as in the second version.
    pub changed: Vec<NodeEntry>,
    /// The nodes of both whose place differs: another parent, or, for a
    /// reorder among the siblings that keep their parent, the fewest nodes
    /// that account for it.
    pub moved: Vec<MovedNode>,
}

#[derive(Serialize, ToSchema)]
pub struct DiffResponse {
    pub diff: DiffBody,
}

/// What saving a version takes: nothing, but the body must be a JSON object.
/// A version that drops a node with an open comment is refused unless the body
/// says `remove_commented`.
#[derive(Deserialize, ToSchema)]
pub struct SaveVersionRequest {
    /// Save even though the version removes nodes that have open comments (the
    /// comments stay, marked as removed). Left out or `false`, such a save is
    /// refused with 409 `commented_nodes_removed`.
    pub remove_commented: Option<bool>,
}

// Chunks in RustFS ------------------------------------------------------------------

fn chunk_key(workspace_id: &str, chunk: &str) -> String {
    format!("ws/{workspace_id}/chunks/{chunk}")
}

/// Stores `text` as the chunk `chunk`, which is the BLAKE3 hash of the text.
async fn put_chunk(
    bucket: &s3::Bucket,
    workspace_id: &str,
    chunk: &str,
    text: &str,
) -> Result<(), ApiError> {
    let stored = bucket
        .put_object(chunk_key(workspace_id, chunk), text.as_bytes())
        .await
        .map_err(|e| {
            eprintln!("omni-api: cannot store chunk {chunk}: {e}");
            ApiError::Unavailable
        })?;
    if stored.status_code() != 200 {
        eprintln!(
            "omni-api: cannot store chunk {chunk}: storage answered {}",
            stored.status_code()
        );
        return Err(ApiError::Unavailable);
    }
    Ok(())
}

/// A chunk that a version lists besides its own: the page or board of a
/// design, with the node index of the nodes it holds.
pub(crate) struct Part {
    /// The name of the chunk: the BLAKE3 hash of `text`.
    pub(crate) hash: String,
    pub(crate) text: String,
    pub(crate) nodes: Vec<IndexedNode>,
}

impl Part {
    pub(crate) fn new(text: String, nodes: Vec<IndexedNode>) -> Self {
        Self {
            hash: blake3_hex(text.as_bytes()),
            text,
            nodes,
        }
    }
}

/// Stores a version's chunks: its own, named by the hash of its text, and the
/// parts it lists. A chunk is named by its content, so one that the chunk
/// index lists is stored already and is not written again, and a chunk without
/// a version is harmless. Returns the name of the version's own chunk.
pub(crate) async fn store_snapshot(
    state: &AppState,
    workspace_id: &str,
    snapshot: &Snapshot,
) -> Result<String, ApiError> {
    let chunk = blake3_hex(snapshot.html.as_bytes());
    let mut texts: Vec<(&str, &str)> = vec![(chunk.as_str(), snapshot.html.as_str())];
    texts.extend(
        snapshot
            .parts
            .iter()
            .map(|part| (part.hash.as_str(), part.text.as_str())),
    );
    let mut seen = HashSet::new();
    texts.retain(|(hash, _)| seen.insert(*hash));
    let names: Vec<&str> = texts.iter().map(|(hash, _)| *hash).collect();

    let mut tx = state.workspace_tx(workspace_id).await?;
    let known: HashSet<String> =
        sqlx::query_scalar("SELECT hash FROM chunks WHERE hash = ANY($1::text[])")
            .bind(&names)
            .fetch_all(&mut *tx)
            .await
            .map_err(ApiError::database)?
            .into_iter()
            .collect();
    tx.commit().await.map_err(ApiError::database)?;

    let missing: Vec<(String, String)> = texts
        .into_iter()
        .filter(|(hash, _)| !known.contains(*hash))
        .map(|(hash, text)| (hash.to_string(), text.to_string()))
        .collect();
    let bucket = state.bucket.clone();
    let workspace = workspace_id.to_string();
    boards::each_concurrently(missing, move |(hash, text)| {
        let (bucket, workspace) = (bucket.clone(), workspace.clone());
        async move { put_chunk(&bucket, &workspace, &hash, &text).await }
    })
    .await?;
    Ok(chunk)
}

/// The HTML that a chunk holds.
pub(crate) async fn read_chunk(
    state: &AppState,
    workspace_id: &str,
    chunk: &str,
) -> Result<String, ApiError> {
    let unavailable = |why: String| {
        eprintln!("omni-api: cannot read chunk {chunk}: {why}");
        ApiError::Unavailable
    };
    let object = state
        .bucket
        .get_object(chunk_key(workspace_id, chunk))
        .await
        .map_err(|e| unavailable(e.to_string()))?;
    if object.status_code() != 200 {
        return Err(unavailable(format!(
            "storage answered {}",
            object.status_code()
        )));
    }
    String::from_utf8(object.to_vec()).map_err(|e| unavailable(e.to_string()))
}

// Looking things up ------------------------------------------------------------------

#[derive(sqlx::FromRow)]
pub(crate) struct VersionRow {
    pub(crate) id: String,
    pub(crate) doc_id: String,
    changeset_id: String,
    draft_id: Option<String>,
    removed: bool,
    pub(crate) chunk: Option<String>,
    agent: String,
    pub(crate) design_system_version_id: Option<String>,
    /// A design version's manifest as JSON text.
    manifest: Option<String>,
    pub(crate) doc_kind: String,
    created_at: String,
}

impl VersionRow {
    fn manifest(&self) -> Option<Manifest> {
        self.manifest
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
    }
}

/// The version `version_id`, 404 when it is not there or its doc is not in
/// this project.
pub(crate) async fn find_version(
    tx: &mut Tx,
    project_id: &str,
    version_id: &str,
) -> Result<VersionRow, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {VERSION_COLUMNS} FROM versions v \
         JOIN docs d ON d.workspace_id = v.workspace_id AND d.id = v.doc_id \
         WHERE v.id = $1::uuid AND d.project_id = $2::uuid"
    ))
    .bind(version_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)
}

/// The chunk of a version of `doc_id`, `None` for a version without content.
/// 404 when the doc has no such version.
pub(crate) async fn chunk_of_version(
    tx: &mut Tx,
    doc_id: &str,
    version_id: &str,
) -> Result<Option<String>, ApiError> {
    let found: Option<(Option<String>,)> =
        sqlx::query_as("SELECT chunk FROM versions WHERE id = $1::uuid AND doc_id = $2::uuid")
            .bind(version_id)
            .bind(doc_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(ApiError::database)?;
    found.map(|(chunk,)| chunk).ok_or(ApiError::NotFound)
}

async fn nodes_of(tx: &mut Tx, chunk: Option<&str>) -> Result<Vec<NodeEntry>, ApiError> {
    let Some(chunk) = chunk else {
        return Ok(Vec::new());
    };
    sqlx::query_as(
        "SELECT node AS id, type AS kind, parent, fingerprint FROM node_index \
         WHERE chunk = $1 ORDER BY ordinal, node",
    )
    .bind(chunk)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}

#[derive(sqlx::FromRow)]
struct ChunkNode {
    chunk: String,
    #[sqlx(flatten)]
    entry: NodeEntry,
}

/// The nodes of a design version in document order: each page, then the
/// nodes of each of its boards, read from the node index of their chunks. A
/// board's root has no parent in its chunk, so the page is its parent here.
async fn manifest_nodes(tx: &mut Tx, manifest: &Manifest) -> Result<Vec<NodeEntry>, ApiError> {
    let chunks: Vec<&str> = manifest
        .pages
        .iter()
        .flat_map(|page| {
            std::iter::once(page.chunk.as_str())
                .chain(page.boards.iter().map(|board| board.chunk.as_str()))
        })
        .collect();
    let rows: Vec<ChunkNode> = sqlx::query_as(
        "SELECT chunk, node AS id, type AS kind, parent, fingerprint FROM node_index \
         WHERE chunk = ANY($1::text[]) ORDER BY chunk, ordinal, node",
    )
    .bind(&chunks)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let mut by_chunk: HashMap<&str, Vec<&NodeEntry>> = HashMap::new();
    for row in &rows {
        by_chunk.entry(&row.chunk).or_default().push(&row.entry);
    }
    let mut nodes = Vec::with_capacity(rows.len());
    let listed = |chunk: &str| by_chunk.get(chunk).cloned().unwrap_or_default();
    for page in &manifest.pages {
        nodes.extend(listed(&page.chunk).into_iter().cloned());
        for board in &page.boards {
            for (index, node) in listed(&board.chunk).into_iter().enumerate() {
                let mut node = node.clone();
                if index == 0 {
                    node.parent = Some(page.id.clone());
                }
                nodes.push(node);
            }
        }
    }
    Ok(nodes)
}

/// The nodes of a version in document order.
pub(crate) async fn nodes_of_version(
    tx: &mut Tx,
    row: &VersionRow,
) -> Result<Vec<NodeEntry>, ApiError> {
    match row.manifest() {
        Some(manifest) => manifest_nodes(tx, &manifest).await,
        None => nodes_of(tx, row.chunk.as_deref()).await,
    }
}

/// The nodes of the version `version_id` in document order, 404 when the
/// workspace has no such version.
pub(crate) async fn nodes_of_version_id(
    tx: &mut Tx,
    version_id: &str,
) -> Result<Vec<NodeEntry>, ApiError> {
    let row: VersionRow = sqlx::query_as(&format!(
        "SELECT {VERSION_COLUMNS} FROM versions v \
         JOIN docs d ON d.workspace_id = v.workspace_id AND d.id = v.doc_id \
         WHERE v.id = $1::uuid"
    ))
    .bind(version_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    nodes_of_version(tx, &row).await
}

async fn version_body(tx: &mut Tx, row: VersionRow) -> Result<VersionBody, ApiError> {
    let nodes = nodes_of_version(tx, &row).await?;
    let manifest = row.manifest();
    Ok(VersionBody {
        id: row.id,
        doc_id: row.doc_id,
        changeset_id: row.changeset_id,
        draft_id: row.draft_id,
        removed: row.removed,
        created_at: row.created_at,
        agent: serde_json::from_str(&row.agent).unwrap_or(Value::Null),
        design_system_version_id: row.design_system_version_id,
        manifest,
        nodes,
        tags: None,
        pages: None,
        components: None,
        tokens: None,
    })
}

// Saving ------------------------------------------------------------------------------

/// What a version holds: the doc's HTML and its node index, or nothing for a
/// removal. A design version holds its manifest as `html` and the chunks it
/// lists as `parts`, each with its own node index.
pub(crate) struct Snapshot {
    pub(crate) html: String,
    pub(crate) nodes: Vec<IndexedNode>,
    pub(crate) parts: Vec<Part>,
    /// A design version's manifest, which `html` is the text of.
    pub(crate) manifest: Option<Manifest>,
}

impl Snapshot {
    /// The IDs of every node that a version of this snapshot indexes.
    pub(crate) fn node_ids(&self) -> HashSet<String> {
        self.nodes
            .iter()
            .chain(self.parts.iter().flat_map(|part| part.nodes.iter()))
            .map(|node| node.id.to_string())
            .collect()
    }

    /// A version whose content is the one chunk `html`.
    pub(crate) fn new(html: String, nodes: Vec<IndexedNode>) -> Self {
        Self {
            html,
            nodes,
            parts: Vec::new(),
            manifest: None,
        }
    }
}

/// Lists the chunks in the chunk index, and the nodes of those that were not
/// in it, in the node index. A chunk that is listed has these very nodes
/// indexed already: its name is the hash of the content they come from.
async fn index_chunks(
    tx: &mut Tx,
    workspace_id: &str,
    chunks: &[(&str, &[IndexedNode])],
) -> Result<(), ApiError> {
    let names: Vec<&str> = chunks.iter().map(|(chunk, _)| *chunk).collect();
    let added: HashSet<String> = sqlx::query_scalar(
        "INSERT INTO chunks (workspace_id, hash) \
         SELECT $1::uuid, hash FROM unnest($2::text[]) AS c (hash) \
         ON CONFLICT (workspace_id, hash) DO NOTHING \
         RETURNING hash",
    )
    .bind(workspace_id)
    .bind(&names)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .into_iter()
    .collect();
    let mut indexed_chunks = Vec::new();
    let mut ids = Vec::new();
    let mut kinds = Vec::new();
    let mut parents = Vec::new();
    let mut fingerprints = Vec::new();
    let mut ordinals = Vec::new();
    let mut done = HashSet::new();
    for (chunk, nodes) in chunks {
        if !added.contains(*chunk) || !done.insert(*chunk) {
            continue;
        }
        for (ordinal, node) in nodes.iter().enumerate() {
            indexed_chunks.push(*chunk);
            ids.push(node.id.to_string());
            kinds.push(node.kind);
            parents.push(node.parent.map(|parent| parent.to_string()));
            fingerprints.push(sha256_hex(&node.content));
            ordinals.push(ordinal as i32);
        }
    }
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO node_index (workspace_id, chunk, node, type, parent, fingerprint, ordinal) \
         SELECT $1::uuid, chunk, node, type, parent, fingerprint, ordinal \
         FROM unnest($2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::int[]) \
              AS n (chunk, node, type, parent, fingerprint, ordinal) \
         ON CONFLICT (workspace_id, chunk, node) DO NOTHING",
    )
    .bind(workspace_id)
    .bind(&indexed_chunks)
    .bind(&ids)
    .bind(&kinds)
    .bind(&parents)
    .bind(&fingerprints)
    .bind(&ordinals)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(())
}

/// Inserts a version of the draft, and the chunk index and node index of its
/// chunks when they are new, in the caller's transaction; returns the version's
/// ID. `chunk` and `snapshot` are both `None` for a removal.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_version(
    tx: &mut Tx,
    workspace_id: &str,
    draft: &DraftRow,
    agent: &Value,
    chunk: Option<&str>,
    snapshot: Option<&Snapshot>,
    design_system_version: Option<&str>,
) -> Result<String, ApiError> {
    let manifest = snapshot
        .and_then(|snapshot| snapshot.manifest.as_ref())
        .map(|manifest| serde_json::to_string(manifest).expect("a manifest serializes to JSON"));
    let id: String = sqlx::query_scalar(
        "INSERT INTO versions (workspace_id, doc_id, changeset_id, draft_id, removed, chunk, agent, \
                               design_system_version_id, manifest) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, $6, $7::text::jsonb, $8::uuid, \
                 $9::text::jsonb) \
         RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(&draft.doc_id)
    .bind(&draft.changeset_id)
    .bind(&draft.id)
    .bind(chunk.is_none())
    .bind(chunk)
    .bind(agent.to_string())
    .bind(design_system_version)
    .bind(manifest)
    .fetch_one(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    if let (Some(chunk), Some(snapshot)) = (chunk, snapshot) {
        let mut chunks: Vec<(&str, &[IndexedNode])> = vec![(chunk, &snapshot.nodes)];
        chunks.extend(
            snapshot
                .parts
                .iter()
                .map(|part| (part.hash.as_str(), part.nodes.as_slice())),
        );
        index_chunks(tx, workspace_id, &chunks).await?;
    }
    Ok(id)
}

/// Emits `version.saved` for a version that the transaction inserted.
pub(crate) async fn emit_version_saved(
    tx: &mut Tx,
    workspace_id: &str,
    project_id: &str,
    draft: &DraftRow,
    agent: &Value,
    version_id: &str,
    removed: bool,
) -> Result<(), ApiError> {
    live::emit(
        tx,
        workspace_id,
        project_id,
        "version.saved",
        &json!({
            "version_id": version_id,
            "doc_id": draft.doc_id,
            "changeset_id": draft.changeset_id,
            "draft_id": draft.id,
            "removed": removed,
            "agent": agent,
        }),
    )
    .await?;
    Ok(())
}

/// Writes a version of the draft: the chunk first (it is named by its content,
/// so a chunk without a version is harmless), then, in one transaction, the
/// version, the chunk's node index when it is new, the move out of an approval
/// and the `version.saved` event. A design version records the design system
/// version it was saved against.
pub(crate) async fn store_version(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    draft: &DraftRow,
    agent: &Value,
    snapshot: Option<Snapshot>,
    design_system_version: Option<&str>,
) -> Result<VersionBody, ApiError> {
    let removed = snapshot.is_none();
    let chunk = match &snapshot {
        Some(snapshot) => Some(store_snapshot(state, workspace_id, snapshot).await?),
        None => None,
    };
    let mut tx = state.workspace_tx(workspace_id).await?;
    let id = insert_version(
        &mut tx,
        workspace_id,
        draft,
        agent,
        chunk.as_deref(),
        snapshot.as_ref(),
        design_system_version,
    )
    .await?;
    clear_approval(&mut tx, workspace_id, project_id, &draft.changeset_id).await?;
    emit_version_saved(
        &mut tx,
        workspace_id,
        project_id,
        draft,
        agent,
        &id,
        removed,
    )
    .await?;
    let row = find_version(&mut tx, project_id, &id).await?;
    let version = version_body(&mut tx, row).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, project_id);
    Ok(version)
}

/// Saves a change set's draft of a doc as a version: an unchangeable snapshot
/// with each node's ID, type, parent and fingerprint. The draft stays open for
/// more operations. A version does not become the doc's mainline version; that
/// happens when its change set merges. Saving while the change set is
/// `approved` or `needs_rebase` clears the approval: the change set moves back
/// to `in_review`.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/versions",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = SaveVersionRequest,
    responses(
        (status = 201, description = "The saved version", body = VersionResponse),
        (status = 400, description = "The body is not a JSON object", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "The change set has no draft of this doc in this project and workspace", body = ErrorBody),
        (status = 409, description = "The draft of a design system has neither tokens nor components yet (`no_tokens`): import them first; or the change set emptied a design system that has a mainline version, so a design cannot be saved against it (`design_system_emptied`); or the version removes nodes that have open comments (`commented_nodes_removed`, with the `comments` that name them) and the body does not say `remove_commented: true`", body = ErrorBody),
    )
)]
pub async fn save_version(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id, doc_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<VersionResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let sent = parse_body::<SaveVersionRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let draft = find_draft(&mut tx, &project_id, &changeset_id, &doc_id).await?;
    let kind: String = sqlx::query_scalar("SELECT kind FROM docs WHERE id = $1::uuid")
        .bind(&doc_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let remove_commented = sent?.remove_commented == Some(true);

    // A design version records the design system version it uses, which is
    // saved first when the change set's draft of it has changes that no
    // version holds. That takes the design system draft's lock, so it is done
    // before this draft's lock is taken.
    let design_system_version = if kind == "design" {
        design::settle_design_system(
            &state,
            &request.workspace_id,
            &project_id,
            &changeset_id,
            &identity.to_json(),
        )
        .await?
    } else {
        None
    };

    // The snapshot is taken under the draft's lock, so it holds whole batches
    // only: the ones that are committed. The lock is held until the version is
    // stored, so that a rebase cannot run in between and leave a version of
    // the draft from before it in a change set that it has rebased.
    let slot = state.drafts.slot(&request.workspace_id, &draft.id);
    let mut held = slot.lock().await;
    // A rebase that held the lock meanwhile has rewritten the draft's row.
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let draft = current_draft(&mut tx, &draft.id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    ensure_loaded(&state, &request.workspace_id, &draft, &mut held).await?;
    let loaded = held.as_ref().expect("the draft is loaded");
    // A design system's version is its token tree and its components as one
    // chunk of JSON; the tokens are no nodes, so it has no node index.
    let Some(snapshot) = loaded.snapshot() else {
        return Err(ApiError::conflict("no_tokens"));
    };
    // The comments on nodes that this version drops are looked at under the
    // draft's lock, with the draft that the version is made of.
    comments::refuse_commented_removals(
        &state,
        &request.workspace_id,
        &project_id,
        &draft,
        &snapshot.node_ids(),
        remove_commented,
    )
    .await?;
    let version = store_version(
        &state,
        &request.workspace_id,
        &project_id,
        &draft,
        &identity.to_json(),
        Some(snapshot),
        design_system_version.as_deref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(VersionResponse { version })))
}

/// Removes a doc in a change set: saves a version that marks the doc removed,
/// with no nodes. Nothing is deleted: the doc and its earlier versions stay.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/remove",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = SaveVersionRequest,
    responses(
        (status = 201, description = "The version that marks the doc removed", body = VersionResponse),
        (status = 400, description = "The body is not a JSON object", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "The change set has no draft of this doc in this project and workspace", body = ErrorBody),
        (status = 409, description = "The doc has nodes with open comments (`commented_nodes_removed`, with the `comments` that name them) and the body does not say `remove_commented: true`", body = ErrorBody),
    )
)]
pub async fn remove_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id, doc_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<VersionResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let sent = parse_body::<SaveVersionRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let draft = find_draft(&mut tx, &project_id, &changeset_id, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let remove_commented = sent?.remove_commented == Some(true);
    // Under the draft's lock, as a save is, so that a rebase either comes
    // first (the removal then follows its version) or finds the change set
    // moved out of `needs_rebase` by the removal.
    let slot = state.drafts.slot(&request.workspace_id, &draft.id);
    let _held = slot.lock().await;
    // A removal drops every node of the doc, so it needs the same go-ahead. A
    // rebase that held the lock meanwhile has rewritten the draft's row.
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let current = current_draft(&mut tx, &draft.id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    comments::refuse_commented_removals(
        &state,
        &request.workspace_id,
        &project_id,
        &current,
        &HashSet::new(),
        remove_commented,
    )
    .await?;
    let version = store_version(
        &state,
        &request.workspace_id,
        &project_id,
        &draft,
        &identity.to_json(),
        None,
        None,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(VersionResponse { version })))
}

// Reading and comparing -----------------------------------------------------------------

/// A saved version with its node index.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("version_id" = String, Path, description = "The version's UUID"),
    ),
    responses(
        (status = 200, description = "The version", body = VersionResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such version in this project and workspace", body = ErrorBody),
    )
)]
pub async fn show_version(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, version_id)): Path<(String, String, String)>,
) -> Result<Json<VersionResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let version_id = parse_uuid(&version_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row = find_version(&mut tx, &project_id, &version_id).await?;
    let kind = row.doc_kind.clone();
    let chunk = row.chunk.clone();
    let mut version = version_body(&mut tx, row).await?;
    tx.commit().await.map_err(ApiError::database)?;
    // A design version is shown with the design system version it recorded,
    // whatever the mainline design system is now.
    let view = design::view_of_version(
        &state,
        &request.workspace_id,
        &kind,
        &version.id,
        chunk.as_deref(),
        version.design_system_version_id.as_deref(),
    )
    .await?;
    if let Some(view) = view {
        version.tags = view.tags;
        version.pages = view.pages;
        version.components = Some(view.components);
        version.tokens = view.tokens;
    }
    Ok(Json(VersionResponse { version }))
}

/// Every saved version of a doc, in every change set and the removal of the
/// doc included, oldest first.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/versions",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
    ),
    responses(
        (status = 200, description = "The doc's versions, without their nodes", body = VersionListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace", body = ErrorBody),
    )
)]
pub async fn list_versions(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
) -> Result<Json<VersionListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let doc = find_doc(&mut tx, &project_id, &doc_id).await?;
    let rows: Vec<VersionRow> = sqlx::query_as(&format!(
        "SELECT {VERSION_COLUMNS} FROM versions v \
         JOIN docs d ON d.workspace_id = v.workspace_id AND d.id = v.doc_id \
         WHERE v.doc_id = $1::uuid ORDER BY v.created_at, v.id"
    ))
    .bind(&doc.id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let versions = rows
        .into_iter()
        .map(|row| VersionItem {
            id: row.id,
            doc_id: row.doc_id,
            changeset_id: row.changeset_id,
            draft_id: row.draft_id,
            removed: row.removed,
            created_at: row.created_at,
            agent: serde_json::from_str(&row.agent).unwrap_or(Value::Null),
        })
        .collect();
    Ok(Json(VersionListResponse {
        doc_id: doc.id,
        versions,
    }))
}

/// What differs between two versions of one doc, from the first to the second:
/// the nodes added and removed, the nodes of both whose own content changed,
/// and the nodes of both that moved to another parent or to another place
/// among their siblings.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/diff/{other_version_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("version_id" = String, Path, description = "The version to compare from"),
        ("other_version_id" = String, Path, description = "The version to compare to"),
    ),
    responses(
        (status = 200, description = "The differences", body = DiffResponse),
        (status = 400, description = "The versions belong to different docs (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such version in this project and workspace", body = ErrorBody),
    )
)]
pub async fn diff_versions(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, from, to)): Path<(String, String, String, String)>,
) -> Result<Json<DiffResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let from = parse_uuid(&from).ok_or(ApiError::NotFound)?;
    let to = parse_uuid(&to).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let first = find_version(&mut tx, &project_id, &from).await?;
    let second = find_version(&mut tx, &project_id, &to).await?;
    if first.doc_id != second.doc_id {
        return Err(ApiError::bad_request("invalid_request"));
    }
    let before = nodes_of_version(&mut tx, &first).await?;
    let after = nodes_of_version(&mut tx, &second).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let (from_design_system, to_design_system) = (
        first.design_system_version_id.clone(),
        second.design_system_version_id.clone(),
    );
    let mut diff = compare(first.doc_id, first.id, second.id, before, after);
    diff.from_design_system_version_id = from_design_system;
    diff.to_design_system_version_id = to_design_system;
    Ok(Json(DiffResponse { diff }))
}

/// The differences between the nodes of two versions, each list in the document
/// order of the version it is read from.
fn compare(
    doc_id: String,
    from: String,
    to: String,
    before: Vec<NodeEntry>,
    after: Vec<NodeEntry>,
) -> DiffBody {
    let was: HashMap<&str, &NodeEntry> = before.iter().map(|n| (n.id.as_str(), n)).collect();
    let is: HashMap<&str, &NodeEntry> = after.iter().map(|n| (n.id.as_str(), n)).collect();
    let added = after
        .iter()
        .filter(|n| !was.contains_key(n.id.as_str()))
        .cloned()
        .collect();
    let removed = before
        .iter()
        .filter(|n| !is.contains_key(n.id.as_str()))
        .cloned()
        .collect();
    let changed = after
        .iter()
        .filter(|n| {
            was.get(n.id.as_str())
                .is_some_and(|w| w.fingerprint != n.fingerprint)
        })
        .cloned()
        .collect();
    let was_after = preceding_siblings(&before);
    let is_after = preceding_siblings(&after);
    let reordered = reordered(&before, &after, &was);
    let moved = after
        .iter()
        .filter_map(|n| {
            let w = was.get(n.id.as_str())?;
            (w.parent != n.parent || reordered.contains(n.id.as_str())).then(|| MovedNode {
                id: n.id.clone(),
                kind: n.kind.clone(),
                from_parent: w.parent.clone(),
                to_parent: n.parent.clone(),
                from_after: was_after[n.id.as_str()].map(str::to_string),
                to_after: is_after[n.id.as_str()].map(str::to_string),
            })
        })
        .collect();
    DiffBody {
        doc_id,
        from,
        to,
        from_design_system_version_id: None,
        to_design_system_version_id: None,
        added,
        removed,
        changed,
        moved,
    }
}

/// The sibling each node follows, by node ID; `None` for a first child. The
/// nodes are in document order, so each parent's children come in their order.
fn preceding_siblings(nodes: &[NodeEntry]) -> HashMap<&str, Option<&str>> {
    let mut last: HashMap<Option<&str>, &str> = HashMap::new();
    nodes
        .iter()
        .map(|n| {
            (
                n.id.as_str(),
                last.insert(n.parent.as_deref(), n.id.as_str()),
            )
        })
        .collect()
}

/// The nodes that keep their parent but change their order among the siblings
/// that also keep it: for each parent, those outside a longest common
/// subsequence of the two orders, the fewest nodes that account for the
/// reorder.
fn reordered<'a>(
    before: &[NodeEntry],
    after: &'a [NodeEntry],
    was: &HashMap<&str, &NodeEntry>,
) -> HashSet<&'a str> {
    let position: HashMap<&str, usize> = before
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    // The siblings that keep their parent, in the second version's order, each
    // with its position in the first version.
    let mut kept: HashMap<Option<&str>, Vec<(&str, usize)>> = HashMap::new();
    for n in after {
        if was.get(n.id.as_str()).is_some_and(|w| w.parent == n.parent) {
            kept.entry(n.parent.as_deref())
                .or_default()
                .push((n.id.as_str(), position[n.id.as_str()]));
        }
    }
    // Both orders hold the same siblings, so a longest common subsequence is a
    // longest increasing run of first-version positions.
    let mut out = HashSet::new();
    for siblings in kept.values() {
        let stays = longest_increasing(siblings.iter().map(|&(_, p)| p).collect());
        out.extend(
            siblings
                .iter()
                .zip(stays)
                .filter(|&(_, stays)| !stays)
                .map(|(&(id, _), _)| id),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, parent: Option<&str>, fingerprint: &str) -> NodeEntry {
        NodeEntry {
            id: id.to_string(),
            kind: "paragraph".to_string(),
            parent: parent.map(str::to_string),
            fingerprint: fingerprint.to_string(),
        }
    }

    fn ids(nodes: &[NodeEntry]) -> Vec<&str> {
        nodes.iter().map(|n| n.id.as_str()).collect()
    }

    #[test]
    fn a_comparison_sorts_nodes_into_added_removed_changed_and_moved() {
        let before = vec![
            node("keep0000", None, "a"),
            node("edit0000", None, "b"),
            node("drop0000", None, "c"),
            node("move0000", None, "d"),
        ];
        let after = vec![
            node("keep0000", None, "a"),
            node("edit0000", None, "b2"),
            node("move0000", Some("keep0000"), "d"),
            node("fresh000", None, "e"),
        ];
        let diff = compare("doc".into(), "v1".into(), "v2".into(), before, after);
        assert_eq!(ids(&diff.added), ["fresh000"]);
        assert_eq!(ids(&diff.removed), ["drop0000"]);
        assert_eq!(ids(&diff.changed), ["edit0000"]);
        assert_eq!(diff.moved.len(), 1);
        assert_eq!(diff.moved[0].id, "move0000");
        assert_eq!(diff.moved[0].from_parent, None);
        assert_eq!(diff.moved[0].to_parent.as_deref(), Some("keep0000"));
        assert_eq!(diff.moved[0].from_after.as_deref(), Some("drop0000"));
        assert_eq!(diff.moved[0].to_after, None);
    }

    #[test]
    fn a_reorder_lists_only_the_nodes_outside_a_longest_common_order() {
        let before = vec![
            node("a0000000", None, "a"),
            node("b0000000", None, "b"),
            node("c0000000", None, "c"),
            node("d0000000", None, "d"),
            node("e0000000", None, "e"),
        ];
        // D moves to the front, B is removed and F is inserted: only D moved.
        let after = vec![
            node("d0000000", None, "d"),
            node("a0000000", None, "a"),
            node("f0000000", None, "f"),
            node("c0000000", None, "c"),
            node("e0000000", None, "e"),
        ];
        let diff = compare("doc".into(), "v1".into(), "v2".into(), before, after);
        let moved: Vec<&str> = diff.moved.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(moved, ["d0000000"]);
        assert_eq!(diff.moved[0].from_after.as_deref(), Some("c0000000"));
        assert_eq!(diff.moved[0].to_after, None);
    }

    #[test]
    fn longest_increasing_marks_one_longest_run() {
        assert_eq!(
            longest_increasing(vec![1, 2, 0, 3]),
            [true, true, false, true]
        );
        assert_eq!(
            longest_increasing(vec![2, 0, 1, 3]),
            [false, true, true, true]
        );
        assert_eq!(longest_increasing(Vec::new()), Vec::<bool>::new());
    }
}
