//! Rebasing and conflicts (US-026). A change set that `needs_rebase` has a doc
//! whose mainline version moved on after the change set's draft was based on
//! it. The agent rebases: the change set's changes (what differs from the
//! draft's base to the change set's latest version of the doc) are replayed
//! onto the doc's mainline version by the three-way merge of `omni-core`, which
//! keeps every node ID. Every node that both sides changed becomes a conflict,
//! recorded in the tenant table `conflicts` with both versions of the node, and
//! keeps its mainline state in the draft.
//!
//! The replayed content of each doc is saved as a new version of the change
//! set, so that what the human approves next is the rebased state and never the
//! version on the old base, and the draft is based on the mainline version from
//! then on (`drafts.content_version_id`, see the migration). Without conflicts
//! the approval is kept, moved to the new versions, and the change set merges;
//! with conflicts the approval is cleared and the change set is `open` again.
//! It cannot be submitted until the agent has fixed the draft with normal
//! operations and resolved each conflict with a note.

use crate::AppState;
use crate::changesets::{ChangesetBody, load_changeset, move_status};
use crate::docs::{
    AgentIdentity, DRAFT_ROW_COLUMNS, DraftRow, DraftState, Tx, ensure_loaded, require_changeset,
};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::versions::{
    Snapshot, chunk_of_version, emit_version_saved, insert_version, read_chunk, store_chunk,
};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use omni_core::ops::EditDoc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use tokio::sync::OwnedMutexGuard;
use utoipa::ToSchema;

/// The columns of a conflict; `resolved_at` is ISO 8601 text in UTC.
const CONFLICT_COLUMNS: &str = "id::text, changeset_id::text, doc_id::text, node, \
     mainline_version_id::text, changeset_version_id::text, mainline_html, changeset_html, \
     note, to_char(resolved_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
     AS resolved_at";

/// One side of a conflict: the version that its state of the node comes from
/// and the node there.
#[derive(Serialize, ToSchema)]
pub struct ConflictSide {
    pub version_id: String,
    /// The node and what is inside it as HTML with `data-node-id`; `null` when
    /// this side removed the node.
    pub html: Option<String>,
}

/// How the agent resolved a conflict.
#[derive(Serialize, ToSchema)]
pub struct ConflictResolution {
    pub note: String,
    pub resolved_at: String,
}

/// A node that both the mainline and the change set changed, as a rebase
/// recorded it.
#[derive(Serialize, ToSchema)]
pub struct RecordedConflict {
    pub id: String,
    pub changeset_id: String,
    pub doc_id: String,
    pub node: String,
    /// The node in the mainline version that the rebase found.
    pub mainline: ConflictSide,
    /// The node in the change set's latest version of the doc.
    pub changeset: ConflictSide,
    /// `null` until the agent resolves the conflict.
    pub resolution: Option<ConflictResolution>,
}

#[derive(Serialize, ToSchema)]
pub struct RebaseResponse {
    pub changeset: ChangesetBody,
    /// The conflicts that this rebase recorded; empty for a clean one.
    pub conflicts: Vec<RecordedConflict>,
}

#[derive(Serialize, ToSchema)]
pub struct RecordedConflictResponse {
    pub conflict: RecordedConflict,
}

#[derive(Serialize, ToSchema)]
pub struct RecordedConflictListResponse {
    pub conflicts: Vec<RecordedConflict>,
}

#[derive(Deserialize, ToSchema)]
pub struct ResolveConflictRequest {
    /// How the conflict was resolved; required.
    pub note: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ConflictRow {
    id: String,
    changeset_id: String,
    doc_id: String,
    node: String,
    mainline_version_id: String,
    changeset_version_id: String,
    mainline_html: Option<String>,
    changeset_html: Option<String>,
    note: Option<String>,
    resolved_at: Option<String>,
}

impl From<ConflictRow> for RecordedConflict {
    fn from(row: ConflictRow) -> Self {
        let resolution = match (row.note, row.resolved_at) {
            (Some(note), Some(resolved_at)) => Some(ConflictResolution { note, resolved_at }),
            _ => None,
        };
        Self {
            id: row.id,
            changeset_id: row.changeset_id,
            doc_id: row.doc_id,
            node: row.node,
            mainline: ConflictSide {
                version_id: row.mainline_version_id,
                html: row.mainline_html,
            },
            changeset: ConflictSide {
                version_id: row.changeset_version_id,
                html: row.changeset_html,
            },
            resolution,
        }
    }
}

/// A doc that the change set changes: its latest version in the change set and
/// the change set's draft of it.
#[derive(sqlx::FromRow)]
struct Changed {
    version_id: String,
    /// `None` when the version removes the doc.
    chunk: Option<String>,
    #[sqlx(flatten)]
    draft: DraftRow,
}

/// The change set's docs with a saved version, with the latest version of each,
/// in doc order. Those are the docs that an approval covers and that a rebase
/// replays.
async fn changed_docs(tx: &mut Tx, changeset_id: &str) -> Result<Vec<Changed>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT DISTINCT ON (v.doc_id) v.id::text AS version_id, v.chunk AS chunk, \
                {DRAFT_ROW_COLUMNS} \
         FROM versions v \
         JOIN drafts d ON d.workspace_id = v.workspace_id AND d.doc_id = v.doc_id \
                      AND d.changeset_id = v.changeset_id \
         WHERE v.changeset_id = $1::uuid \
         ORDER BY v.doc_id, v.created_at DESC, v.id DESC"
    ))
    .bind(changeset_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// The doc that a chunk holds, an empty one for a version without content.
async fn load_doc(
    state: &AppState,
    workspace_id: &str,
    chunk: Option<&str>,
) -> Result<EditDoc, ApiError> {
    let html = match chunk {
        Some(chunk) => read_chunk(state, workspace_id, chunk).await?,
        None => String::new(),
    };
    EditDoc::from_html(&html).map_err(|e| {
        eprintln!("omni-api: a saved version cannot be loaded for a rebase: {e}");
        ApiError::Unavailable
    })
}

/// Replays the change set's latest version of a doc onto the doc's mainline
/// version: saves the result as a version of the change set, records the
/// conflicts and bases the draft on the mainline. Returns the new version's ID
/// and the conflicts.
async fn rebase_doc(
    state: &AppState,
    tx: &mut Tx,
    ids: (&str, &str),
    changed: &Changed,
    mainline_version: &str,
    agent: &Value,
) -> Result<(String, Vec<RecordedConflict>), ApiError> {
    let (workspace_id, project_id) = ids;
    let draft = &changed.draft;
    let base_chunk = match &draft.base_version_id {
        Some(base) => chunk_of_version(tx, &draft.doc_id, base).await?,
        None => None,
    };
    let mainline_chunk = chunk_of_version(tx, &draft.doc_id, mainline_version).await?;
    let base = load_doc(state, workspace_id, base_chunk.as_deref()).await?;
    let mainline = load_doc(state, workspace_id, mainline_chunk.as_deref()).await?;
    let mine = load_doc(state, workspace_id, changed.chunk.as_deref()).await?;
    // A removed doc is a doc-level change: when one side removed the doc and
    // the other changed anything in it, that is a conflict and the mainline
    // state stays; otherwise the removal stands.
    let (doc, conflict_nodes) = match (mainline_chunk.is_none(), changed.chunk.is_none()) {
        (false, false) => {
            let rebased = EditDoc::rebase(&base, &mainline, &mine);
            (Some(rebased.doc), rebased.conflicts)
        }
        (true, true) => (None, Vec::new()),
        (true, false) => (None, EditDoc::changed_from(&base, &mine)),
        (false, true) => {
            let theirs = EditDoc::changed_from(&base, &mainline);
            let doc = if theirs.is_empty() {
                None
            } else {
                Some(mainline.clone())
            };
            (doc, theirs)
        }
    };

    let snapshot = doc.as_ref().map(|doc| Snapshot {
        html: doc.to_html(),
        nodes: doc.index(),
    });
    let removed = snapshot.is_none();
    let chunk = match &snapshot {
        Some(snapshot) => Some(store_chunk(state, workspace_id, &snapshot.html).await?),
        None => None,
    };
    let version_id = insert_version(
        tx,
        workspace_id,
        draft,
        agent,
        chunk.as_deref(),
        snapshot.as_ref(),
    )
    .await?;
    emit_version_saved(
        tx,
        workspace_id,
        project_id,
        draft,
        agent,
        &version_id,
        removed,
    )
    .await?;

    let mut conflicts = Vec::with_capacity(conflict_nodes.len());
    for node in conflict_nodes {
        let mainline_html = mainline.node_html(node);
        let changeset_html = mine.node_html(node);
        let id: String = sqlx::query_scalar(
            "INSERT INTO conflicts (workspace_id, changeset_id, doc_id, node, \
             mainline_version_id, changeset_version_id, mainline_html, changeset_html) \
             VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5::uuid, $6::uuid, $7, $8) \
             RETURNING id::text",
        )
        .bind(workspace_id)
        .bind(&draft.changeset_id)
        .bind(&draft.doc_id)
        .bind(node.to_string())
        .bind(mainline_version)
        .bind(&changed.version_id)
        .bind(&mainline_html)
        .bind(&changeset_html)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::database)?;
        live::emit(
            tx,
            workspace_id,
            project_id,
            "conflict.recorded",
            &json!({
                "conflict_id": id,
                "changeset_id": draft.changeset_id,
                "doc_id": draft.doc_id,
                "node": node.to_string(),
            }),
        )
        .await?;
        conflicts.push(RecordedConflict {
            id,
            changeset_id: draft.changeset_id.clone(),
            doc_id: draft.doc_id.clone(),
            node: node.to_string(),
            mainline: ConflictSide {
                version_id: mainline_version.to_string(),
                html: mainline_html,
            },
            changeset: ConflictSide {
                version_id: changed.version_id.clone(),
                html: changeset_html,
            },
            resolution: None,
        });
    }

    // From now on the draft is the rebased version, and the operations that
    // were logged before this point (already part of the change set's version)
    // are not replayed again.
    sqlx::query(
        "UPDATE drafts SET base_version_id = $2::uuid, content_version_id = $3::uuid, \
         content_after_seq = (SELECT COALESCE(MAX(seq), 0) FROM draft_ops WHERE draft_id = $1::uuid) \
         WHERE id = $1::uuid",
    )
    .bind(&draft.id)
    .bind(mainline_version)
    .bind(&version_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        tx,
        workspace_id,
        project_id,
        "draft.rebased",
        &json!({
            "doc_id": draft.doc_id,
            "changeset_id": draft.changeset_id,
            "draft_id": draft.id,
            "base_version_id": mainline_version,
            "version_id": version_id,
            "conflicts": conflicts.len(),
        }),
    )
    .await?;
    Ok((version_id, conflicts))
}

/// Rebases a change set that `needs_rebase` onto the current mainline. The
/// agent's changes are replayed onto each doc whose draft is based on an
/// outdated version, which gives a new version of the change set. Without
/// conflicts the approval is kept and the change set merges; with conflicts
/// nothing merges, the approval is cleared and the change set is `open`, with
/// each conflicting node as the mainline has it in the draft. A removed doc is
/// a doc-level change: removed on one side and changed on the other is a
/// conflict that keeps the mainline state. A draft with operations after its
/// latest saved version is not rebased (409 `unsaved_operations`).
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/rebase",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    responses(
        (status = 200, description = "The change set, `merged` or `open`, and the conflicts this rebase recorded", body = RebaseResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
        (status = 409, description = "The change set is not `needs_rebase` (`invalid_transition`), or a draft has operations after its latest saved version (`unsaved_operations`); nothing changed", body = ErrorBody),
    )
)]
pub async fn rebase(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
) -> Result<Json<RebaseResponse>, ApiError> {
    let workspace_id = &request.workspace_id;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let agent = identity.to_json();

    // The drafts that the rebase rewrites are locked before its transaction
    // starts, in the order of their IDs, as an edit locks its draft: no
    // operation is applied while the draft is replaced, and the copy in memory
    // is dropped when the rebase has committed.
    let mut tx = state.workspace_tx(workspace_id).await?;
    require_status(&mut tx, &project_id, &changeset_id, "").await?;
    let mut draft_ids: Vec<String> = changed_docs(&mut tx, &changeset_id)
        .await?
        .into_iter()
        .map(|changed| changed.draft.id)
        .collect();
    tx.commit().await.map_err(ApiError::database)?;
    draft_ids.sort();
    let mut held: Vec<(String, OwnedMutexGuard<Option<DraftState>>)> = Vec::new();
    for draft_id in draft_ids {
        let guard = state
            .drafts
            .slot(workspace_id, &draft_id)
            .lock_owned()
            .await;
        held.push((draft_id, guard));
    }

    let mut tx = state.workspace_tx(workspace_id).await?;
    require_status(&mut tx, &project_id, &changeset_id, "FOR NO KEY UPDATE").await?;
    // The same order of locks as an approval: the change set, the project (its
    // events are numbered under that lock), then the docs in the order of
    // their IDs.
    sqlx::query("SELECT 1 FROM projects WHERE id = $1::uuid FOR NO KEY UPDATE")
        .bind(&project_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    let mut changed = changed_docs(&mut tx, &changeset_id).await?;
    if changed.len() != held.len()
        || changed
            .iter()
            .any(|c| held.iter().all(|(id, _)| *id != c.draft.id))
    {
        // A draft appeared since the locks were taken; try again.
        return Err(ApiError::Unavailable);
    }
    let docs: Vec<&str> = changed.iter().map(|c| c.draft.doc_id.as_str()).collect();
    let mainlines: HashMap<String, Option<String>> = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT id::text, mainline_version_id::text FROM docs \
         WHERE id = ANY($1::text[]::uuid[]) ORDER BY id FOR NO KEY UPDATE",
    )
    .bind(&docs)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .into_iter()
    .collect();

    // A rebase replaces the draft with the change set's latest saved version
    // on the new mainline, so operations applied after that version would be
    // lost: it refuses and leaves everything as it is.
    for changed in &changed {
        let Some(Some(mainline)) = mainlines.get(&changed.draft.doc_id) else {
            continue;
        };
        if changed.draft.base_version_id.as_ref() == Some(mainline) {
            continue;
        }
        let guard = held
            .iter_mut()
            .find(|(id, _)| *id == changed.draft.id)
            .map(|(_, guard)| guard)
            .expect("the draft is held");
        if has_unsaved_operations(&state, &mut tx, workspace_id, changed, guard).await? {
            return Err(ApiError::conflict("unsaved_operations"));
        }
    }

    let mut conflicts = Vec::new();
    let mut rebased: Vec<String> = Vec::new();
    let mut renewed: Vec<(String, String)> = Vec::new();
    for changed in &mut changed {
        let Some(Some(mainline)) = mainlines.get(&changed.draft.doc_id) else {
            continue;
        };
        if changed.draft.base_version_id.as_ref() == Some(mainline) {
            continue;
        }
        let ids = (workspace_id.as_str(), project_id.as_str());
        let (version_id, found) =
            rebase_doc(&state, &mut tx, ids, changed, mainline, &agent).await?;
        conflicts.extend(found);
        rebased.push(changed.draft.id.clone());
        renewed.push((changed.draft.doc_id.clone(), version_id.clone()));
        changed.version_id = version_id;
    }

    let to = if conflicts.is_empty() {
        // The approval stays: it moves to the rebased versions, which merge.
        for (doc_id, version_id) in &renewed {
            sqlx::query(
                "UPDATE approvals SET cleared = true \
                 WHERE changeset_id = $1::uuid AND doc_id = $2::uuid AND NOT cleared",
            )
            .bind(&changeset_id)
            .bind(doc_id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::database)?;
            sqlx::query(
                "INSERT INTO approvals (workspace_id, changeset_id, doc_id, version_id) \
                 VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid)",
            )
            .bind(workspace_id)
            .bind(&changeset_id)
            .bind(doc_id)
            .bind(version_id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::database)?;
        }
        let docs: Vec<&str> = changed.iter().map(|c| c.draft.doc_id.as_str()).collect();
        let versions: Vec<&str> = changed.iter().map(|c| c.version_id.as_str()).collect();
        sqlx::query(
            "UPDATE docs SET mainline_version_id = a.version \
             FROM unnest($1::text[]::uuid[], $2::text[]::uuid[]) AS a (doc, version) \
             WHERE docs.id = a.doc",
        )
        .bind(&docs)
        .bind(&versions)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        "merged"
    } else {
        sqlx::query(
            "UPDATE approvals SET cleared = true WHERE changeset_id = $1::uuid AND NOT cleared",
        )
        .bind(&changeset_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        "open"
    };
    move_status(
        &mut tx,
        workspace_id,
        &project_id,
        &changeset_id,
        "needs_rebase",
        to,
        "agent",
    )
    .await?;
    let changeset = load_changeset(&mut tx, &changeset_id).await?;
    tx.commit().await.map_err(ApiError::database)?;

    for (draft_id, guard) in &mut held {
        if rebased.contains(draft_id) {
            **guard = None;
        }
    }
    drop(held);
    state.live.announce(workspace_id, &project_id);
    Ok(Json(RebaseResponse {
        changeset,
        conflicts,
    }))
}

/// Whether the draft has operations after its latest saved version, `changed`.
/// A saved doc is compared with the draft in memory; a removal holds no content,
/// so for that the log is read.
async fn has_unsaved_operations(
    state: &AppState,
    tx: &mut Tx,
    workspace_id: &str,
    changed: &Changed,
    held: &mut Option<DraftState>,
) -> Result<bool, ApiError> {
    let draft = &changed.draft;
    if changed.chunk.is_some() {
        ensure_loaded(state, workspace_id, draft, held).await?;
        let html = read_chunk(
            state,
            workspace_id,
            changed.chunk.as_deref().unwrap_or_default(),
        )
        .await?;
        let draft_html = held.as_ref().map(|loaded| loaded.doc.to_html());
        return Ok(draft_html.is_some_and(|draft_html| draft_html != html));
    }
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM draft_ops o, versions v \
         WHERE o.draft_id = $1::uuid AND o.seq > $2 AND v.id = $3::uuid \
           AND o.applied_at > v.created_at)",
    )
    .bind(&draft.id)
    .bind(draft.content_after_seq)
    .bind(&changed.version_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// Fails with 404 when the change set is not in the project, and with 409
/// `invalid_transition` unless it needs a rebase. `lock` is a row-locking
/// clause for the change set's row, or empty.
async fn require_status(
    tx: &mut Tx,
    project_id: &str,
    changeset_id: &str,
    lock: &str,
) -> Result<(), ApiError> {
    let status: String = sqlx::query_scalar(&format!(
        "SELECT status FROM changesets WHERE id = $1::uuid AND project_id = $2::uuid {lock}"
    ))
    .bind(changeset_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    if status != "needs_rebase" {
        return Err(ApiError::conflict("invalid_transition"));
    }
    Ok(())
}

/// The conflicts that the rebases of a change set recorded, oldest first,
/// resolved ones included and after a merge too: the review screen shows them.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/conflicts",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
    ),
    responses(
        (status = 200, description = "The change set's conflicts", body = RecordedConflictListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
    )
)]
pub async fn list_conflicts(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
) -> Result<Json<RecordedConflictListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_changeset(&mut tx, &project_id, &changeset_id).await?;
    let rows: Vec<ConflictRow> = sqlx::query_as(&format!(
        "SELECT {CONFLICT_COLUMNS} FROM conflicts WHERE changeset_id = $1::uuid \
         ORDER BY created_at, id"
    ))
    .bind(&changeset_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(RecordedConflictListResponse {
        conflicts: rows.into_iter().map(Into::into).collect(),
    }))
}

/// Resolves a conflict with a note on how it was fixed. The agent fixes the
/// draft with normal operations first; the note is required and a conflict is
/// resolved once.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/conflicts/{conflict_id}/resolve",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("conflict_id" = String, Path, description = "The conflict's UUID"),
    ),
    request_body = ResolveConflictRequest,
    responses(
        (status = 200, description = "The conflict, resolved", body = RecordedConflictResponse),
        (status = 400, description = "The note is missing or blank (`note_required`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such conflict in this project and workspace", body = ErrorBody),
        (status = 409, description = "The conflict is resolved already (`already_resolved`)", body = ErrorBody),
    )
)]
pub async fn resolve(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, conflict_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<RecordedConflictResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let conflict_id = parse_uuid(&conflict_id).ok_or(ApiError::NotFound)?;
    // A body that is not JSON has no note either.
    let note = crate::changesets::parse_body::<ResolveConflictRequest>(&body)
        .ok()
        .and_then(|resolution| crate::changesets::non_blank(resolution.note));
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT c.id::text FROM conflicts c \
         JOIN changesets s ON s.workspace_id = c.workspace_id AND s.id = c.changeset_id \
         WHERE c.id = $1::uuid AND s.project_id = $2::uuid",
    )
    .bind(&conflict_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    found.ok_or(ApiError::NotFound)?;
    let note = note.ok_or_else(|| ApiError::bad_request("note_required"))?;
    // Of two requests that resolve the same conflict, one updates the row and
    // the other finds it resolved.
    let row: ConflictRow = sqlx::query_as(&format!(
        "UPDATE conflicts SET note = $2, resolved_at = clock_timestamp() \
         WHERE id = $1::uuid AND resolved_at IS NULL RETURNING {CONFLICT_COLUMNS}"
    ))
    .bind(&conflict_id)
    .bind(&note)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or_else(|| ApiError::conflict("already_resolved"))?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "conflict.resolved",
        &json!({
            "conflict_id": row.id,
            "changeset_id": row.changeset_id,
            "doc_id": row.doc_id,
            "node": row.node,
        }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(RecordedConflictResponse {
        conflict: row.into(),
    }))
}
