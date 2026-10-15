//! Versions: the unchangeable snapshots that reviewers comment on and that get
//! merged. An agent saves a change set's draft of a doc as a version, and
//! removes a doc by saving a version that marks it removed; nothing is deleted,
//! so the doc and every earlier version stay readable.
//!
//! A version records the doc's content once, as a chunk of RustFS named by the
//! hash of its text (`ws/{workspace_id}/chunks/{chunk}`), and indexes its nodes
//! in the tenant table `node_index`: the ID, type, parent and own-content
//! fingerprint of each, in document order. The fingerprint hashes what the
//! node itself holds, so a change to a child or a move leaves it alone, and
//! comparing two versions is a comparison of two indexes. Both tables are
//! append-only: the API role has no UPDATE privilege on them.

use crate::AppState;
use crate::changesets::{clear_approval, parse_body};
use crate::docs::{AgentIdentity, DraftRow, Tx, ensure_loaded, find_draft};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid, sha256_hex};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use omni_core::ops::IndexedNode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use utoipa::ToSchema;

/// The columns of a version; `created_at` is ISO 8601 text in UTC.
const VERSION_COLUMNS: &str = "v.id::text AS id, v.doc_id::text AS doc_id, \
     v.changeset_id::text AS changeset_id, v.draft_id::text AS draft_id, v.removed, v.chunk, \
     v.agent::text AS agent, \
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
    /// The version's nodes in document order.
    pub nodes: Vec<NodeEntry>,
}

#[derive(Serialize, ToSchema)]
pub struct VersionResponse {
    pub version: VersionBody,
}

/// A node that moved to another parent.
#[derive(Serialize, ToSchema)]
pub struct MovedNode {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub from_parent: Option<String>,
    pub to_parent: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DiffBody {
    pub doc_id: String,
    pub from: String,
    pub to: String,
    /// The nodes only the second version has.
    pub added: Vec<NodeEntry>,
    /// The nodes only the first version has.
    pub removed: Vec<NodeEntry>,
    /// The nodes of both whose own content differs, as in the second version.
    pub changed: Vec<NodeEntry>,
    /// The nodes of both whose parent differs.
    pub moved: Vec<MovedNode>,
}

#[derive(Serialize, ToSchema)]
pub struct DiffResponse {
    pub diff: DiffBody,
}

/// What saving a version takes: nothing, but the body must be a JSON object.
#[derive(Deserialize, ToSchema)]
pub struct SaveVersionRequest {}

// Chunks in RustFS ------------------------------------------------------------------

fn chunk_key(workspace_id: &str, chunk: &str) -> String {
    format!("ws/{workspace_id}/chunks/{chunk}")
}

/// Stores a doc's HTML as a chunk and returns its name, the hash of the text.
/// The name follows from the content, so storing the same text again changes
/// nothing: a chunk is never overwritten with other content.
async fn store_chunk(state: &AppState, workspace_id: &str, html: &str) -> Result<String, ApiError> {
    let chunk = sha256_hex(html);
    let stored = state
        .bucket
        .put_object(chunk_key(workspace_id, &chunk), html.as_bytes())
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
struct VersionRow {
    id: String,
    doc_id: String,
    changeset_id: String,
    draft_id: Option<String>,
    removed: bool,
    chunk: Option<String>,
    agent: String,
    created_at: String,
}

/// The version `version_id`, 404 when it is not there or its doc is not in
/// this project.
async fn find_version(
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

async fn version_body(tx: &mut Tx, row: VersionRow) -> Result<VersionBody, ApiError> {
    let nodes = nodes_of(tx, row.chunk.as_deref()).await?;
    Ok(VersionBody {
        id: row.id,
        doc_id: row.doc_id,
        changeset_id: row.changeset_id,
        draft_id: row.draft_id,
        removed: row.removed,
        created_at: row.created_at,
        agent: serde_json::from_str(&row.agent).unwrap_or(Value::Null),
        nodes,
    })
}

// Saving ------------------------------------------------------------------------------

/// What a version holds: the doc's HTML and its node index, or nothing for a
/// removal.
struct Snapshot {
    html: String,
    nodes: Vec<IndexedNode>,
}

/// Writes a version of the draft: the chunk first (it is named by its content,
/// so a chunk without a version is harmless), then, in one transaction, the
/// version, the chunk's node index when it is new, the move out of an approval
/// and the `version.saved` event.
async fn store_version(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    draft: &DraftRow,
    agent: &Value,
    snapshot: Option<Snapshot>,
) -> Result<VersionBody, ApiError> {
    let removed = snapshot.is_none();
    let chunk = match &snapshot {
        Some(snapshot) => Some(store_chunk(state, workspace_id, &snapshot.html).await?),
        None => None,
    };
    let mut tx = state.workspace_tx(workspace_id).await?;
    let id: String = sqlx::query_scalar(
        "INSERT INTO versions (workspace_id, doc_id, changeset_id, draft_id, removed, chunk, agent) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, $6, $7::text::jsonb) \
         RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(&draft.doc_id)
    .bind(&draft.changeset_id)
    .bind(&draft.id)
    .bind(removed)
    .bind(&chunk)
    .bind(agent.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    if let (Some(chunk), Some(snapshot)) = (&chunk, &snapshot) {
        // A chunk that is indexed already has these very nodes: its name is
        // the hash of the content they come from.
        let ids: Vec<String> = snapshot.nodes.iter().map(|n| n.id.to_string()).collect();
        let kinds: Vec<&str> = snapshot.nodes.iter().map(|n| n.kind).collect();
        let parents: Vec<Option<String>> = snapshot
            .nodes
            .iter()
            .map(|n| n.parent.map(|parent| parent.to_string()))
            .collect();
        let fingerprints: Vec<String> = snapshot
            .nodes
            .iter()
            .map(|n| sha256_hex(&n.content))
            .collect();
        let ordinals: Vec<i32> = (0..snapshot.nodes.len() as i32).collect();
        sqlx::query(
            "INSERT INTO node_index (workspace_id, chunk, node, type, parent, fingerprint, ordinal) \
             SELECT $1::uuid, $2, node, type, parent, fingerprint, ordinal \
             FROM unnest($3::text[], $4::text[], $5::text[], $6::text[], $7::int[]) \
                  AS n (node, type, parent, fingerprint, ordinal) \
             ON CONFLICT (workspace_id, chunk, node) DO NOTHING",
        )
        .bind(workspace_id)
        .bind(chunk)
        .bind(&ids)
        .bind(&kinds)
        .bind(&parents)
        .bind(&fingerprints)
        .bind(&ordinals)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    }
    clear_approval(&mut tx, workspace_id, project_id, &draft.changeset_id).await?;
    live::emit(
        &mut tx,
        workspace_id,
        project_id,
        "version.saved",
        &json!({
            "version_id": id,
            "doc_id": draft.doc_id,
            "changeset_id": draft.changeset_id,
            "draft_id": draft.id,
            "removed": removed,
            "agent": agent,
        }),
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
    tx.commit().await.map_err(ApiError::database)?;
    sent?;

    // The snapshot is taken under the draft's lock, so it holds whole batches
    // only: the ones that are committed.
    let slot = state.drafts.slot(&request.workspace_id, &draft.id);
    let snapshot = {
        let mut held = slot.lock().await;
        ensure_loaded(&state, &request.workspace_id, &draft, &mut held).await?;
        let doc = &held.as_ref().expect("the draft is loaded").doc;
        Snapshot {
            html: doc.to_html(),
            nodes: doc.index(),
        }
    };
    let version = store_version(
        &state,
        &request.workspace_id,
        &project_id,
        &draft,
        &identity.to_json(),
        Some(snapshot),
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
    sent?;
    let version = store_version(
        &state,
        &request.workspace_id,
        &project_id,
        &draft,
        &identity.to_json(),
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
    let version = version_body(&mut tx, row).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(VersionResponse { version }))
}

/// What differs between two versions of one doc, from the first to the second:
/// the nodes added and removed, the nodes of both whose own content changed,
/// and the nodes of both that moved to another parent.
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
    let before = nodes_of(&mut tx, first.chunk.as_deref()).await?;
    let after = nodes_of(&mut tx, second.chunk.as_deref()).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(DiffResponse {
        diff: compare(first.doc_id, first.id, second.id, before, after),
    }))
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
    let moved = after
        .iter()
        .filter_map(|n| {
            let w = was.get(n.id.as_str())?;
            (w.parent != n.parent).then(|| MovedNode {
                id: n.id.clone(),
                kind: n.kind.clone(),
                from_parent: w.parent.clone(),
                to_parent: n.parent.clone(),
            })
        })
        .collect();
    DiffBody {
        doc_id,
        from,
        to,
        added,
        removed,
        changed,
        moved,
    }
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
    }
}
