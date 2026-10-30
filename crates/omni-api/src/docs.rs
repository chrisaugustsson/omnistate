//! Docs. A doc belongs to a project, at its app level or in one of its
//! features, and is written straight to its head with batches of operations
//! (docs/formats/operations.md) that the operation engine of omni-core applies
//! all or nothing. A doc has a head, the chunk of its content now: a new doc is
//! readable at once as the empty doc of its kind, and `create_app_doc` and
//! `create_feature_doc` make one. Every write to a doc is a row of
//! `doc_changes`.

use crate::AppState;
use crate::boards;
use crate::chunks::{self, Snapshot};
use crate::design;
use crate::live;
use crate::pins;
use crate::previews;
use crate::projects::{non_blank, parse_body, require_project};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid};
use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use omni_core::design::NodeId;
use omni_core::hash::blake3_hex;
use omni_core::ops::{ApplyError, Conflict, EditDoc, Operation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use utoipa::ToSchema;

pub(crate) type Tx = Transaction<'static, Postgres>;

/// The columns of a doc; `created_at` is ISO 8601 text in UTC.
const DOC_COLUMNS: &str = "id::text, project_id::text, kind, pillar, title, \
     to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at, \
     feature_id::text, head";

/// The alphabet of node IDs: Crockford's base32 in lowercase.
const NODE_ID_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

#[derive(sqlx::FromRow)]
pub(crate) struct DocRow {
    pub(crate) id: String,
    project_id: String,
    pub(crate) kind: String,
    pillar: String,
    title: String,
    created_at: String,
    pub(crate) feature_id: Option<String>,
    /// The chunk of the doc's content now; `None` for a doc without one, which
    /// reads as empty.
    pub(crate) head: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DocBody {
    pub id: String,
    pub project_id: String,
    /// `html`, `design` or `design-system`.
    pub kind: String,
    /// `spec`, `ui`, `architecture`, `db` or `research`.
    pub pillar: String,
    pub title: String,
    pub created_at: String,
    /// The feature the doc belongs to; `null` for a doc at the app level. Fixed
    /// when the doc is created.
    pub feature_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DocResponse {
    pub doc: DocBody,
}

#[derive(Serialize, ToSchema)]
pub struct DocListResponse {
    pub docs: Vec<DocBody>,
}

#[derive(Serialize, ToSchema)]
pub struct HeadHtmlResponse {
    pub doc_id: String,
    /// The head as HTML with `data-omni-id`; the empty text for an
    /// empty doc, and for a design's head, whose content is `tags` and `pages`.
    pub html: String,
    /// A design's head as tags, one `<Page>` after the other; only for the
    /// head of a design.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    /// A design's head as JSON nodes, each page with its boards under
    /// `children`; only for the head of a design.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub pages: Option<Vec<Value>>,
    /// The token tree of the project's design system head, in every mode; only
    /// for the head of a design, and left out when the design system has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub tokens: Option<Value>,
    /// The components that the design's instances use, as the design system's
    /// head defines them; only for the head of a design.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub components: Option<Vec<Value>>,
    /// A design's head as its outline: the pages in order, each as its own JSON
    /// with its boards under `children` as placeholders (the board's own JSON
    /// without its children); only for the head of a design read with
    /// `?view=outline`, which leaves out `tags` and `pages`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub outline: Option<Vec<Value>>,
    /// The boards of the outline in page order, each with the chunk that holds
    /// it, so that a viewer reads again only the boards whose chunk changed;
    /// only with `outline`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boards: Option<Vec<boards::HeadBoard>>,
    /// Small pictures of boards of a design's head, as Base64 PNGs by board ID,
    /// for a viewer to draw a board from until it has read it; only with
    /// `?view=previews`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previews: Option<BTreeMap<String, String>>,
    /// Where nodes of a design's head sit on their boards, by node address, for
    /// a viewer to pin a node on a board it has not read; only with
    /// `?view=node-boxes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_boxes: Option<BTreeMap<String, pins::NodeBox>>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateDocRequest {
    /// `html`, `design` or `design-system`.
    pub kind: Option<String>,
    /// `spec`, `ui`, `architecture`, `db` or `research`; the design kinds only go in `ui`.
    pub pillar: Option<String>,
    pub title: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct EditRequest {
    /// A batch of operations: `insert`, `replace`, `set`, `move` and `delete`.
    #[schema(value_type = Vec<Object>)]
    pub ops: Vec<Value>,
    /// For a design: the ID of a board that every operation must stay inside
    /// (its node, and its parent for an insert or a move, are the board or in
    /// it). An ID that is no board of the doc is 404 `unknown_board`.
    pub board: Option<String>,
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
    /// The node as it is now: HTML with `data-omni-id` for an HTML doc, tags
    /// with `rev` for a design.
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
    /// What is wrong; it names an ID that is not in the doc.
    pub message: String,
}

/// An error of an edit: the usual ones, a revision conflict or an operation
/// that the engine refuses. Each of the last two changes nothing.
pub enum DocError {
    Api(ApiError),
    Conflict(Conflict),
    Invalid { index: usize, message: String },
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
    /// The run the agent declares, `None` when it declares none.
    pub(crate) fn run(&self) -> Option<&str> {
        self.run.as_deref()
    }

    /// The role the agent declares, `None` when it declares none.
    pub(crate) fn role(&self) -> Option<&str> {
        self.role.as_deref()
    }

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

// Node IDs ----------------------------------------------------------------------

/// A fresh random node ID.
pub(crate) fn random_node_id() -> NodeId {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    // 256 is a multiple of 32, so every character is equally likely.
    let id: String = bytes
        .iter()
        .map(|byte| NODE_ID_ALPHABET[usize::from(*byte) % NODE_ID_ALPHABET.len()] as char)
        .collect();
    id.parse().expect("the alphabet gives a node ID")
}

// Looking things up ----------------------------------------------------------------

pub(crate) async fn find_doc(
    tx: &mut Tx,
    project_id: &str,
    doc_id: &str,
) -> Result<DocRow, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {DOC_COLUMNS} FROM docs \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL"
    ))
    .bind(doc_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)
}

/// Which docs of a project a list asks for.
#[derive(Clone, Copy)]
pub(crate) enum Scope<'a> {
    /// Every doc of the project.
    Project,
    /// Only the app-level ones.
    Global,
    /// Only the ones of this feature (a UUID).
    Feature(&'a str),
}

/// The docs of a project in the scope, oldest first, each as `show_doc` answers
/// it.
pub(crate) async fn docs_of_project(
    tx: &mut Tx,
    project_id: &str,
    scope: Scope<'_>,
    pillar: Option<&str>,
) -> Result<Vec<DocBody>, ApiError> {
    let all = matches!(scope, Scope::Project);
    let feature = match scope {
        Scope::Feature(id) => Some(id),
        _ => None,
    };
    let rows: Vec<DocRow> = sqlx::query_as(&format!(
        "SELECT {DOC_COLUMNS} FROM docs WHERE project_id = $1::uuid AND deleted_at IS NULL \
         AND ($2::bool OR feature_id IS NOT DISTINCT FROM $3::uuid) \
         AND ($4::text IS NULL OR pillar = $4) ORDER BY created_at, id"
    ))
    .bind(project_id)
    .bind(all)
    .bind(feature)
    .bind(pillar)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(rows.into_iter().map(doc_body).collect())
}

fn doc_body(row: DocRow) -> DocBody {
    DocBody {
        id: row.id,
        project_id: row.project_id,
        kind: row.kind,
        pillar: row.pillar,
        title: row.title,
        created_at: row.created_at,
        feature_id: row.feature_id,
    }
}

/// The five pillars of a scope (FR-5).
const PILLARS: [&str; 5] = ["spec", "ui", "architecture", "db", "research"];

// Routes ---------------------------------------------------------------------------

/// A new doc as the request describes it, once its kind, pillar and title are
/// checked.
struct NewDoc {
    kind: String,
    pillar: String,
    title: String,
}

/// Checks the body of a request that creates a doc: 400 `invalid_request` for
/// a missing or unknown kind, pillar or title, 422 `design_needs_ui_pillar` for
/// a design kind outside `ui` and 422 `design_system_is_global` for a design
/// system in a feature.
fn check_new_doc(new: CreateDocRequest, in_feature: bool) -> Result<NewDoc, ApiError> {
    let invalid = || ApiError::bad_request("invalid_request");
    let kind = non_blank(new.kind).ok_or_else(invalid)?;
    let pillar = non_blank(new.pillar).ok_or_else(invalid)?;
    let title = non_blank(new.title).ok_or_else(invalid)?;
    if !matches!(kind.as_str(), "html" | "design" | "design-system")
        || !PILLARS.contains(&pillar.as_str())
    {
        return Err(invalid());
    }
    if kind != "html" && pillar != "ui" {
        return Err(ApiError::Refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "design_needs_ui_pillar",
        ));
    }
    if kind == "design-system" && in_feature {
        return Err(ApiError::Refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "design_system_is_global",
        ));
    }
    Ok(NewDoc {
        kind,
        pillar,
        title,
    })
}

fn design_system_exists() -> ApiError {
    ApiError::conflict("design_system_exists")
}

/// A project has one design system doc: 409 `design_system_exists` when a
/// second one is asked for. The unique index of the migration settles two
/// requests that start one at once.
async fn require_no_design_system(
    tx: &mut Tx,
    project_id: &str,
    new: &NewDoc,
) -> Result<(), ApiError> {
    if new.kind != "design-system" {
        return Ok(());
    }
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM docs \
         WHERE project_id = $1::uuid AND kind = 'design-system' AND deleted_at IS NULL",
    )
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    match found {
        Some(_) => Err(design_system_exists()),
        None => Ok(()),
    }
}

/// What the head of an empty doc of `kind` holds: the empty text for an HTML
/// doc or a design system, a manifest of one page without boards for a design.
fn empty_snapshot(kind: &str) -> Snapshot {
    if kind != "design" {
        return Snapshot::new(String::new(), Vec::new());
    }
    let mut ids = HashSet::new();
    let mut next = || loop {
        let id = random_node_id();
        if ids.insert(id) {
            return id;
        }
    };
    let page = omni_core::design::tags::parse_all(r#"<Page name="Page 1"/>"#)
        .expect("the tag of an empty page parses")
        .remove(0)
        .assign_ids(&mut next);
    boards::snapshot_of(&[page])
}

/// The head of an empty doc of `kind`, with what its chunks hold, which the
/// caller lists in the chunk index in the transaction that writes the doc.
struct EmptyHead {
    /// The name of the head's chunk, the BLAKE3 hash of its text.
    name: String,
    snapshot: Snapshot,
}

/// Stores the chunks of an empty doc of `kind`.
async fn store_empty_head(
    state: &AppState,
    workspace_id: &str,
    kind: &str,
) -> Result<EmptyHead, ApiError> {
    let snapshot = empty_snapshot(kind);
    let name = chunks::store_snapshot(state, workspace_id, &snapshot).await?;
    Ok(EmptyHead { name, snapshot })
}

/// Adds the row of `doc_changes` for a write to a doc, in the transaction of the
/// write: the next `seq` of the doc, the agent, the operations, and the head
/// before and after. A write to one doc is serialised by the lock on the doc's
/// row, or by its being new.
pub(crate) async fn log_change(
    tx: &mut Tx,
    workspace_id: &str,
    doc_id: &str,
    agent: &Value,
    operations: &Value,
    head_before: Option<&str>,
    head_after: &str,
) -> Result<(), ApiError> {
    log_change_of_boards(
        tx,
        workspace_id,
        doc_id,
        agent,
        operations,
        head_before,
        head_after,
        None,
        None,
    )
    .await
}

/// Like `log_change`, for a write to a design: `boards` lists each board whose
/// chunk the write changed as `{board, before, after}`. For a write to an HTML
/// doc, `results` lists what each operation did, as the edit answered it.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn log_change_of_boards(
    tx: &mut Tx,
    workspace_id: &str,
    doc_id: &str,
    agent: &Value,
    operations: &Value,
    head_before: Option<&str>,
    head_after: &str,
    boards: Option<&Value>,
    results: Option<&Value>,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO doc_changes \
         (workspace_id, doc_id, seq, agent, operations, head_before, head_after, boards, results) \
         VALUES ($1::uuid, $2::uuid, \
                 (SELECT COALESCE(MAX(seq), 0) + 1 FROM doc_changes WHERE doc_id = $2::uuid), \
                 $3::text::jsonb, $4::text::jsonb, $5, $6, $7::text::jsonb, $8::text::jsonb)",
    )
    .bind(workspace_id)
    .bind(doc_id)
    .bind(agent.to_string())
    .bind(operations.to_string())
    .bind(head_before)
    .bind(head_after)
    .bind(boards.map(Value::to_string))
    .bind(results.map(Value::to_string))
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(())
}

/// Writes a new doc with its head: the row, the head's chunks in the chunk
/// index, the first row of `doc_changes` and the `doc.created` event, in the
/// transaction of the request.
async fn insert_doc(
    tx: &mut Tx,
    workspace_id: &str,
    project_id: &str,
    feature_id: Option<&str>,
    new: &NewDoc,
    empty: &EmptyHead,
    agent: &Value,
) -> Result<DocRow, ApiError> {
    let row: DocRow = sqlx::query_as(&format!(
        "INSERT INTO docs (workspace_id, project_id, feature_id, kind, pillar, title, head) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, $7) RETURNING {DOC_COLUMNS}"
    ))
    .bind(workspace_id)
    .bind(project_id)
    .bind(feature_id)
    .bind(&new.kind)
    .bind(&new.pillar)
    .bind(&new.title)
    .bind(&empty.name)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| {
        if error
            .as_database_error()
            .is_some_and(|e| e.is_unique_violation())
        {
            design_system_exists()
        } else {
            ApiError::database(error)
        }
    })?;
    chunks::index_snapshot(tx, workspace_id, &empty.name, &empty.snapshot).await?;
    log_change(
        tx,
        workspace_id,
        &row.id,
        agent,
        &json!([{
            "op": "create",
            "kind": new.kind,
            "pillar": new.pillar,
            "title": new.title,
            "feature_id": feature_id,
        }]),
        None,
        &empty.name,
    )
    .await?;
    live::emit(
        tx,
        workspace_id,
        project_id,
        "doc.created",
        &json!({
            "doc_id": row.id,
            "feature_id": feature_id,
            "kind": new.kind,
            "pillar": new.pillar,
            "title": new.title,
        }),
    )
    .await?;
    live::emit(
        tx,
        workspace_id,
        project_id,
        "doc.changed",
        &json!({ "doc_id": row.id, "head": empty.name }),
    )
    .await?;
    Ok(row)
}

/// Creates a doc at the app level of a project or in one of its features. The
/// doc is readable at once: its head is the empty doc of its kind, and its
/// creation is the first row of its log.
async fn create_live_doc(
    state: &AppState,
    request: &AgentRequest,
    identity: &AgentIdentity,
    project_id: &str,
    feature_id: Option<&str>,
    body: &Bytes,
) -> Result<(StatusCode, Json<DocResponse>), ApiError> {
    let new = parse_body::<CreateDocRequest>(body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, project_id).await?;
    if let Some(feature_id) = feature_id {
        let found: Option<(String,)> = sqlx::query_as(
            "SELECT id::text FROM features \
             WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL",
        )
        .bind(feature_id)
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        found.ok_or(ApiError::NotFound)?;
    }
    let new = check_new_doc(new?, feature_id.is_some())?;
    require_no_design_system(&mut tx, project_id, &new).await?;
    tx.commit().await.map_err(ApiError::database)?;

    let empty = store_empty_head(state, &request.workspace_id, &new.kind).await?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    if let Some(feature_id) = feature_id {
        // A feature that is deleted in between takes no new doc.
        let found: Option<(String,)> = sqlx::query_as(
            "SELECT id::text FROM features \
             WHERE id = $1::uuid AND deleted_at IS NULL FOR SHARE",
        )
        .bind(feature_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        found.ok_or(ApiError::NotFound)?;
    }
    let row = insert_doc(
        &mut tx,
        &request.workspace_id,
        project_id,
        feature_id,
        &new,
        &empty,
        &identity.to_json(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, project_id);
    Ok((
        StatusCode::CREATED,
        Json(DocResponse { doc: doc_body(row) }),
    ))
}

/// Creates a doc at the app level of a project. The doc is readable at once, as
/// the empty doc of its kind.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = CreateDocRequest,
    responses(
        (status = 201, description = "The new doc", body = DocResponse),
        (status = 400, description = "The kind, pillar or title is missing or not allowed", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
        (status = 409, description = "The project has a design system doc already (`design_system_exists`)", body = ErrorBody),
        (status = 422, description = "A design or design-system doc outside the `ui` pillar (`design_needs_ui_pillar`)", body = ErrorBody),
    )
)]
pub async fn create_app_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<DocResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    create_live_doc(&state, &request, &identity, &project_id, None, &body).await
}

/// Creates a doc in a feature of a project. The doc is readable at once, as the
/// empty doc of its kind.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/docs",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = CreateDocRequest,
    responses(
        (status = 201, description = "The new doc", body = DocResponse),
        (status = 400, description = "The kind, pillar or title is missing or not allowed", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace", body = ErrorBody),
        (status = 422, description = "A design or design-system doc outside the `ui` pillar (`design_needs_ui_pillar`), or a design system in a feature, which is always global (`design_system_is_global`)", body = ErrorBody),
    )
)]
pub async fn create_feature_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<DocResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    create_live_doc(
        &state,
        &request,
        &identity,
        &project_id,
        Some(&feature_id),
        &body,
    )
    .await
}

/// The docs of a project, oldest first, each as `GET .../docs/{doc_id}` answers it.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature" = Option<String>, Query, description = "A feature's UUID lists that feature's docs only; `global` lists the project's global docs only; without it, every doc of the project"),
        ("pillar" = Option<String>, Query, description = "`spec`, `ui`, `architecture`, `db` or `research` lists that pillar's docs only; without it, every pillar"),
    ),
    responses(
        (status = 200, description = "The project's docs", body = DocListResponse),
        (status = 400, description = "`feature` is neither `global` nor a UUID, or `pillar` is none of the five pillars", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn list_docs(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<DocListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature = match query.get("feature").map(String::as_str) {
        None => None,
        Some("global") => Some(None),
        Some(id) => Some(Some(
            parse_uuid(id).ok_or_else(|| ApiError::bad_request("invalid_request"))?,
        )),
    };
    let scope = match &feature {
        None => Scope::Project,
        Some(None) => Scope::Global,
        Some(Some(id)) => Scope::Feature(id),
    };
    let pillar = match query.get("pillar").map(String::as_str) {
        None => None,
        Some(pillar) if PILLARS.contains(&pillar) => Some(pillar),
        Some(_) => return Err(ApiError::bad_request("invalid_request")),
    };
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let docs = docs_of_project(&mut tx, &project_id, scope, pillar).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(DocListResponse { docs }))
}

/// A doc's metadata.
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
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row = find_doc(&mut tx, &project_id, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(DocResponse { doc: doc_body(row) }))
}

#[derive(Deserialize, ToSchema)]
pub struct RenameDocRequest {
    /// The doc's new title; not blank.
    pub title: Option<String>,
}

/// The name in the log of a head that is empty: the chunk of the empty text.
/// A doc that has no head yet is logged with it.
pub(crate) fn empty_head() -> String {
    blake3_hex(b"")
}

/// Locks a doc of the project that is not deleted; 404 when there is none.
/// Returns its head.
async fn lock_doc(tx: &mut Tx, project_id: &str, doc_id: &str) -> Result<Option<String>, ApiError> {
    let found: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT head FROM docs \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(doc_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found.map(|(head,)| head).ok_or(ApiError::NotFound)
}

/// Renames a doc. Its ID, head and comments stay; the rename is one row of the
/// doc's log, with the head before and after the same.
#[utoipa::path(
    patch,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = RenameDocRequest,
    responses(
        (status = 200, description = "The renamed doc", body = DocResponse),
        (status = 400, description = "The title is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, or it was deleted", body = ErrorBody),
    )
)]
pub async fn rename_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<DocResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<RenameDocRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let head = lock_doc(&mut tx, &project_id, &doc_id).await?;
    let title = non_blank(new?.title).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    sqlx::query("UPDATE docs SET title = $2 WHERE id = $1::uuid")
        .bind(&doc_id)
        .bind(&title)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    log_change(
        &mut tx,
        &request.workspace_id,
        &doc_id,
        &identity.to_json(),
        &json!([{ "op": "rename", "title": title }]),
        head.as_deref(),
        &head.clone().unwrap_or_else(empty_head),
    )
    .await?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "doc.renamed",
        &json!({ "doc_id": doc_id, "title": title }),
    )
    .await?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "doc.changed",
        &json!({ "doc_id": doc_id, "head": head.clone().unwrap_or_else(empty_head) }),
    )
    .await?;
    let row = find_doc(&mut tx, &project_id, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(DocResponse { doc: doc_body(row) }))
}

/// Deletes a doc. Nothing is erased: the row gets `deleted_at`, the doc and its
/// comments leave every list and read, and the deletion is one row of the
/// doc's log. A deleted doc takes no more writes.
#[utoipa::path(
    delete,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    responses(
        (status = 204, description = "The doc is deleted"),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, or it was deleted already", body = ErrorBody),
    )
)]
pub async fn delete_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let head = lock_doc(&mut tx, &project_id, &doc_id).await?;
    sqlx::query("UPDATE docs SET deleted_at = clock_timestamp() WHERE id = $1::uuid")
        .bind(&doc_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    log_change(
        &mut tx,
        &request.workspace_id,
        &doc_id,
        &identity.to_json(),
        &json!([{ "op": "delete" }]),
        head.as_deref(),
        &head.clone().unwrap_or_else(empty_head),
    )
    .await?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "doc.deleted",
        &json!({ "doc_id": doc_id }),
    )
    .await?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "doc.changed",
        &json!({ "doc_id": doc_id, "head": head.clone().unwrap_or_else(empty_head) }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(StatusCode::NO_CONTENT)
}

/// A doc's head as HTML with `data-omni-id`: a new doc reads as the empty doc.
/// A design's head answers its `pages` and `tags` and no HTML.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/html",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("view" = Option<String>, Query, description = "`outline` for the head of a design as its outline (its pages with their boards as placeholders, and the chunk of each board) instead of its whole body; the boards come one by one from `GET .../docs/{doc_id}/boards/{board_id}`. `previews` for small pictures of the boards that `boards` names instead. `node-boxes` for where the nodes that `nodes` names sit on their boards instead"),
        ("boards" = Option<String>, Query, description = "With `view=previews`, the IDs of the boards, separated by commas"),
        ("nodes" = Option<String>, Query, description = "With `view=node-boxes`, the node addresses, separated by commas"),
    ),
    responses(
        (status = 200, description = "The head's HTML", body = HeadHtmlResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace", body = ErrorBody),
    )
)]
pub async fn read_doc_html(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<HeadHtmlResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row = find_doc(&mut tx, &project_id, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let view = query.get("view").map(String::as_str);
    if view == Some("previews") && row.kind == "design" {
        let wanted = query.get("boards").map(String::as_str).unwrap_or_default();
        let wanted = wanted.split(',').collect();
        let previews = match &row.head {
            Some(head) => {
                previews::of_boards(
                    &state,
                    &request.workspace_id,
                    &row.project_id,
                    head,
                    &wanted,
                )
                .await?
            }
            None => BTreeMap::new(),
        };
        return Ok(Json(HeadHtmlResponse {
            doc_id: row.id,
            html: String::new(),
            tags: None,
            pages: None,
            tokens: None,
            components: None,
            outline: None,
            boards: None,
            previews: Some(previews),
            node_boxes: None,
        }));
    }
    if view == Some("node-boxes") && row.kind == "design" {
        let wanted = query.get("nodes").map(String::as_str).unwrap_or_default();
        let wanted: Vec<&str> = wanted.split(',').filter(|node| !node.is_empty()).collect();
        let boxes = match &row.head {
            Some(head) if !wanted.is_empty() => {
                pins::of_nodes(
                    &state,
                    &request.workspace_id,
                    &row.project_id,
                    head,
                    &wanted,
                )
                .await?
            }
            _ => BTreeMap::new(),
        };
        return Ok(Json(HeadHtmlResponse {
            doc_id: row.id,
            html: String::new(),
            tags: None,
            pages: None,
            tokens: None,
            components: None,
            outline: None,
            boards: None,
            previews: None,
            node_boxes: Some(boxes),
        }));
    }
    let outline = view == Some("outline");
    read_head(&state, &request.workspace_id, row, outline)
        .await
        .map(Json)
}

/// A doc's head as the read of the doc answers it.
async fn read_head(
    state: &AppState,
    workspace_id: &str,
    row: DocRow,
    outline: bool,
) -> Result<HeadHtmlResponse, ApiError> {
    let mut answer = HeadHtmlResponse {
        doc_id: row.id,
        html: String::new(),
        tags: None,
        pages: None,
        tokens: None,
        components: None,
        outline: None,
        boards: None,
        previews: None,
        node_boxes: None,
    };
    if row.kind == "design" && outline {
        let view =
            design::outline_of_head(state, workspace_id, &row.project_id, row.head.as_deref())
                .await?;
        answer.boards = Some(match &row.head {
            Some(head) => boards::head_boards(state, workspace_id, head).await?,
            None => Vec::new(),
        });
        answer.outline = Some(view.outline);
        answer.tokens = view.tokens;
        answer.components = Some(view.components);
    } else if row.kind == "design" {
        let pages = match &row.head {
            Some(head) => {
                let what = format!("the head of doc {}", answer.doc_id);
                boards::read_pages(state, workspace_id, head, &what).await?
            }
            None => Vec::new(),
        };
        // The design resolves against the design system's head as it is now.
        let system = design::system_head_of_project(state, workspace_id, &row.project_id).await?;
        answer.tags = Some(design::tags_of(&pages));
        answer.pages = Some(design::nodes_values(&pages));
        answer.tokens = system.tokens_json();
        answer.components = Some(system.components_used_by(&pages));
    } else if let Some(head) = &row.head {
        answer.html = chunks::read_chunk(state, workspace_id, head).await?;
    }
    Ok(answer)
}

/// Applies a batch of operations to the head of an HTML doc or a design, all or
/// nothing. The batch runs under the lock on the doc's row, so batches to one
/// doc are applied one after the other, each to the head the one before made.
/// The new head of an HTML doc is a chunk in RustFS that holds
/// the doc as HTML with `data-omni-id` and `data-rev`, which is where a node's
/// revision is kept; the new head of a design is a manifest that lists the
/// chunk of every page and board and the revisions, and only the boards the
/// batch changed get a new chunk (`design::edit_head`). Nothing about a head is
/// held in memory. A batch that is refused changes, logs and emits nothing; an
/// applied one adds one row of `doc_changes` and emits `doc.changed`.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/ops",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = EditRequest,
    responses(
        (status = 200, description = "The batch applied; one result per operation", body = EditResponse),
        (status = 400, description = "The body is not a batch (`invalid_request`), the doc is a design system (`not_an_html_doc`), or an operation is refused (`invalid_operation`, naming an unknown ID, or for a design an instance of a component the design system lacks)", body = InvalidOperationBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, or `board` is no board of the design (`unknown_board`)", body = ErrorBody),
        (status = 409, description = "A node is at another revision than the operation expected (`conflict`); nothing changed", body = ConflictBody),
    )
)]
pub async fn edit_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<EditResponse>, DocError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let workspace_id = request.workspace_id;
    let batch = parse_body::<EditRequest>(&body);

    // The lock comes first and the batch holds it to its commit, and the only
    // connection it uses is its own.
    let mut tx = state.workspace_tx(&workspace_id).await?;
    let locked: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT kind, head FROM docs \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(&doc_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (kind, head_before) = locked.ok_or(ApiError::NotFound)?;
    let batch = batch?;
    let (sent, board) = (batch.ops, batch.board);
    if kind != "html" && kind != "design" {
        return Err(ApiError::bad_request("not_an_html_doc").into());
    }
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

    if kind == "design" {
        let agent = identity.to_json();
        let results = design::edit_head(
            &state,
            &mut tx,
            &workspace_id,
            &project_id,
            &doc_id,
            head_before.as_deref(),
            &agent,
            sent,
            ops,
            board,
        )
        .await?;
        tx.commit().await.map_err(ApiError::database)?;
        if !results.is_empty() {
            state.live.announce(&workspace_id, &project_id);
        }
        return Ok(Json(EditResponse { results }));
    }

    let html = match &head_before {
        Some(head) => chunks::read_chunk(&state, &workspace_id, head).await?,
        None => String::new(),
    };
    let mut doc = EditDoc::from_html(&html).map_err(|e| {
        eprintln!("omni-api: the head of doc {doc_id} cannot be loaded: {e}");
        ApiError::Unavailable
    })?;
    let mut used: HashSet<NodeId> = doc.index().iter().map(|node| node.id).collect();
    let mut new_ids = || loop {
        let id = random_node_id();
        if used.insert(id) {
            return id;
        }
    };
    let applied = match doc.apply(&ops, &mut new_ids) {
        Ok(applied) => applied,
        Err(ApplyError::Conflict(conflict)) => return Err(DocError::Conflict(conflict)),
        Err(ApplyError::Invalid { index, message }) => {
            return Err(DocError::Invalid { index, message });
        }
    };
    let results = results_of(&applied);
    if results.is_empty() {
        return Ok(Json(EditResponse { results }));
    }

    let snapshot = Snapshot::new(doc.to_html_with_revisions(), doc.index());
    let head_after = chunks::store_text(&state, &workspace_id, &snapshot.html).await?;
    chunks::index_snapshot(&mut tx, &workspace_id, &head_after, &snapshot).await?;
    sqlx::query("UPDATE docs SET head = $2 WHERE id = $1::uuid")
        .bind(&doc_id)
        .bind(&head_after)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    log_change_of_boards(
        &mut tx,
        &workspace_id,
        &doc_id,
        &identity.to_json(),
        &Value::Array(sent),
        head_before.as_deref(),
        &head_after,
        None,
        Some(&json!(results)),
    )
    .await?;
    live::emit(
        &mut tx,
        &workspace_id,
        &project_id,
        "doc.changed",
        &json!({ "doc_id": doc_id, "head": head_after }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&workspace_id, &project_id);
    Ok(Json(EditResponse { results }))
}

/// The results of an applied batch as the API answers them.
pub(crate) fn results_of(applied: &omni_core::ops::Applied) -> Vec<OpResultBody> {
    applied
        .ops
        .iter()
        .map(|result| OpResultBody {
            node: result.node.to_string(),
            revision: result.revision,
            created: result.created.iter().map(NodeId::to_string).collect(),
            removed: result.removed.iter().map(NodeId::to_string).collect(),
        })
        .collect()
}
