//! Comments on nodes (US-038, reshaped by US-106, Q4, V17, V18, FR-19 to
//! FR-21). The reviewer pins a comment to one node address of a doc's head, down
//! to a single node inside a component instance of a design
//! (`<instanceId>/<componentNodeId>`), so that feedback lands on the exact
//! element. An agent replies; the human deletes the comment when satisfied. A
//! comment has no status.
//!
//! A comment belongs to a doc and a node address, with its author and its text;
//! the project and the feature are those of its doc. A deletion is soft
//! (`deleted_at`): the API role cannot delete rows, and a deleted comment is
//! left out of every list and read. A comment whose node later leaves the head
//! stays listed (V18); nothing is checked after it was made.
//!
//! A comment `waits on you` (`waiting_on_you`) when its latest reply is an
//! agent's: the human has not answered it yet.
//!
//! `POST .../versions/{version_id}/comments` stays until the web app moves to
//! the live routes (US-113). It pins a comment to a node of that saved version
//! on the version's doc, and records the version.
//!
//! Commenting emits `comment.created`, a reply `comment.replied` and a delete
//! `comment.deleted`.

use crate::AppState;
use crate::boards::Manifest;
use crate::changesets::{non_blank, parse_body, require_project};
use crate::design;
use crate::docs::{AgentIdentity, DocRow, Tx, find_doc};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::versions::{self, nodes_of_version};
use crate::workspace::{Actor, HumanRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use utoipa::ToSchema;

/// The reply to a comment.
#[derive(Serialize, ToSchema)]
pub struct ReplyBody {
    pub id: String,
    /// `{"kind": "human", "user_id"}` or `{"kind": "agent", "run", "role",
    /// "model", "harness"}`.
    #[schema(value_type = Object)]
    pub author: Value,
    pub text: String,
    pub created_at: String,
}

/// A comment pinned to a node of a doc.
#[derive(Serialize, ToSchema)]
pub struct CommentBody {
    pub id: String,
    pub project_id: String,
    pub doc_id: String,
    /// The saved version a comment was pinned to through
    /// `POST .../versions/{version_id}/comments`; `null` for a comment on the
    /// doc's head.
    pub version_id: Option<String>,
    /// The node address as it was sent: a node ID of the doc's head, or
    /// `<instanceId>/<componentNodeId>` inside a component instance.
    pub node: String,
    /// The feature of the doc; `null` at the app level.
    pub feature_id: Option<String>,
    #[schema(value_type = Object)]
    pub author: Value,
    pub text: String,
    /// The replies, oldest first.
    pub replies: Vec<ReplyBody>,
    /// `true` when the latest reply is an agent's: the comment waits for the
    /// human. A comment without replies, or whose latest reply is the human's,
    /// does not.
    #[schema(required = false)]
    pub waiting_on_you: bool,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct CommentResponse {
    pub comment: CommentBody,
}

#[derive(Serialize, ToSchema)]
pub struct SingleCommentResponse {
    pub project_id: String,
    pub comment: CommentBody,
}

#[derive(Serialize, ToSchema)]
pub struct CommentListResponse {
    pub project_id: String,
    /// Oldest first.
    pub comments: Vec<CommentBody>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateCommentRequest {
    /// A node ID of the head (or of the version), or
    /// `<instanceId>/<componentNodeId>` for a node inside a component instance
    /// of a design.
    pub node: Option<String>,
    /// What the comment says; not blank.
    pub text: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct ReplyRequest {
    /// Not blank.
    pub text: Option<String>,
}

fn invalid() -> ApiError {
    ApiError::bad_request("invalid_request")
}

fn not_in_head() -> ApiError {
    ApiError::Refused(StatusCode::UNPROCESSABLE_ENTITY, "node_not_in_head")
}

/// Who writes a comment or a reply, as the JSON that is stored.
fn author_of(actor: &Actor, identity: &AgentIdentity) -> Value {
    match actor {
        Actor::Agent => {
            let mut author = identity.to_json();
            author["kind"] = json!("agent");
            author
        }
        Actor::Human { user_id, .. } => json!({ "kind": "human", "user_id": user_id }),
    }
}

// Reading comments ----------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct CommentRow {
    id: String,
    project_id: String,
    doc_id: String,
    version_id: Option<String>,
    node: String,
    feature_id: Option<String>,
    author: String,
    text: String,
    created_at: String,
}

/// The rows of the comments that satisfy every condition, oldest first. A
/// condition has `{}` where the next parameter goes; the values are bound in the
/// order they were added, and are text that the SQL casts.
#[derive(Default)]
struct Filter {
    conditions: Vec<String>,
    values: Vec<String>,
}

impl Filter {
    fn and(mut self, condition: &str, value: &str) -> Self {
        self.values.push(value.to_string());
        let parameter = format!("${}", self.values.len());
        self.conditions.push(condition.replace("{}", &parameter));
        self
    }
}

/// The comments that satisfy the filter and that nobody deleted; `lock` is a
/// locking clause such as `FOR UPDATE OF c`, or empty. A comment belongs to a
/// project and a feature through its doc.
async fn fetch_rows(tx: &mut Tx, filter: Filter, lock: &str) -> Result<Vec<CommentRow>, ApiError> {
    let sql = format!(
        "SELECT c.id::text AS id, d.project_id::text AS project_id, c.doc_id::text AS doc_id, \
                c.version_id::text AS version_id, c.node AS node, \
                d.feature_id::text AS feature_id, \
                c.author::text AS author, c.body AS text, \
                to_char(c.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at \
         FROM comments c \
         JOIN docs d ON d.workspace_id = c.workspace_id AND d.id = c.doc_id \
         WHERE c.deleted_at IS NULL{}{} ORDER BY c.created_at, c.id {lock}",
        if filter.conditions.is_empty() {
            ""
        } else {
            " AND "
        },
        filter.conditions.join(" AND ")
    );
    let mut query = sqlx::query_as::<_, CommentRow>(&sql);
    for value in &filter.values {
        query = query.bind(value);
    }
    query.fetch_all(&mut **tx).await.map_err(ApiError::database)
}

#[derive(sqlx::FromRow)]
struct ReplyRow {
    comment_id: String,
    id: String,
    author: String,
    text: String,
    created_at: String,
}

fn unreadable(what: &str, id: &str, error: serde_json::Error) -> ApiError {
    eprintln!("omni-api: the {what} of {id} is unreadable: {error}");
    ApiError::Unavailable
}

/// The comments with their replies.
async fn bodies(tx: &mut Tx, rows: Vec<CommentRow>) -> Result<Vec<CommentBody>, ApiError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    let replies: Vec<ReplyRow> = sqlx::query_as(
        "SELECT r.comment_id::text AS comment_id, r.id::text AS id, r.author::text AS author, \
                r.body AS text, \
                to_char(r.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at \
         FROM comment_replies r WHERE r.comment_id = ANY($1::text[]::uuid[]) \
         ORDER BY r.created_at, r.id",
    )
    .bind(&ids)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let mut replied: HashMap<String, Vec<ReplyBody>> = HashMap::new();
    for reply in replies {
        let author =
            serde_json::from_str(&reply.author).map_err(|e| unreadable("author", &reply.id, e))?;
        replied
            .entry(reply.comment_id)
            .or_default()
            .push(ReplyBody {
                id: reply.id,
                author,
                text: reply.text,
                created_at: reply.created_at,
            });
    }
    rows.into_iter()
        .map(|row| {
            let author =
                serde_json::from_str(&row.author).map_err(|e| unreadable("author", &row.id, e))?;
            let replies = replied.remove(&row.id).unwrap_or_default();
            let waiting_on_you = replies
                .last()
                .is_some_and(|reply| reply.author["kind"] == "agent");
            Ok(CommentBody {
                replies,
                id: row.id,
                project_id: row.project_id,
                doc_id: row.doc_id,
                version_id: row.version_id,
                node: row.node,
                feature_id: row.feature_id,
                author,
                text: row.text,
                waiting_on_you,
                created_at: row.created_at,
            })
        })
        .collect()
}

/// The comment of the project, 404 when it has none or it was deleted. `lock`
/// is as in `fetch_rows`.
async fn load_row(
    tx: &mut Tx,
    project_id: &str,
    comment_id: &str,
    lock: &str,
) -> Result<CommentRow, ApiError> {
    let filter = Filter::default()
        .and("c.id = {}::uuid", comment_id)
        .and("d.project_id = {}::uuid", project_id);
    fetch_rows(tx, filter, lock)
        .await?
        .pop()
        .ok_or(ApiError::NotFound)
}

async fn load_comment(
    tx: &mut Tx,
    project_id: &str,
    comment_id: &str,
) -> Result<CommentBody, ApiError> {
    let row = load_row(tx, project_id, comment_id, "").await?;
    bodies(tx, vec![row]).await?.pop().ok_or(ApiError::NotFound)
}

// Making comments -----------------------------------------------------------------------

/// A node address: a node ID, or a component node inside an instance.
enum Address<'a> {
    Node(&'a str),
    Inside { instance: &'a str, inner: &'a str },
}

impl<'a> Address<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let parts: Vec<&str> = text.split('/').collect();
        match parts.as_slice() {
            [node] if !node.is_empty() => Some(Self::Node(node)),
            [instance, inner] if !instance.is_empty() && !inner.is_empty() => {
                Some(Self::Inside { instance, inner })
            }
            _ => None,
        }
    }
}

/// A comment about to be stored.
struct NewComment<'a> {
    doc_id: &'a str,
    /// The saved version a comment through the version route is pinned to.
    version_id: Option<&'a str>,
    node: &'a str,
    text: &'a str,
    author: &'a Value,
}

/// Stores the comment, emits `comment.created` and answers 201 with it.
async fn insert_comment(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    new: NewComment<'_>,
) -> Result<(StatusCode, Json<CommentResponse>), ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    let comment_id: String = sqlx::query_scalar(
        "INSERT INTO comments (workspace_id, doc_id, version_id, node, author, body) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5::text::jsonb, $6) RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(new.doc_id)
    .bind(new.version_id)
    .bind(new.node)
    .bind(new.author.to_string())
    .bind(new.text)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let comment = load_comment(&mut tx, project_id, &comment_id).await?;
    let data = json!({
        "comment_id": comment.id,
        "doc_id": comment.doc_id,
        "node": comment.node,
    });
    live::emit(&mut tx, workspace_id, project_id, "comment.created", &data).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, project_id);
    Ok((StatusCode::CREATED, Json(CommentResponse { comment })))
}

/// Whether the node address is in the head of `doc`: a node of its node index,
/// or, for a design, a node of the component that an instance of the head
/// places, as the design system's head defines it.
async fn head_has(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    doc: &DocRow,
    address: Address<'_>,
) -> Result<bool, ApiError> {
    let manifest: Option<Manifest> = if doc.kind == "design" {
        versions::head_manifest(state, workspace_id, doc.head.as_deref()).await?
    } else {
        None
    };
    let mut tx = state.workspace_tx(workspace_id).await?;
    let found = match address {
        Address::Node(id) => {
            versions::nodes_of_head(&mut tx, doc.head.as_deref(), manifest.as_ref())
                .await?
                .iter()
                .any(|node| node.id == id)
        }
        Address::Inside { instance, inner } => {
            let Some(manifest) = manifest.as_ref() else {
                return Ok(false);
            };
            let chunks: Vec<&str> = manifest.chunks();
            let chunk: Option<String> = sqlx::query_scalar(
                "SELECT chunk FROM node_index \
                 WHERE chunk = ANY($1::text[]) AND node = $2 AND type = 'instance' LIMIT 1",
            )
            .bind(&chunks)
            .bind(instance)
            .fetch_optional(&mut *tx)
            .await
            .map_err(ApiError::database)?;
            tx.commit().await.map_err(ApiError::database)?;
            return match chunk {
                Some(chunk) => {
                    design::head_instance_holds(
                        state,
                        workspace_id,
                        project_id,
                        &chunk,
                        instance,
                        inner,
                    )
                    .await
                }
                None => Ok(false),
            };
        }
    };
    tx.commit().await.map_err(ApiError::database)?;
    Ok(found)
}

/// Pins a comment to a node of a doc's head.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/comments",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc to comment on"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = CreateCommentRequest,
    responses(
        (status = 201, description = "The new comment", body = CommentResponse),
        (status = 400, description = "The node or the text is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace", body = ErrorBody),
        (status = 422, description = "The doc's head does not have the node address (`node_not_in_head`)", body = ErrorBody),
    )
)]
pub async fn create_on_doc(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<CommentResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateCommentRequest>(&body);
    let author = author_of(&request.actor, &identity);
    let workspace_id = &request.workspace_id;

    let mut tx = state.workspace_tx(workspace_id).await?;
    let doc = find_doc(&mut tx, &project_id, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let new = new?;
    let (Some(node), Some(text)) = (non_blank(new.node), non_blank(new.text)) else {
        return Err(invalid());
    };
    let address = Address::parse(&node).ok_or_else(not_in_head)?;
    if !head_has(&state, workspace_id, &project_id, &doc, address).await? {
        return Err(not_in_head());
    }
    let new = NewComment {
        doc_id: &doc_id,
        version_id: None,
        node: &node,
        text: &text,
        author: &author,
    };
    insert_comment(&state, workspace_id, &project_id, new).await
}

/// Pins a comment to a node of a saved version, on the version's doc. This
/// route stays until the web app moves to the live routes (US-113).
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/comments",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("version_id" = String, Path, description = "The saved version to comment on"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = CreateCommentRequest,
    responses(
        (status = 201, description = "The new comment", body = CommentResponse),
        (status = 400, description = "The text is missing or blank, or the node address names a node that the version does not have", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such version in this project and workspace", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, version_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<CommentResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let version_id = parse_uuid(&version_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateCommentRequest>(&body);
    let author = author_of(&request.actor, &identity);
    let workspace_id = &request.workspace_id;

    let mut tx = state.workspace_tx(workspace_id).await?;
    let version = versions::find_version(&mut tx, &project_id, &version_id).await?;
    let new = new?;
    let (Some(node), Some(text)) = (non_blank(new.node), non_blank(new.text)) else {
        return Err(invalid());
    };
    let nodes = nodes_of_version(&mut tx, &version).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let has = |id: &str, kind: Option<&str>| {
        nodes
            .iter()
            .any(|n| n.id == id && kind.is_none_or(|kind| n.kind == kind))
    };
    match Address::parse(&node).ok_or_else(invalid)? {
        Address::Node(id) if has(id, None) => {}
        Address::Inside { instance, inner }
            if version.doc_kind == "design" && has(instance, Some("instance")) =>
        {
            let holds = design::instance_holds(
                &state,
                workspace_id,
                &version_id,
                version.chunk.as_deref(),
                version.design_system_version_id.as_deref(),
                instance,
                inner,
            )
            .await?;
            if !holds {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    let new = NewComment {
        doc_id: &version.doc_id,
        version_id: Some(&version_id),
        node: &node,
        text: &text,
        author: &author,
    };
    insert_comment(&state, workspace_id, &project_id, new).await
}

/// Adds a reply to a comment.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}/replies",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("comment_id" = String, Path, description = "The comment's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = ReplyRequest,
    responses(
        (status = 201, description = "The comment with the new reply last", body = CommentResponse),
        (status = 400, description = "The text is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such comment in this project and workspace, or it was deleted", body = ErrorBody),
    )
)]
pub async fn reply(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, comment_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<CommentResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let comment_id = parse_uuid(&comment_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<ReplyRequest>(&body);
    let author = author_of(&request.actor, &identity);
    let workspace_id = &request.workspace_id;

    let mut tx = state.workspace_tx(workspace_id).await?;
    load_row(&mut tx, &project_id, &comment_id, "").await?;
    let text = non_blank(new?.text).ok_or_else(invalid)?;
    let reply_id: String = sqlx::query_scalar(
        "INSERT INTO comment_replies (workspace_id, comment_id, author, body) \
         VALUES ($1::uuid, $2::uuid, $3::text::jsonb, $4) RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(&comment_id)
    .bind(author.to_string())
    .bind(&text)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let comment = load_comment(&mut tx, &project_id, &comment_id).await?;
    let data = json!({
        "comment_id": comment.id,
        "reply_id": reply_id,
        "doc_id": comment.doc_id,
    });
    live::emit(&mut tx, workspace_id, &project_id, "comment.replied", &data).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(CommentResponse { comment })))
}

/// Deletes a comment. Only the human does. The row stays, with `deleted_at` set,
/// and the comment is left out of every list and read.
#[utoipa::path(
    delete,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("comment_id" = String, Path, description = "The comment's UUID"),
    ),
    responses(
        (status = 204, description = "The comment is deleted"),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such comment in this project and workspace, or it was deleted already", body = ErrorBody),
    )
)]
pub async fn delete(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, comment_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let comment_id = parse_uuid(&comment_id).ok_or(ApiError::NotFound)?;
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    let row = load_row(&mut tx, &project_id, &comment_id, "FOR UPDATE OF c").await?;
    sqlx::query("UPDATE comments SET deleted_at = clock_timestamp() WHERE id = $1::uuid")
        .bind(&comment_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    let data = json!({ "comment_id": comment_id, "doc_id": row.doc_id });
    live::emit(&mut tx, workspace_id, &project_id, "comment.deleted", &data).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok(StatusCode::NO_CONTENT)
}

// Reading -------------------------------------------------------------------------------

/// One comment of a project.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("comment_id" = String, Path, description = "The comment's UUID"),
    ),
    responses(
        (status = 200, description = "The comment with its replies", body = SingleCommentResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such comment in this project and workspace, or it was deleted", body = ErrorBody),
    )
)]
pub async fn show(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, comment_id)): Path<(String, String, String)>,
) -> Result<Json<SingleCommentResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let comment_id = parse_uuid(&comment_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let comment = load_comment(&mut tx, &project_id, &comment_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(SingleCommentResponse {
        project_id,
        comment,
    }))
}

/// The comments of a project that nobody deleted, oldest first, narrowed by
/// `doc` and `feature`, which combine.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/comments",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc" = Option<String>, Query, description = "Only the comments on this doc"),
        ("feature" = Option<String>, Query, description = "Only the comments on the docs of this feature"),
    ),
    responses(
        (status = 200, description = "The comments", body = CommentListResponse),
        (status = 400, description = "A filter is not valid (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<CommentListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let mut filter = Filter::default().and("d.project_id = {}::uuid", &project_id);
    if let Some(doc) = query.get("doc") {
        let doc = parse_uuid(doc).ok_or_else(invalid)?;
        filter = filter.and("c.doc_id = {}::uuid", &doc);
    }
    if let Some(feature) = query.get("feature") {
        let feature = parse_uuid(feature).ok_or_else(invalid)?;
        filter = filter.and("d.feature_id = {}::uuid", &feature);
    }
    let rows = fetch_rows(&mut tx, filter, "").await?;
    let comments = bodies(&mut tx, rows).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(CommentListResponse {
        project_id,
        comments,
    }))
}

/// The comments of a project that wait on the human, oldest first: those whose
/// latest reply is an agent's.
pub(crate) async fn waiting_of_project(
    tx: &mut Tx,
    project_id: &str,
) -> Result<Vec<CommentBody>, ApiError> {
    let filter = Filter::default().and("d.project_id = {}::uuid", project_id);
    let rows = fetch_rows(tx, filter, "").await?;
    let mut comments = bodies(tx, rows).await?;
    comments.retain(|comment| comment.waiting_on_you);
    Ok(comments)
}
