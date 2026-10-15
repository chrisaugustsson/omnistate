//! Docs and their drafts. A doc belongs to a project; a change set edits it
//! through its own draft, with batches of operations (docs/formats/operations.md)
//! that the operation engine of omni-core applies all or nothing. A new doc has
//! no mainline version: its first version comes through the change set that
//! started it (Q18), so `create_doc` makes the doc and that change set's draft
//! together.
//!
//! A draft is kept in memory, keyed by workspace and draft, because an edit
//! must take well under 50 ms (FR-48). Postgres holds the truth: every applied
//! operation is a row of `draft_ops` (with the agent and the IDs the server
//! assigned), written in the transaction of the request, and a draft that is
//! not in memory (after a restart) is rebuilt by replaying its log. One lock
//! per draft orders the batches of agents that edit the same draft at once.

use crate::AppState;
use crate::changesets::{non_blank, parse_body};
use crate::claims::{self, ClaimBody};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::versions;
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid};
use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use omni_core::design::NodeId;
use omni_core::ops::{ApplyError, Conflict, EditDoc, Operation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use utoipa::ToSchema;

pub(crate) type Tx = Transaction<'static, Postgres>;

/// The columns of a doc; `created_at` is ISO 8601 text in UTC.
const DOC_COLUMNS: &str = "id::text, project_id::text, kind, module, title, \
     to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at, \
     mainline_version_id::text";

const DRAFT_COLUMNS: &str = "id::text, doc_id::text, changeset_id::text, base_version_id::text";

/// The columns of a `DraftRow` for a query that names the draft table `d`.
pub(crate) const DRAFT_ROW_COLUMNS: &str = "d.id::text AS id, d.doc_id::text AS doc_id, \
     d.changeset_id::text AS changeset_id, d.base_version_id::text AS base_version_id, \
     d.content_version_id::text AS content_version_id, d.content_after_seq AS content_after_seq";

/// The alphabet of node IDs: Crockford's base32 in lowercase.
const NODE_ID_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct DraftBody {
    pub id: String,
    pub doc_id: String,
    pub changeset_id: String,
    /// The version the draft started from; `null` for a new doc, whose first
    /// version comes through this change set.
    pub base_version_id: Option<String>,
}

#[derive(sqlx::FromRow)]
struct DocRow {
    id: String,
    project_id: String,
    kind: String,
    module: String,
    title: String,
    created_at: String,
    mainline_version_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DocBody {
    pub id: String,
    pub project_id: String,
    /// `html`, `design` or `design-system`.
    pub kind: String,
    /// `prd`, `architecture`, `db` or `ui`.
    pub module: String,
    pub title: String,
    pub created_at: String,
    /// `null` until a change set that holds the doc's first version merges.
    pub mainline_version_id: Option<String>,
    /// The doc's drafts, one per change set, oldest first.
    pub drafts: Vec<DraftBody>,
    /// The doc's live claims, oldest first.
    pub claims: Vec<ClaimBody>,
}

#[derive(Serialize, ToSchema)]
pub struct DocResponse {
    pub doc: DocBody,
}

#[derive(Serialize, ToSchema)]
pub struct CreatedDocResponse {
    pub doc: DocBody,
    /// The change set's draft of the new doc: empty, and based on no version.
    pub draft: DraftBody,
}

#[derive(Serialize, ToSchema)]
pub struct DraftResponse {
    pub draft: DraftBody,
}

#[derive(Serialize, ToSchema)]
pub struct DraftWithHtml {
    #[serde(flatten)]
    pub draft: DraftBody,
    /// The draft as HTML: `data-node-id` and `data-rev` (the node's revision)
    /// on every block.
    pub html: String,
}

#[derive(Serialize, ToSchema)]
pub struct DraftHtmlResponse {
    pub draft: DraftWithHtml,
}

#[derive(Serialize, ToSchema)]
pub struct MainlineHtmlResponse {
    pub doc_id: String,
    /// The version that was read; `null` when the doc has no mainline version.
    pub version_id: Option<String>,
    /// The version as HTML with `data-node-id`; the empty text without a version.
    pub html: String,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateDocRequest {
    /// `html`, `design` or `design-system`.
    pub kind: Option<String>,
    /// `prd`, `architecture` or `db` for `html`, `ui` for the design kinds.
    pub module: Option<String>,
    pub title: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct EditRequest {
    /// A batch of operations: `insert`, `replace`, `set`, `move` and `delete`.
    #[schema(value_type = Vec<Object>)]
    pub ops: Vec<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct OpResultBody {
    /// The node the operation inserted, replaced, set, moved or deleted.
    pub node: String,
    /// The node's revision after the operation.
    pub revision: u64,
    /// The IDs the server assigned to new nodes, in document order.
    pub created: Vec<String>,
    /// The nodes the operation removed, in document order.
    pub removed: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct EditResponse {
    /// One result per operation, in order.
    pub results: Vec<OpResultBody>,
}

#[derive(Serialize, ToSchema)]
pub struct ConflictDetails {
    /// The position of the operation in the batch, counting from 0.
    pub index: usize,
    pub node: String,
    /// The revision the operation listed.
    pub expected: u64,
    /// The revision the node has now.
    pub current_revision: u64,
    /// The node as it is now, as HTML with `data-node-id`.
    pub current_html: String,
}

#[derive(Serialize, ToSchema)]
pub struct ConflictBody {
    /// `conflict`
    pub error: String,
    pub conflict: ConflictDetails,
}

#[derive(Serialize, ToSchema)]
pub struct InvalidOperationBody {
    /// `invalid_operation`
    pub error: String,
    /// The position of the operation in the batch, counting from 0.
    pub index: usize,
    /// What is wrong; it names an ID that is not in the draft.
    pub message: String,
}

/// An error of an edit or a claim: the usual ones, a revision conflict, an
/// operation that the engine refuses, or a claim of another agent in the way.
/// Each of the last three changes nothing.
pub enum DocError {
    Api(ApiError),
    Conflict(Conflict),
    Invalid {
        index: usize,
        message: String,
    },
    /// A claim of another agent is in the way (US-023); nothing changed.
    Claimed {
        /// The operation that touches the claim; `None` for a claim request.
        index: Option<usize>,
        message: String,
        claim: Box<ClaimBody>,
    },
}

impl From<ApiError> for DocError {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for DocError {
    fn into_response(self) -> Response {
        match self {
            Self::Api(error) => error.into_response(),
            Self::Conflict(conflict) => (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "conflict",
                    "conflict": {
                        "index": conflict.index,
                        "node": conflict.node,
                        "expected": conflict.expected,
                        "current_revision": conflict.current_revision,
                        "current_html": conflict.current_html,
                    }
                })),
            )
                .into_response(),
            Self::Invalid { index, message } => (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "invalid_operation",
                    "index": index,
                    "message": message,
                })),
            )
                .into_response(),
            Self::Claimed {
                index,
                message,
                claim,
            } => {
                let mut body = json!({ "error": "claimed", "message": message, "claim": claim });
                if let Some(index) = index {
                    body["index"] = json!(index);
                }
                (StatusCode::CONFLICT, Json(body)).into_response()
            }
        }
    }
}

// The agent -----------------------------------------------------------------------

/// The agent identity that a request declares in `X-Omni-Run`, `X-Omni-Role`,
/// `X-Omni-Model` and `X-Omni-Harness`. The server records what the agent
/// declares and does not verify it; a header that is missing is `null`.
pub(crate) struct AgentIdentity {
    run: Option<String>,
    role: Option<String>,
    model: Option<String>,
    harness: Option<String>,
}

/// The longest identity value that is kept.
const MAX_IDENTITY_CHARS: usize = 200;

impl AgentIdentity {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "run": self.run,
            "role": self.role,
            "model": self.model,
            "harness": self.harness,
        })
    }
}

impl<S: Send + Sync> FromRequestParts<S> for AgentIdentity {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let header = |name: &str| {
            parts
                .headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.trim().chars().take(MAX_IDENTITY_CHARS).collect())
                .filter(|value: &String| !value.is_empty())
        };
        Ok(Self {
            run: header("x-omni-run"),
            role: header("x-omni-role"),
            model: header("x-omni-model"),
            harness: header("x-omni-harness"),
        })
    }
}

// Drafts in memory -----------------------------------------------------------------

/// A draft that is in memory: the doc under edit and the `seq` of the next
/// operation of its log.
pub(crate) struct DraftState {
    pub(crate) doc: EditDoc,
    next_seq: i64,
}

pub(crate) type Slot = Arc<tokio::sync::Mutex<Option<DraftState>>>;
type UsedIds = Arc<Mutex<HashSet<NodeId>>>;

/// The drafts that are in memory, and the node IDs of every project in use.
/// Held by the `AppState`; clones share it.
#[derive(Clone, Default)]
pub struct DraftStore {
    /// One slot per (workspace, draft). Its lock orders the batches of the
    /// draft, and it holds `None` until the draft is loaded, and again after a
    /// write that failed in the database, so the next request reloads it.
    drafts: Arc<Mutex<HashMap<(String, String), Slot>>>,
    /// The node IDs of each (workspace, project): IDs are unique within a
    /// project, so a new ID must be new to every doc of it.
    node_ids: Arc<tokio::sync::Mutex<HashMap<(String, String), UsedIds>>>,
}

impl DraftStore {
    pub(crate) fn slot(&self, workspace_id: &str, draft_id: &str) -> Slot {
        let mut drafts = self.drafts.lock().unwrap_or_else(|e| e.into_inner());
        drafts
            .entry((workspace_id.to_string(), draft_id.to_string()))
            .or_default()
            .clone()
    }

    /// The IDs in use in a project, read from the logs of its drafts the first
    /// time the project needs them.
    async fn used_ids(
        &self,
        tx: &mut Tx,
        workspace_id: &str,
        project_id: &str,
    ) -> Result<UsedIds, ApiError> {
        let mut projects = self.node_ids.lock().await;
        let key = (workspace_id.to_string(), project_id.to_string());
        if let Some(used) = projects.get(&key) {
            return Ok(used.clone());
        }
        let known: Vec<String> = sqlx::query_scalar(
            "SELECT unnest(o.created) FROM draft_ops o \
             JOIN drafts d ON d.workspace_id = o.workspace_id AND d.id = o.draft_id \
             JOIN docs x ON x.workspace_id = d.workspace_id AND x.id = d.doc_id \
             WHERE x.project_id = $1::uuid",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(ApiError::database)?;
        let used: HashSet<NodeId> = known.iter().filter_map(|id| id.parse().ok()).collect();
        let used = Arc::new(Mutex::new(used));
        projects.insert(key, used.clone());
        Ok(used)
    }
}

/// A fresh random node ID.
fn random_node_id() -> NodeId {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    // 256 is a multiple of 32, so every character is equally likely.
    let id: String = bytes
        .iter()
        .map(|byte| NODE_ID_ALPHABET[usize::from(*byte) % NODE_ID_ALPHABET.len()] as char)
        .collect();
    id.parse().expect("the alphabet gives a node ID")
}

/// Rebuilds a draft from its base version and its log. The log holds the IDs
/// that the server assigned, which the replay hands out again in the same order.
async fn load_state(
    state: &AppState,
    tx: &mut Tx,
    workspace_id: &str,
    draft: &DraftRow,
) -> Result<DraftState, ApiError> {
    // A rebased draft starts from the version the rebase saved, a draft that
    // was never rebased from its base.
    let start = draft
        .content_version_id
        .as_ref()
        .or(draft.base_version_id.as_ref());
    let mut doc = match start {
        None => EditDoc::from_html("").expect("an empty doc parses"),
        Some(base) => {
            let chunk = versions::chunk_of_version(tx, &draft.doc_id, base).await?;
            let html = match chunk {
                Some(chunk) => versions::read_chunk(state, workspace_id, &chunk).await?,
                None => String::new(),
            };
            EditDoc::from_html(&html).map_err(|e| {
                eprintln!(
                    "omni-api: the base version {base} of draft {} cannot be loaded: {e}",
                    draft.id
                );
                ApiError::Unavailable
            })?
        }
    };
    let rows: Vec<(i64, String, Vec<String>)> = sqlx::query_as(
        "SELECT seq, op::text, created FROM draft_ops \
         WHERE draft_id = $1::uuid AND seq > $2 ORDER BY seq",
    )
    .bind(&draft.id)
    .bind(draft.content_after_seq)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let corrupt = |seq: i64, why: &str| {
        eprintln!(
            "omni-api: cannot replay operation {seq} of draft {}: {why}",
            draft.id
        );
        ApiError::Unavailable
    };
    let mut next_seq = draft.content_after_seq + 1;
    for (seq, op, created) in rows {
        let op: Operation = serde_json::from_str(&op).map_err(|e| corrupt(seq, &e.to_string()))?;
        let mut ids: VecDeque<NodeId> = created.iter().filter_map(|id| id.parse().ok()).collect();
        let mut ran_out = false;
        let applied = doc.apply(&[op], &mut || {
            ids.pop_front().unwrap_or_else(|| {
                ran_out = true;
                random_node_id()
            })
        });
        if let Err(error) = applied {
            return Err(corrupt(seq, &error.to_string()));
        }
        if ran_out {
            return Err(corrupt(seq, "it created more nodes than the log records"));
        }
        next_seq = seq + 1;
    }
    Ok(DraftState { doc, next_seq })
}

/// Loads the draft into its locked slot when it is not in memory.
pub(crate) async fn ensure_loaded(
    state: &AppState,
    workspace_id: &str,
    draft: &DraftRow,
    held: &mut Option<DraftState>,
) -> Result<(), ApiError> {
    if held.is_none() {
        let mut tx = state.workspace_tx(workspace_id).await?;
        *held = Some(load_state(state, &mut tx, workspace_id, draft).await?);
        tx.commit().await.map_err(ApiError::database)?;
    }
    Ok(())
}

// Looking things up ----------------------------------------------------------------

#[derive(sqlx::FromRow)]
pub(crate) struct DraftRow {
    pub(crate) id: String,
    pub(crate) doc_id: String,
    pub(crate) changeset_id: String,
    pub(crate) base_version_id: Option<String>,
    /// The version a rebase saved as the draft's content, `None` before one.
    pub(crate) content_version_id: Option<String>,
    /// The draft's content takes the operations of the log after this `seq`.
    pub(crate) content_after_seq: i64,
}

impl From<DraftRow> for DraftBody {
    fn from(row: DraftRow) -> Self {
        Self {
            id: row.id,
            doc_id: row.doc_id,
            changeset_id: row.changeset_id,
            base_version_id: row.base_version_id,
        }
    }
}

/// The draft that `changeset_id` has of `doc_id`, 404 when the change set, the
/// doc or the draft is not there (or not in this project).
pub(crate) async fn find_draft(
    tx: &mut Tx,
    project_id: &str,
    changeset_id: &str,
    doc_id: &str,
) -> Result<DraftRow, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {DRAFT_ROW_COLUMNS} \
         FROM drafts d \
         JOIN docs x ON x.workspace_id = d.workspace_id AND x.id = d.doc_id \
         JOIN changesets c ON c.workspace_id = d.workspace_id AND c.id = d.changeset_id \
         WHERE d.changeset_id = $1::uuid AND d.doc_id = $2::uuid \
           AND x.project_id = $3::uuid AND c.project_id = $3::uuid"
    ))
    .bind(changeset_id)
    .bind(doc_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)
}

pub(crate) async fn require_changeset(
    tx: &mut Tx,
    project_id: &str,
    changeset_id: &str,
) -> Result<(), ApiError> {
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM changesets WHERE id = $1::uuid AND project_id = $2::uuid",
    )
    .bind(changeset_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found.map(|_| ()).ok_or(ApiError::NotFound)
}

async fn find_doc(tx: &mut Tx, project_id: &str, doc_id: &str) -> Result<DocRow, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {DOC_COLUMNS} FROM docs WHERE id = $1::uuid AND project_id = $2::uuid"
    ))
    .bind(doc_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)
}

async fn drafts_of(tx: &mut Tx, doc_id: &str) -> Result<Vec<DraftBody>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {DRAFT_COLUMNS} FROM drafts WHERE doc_id = $1::uuid ORDER BY created_at, id"
    ))
    .bind(doc_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}

fn doc_body(row: DocRow, drafts: Vec<DraftBody>, claims: Vec<ClaimBody>) -> DocBody {
    DocBody {
        id: row.id,
        project_id: row.project_id,
        kind: row.kind,
        module: row.module,
        title: row.title,
        created_at: row.created_at,
        mainline_version_id: row.mainline_version_id,
        drafts,
        claims,
    }
}

/// The kind and module of a doc: HTML docs hold PRDs, architecture and the
/// database design, design files hold the UI (FR-13).
fn valid_kind_and_module(kind: &str, module: &str) -> bool {
    matches!(
        (kind, module),
        ("html", "prd" | "architecture" | "db") | ("design" | "design-system", "ui")
    )
}

// Routes ---------------------------------------------------------------------------

/// Starts a new doc in a change set. The doc has no mainline version: its
/// first version comes through the change set, which gets an empty draft of it.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
    ),
    request_body = CreateDocRequest,
    responses(
        (status = 201, description = "The new doc and the change set's draft of it", body = CreatedDocResponse),
        (status = 400, description = "The kind, module or title is missing or not allowed", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
    )
)]
pub async fn create_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<CreatedDocResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateDocRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_changeset(&mut tx, &project_id, &changeset_id).await?;
    let new = new?;
    let invalid = || ApiError::bad_request("invalid_request");
    let kind = non_blank(new.kind).ok_or_else(invalid)?;
    let module = non_blank(new.module).ok_or_else(invalid)?;
    let title = non_blank(new.title).ok_or_else(invalid)?;
    if !valid_kind_and_module(&kind, &module) {
        return Err(invalid());
    }
    let row: DocRow = sqlx::query_as(&format!(
        "INSERT INTO docs (workspace_id, project_id, kind, module, title) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5) RETURNING {DOC_COLUMNS}"
    ))
    .bind(&request.workspace_id)
    .bind(&project_id)
    .bind(&kind)
    .bind(&module)
    .bind(&title)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let draft: DraftBody = sqlx::query_as(&format!(
        "INSERT INTO drafts (workspace_id, doc_id, changeset_id) \
         VALUES ($1::uuid, $2::uuid, $3::uuid) RETURNING {DRAFT_COLUMNS}"
    ))
    .bind(&request.workspace_id)
    .bind(&row.id)
    .bind(&changeset_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "doc.created",
        &json!({
            "doc_id": row.id,
            "changeset_id": changeset_id,
            "kind": kind,
            "module": module,
            "title": title,
        }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    let doc = doc_body(
        row,
        vec![DraftBody {
            id: draft.id.clone(),
            doc_id: draft.doc_id.clone(),
            changeset_id: draft.changeset_id.clone(),
            base_version_id: None,
        }],
        Vec::new(),
    );
    Ok((StatusCode::CREATED, Json(CreatedDocResponse { doc, draft })))
}

/// A doc's metadata, its mainline version, its drafts and its claims.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
    ),
    responses(
        (status = 200, description = "The doc", body = DocResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace", body = ErrorBody),
    )
)]
pub async fn show_doc(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
) -> Result<Json<DocResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    claims::sweep(&state, &request.workspace_id).await;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row = find_doc(&mut tx, &project_id, &doc_id).await?;
    let drafts = drafts_of(&mut tx, &row.id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let claims = state.claims.live_of_doc(&request.workspace_id, &row.id);
    Ok(Json(DocResponse {
        doc: doc_body(row, drafts, claims),
    }))
}

/// A doc's mainline version as HTML with `data-node-id`, or the version named
/// by `?version=` (a version has no `data-rev`). A doc that has no mainline
/// version gives no version and no HTML.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/html",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("version" = Option<String>, Query, description = "The version to read instead of the mainline version"),
    ),
    responses(
        (status = 200, description = "The version's HTML", body = MainlineHtmlResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc, or the doc has no such version", body = ErrorBody),
    )
)]
pub async fn read_doc_html(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<MainlineHtmlResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row = find_doc(&mut tx, &project_id, &doc_id).await?;
    let version_id = match query.get("version") {
        Some(version) => Some(parse_uuid(version).ok_or(ApiError::NotFound)?),
        None => row.mainline_version_id.clone(),
    };
    let Some(version_id) = version_id else {
        tx.commit().await.map_err(ApiError::database)?;
        return Ok(Json(MainlineHtmlResponse {
            doc_id: row.id,
            version_id: None,
            html: String::new(),
        }));
    };
    let chunk = versions::chunk_of_version(&mut tx, &row.id, &version_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    // A version that holds no content (the removal of the doc) reads as empty.
    let html = match chunk {
        Some(chunk) => versions::read_chunk(&state, &request.workspace_id, &chunk).await?,
        None => String::new(),
    };
    Ok(Json(MainlineHtmlResponse {
        doc_id: row.id,
        version_id: Some(version_id),
        html,
    }))
}

/// Opens a doc in a change set: makes the change set's draft from the doc's
/// mainline version, or returns the draft the change set already has.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/open",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
    ),
    responses(
        (status = 200, description = "The draft the change set already had", body = DraftResponse),
        (status = 201, description = "The new draft, based on the doc's mainline version", body = DraftResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set or doc in this project and workspace", body = ErrorBody),
        (status = 409, description = "The doc has no mainline version, so only its own change set has it (`no_mainline_version`)", body = ErrorBody),
    )
)]
pub async fn open_draft(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, changeset_id, doc_id)): Path<(String, String, String, String)>,
) -> Result<(StatusCode, Json<DraftResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_changeset(&mut tx, &project_id, &changeset_id).await?;
    let doc = find_doc(&mut tx, &project_id, &doc_id).await?;
    let existing = sqlx::query_as::<_, DraftBody>(&format!(
        "SELECT {DRAFT_COLUMNS} FROM drafts WHERE doc_id = $1::uuid AND changeset_id = $2::uuid"
    ))
    .bind(&doc_id)
    .bind(&changeset_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    if let Some(draft) = existing {
        tx.commit().await.map_err(ApiError::database)?;
        return Ok((StatusCode::OK, Json(DraftResponse { draft })));
    }
    let Some(mainline) = doc.mainline_version_id else {
        return Err(ApiError::conflict("no_mainline_version"));
    };
    // Two requests may open the same doc at once: the unique key lets one make
    // the draft, and the other reads it.
    let inserted: Option<DraftBody> = sqlx::query_as(&format!(
        "INSERT INTO drafts (workspace_id, doc_id, changeset_id, base_version_id) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid) \
         ON CONFLICT (workspace_id, doc_id, changeset_id) DO NOTHING \
         RETURNING {DRAFT_COLUMNS}"
    ))
    .bind(&request.workspace_id)
    .bind(&doc_id)
    .bind(&changeset_id)
    .bind(&mainline)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (status, draft) = match inserted {
        Some(draft) => {
            live::emit(
                &mut tx,
                &request.workspace_id,
                &project_id,
                "draft.opened",
                &json!({
                    "doc_id": doc_id,
                    "changeset_id": changeset_id,
                    "draft_id": draft.id,
                    "base_version_id": mainline,
                }),
            )
            .await?;
            (StatusCode::CREATED, draft)
        }
        None => {
            let draft = sqlx::query_as::<_, DraftBody>(&format!(
                "SELECT {DRAFT_COLUMNS} FROM drafts \
                 WHERE doc_id = $1::uuid AND changeset_id = $2::uuid"
            ))
            .bind(&doc_id)
            .bind(&changeset_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::database)?;
            (StatusCode::OK, draft)
        }
    };
    tx.commit().await.map_err(ApiError::database)?;
    if status == StatusCode::CREATED {
        state.live.announce(&request.workspace_id, &project_id);
    }
    Ok((status, Json(DraftResponse { draft })))
}

/// A change set's draft of a doc, as HTML with `data-node-id` and `data-rev` on
/// every block.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
    ),
    responses(
        (status = 200, description = "The draft with its HTML", body = DraftHtmlResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "The change set has no draft of this doc in this project and workspace", body = ErrorBody),
    )
)]
pub async fn read_draft(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, changeset_id, doc_id)): Path<(String, String, String, String)>,
) -> Result<Json<DraftHtmlResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let draft = find_draft(&mut tx, &project_id, &changeset_id, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;

    let slot = state.drafts.slot(&request.workspace_id, &draft.id);
    let mut held = slot.lock().await;
    ensure_loaded(&state, &request.workspace_id, &draft, &mut held).await?;
    let html = held
        .as_ref()
        .map(|loaded| loaded.doc.to_html_with_revisions())
        .unwrap_or_default();
    drop(held);
    Ok(Json(DraftHtmlResponse {
        draft: DraftWithHtml {
            draft: draft.into(),
            html,
        },
    }))
}

/// What an edit needs, gathered before the draft is locked.
struct Edit {
    workspace_id: String,
    project_id: String,
    draft: DraftRow,
    agent: Value,
    /// The operations as the agent sent them, kept in the draft's log.
    sent: Vec<Value>,
    ops: Vec<Operation>,
}

/// Applies a batch of operations to a change set's draft, all or nothing. Each
/// applied operation is logged with the agent identity and emits `op.applied`;
/// a batch that is refused changes, logs and emits nothing.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/ops",
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
    request_body = EditRequest,
    responses(
        (status = 200, description = "The batch applied; one result per operation", body = EditResponse),
        (status = 400, description = "The body is not a batch (`invalid_request`), or an operation is refused (`invalid_operation`, naming an unknown ID)", body = InvalidOperationBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "The change set has no draft of this doc in this project and workspace", body = ErrorBody),
        (status = 409, description = "A node is at another revision than the operation expected (`conflict`), or an operation touches a claim of another agent (`claimed`); nothing changed", body = ConflictBody),
    )
)]
pub async fn edit_draft(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id, doc_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<EditResponse>, DocError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let batch = parse_body::<EditRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let draft = find_draft(&mut tx, &project_id, &changeset_id, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;

    let sent = batch?.ops;
    claims::sweep(&state, &request.workspace_id).await;
    let mut ops = Vec::with_capacity(sent.len());
    for (index, op) in sent.iter().enumerate() {
        match serde_json::from_value::<Operation>(op.clone()) {
            Ok(op) => ops.push(op),
            Err(error) => {
                return Err(DocError::Invalid {
                    index,
                    message: error.to_string(),
                });
            }
        }
    }
    let edit = Edit {
        workspace_id: request.workspace_id,
        project_id,
        draft,
        agent: identity.to_json(),
        sent,
        ops,
    };
    // A client that gives up must not leave the draft in memory ahead of its
    // log, so the batch runs to its end whatever happens to this request.
    let state_for_task = state.clone();
    let results = tokio::spawn(async move { apply_batch(&state_for_task, edit).await })
        .await
        .map_err(|e| {
            eprintln!("omni-api: an edit failed: {e}");
            ApiError::Unavailable
        })??;
    Ok(Json(EditResponse { results }))
}

async fn apply_batch(state: &AppState, edit: Edit) -> Result<Vec<OpResultBody>, DocError> {
    let Edit {
        workspace_id,
        project_id,
        draft,
        agent,
        sent,
        ops,
    } = edit;
    let slot = state.drafts.slot(&workspace_id, &draft.id);
    let mut held = slot.lock().await;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    if held.is_none() {
        *held = Some(load_state(state, &mut tx, &workspace_id, &draft).await?);
    }
    let used = state
        .drafts
        .used_ids(&mut tx, &workspace_id, &project_id)
        .await?;
    let loaded = held.as_mut().expect("the draft was just loaded");
    let first_seq = loaded.next_seq;

    let live = state.claims.live_in_draft(&workspace_id, &draft.id);
    if let Some((index, claim)) = claims::blocked(&loaded.doc, &ops, &live, &agent) {
        return Err(state.claims.refusal(Some(index), claim));
    }

    let mut new_ids = || loop {
        let id = random_node_id();
        if used.lock().unwrap_or_else(|e| e.into_inner()).insert(id) {
            return id;
        }
    };
    let applied = match loaded.doc.apply(&ops, &mut new_ids) {
        Ok(applied) => applied,
        Err(ApplyError::Conflict(conflict)) => return Err(DocError::Conflict(conflict)),
        Err(ApplyError::Invalid { index, message }) => {
            return Err(DocError::Invalid { index, message });
        }
    };

    let results: Vec<OpResultBody> = applied
        .ops
        .iter()
        .map(|result| OpResultBody {
            node: result.node.to_string(),
            revision: result.revision,
            created: result.created.iter().map(NodeId::to_string).collect(),
            removed: result.removed.iter().map(NodeId::to_string).collect(),
        })
        .collect();
    let persisted = log_and_emit(
        &mut tx,
        &workspace_id,
        &project_id,
        &draft,
        &agent,
        first_seq,
        &sent,
        &results,
    )
    .await;
    let committed = match persisted {
        Ok(()) => tx.commit().await.map_err(ApiError::database),
        Err(error) => Err(error),
    };
    match committed {
        Ok(()) => {
            held.as_mut().expect("the draft is loaded").next_seq += results.len() as i64;
            drop(held);
            if !results.is_empty() {
                state.claims.touch(&workspace_id, &draft.id, &agent);
                state.live.announce(&workspace_id, &project_id);
            }
            Ok(results)
        }
        Err(error) => {
            // The draft in memory has the batch and the database does not: the
            // next request loads the draft again from its log.
            *held = None;
            Err(error.into())
        }
    }
}

/// Writes one row of the draft's log for each applied operation and emits its
/// `op.applied`, in the transaction of the batch.
#[allow(clippy::too_many_arguments)]
async fn log_and_emit(
    tx: &mut Tx,
    workspace_id: &str,
    project_id: &str,
    draft: &DraftRow,
    agent: &Value,
    first_seq: i64,
    sent: &[Value],
    results: &[OpResultBody],
) -> Result<(), ApiError> {
    for (offset, (op, result)) in sent.iter().zip(results).enumerate() {
        let seq = first_seq + offset as i64;
        sqlx::query(
            "INSERT INTO draft_ops \
             (workspace_id, draft_id, seq, op, agent, node, revision, created, removed) \
             VALUES ($1::uuid, $2::uuid, $3, $4::text::jsonb, $5::text::jsonb, $6, $7, $8, $9)",
        )
        .bind(workspace_id)
        .bind(&draft.id)
        .bind(seq)
        .bind(op.to_string())
        .bind(agent.to_string())
        .bind(&result.node)
        .bind(result.revision as i64)
        .bind(&result.created)
        .bind(&result.removed)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::database)?;
        live::emit(
            tx,
            workspace_id,
            project_id,
            "op.applied",
            &json!({
                "doc_id": draft.doc_id,
                "changeset_id": draft.changeset_id,
                "draft_id": draft.id,
                "node": result.node,
                "revision": result.revision,
                "created": result.created,
                "removed": result.removed,
                "agent": agent,
            }),
        )
        .await?;
    }
    Ok(())
}
