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
//! A comment stores a `snapshot`: what its node held when the comment was made
//! (an HTML doc's block as HTML, a design's node in tag form, and for
//! `<instanceId>/<componentNodeId>` the component's node). It is `orphaned`
//! when the doc's head no longer has the node address, which is computed when
//! the comment is read; a replace or a set that keeps the node's ID leaves it
//! in the head. Removing a commented node never stops a write (V18).
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
use omni_core::ops::EditDoc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
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
    /// What the node held when the comment was made: the HTML of a block of
    /// an HTML doc, the tags of a node of a design, or of the component's node
    /// for an address inside an instance. Later edits do not change it; it is
    /// empty for a comment made before snapshots, and cut off after
    /// `SNAPSHOT_LIMIT` bytes.
    #[schema(required = false)]
    pub snapshot: String,
    /// `true` when the doc's head no longer has the node address: an
    /// operation deleted the node, or a node above it, or a replace left it
    /// out. A replace or a set that keeps the node's ID does not orphan it.
    /// The comment stays listed with its snapshot until the human deletes it.
    #[schema(required = false)]
    pub orphaned: bool,
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
    snapshot: String,
    doc_kind: String,
    doc_head: Option<String>,
    created_at: String,
}

/// The longest snapshot that is stored, in bytes; a longer one is cut off.
const SNAPSHOT_LIMIT: usize = 16_000;

/// The snapshot as stored: `text`, cut off at a character boundary when it
/// is longer than `SNAPSHOT_LIMIT`.
fn clipped(text: String) -> String {
    if text.len() <= SNAPSHOT_LIMIT {
        return text;
    }
    let mut end = SNAPSHOT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
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
                COALESCE(c.snapshot, '') AS snapshot, d.kind AS doc_kind, d.head AS doc_head, \
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

/// A comment read from the database, with where its doc's head is: `finish`
/// computes `orphaned` from it once the transaction is closed.
pub(crate) struct Listed {
    body: CommentBody,
    doc_kind: String,
    doc_head: Option<String>,
}

/// The comments with their replies, not yet checked against the heads.
async fn bodies(tx: &mut Tx, rows: Vec<CommentRow>) -> Result<Vec<Listed>, ApiError> {
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
            Ok(Listed {
                body: CommentBody {
                    replies,
                    id: row.id,
                    project_id: row.project_id,
                    doc_id: row.doc_id,
                    version_id: row.version_id,
                    node: row.node,
                    feature_id: row.feature_id,
                    author,
                    text: row.text,
                    snapshot: row.snapshot,
                    orphaned: false,
                    waiting_on_you,
                    created_at: row.created_at,
                },
                doc_kind: row.doc_kind,
                doc_head: row.doc_head,
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

async fn load_comment(tx: &mut Tx, project_id: &str, comment_id: &str) -> Result<Listed, ApiError> {
    let row = load_row(tx, project_id, comment_id, "").await?;
    bodies(tx, vec![row]).await?.pop().ok_or(ApiError::NotFound)
}

/// The comments with `orphaned` set: `true` when the head of the comment's doc
/// no longer has its node address. The heads are checked through the node
/// index, one query for the plain addresses of a doc, and a design's manifest
/// is the only thing read from RustFS besides the board of an instance. Call
/// it after the transaction that read the comments is closed.
pub(crate) async fn finish(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    listed: Vec<Listed>,
) -> Result<Vec<CommentBody>, ApiError> {
    let mut by_doc: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, comment) in listed.iter().enumerate() {
        by_doc
            .entry(comment.body.doc_id.as_str())
            .or_default()
            .push(index);
    }
    let mut orphaned: HashSet<usize> = HashSet::new();
    for indexes in by_doc.values() {
        let first = &listed[indexes[0]];
        let addresses: Vec<(usize, &str)> = indexes
            .iter()
            .map(|index| (*index, listed[*index].body.node.as_str()))
            .collect();
        let kept = addresses_in_head(
            state,
            workspace_id,
            project_id,
            &first.doc_kind,
            first.doc_head.as_deref(),
            &addresses,
        )
        .await?;
        orphaned.extend(
            addresses
                .iter()
                .map(|(index, _)| *index)
                .filter(|index| !kept.contains(index)),
        );
    }
    Ok(listed
        .into_iter()
        .enumerate()
        .map(|(index, mut comment)| {
            comment.body.orphaned = orphaned.contains(&index);
            comment.body
        })
        .collect())
}

/// One comment with `orphaned` set; see `finish`.
async fn finish_one(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    listed: Listed,
) -> Result<CommentBody, ApiError> {
    finish(state, workspace_id, project_id, vec![listed])
        .await?
        .pop()
        .ok_or(ApiError::NotFound)
}

/// Which of the node addresses (each with the index that names it) the head of
/// a doc has: the index of every address that is still there.
async fn addresses_in_head(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    doc_kind: &str,
    head: Option<&str>,
    addresses: &[(usize, &str)],
) -> Result<HashSet<usize>, ApiError> {
    let Some(head) = head else {
        return Ok(HashSet::new());
    };
    let manifest = if doc_kind == "design" {
        versions::head_manifest(state, workspace_id, Some(head)).await?
    } else {
        None
    };
    let chunks: Vec<&str> = match &manifest {
        Some(manifest) => manifest.chunks(),
        None => vec![head],
    };
    let parsed: Vec<(usize, Address)> = addresses
        .iter()
        .filter_map(|(index, node)| Address::parse(node).map(|address| (*index, address)))
        .collect();
    let wanted: Vec<&str> = parsed
        .iter()
        .map(|(_, address)| match address {
            Address::Node(id) => *id,
            Address::Inside { instance, .. } => *instance,
        })
        .collect();
    let mut tx = state.workspace_tx(workspace_id).await?;
    let found: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT chunk, node, type FROM node_index \
         WHERE chunk = ANY($1::text[]) AND node = ANY($2::text[])",
    )
    .bind(&chunks)
    .bind(&wanted)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;

    let mut kept = HashSet::new();
    for (index, address) in parsed {
        match address {
            Address::Node(id) => {
                if found.iter().any(|(_, node, _)| node == id) {
                    kept.insert(index);
                }
            }
            Address::Inside { instance, inner } => {
                let placed = found
                    .iter()
                    .find(|(_, node, kind)| node == instance && kind == "instance");
                if let (Some((chunk, _, _)), Some(_)) = (placed, &manifest) {
                    let node = design::head_instance_node(
                        state,
                        workspace_id,
                        project_id,
                        chunk,
                        instance,
                        inner,
                    )
                    .await?;
                    if node.is_some() {
                        kept.insert(index);
                    }
                }
            }
        }
    }
    Ok(kept)
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
    /// What the node holds now, stored as the comment's snapshot.
    snapshot: String,
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
        "INSERT INTO comments (workspace_id, doc_id, version_id, node, author, body, snapshot) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5::text::jsonb, $6, $7) RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(new.doc_id)
    .bind(new.version_id)
    .bind(new.node)
    .bind(new.author.to_string())
    .bind(new.text)
    .bind(clipped(new.snapshot))
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let comment = load_comment(&mut tx, project_id, &comment_id).await?;
    let data = json!({
        "comment_id": comment.body.id,
        "doc_id": comment.body.doc_id,
        "node": comment.body.node,
    });
    live::emit(&mut tx, workspace_id, project_id, "comment.created", &data).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, project_id);
    let comment = finish_one(state, workspace_id, project_id, comment).await?;
    Ok((StatusCode::CREATED, Json(CommentResponse { comment })))
}

/// The HTML of the block `id` in the HTML chunk `chunk`, `None` when the chunk
/// has no such block.
async fn block_html(
    state: &AppState,
    workspace_id: &str,
    chunk: &str,
    id: &str,
) -> Result<Option<String>, ApiError> {
    let text = versions::read_chunk(state, workspace_id, chunk).await?;
    let doc = EditDoc::from_html(&text).map_err(|e| {
        eprintln!("omni-api: the chunk {chunk} cannot be loaded: {e}");
        ApiError::Unavailable
    })?;
    Ok(id.parse().ok().and_then(|id| doc.node_html(id)))
}

/// What the node address holds in the head of `doc`: `None` when the head does
/// not have it, else the HTML of an HTML doc's block, the tags of a design's
/// node, or the tags of the component's node for an address inside an
/// instance. Only the chunk of the node is read.
async fn head_snapshot(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    doc: &DocRow,
    address: Address<'_>,
) -> Result<Option<String>, ApiError> {
    let Some(head) = doc.head.as_deref() else {
        return Ok(None);
    };
    let manifest: Option<Manifest> = if doc.kind == "design" {
        versions::head_manifest(state, workspace_id, Some(head)).await?
    } else {
        None
    };
    let chunks: Vec<&str> = match &manifest {
        Some(manifest) => manifest.chunks(),
        None => vec![head],
    };
    let (id, inner) = match address {
        Address::Node(id) => (id, None),
        Address::Inside { instance, inner } => (instance, Some(inner)),
    };
    let mut tx = state.workspace_tx(workspace_id).await?;
    let found: Option<(String, String)> = sqlx::query_as(
        "SELECT chunk, type FROM node_index \
         WHERE chunk = ANY($1::text[]) AND node = $2 LIMIT 1",
    )
    .bind(&chunks)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let Some((chunk, kind)) = found else {
        return Ok(None);
    };
    match inner {
        Some(inner) if manifest.is_some() && kind == "instance" => {
            design::head_instance_node(state, workspace_id, project_id, &chunk, id, inner).await
        }
        Some(_) => Ok(None),
        None if doc.kind != "design" => block_html(state, workspace_id, &chunk, id).await,
        // A head in the older form holds all its pages in one chunk.
        None if manifest.is_none() => Ok(Some(String::new())),
        None => design::head_node_tags(state, workspace_id, &chunk, id).await,
    }
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
    let Some(snapshot) = head_snapshot(&state, workspace_id, &project_id, &doc, address).await?
    else {
        return Err(not_in_head());
    };
    let new = NewComment {
        doc_id: &doc_id,
        version_id: None,
        node: &node,
        text: &text,
        snapshot,
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
    let snapshot = match Address::parse(&node).ok_or_else(invalid)? {
        Address::Node(id) if has(id, None) => match version.chunk.as_deref() {
            Some(chunk) if version.doc_kind == "design" => {
                design::version_node_tags(&state, workspace_id, &version_id, chunk, id).await?
            }
            Some(chunk) => block_html(&state, workspace_id, chunk, id).await?,
            None => None,
        },
        Address::Inside { instance, inner }
            if version.doc_kind == "design" && has(instance, Some("instance")) =>
        {
            design::instance_node(
                &state,
                workspace_id,
                &version_id,
                version.chunk.as_deref(),
                version.design_system_version_id.as_deref(),
                instance,
                inner,
            )
            .await?
        }
        _ => None,
    };
    let new = NewComment {
        doc_id: &version.doc_id,
        version_id: Some(&version_id),
        node: &node,
        text: &text,
        snapshot: snapshot.ok_or_else(invalid)?,
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
        "comment_id": comment.body.id,
        "reply_id": reply_id,
        "doc_id": comment.body.doc_id,
    });
    live::emit(&mut tx, workspace_id, &project_id, "comment.replied", &data).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    let comment = finish_one(&state, workspace_id, &project_id, comment).await?;
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
    let comment = finish_one(&state, &request.workspace_id, &project_id, comment).await?;
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
    let listed = bodies(&mut tx, rows).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let comments = finish(&state, &request.workspace_id, &project_id, listed).await?;
    Ok(Json(CommentListResponse {
        project_id,
        comments,
    }))
}

/// The comments of a project that wait on the human, oldest first: those whose
/// latest reply is an agent's. Pass them to `finish` once the transaction is
/// closed.
pub(crate) async fn waiting_of_project(
    tx: &mut Tx,
    project_id: &str,
) -> Result<Vec<Listed>, ApiError> {
    let filter = Filter::default().and("d.project_id = {}::uuid", project_id);
    let rows = fetch_rows(tx, filter, "").await?;
    let mut comments = bodies(tx, rows).await?;
    comments.retain(|comment| comment.body.waiting_on_you);
    Ok(comments)
}
