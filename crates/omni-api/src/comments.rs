//! Comments on nodes (US-038, Q4, Q19, FR-40, FR-41). The reviewer pins a
//! comment to one node of a saved version, down to a single node inside a
//! component instance of a design (`<instanceId>/<componentNodeId>`), so that
//! feedback lands on the exact element and stays traceable. Comments go on
//! saved versions and never on live drafts.
//!
//! A comment records the version it was made on, its author and its text; the
//! doc, the project and the change set are those of its version. During a
//! review it belongs to the version's change set; once that change set has
//! merged it is a comment on the mainline. It has replies, and goes `open`, then
//! `addressed` (an agent names the version that fixes it), then `resolved`. Only
//! the human resolves and reopens. A comment is never deleted.
//!
//! Whether a comment's node changed is not stored: the node index of the
//! version it was made on is compared with the newest version of the doc saved
//! in the comment's change set, or with the doc's mainline version for a
//! mainline comment. A node that has another fingerprint there makes the
//! comment `changed` (it was made on an older version) and a node that is gone
//! makes it `removed`; the comment stays either way. For an address inside an
//! instance the instance is the node that is compared.
//!
//! Saving a version that drops a node with an open comment is refused with 409
//! `commented_nodes_removed`, which lists those comments, unless the body says
//! `remove_commented: true` (`refuse_commented_removals`).
//!
//! Commenting emits `comment.created`, a reply `comment.replied` and every
//! change of status `comment.status_changed`.

use crate::AppState;
use crate::changesets::{non_blank, parse_body, require_project};
use crate::design;
use crate::docs::{AgentIdentity, DraftRow, Tx};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::versions::{self, nodes_of_version, nodes_of_version_id};
use crate::workspace::{Actor, AgentRequest, HumanRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

/// The statuses of a comment, in the order it goes through them.
const STATUSES: [&str; 3] = ["open", "addressed", "resolved"];

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

/// A comment pinned to a node of a saved version.
#[derive(Serialize, ToSchema)]
pub struct CommentBody {
    pub id: String,
    pub project_id: String,
    /// The doc of the version.
    pub doc_id: String,
    /// The version the comment was made on; it never changes.
    pub version_id: String,
    /// The node address as it was sent: a node ID of the version, or
    /// `<instanceId>/<componentNodeId>` inside a component instance.
    pub node: String,
    /// The change set of the version, while it is under review; `null` for a
    /// comment on the mainline, which is one whose change set has merged.
    pub changeset_id: Option<String>,
    #[schema(value_type = Object)]
    pub author: Value,
    pub text: String,
    /// `open`, `addressed` or `resolved`.
    pub status: String,
    /// The version that an agent named when it marked the comment addressed.
    pub addressed_in: Option<String>,
    /// The replies, oldest first.
    pub replies: Vec<ReplyBody>,
    /// `current` when the version that the comment is compared with has the node
    /// as it was, `changed` when it has the node with other content (the comment
    /// was made on an older version) and `removed` when it no longer has the
    /// node.
    pub node_status: String,
    /// The version the node is compared with: the newest version of the doc
    /// saved in the comment's change set, or the doc's mainline version for a
    /// mainline comment.
    pub compared_to: Option<String>,
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
    /// A node ID of the version, or `<instanceId>/<componentNodeId>` for a node
    /// inside a component instance of a design version.
    pub node: Option<String>,
    /// What the comment says; not blank.
    pub text: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct ReplyRequest {
    /// Not blank.
    pub text: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct AddressedRequest {
    /// The version that fixes the comment: a version of the comment's doc saved
    /// after the version the comment was made on.
    pub version_id: Option<String>,
}

fn invalid() -> ApiError {
    ApiError::bad_request("invalid_request")
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

/// The node of the version that a comment address is compared by: the whole
/// address, or the instance of an address inside an instance.
fn node_of(address: &str) -> &str {
    address.split('/').next().unwrap_or(address)
}

// Reading comments ----------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct CommentRow {
    id: String,
    project_id: String,
    doc_id: String,
    version_id: String,
    node: String,
    changeset_id: Option<String>,
    author: String,
    text: String,
    status: String,
    addressed_in: Option<String>,
    created_at: String,
    compared_to: Option<String>,
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

    fn and_that(mut self, condition: &str) -> Self {
        self.conditions.push(condition.to_string());
        self
    }
}

/// The comments that satisfy the filter; `lock` is a locking clause such as
/// `FOR UPDATE OF c`, or empty. A comment belongs to a project through the doc
/// of its version. A comment of a merged change set is on the mainline, so it
/// names no change set and is compared with the doc's mainline version.
async fn fetch_rows(tx: &mut Tx, filter: Filter, lock: &str) -> Result<Vec<CommentRow>, ApiError> {
    let sql = format!(
        "SELECT c.id::text AS id, d.project_id::text AS project_id, v.doc_id::text AS doc_id, \
                c.version_id::text AS version_id, c.node AS node, \
                CASE WHEN cs.status = 'merged' THEN NULL ELSE v.changeset_id::text END AS changeset_id, \
                c.author::text AS author, c.body AS text, c.status AS status, \
                c.addressed_in_version_id::text AS addressed_in, \
                to_char(c.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at, \
                CASE WHEN cs.status = 'merged' THEN d.mainline_version_id::text ELSE \
                    (SELECT nv.id::text FROM versions nv \
                     WHERE nv.workspace_id = v.workspace_id AND nv.doc_id = v.doc_id \
                       AND nv.changeset_id = v.changeset_id \
                     ORDER BY nv.created_at DESC, nv.id DESC LIMIT 1) \
                END AS compared_to \
         FROM comments c \
         JOIN versions v ON v.workspace_id = c.workspace_id AND v.id = c.version_id \
         JOIN docs d ON d.workspace_id = v.workspace_id AND d.id = v.doc_id \
         JOIN changesets cs ON cs.workspace_id = v.workspace_id AND cs.id = v.changeset_id \
         WHERE {} ORDER BY c.created_at, c.id {lock}",
        if filter.conditions.is_empty() {
            "true".to_string()
        } else {
            filter.conditions.join(" AND ")
        }
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

/// The fingerprint of every node of each version, by version and node ID.
async fn fingerprints(
    tx: &mut Tx,
    versions: HashSet<&str>,
) -> Result<HashMap<String, HashMap<String, String>>, ApiError> {
    let mut found = HashMap::new();
    for version in versions {
        let nodes = nodes_of_version_id(tx, version).await?;
        found.insert(
            version.to_string(),
            nodes
                .into_iter()
                .map(|node| (node.id, node.fingerprint))
                .collect(),
        );
    }
    Ok(found)
}

/// Whether the node is as it was when the comment was made (`current`), has
/// other content in the version it is compared with (`changed`) or is gone from
/// it (`removed`).
fn node_status(
    address: &str,
    own: Option<&HashMap<String, String>>,
    compared: Option<&HashMap<String, String>>,
) -> &'static str {
    let node = node_of(address);
    match compared.and_then(|nodes| nodes.get(node)) {
        None => "removed",
        Some(now) if own.and_then(|nodes| nodes.get(node)) == Some(now) => "current",
        Some(_) => "changed",
    }
}

/// The comments with their replies and the state of their nodes.
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

    let wanted: HashSet<&str> = rows
        .iter()
        .flat_map(|row| [Some(row.version_id.as_str()), row.compared_to.as_deref()])
        .flatten()
        .collect();
    let prints = fingerprints(tx, wanted).await?;
    rows.into_iter()
        .map(|row| {
            let status = node_status(
                &row.node,
                prints.get(&row.version_id),
                row.compared_to
                    .as_ref()
                    .and_then(|version| prints.get(version)),
            );
            let author =
                serde_json::from_str(&row.author).map_err(|e| unreadable("author", &row.id, e))?;
            Ok(CommentBody {
                replies: replied.remove(&row.id).unwrap_or_default(),
                id: row.id,
                project_id: row.project_id,
                doc_id: row.doc_id,
                version_id: row.version_id,
                node: row.node,
                changeset_id: row.changeset_id,
                author,
                text: row.text,
                status: row.status,
                addressed_in: row.addressed_in,
                node_status: status.to_string(),
                compared_to: row.compared_to,
                created_at: row.created_at,
            })
        })
        .collect()
}

/// The comment of the project, 404 when it has none. `lock` is as in
/// `fetch_rows`.
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
    fn parse(text: &'a str) -> Result<Self, ApiError> {
        let parts: Vec<&str> = text.split('/').collect();
        match parts.as_slice() {
            [node] if !node.is_empty() => Ok(Self::Node(node)),
            [instance, inner] if !instance.is_empty() && !inner.is_empty() => {
                Ok(Self::Inside { instance, inner })
            }
            _ => Err(invalid()),
        }
    }
}

/// Pins a comment to a node of a saved version.
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
        (status = 201, description = "The new comment, `open`", body = CommentResponse),
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
    match Address::parse(&node)? {
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

    let mut tx = state.workspace_tx(workspace_id).await?;
    let comment_id: String = sqlx::query_scalar(
        "INSERT INTO comments (workspace_id, version_id, node, author, body) \
         VALUES ($1::uuid, $2::uuid, $3, $4::text::jsonb, $5) RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(&version_id)
    .bind(&node)
    .bind(author.to_string())
    .bind(&text)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let comment = load_comment(&mut tx, &project_id, &comment_id).await?;
    let data = json!({
        "comment_id": comment.id,
        "doc_id": comment.doc_id,
        "version_id": comment.version_id,
        "node": comment.node,
        "changeset_id": comment.changeset_id,
        "status": comment.status,
    });
    live::emit(&mut tx, workspace_id, &project_id, "comment.created", &data).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(CommentResponse { comment })))
}

/// Adds a reply to a comment. The comment keeps its status.
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
        (status = 404, description = "No such comment in this project and workspace", body = ErrorBody),
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
        (status = 200, description = "The comment with its replies and the state of its node", body = SingleCommentResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such comment in this project and workspace", body = ErrorBody),
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

/// The comments of a project, oldest first, narrowed by `doc`, `changeset` and
/// `status`, which combine.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/comments",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc" = Option<String>, Query, description = "Only the comments on versions of this doc"),
        ("changeset" = Option<String>, Query, description = "Only the comments made in this change set during its review"),
        ("status" = Option<String>, Query, description = "Only the comments with this status: open, addressed or resolved"),
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
        filter = filter.and("v.doc_id = {}::uuid", &doc);
    }
    if let Some(changeset) = query.get("changeset") {
        let changeset = parse_uuid(changeset).ok_or_else(invalid)?;
        filter = filter
            .and("v.changeset_id = {}::uuid", &changeset)
            .and_that("cs.status <> 'merged'");
    }
    if let Some(status) = query.get("status") {
        if !STATUSES.contains(&status.as_str()) {
            return Err(invalid());
        }
        filter = filter.and("c.status = {}", status);
    }
    let rows = fetch_rows(&mut tx, filter, "").await?;
    let comments = bodies(&mut tx, rows).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(CommentListResponse {
        project_id,
        comments,
    }))
}

// Statuses ------------------------------------------------------------------------------

/// What happens to a comment's status.
enum Change {
    /// An agent names the version that fixes it: the version's ID as sent.
    Addressed(Result<String, ApiError>),
    Resolve,
    Reopen,
}

/// Moves a comment along its statuses. The comment's row is locked first, so of
/// two requests that race for it one wins and the other is refused or applied
/// to the new status.
async fn change_status(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    comment_id: &str,
    change: Change,
) -> Result<Json<CommentResponse>, ApiError> {
    let project_id = parse_uuid(project_id).ok_or(ApiError::NotFound)?;
    let comment_id = parse_uuid(comment_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(workspace_id).await?;
    let current = load_row(&mut tx, &project_id, &comment_id, "FOR UPDATE OF c").await?;
    let from = current.status.clone();
    let (to, addressed_in) = match change {
        Change::Addressed(version) => {
            let version = parse_uuid(&version?).ok_or_else(invalid)?;
            let later: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM versions nv, versions cv \
                 WHERE nv.id = $1::uuid AND cv.id = $2::uuid AND nv.doc_id = cv.doc_id \
                   AND (nv.created_at, nv.id) > (cv.created_at, cv.id))",
            )
            .bind(&version)
            .bind(&current.version_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::database)?;
            if !later {
                return Err(invalid());
            }
            if from == "resolved" {
                return Err(ApiError::conflict("invalid_transition"));
            }
            ("addressed", Some(version))
        }
        Change::Resolve if from == "addressed" => ("resolved", current.addressed_in.clone()),
        Change::Reopen if from != "open" => ("open", None),
        Change::Resolve | Change::Reopen => return Err(ApiError::conflict("invalid_transition")),
    };
    sqlx::query(
        "UPDATE comments SET status = $2, addressed_in_version_id = $3::uuid WHERE id = $1::uuid",
    )
    .bind(&comment_id)
    .bind(to)
    .bind(&addressed_in)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let comment = load_comment(&mut tx, &project_id, &comment_id).await?;
    let data = json!({
        "comment_id": comment.id,
        "doc_id": comment.doc_id,
        "version_id": comment.version_id,
        "node": comment.node,
        "changeset_id": comment.changeset_id,
        "from_status": from,
        "status": comment.status,
        "addressed_in": comment.addressed_in,
    });
    live::emit(
        &mut tx,
        workspace_id,
        &project_id,
        "comment.status_changed",
        &data,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok(Json(CommentResponse { comment }))
}

/// Marks a comment addressed, naming the version that fixes it. Only an agent
/// does.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}/addressed",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("comment_id" = String, Path, description = "The comment's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
    ),
    request_body = AddressedRequest,
    responses(
        (status = 200, description = "The comment, now `addressed` in that version", body = CommentResponse),
        (status = 400, description = "The version is missing, unknown, of another doc or not saved after the comment's version", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such comment in this project and workspace", body = ErrorBody),
        (status = 409, description = "The comment is `resolved` (`invalid_transition`)", body = ErrorBody),
    )
)]
pub async fn addressed(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, comment_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<CommentResponse>, ApiError> {
    let version = parse_body::<AddressedRequest>(&body)
        .and_then(|sent| non_blank(sent.version_id).ok_or_else(invalid));
    change_status(
        &state,
        &request.workspace_id,
        &project_id,
        &comment_id,
        Change::Addressed(version),
    )
    .await
}

/// Resolves an addressed comment. Only the human does.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}/resolve",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("comment_id" = String, Path, description = "The comment's UUID"),
    ),
    responses(
        (status = 200, description = "The comment, now `resolved`", body = CommentResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such comment in this project and workspace", body = ErrorBody),
        (status = 409, description = "The comment is not `addressed` (`invalid_transition`)", body = ErrorBody),
    )
)]
pub async fn resolve(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, comment_id)): Path<(String, String, String)>,
) -> Result<Json<CommentResponse>, ApiError> {
    change_status(
        &state,
        &request.workspace_id,
        &project_id,
        &comment_id,
        Change::Resolve,
    )
    .await
}

/// Opens an addressed or resolved comment again. Only the human does.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}/reopen",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("comment_id" = String, Path, description = "The comment's UUID"),
    ),
    responses(
        (status = 200, description = "The comment, now `open`", body = CommentResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such comment in this project and workspace", body = ErrorBody),
        (status = 409, description = "The comment is `open` already (`invalid_transition`)", body = ErrorBody),
    )
)]
pub async fn reopen(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, comment_id)): Path<(String, String, String)>,
) -> Result<Json<CommentResponse>, ApiError> {
    change_status(
        &state,
        &request.workspace_id,
        &project_id,
        &comment_id,
        Change::Reopen,
    )
    .await
}

// Saving a version ----------------------------------------------------------------------

/// Refuses a save of `draft` whose version, with the node IDs `kept`, drops a
/// node that has an open comment: 409 `commented_nodes_removed` with the
/// comments. The nodes that the version drops are those of the doc's previous
/// version in the change set that it no longer has, or, when the change set has
/// saved none of the doc yet, those of the version the draft started from; a
/// comment inside an instance goes with the instance. Resolved and addressed
/// comments never stop a save, and neither does `remove_commented`.
pub(crate) async fn refuse_commented_removals(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    draft: &DraftRow,
    kept: &HashSet<String>,
    remove_commented: bool,
) -> Result<(), ApiError> {
    if remove_commented {
        return Ok(());
    }
    let mut tx = state.workspace_tx(workspace_id).await?;
    let previous: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM versions WHERE doc_id = $1::uuid AND changeset_id = $2::uuid \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(&draft.doc_id)
    .bind(&draft.changeset_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let Some(previous) = previous.or_else(|| draft.base_version_id.clone()) else {
        return Ok(());
    };
    let dropped: HashSet<String> = nodes_of_version_id(&mut tx, &previous)
        .await?
        .into_iter()
        .map(|node| node.id)
        .filter(|id| !kept.contains(id))
        .collect();
    if dropped.is_empty() {
        return Ok(());
    }
    let filter = Filter::default()
        .and("d.project_id = {}::uuid", project_id)
        .and("v.doc_id = {}::uuid", &draft.doc_id)
        .and_that("c.status = 'open'");
    let blocking: Vec<CommentRow> = fetch_rows(&mut tx, filter, "")
        .await?
        .into_iter()
        .filter(|row| dropped.contains(node_of(&row.node)))
        .collect();
    if blocking.is_empty() {
        return Ok(());
    }
    let comments = bodies(&mut tx, blocking).await?;
    Err(ApiError::Listing(
        StatusCode::CONFLICT,
        "commented_nodes_removed",
        "comments",
        serde_json::to_value(comments).map_err(|error| {
            eprintln!("omni-api: the comments cannot be written: {error}");
            ApiError::Unavailable
        })?,
    ))
}
