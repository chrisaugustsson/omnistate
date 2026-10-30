//! Threads. A thread is a question that agents work out at the app level of a
//! project or in one of its features. It is sealed or not, and an agent
//! writes its conclusion when it has worked the question out
//! (`thread.concluded`); the thread keeps taking posts after that, so builders
//! can raise problems with the plan. Starting a thread emits `thread.created`.
//! A thread has one phase, which is sealed when the thread is, so the rule of
//! sealed phases below is the rule of sealed threads.
//!
//! A thread takes posts (US-034): messages with a one-line summary and a
//! Markdown body, recorded in the thread's phase and never edited or deleted.
//!
//! Sealed threads (US-035): an agent identity (run and role) may read a post
//! when the human wrote it, when its phase is open, or when the identity has
//! posted in it; the human reads every post. The rule is one SQL predicate,
//! `visible`, which every read path uses: the thread, the digest, a single
//! post, the links of a post and the live events of a socket. Nothing is
//! cached, so the next read after the agent posted shows what was hidden.

use crate::AppState;
use crate::docs::AgentIdentity;
use crate::live;
use crate::projects::{non_blank, parse_body, require_project};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{Actor, AgentRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use omni_core::design::NodeId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::str::FromStr;
use utoipa::ToSchema;

/// A timestamp column as ISO 8601 text in UTC.
macro_rules! timestamp {
    ($column:literal) => {
        concat!(
            "to_char(",
            $column,
            " AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')"
        )
    };
}

/// The longest summary of a post, in characters.
const MAX_SUMMARY_CHARS: usize = 200;
/// How many links, and how many attachments, one post may carry.
const MAX_REFERENCES: usize = 100;
/// The longest web address in a link, in characters.
const MAX_URL_CHARS: usize = 2048;

#[derive(Serialize, ToSchema)]
pub struct ThreadBody {
    pub id: String,
    pub project_id: String,
    /// `app` for a thread at the app level, `feature` for one in a feature.
    #[schema(required = false)]
    pub scope: String,
    /// The feature the thread belongs to; `null` at the app level.
    pub feature_id: Option<String>,
    pub question: String,
    /// Whether an agent that has not posted in the thread yet reads no other
    /// agent's posts.
    #[schema(required = false)]
    pub sealed: bool,
    /// What the agents concluded; `null` until an agent writes it.
    #[schema(required = false)]
    pub conclusion: Option<String>,
    /// The run of the agent that started the thread.
    pub run_tag: Option<String>,
    pub created_at: String,
    /// The posts the caller may read, oldest first. Only in the answer to a
    /// read of the thread; an agent does not get what a sealed phase hides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<PostBody>>,
}

#[derive(Serialize, ToSchema)]
pub struct ThreadResponse {
    pub thread: ThreadBody,
}

#[derive(Serialize, ToSchema)]
pub struct ThreadListResponse {
    /// The project's threads of every scope, oldest first, without their posts.
    pub threads: Vec<ThreadBody>,
}

#[derive(Deserialize, ToSchema)]
pub struct StartThreadRequest {
    /// The question to work out; required.
    pub question: Option<String>,
    /// Whether an agent that has not posted yet reads no other agent's posts.
    /// Defaults to `false`.
    pub sealed: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct ConcludeThreadRequest {
    /// What the agents concluded; required and not blank.
    pub conclusion: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreatePostRequest {
    /// One line of at most 200 characters; required.
    pub summary: Option<String>,
    /// The Markdown text; required.
    pub body: Option<String>,
    /// What the post points to, in order: `{"kind": "post", "post_id"}` (a post
    /// of the project), `{"kind": "url", "url"}` (an http or https address),
    /// `{"kind": "node", "doc_id", "address"}` (a doc of the project and a
    /// node address) and `{"kind": "attachment", "attachment_id"}`. Defaults
    /// to none.
    #[schema(value_type = Option<Vec<Object>>)]
    pub links: Option<Vec<Value>>,
    /// IDs of the workspace's attachments, in order. Defaults to none.
    pub attachments: Option<Vec<String>>,
    /// How sure the author is, from 0 to 1.
    pub confidence: Option<f64>,
    /// The earlier post of this thread by the same author (an agent's run and
    /// role) that this one replaces. That post stays as it was.
    pub replaces: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PostBody {
    pub id: String,
    pub thread_id: String,
    /// The feature of the thread; `null` for a thread at the app level.
    pub feature_id: Option<String>,
    /// `{"kind": "agent", "run", "role", "model", "harness"}` for an agent
    /// (each `null` when the request did not say), or `{"kind": "human",
    /// "user_id"}`.
    #[schema(value_type = Object)]
    pub author: Value,
    pub summary: String,
    /// The Markdown as it was sent.
    pub body: String,
    /// The body rendered to HTML with raw HTML switched off, links only to
    /// http, https and mailto, and images only from `attachment:<id>`.
    pub body_html: String,
    #[schema(value_type = Vec<Object>)]
    pub links: Vec<Value>,
    pub attachments: Vec<String>,
    pub confidence: Option<f64>,
    pub replaces: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct PostResponse {
    pub post: PostBody,
}

/// A post in a digest: what it is, without its text.
#[derive(Serialize, ToSchema)]
pub struct DigestEntry {
    pub id: String,
    /// As for a post.
    #[schema(value_type = Object)]
    pub author: Value,
    pub summary: String,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct DigestResponse {
    pub project_id: String,
    pub thread_id: String,
    /// The posts the caller may read, oldest first.
    pub posts: Vec<DigestEntry>,
}

/// A post read on its own, with the project it belongs to.
#[derive(Serialize, ToSchema)]
pub struct SinglePostResponse {
    pub project_id: String,
    pub post: PostBody,
}

#[derive(sqlx::FromRow)]
struct DigestRow {
    id: String,
    author: String,
    summary: String,
    created_at: String,
}

/// Who reads posts: the human, who reads every post, or an agent identity.
pub(crate) struct Viewer {
    human: bool,
    run: Option<String>,
    role: Option<String>,
}

impl Viewer {
    pub(crate) fn agent(identity: &AgentIdentity) -> Self {
        Self {
            human: false,
            run: identity.run().map(str::to_string),
            role: identity.role().map(str::to_string),
        }
    }

    /// The human, who reads everything.
    pub(crate) fn human() -> Self {
        Self {
            human: true,
            run: None,
            role: None,
        }
    }

    pub(crate) fn of(actor: &Actor, identity: &AgentIdentity) -> Self {
        match actor {
            Actor::Agent => Self::agent(identity),
            Actor::Human { .. } => Self::human(),
        }
    }

    fn of_author(author: &Author) -> Self {
        match author {
            Author::Agent(identity) => Self::agent(identity),
            Author::Human(_) => Self::human(),
        }
    }
}

/// The SQL condition that the viewer may read the post of the alias `post`. It
/// takes three parameters, from `$first` on: whether the viewer is the human
/// (bool), the run and the role (text, each may be null). Bind them with
/// `bind_viewer`.
pub(crate) fn visible(post: &str, first: usize) -> String {
    let human = first;
    format!(
        "(${human}::bool \
         OR {post}.author->>'kind' = 'human' \
         OR {})",
        phase_readable(post, first)
    )
}

/// The part of the rule of sealed phases that depends on the phase of the post
/// of the alias `row` (it has `workspace_id`, `thread_id` and `phase`): the
/// phase is not the current one, it is open, or the viewer has posted in it.
fn phase_readable(row: &str, first: usize) -> String {
    let (run, role) = (first + 1, first + 2);
    format!(
        "{row}.phase < (SELECT max(ph.number) FROM thread_phases ph \
             WHERE ph.workspace_id = {row}.workspace_id AND ph.thread_id = {row}.thread_id) \
         OR NOT COALESCE((SELECT ph.sealed FROM thread_phases ph \
             WHERE ph.workspace_id = {row}.workspace_id AND ph.thread_id = {row}.thread_id \
               AND ph.number = {row}.phase), true) \
         OR EXISTS (SELECT 1 FROM posts mine \
             WHERE mine.workspace_id = {row}.workspace_id AND mine.thread_id = {row}.thread_id \
               AND mine.phase = {row}.phase AND mine.author->>'kind' = 'agent' \
               AND mine.author->>'run' IS NOT DISTINCT FROM ${run}::text \
               AND mine.author->>'role' IS NOT DISTINCT FROM ${role}::text)"
    )
}

/// An agent that reads or writes posts must say who it is, its run and its
/// role: two callers without an identity would share one view of a sealed
/// phase. The human needs none.
pub(crate) fn require_identity(actor: &Actor, identity: &AgentIdentity) -> Result<(), ApiError> {
    match actor {
        Actor::Agent if identity.run().is_none() || identity.role().is_none() => {
            Err(ApiError::bad_request("identity_required"))
        }
        _ => Ok(()),
    }
}

/// Binds the three parameters of `visible`.
pub(crate) fn bind_viewer<'q, O>(
    query: sqlx::query::QueryAs<'q, Postgres, O, sqlx::postgres::PgArguments>,
    viewer: &'q Viewer,
) -> sqlx::query::QueryAs<'q, Postgres, O, sqlx::postgres::PgArguments> {
    query
        .bind(viewer.human)
        .bind(viewer.run.as_deref())
        .bind(viewer.role.as_deref())
}

/// Leaves out of the links of the posts the links to posts that the viewer may
/// not read, so a link does not give away that a hidden post exists.
pub(crate) async fn hide_unseen_links(
    tx: &mut Transaction<'static, Postgres>,
    viewer: &Viewer,
    posts: &mut [PostBody],
) -> Result<(), ApiError> {
    if viewer.human {
        return Ok(());
    }
    let targets: Vec<String> = posts
        .iter()
        .flat_map(|post| post.links.iter())
        .filter(|link| link["kind"] == "post")
        .filter_map(|link| link["post_id"].as_str().map(str::to_string))
        .collect();
    if targets.is_empty() {
        return Ok(());
    }
    let seen: Vec<String> = bind_viewer(
        sqlx::query_as::<_, (String,)>(&format!(
            "SELECT p.id::text FROM posts p WHERE p.id = ANY($1::text[]::uuid[]) AND {}",
            visible("p", 2)
        ))
        .bind(&targets),
        viewer,
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .into_iter()
    .map(|(id,)| id)
    .collect();
    for post in posts {
        post.links.retain(|link| {
            link["kind"] != "post"
                || link["post_id"]
                    .as_str()
                    .is_some_and(|id| seen.iter().any(|seen| seen == id))
        });
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
pub(crate) struct PostRow {
    id: String,
    thread_id: String,
    feature_id: Option<String>,
    author: String,
    summary: String,
    body: String,
    links: String,
    attachments: String,
    confidence: Option<f64>,
    replaces: Option<String>,
    created_at: String,
}

impl PostRow {
    /// The post as the API answers it, its body rendered.
    pub(crate) fn into_body(self) -> Result<PostBody, ApiError> {
        let unreadable = |what: &str, error: serde_json::Error| {
            eprintln!(
                "omni-api: the {what} of post {} are unreadable: {error}",
                self.id
            );
            ApiError::Unavailable
        };
        let author = serde_json::from_str(&self.author).map_err(|e| unreadable("author", e))?;
        let links = serde_json::from_str(&self.links).map_err(|e| unreadable("links", e))?;
        let attachments =
            serde_json::from_str(&self.attachments).map_err(|e| unreadable("attachments", e))?;
        Ok(PostBody {
            id: self.id,
            thread_id: self.thread_id,
            feature_id: self.feature_id,
            author,
            summary: self.summary,
            body_html: omni_core::markdown::render(&self.body),
            body: self.body,
            links,
            attachments,
            confidence: self.confidence,
            replaces: self.replaces,
            created_at: self.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ThreadRow {
    id: String,
    project_id: String,
    feature_id: Option<String>,
    question: String,
    conclusion: Option<String>,
    run_tag: Option<String>,
    created_at: String,
}

const THREAD_COLUMNS: &str = concat!(
    "id::text, project_id::text, feature_id::text, ",
    "question, conclusion, run_tag, ",
    timestamp!("created_at"),
    " AS created_at"
);

pub(crate) const POST_COLUMNS: &str = concat!(
    "id::text, thread_id::text, \
     (SELECT t.feature_id::text FROM threads t WHERE t.id = thread_id) AS feature_id, \
     author::text, summary, body, links::text, \
     attachments::text, confidence, replaces::text, ",
    timestamp!("created_at"),
    " AS created_at"
);

/// Whether the thread is sealed: its one phase is.
async fn is_sealed(
    tx: &mut Transaction<'static, Postgres>,
    thread_id: &str,
) -> Result<bool, ApiError> {
    let sealed: Option<bool> = sqlx::query_scalar(
        "SELECT sealed FROM thread_phases WHERE thread_id = $1::uuid ORDER BY number DESC LIMIT 1",
    )
    .bind(thread_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(sealed.unwrap_or(false))
}

/// The thread as the API answers it, 404 unless the project has it. With
/// `lock`, the thread's row is locked until the transaction ends, so of
/// several requests that start a phase or post one at a time, each sees the
/// phases the others made.
pub(crate) async fn load_thread(
    tx: &mut Transaction<'static, Postgres>,
    project_id: &str,
    thread_id: &str,
    lock: Option<&str>,
) -> Result<ThreadBody, ApiError> {
    let row: ThreadRow = sqlx::query_as(&format!(
        "SELECT {THREAD_COLUMNS} FROM threads \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL {}",
        lock.unwrap_or("")
    ))
    .bind(thread_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    let sealed = is_sealed(tx, &row.id).await?;
    Ok(ThreadBody {
        id: row.id,
        project_id: row.project_id,
        scope: if row.feature_id.is_some() {
            "feature"
        } else {
            "app"
        }
        .to_string(),
        feature_id: row.feature_id,
        question: row.question,
        sealed,
        conclusion: row.conclusion,
        run_tag: row.run_tag,
        created_at: row.created_at,
        posts: None,
    })
}

/// The threads of a project, oldest first, each as `show_thread` answers but
/// without its posts.
pub(crate) async fn threads_of_project(
    tx: &mut Transaction<'static, Postgres>,
    project_id: &str,
) -> Result<Vec<ThreadBody>, ApiError> {
    let ids: Vec<(String,)> = sqlx::query_as(
        "SELECT id::text FROM threads WHERE project_id = $1::uuid AND deleted_at IS NULL \
         ORDER BY created_at, id",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let mut threads = Vec::with_capacity(ids.len());
    for (id,) in ids {
        threads.push(load_thread(tx, project_id, &id, None).await?);
    }
    Ok(threads)
}

/// The number of a thread's one phase, which its posts are made in.
const PHASE: i32 = 1;

async fn insert_phase(
    tx: &mut Transaction<'static, Postgres>,
    workspace_id: &str,
    thread_id: &str,
    sealed: bool,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO thread_phases (workspace_id, thread_id, number, sealed) \
         VALUES ($1::uuid, $2::uuid, $3, $4)",
    )
    .bind(workspace_id)
    .bind(thread_id)
    .bind(PHASE)
    .bind(sealed)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(())
}

/// Starts a thread at the app level or in a feature of the project. Its one
/// phase is sealed when the thread is.
async fn start_live_thread(
    state: &AppState,
    request: &AgentRequest,
    identity: &AgentIdentity,
    project_id: &str,
    feature_id: Option<&str>,
    body: &Bytes,
) -> Result<(StatusCode, Json<ThreadResponse>), ApiError> {
    let new = parse_body::<StartThreadRequest>(body);
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    require_project(&mut tx, project_id).await?;
    if let Some(feature_id) = feature_id {
        let found: Option<(String,)> = sqlx::query_as(
            "SELECT id::text FROM features \
             WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL FOR SHARE",
        )
        .bind(feature_id)
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        found.ok_or(ApiError::NotFound)?;
    }
    let new = new?;
    let question = non_blank(new.question).ok_or_else(invalid)?;
    let sealed = new.sealed.unwrap_or(false);

    let thread_id: String = sqlx::query_scalar(
        "INSERT INTO threads (workspace_id, project_id, feature_id, question, criteria, run_tag) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, '[]'::jsonb, $5) RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(project_id)
    .bind(feature_id)
    .bind(&question)
    .bind(identity.run())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    insert_phase(&mut tx, workspace_id, &thread_id, sealed).await?;
    live::emit(
        &mut tx,
        workspace_id,
        project_id,
        "thread.created",
        &json!({
            "thread_id": thread_id,
            "feature_id": feature_id,
            "question": question,
            "sealed": sealed,
        }),
    )
    .await?;
    let thread = load_thread(&mut tx, project_id, &thread_id, None).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, project_id);
    Ok((StatusCode::CREATED, Json(ThreadResponse { thread })))
}

/// Starts a thread at the app level of a project.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/threads",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run; the thread's run tag"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = StartThreadRequest,
    responses(
        (status = 201, description = "The new thread", body = ThreadResponse),
        (status = 400, description = "The question is missing or blank, or `sealed` is not a boolean", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn create_app_thread(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<ThreadResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    start_live_thread(&state, &request, &identity, &project_id, None, &body).await
}

/// Starts a thread in a feature of a project.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/threads",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run; the thread's run tag"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = StartThreadRequest,
    responses(
        (status = 201, description = "The new thread", body = ThreadResponse),
        (status = 400, description = "The question is missing or blank, or `sealed` is not a boolean", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace", body = ErrorBody),
    )
)]
pub async fn create_feature_thread(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<ThreadResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    start_live_thread(
        &state,
        &request,
        &identity,
        &project_id,
        Some(&feature_id),
        &body,
    )
    .await
}

/// The threads of a project of every scope, oldest first, without their posts.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/threads",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
    ),
    responses(
        (status = 200, description = "The project's threads", body = ThreadListResponse),
        (status = 400, description = "An agent request without both `X-Omni-Run` and `X-Omni-Role`", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn list_threads(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id)): Path<(String, String)>,
) -> Result<Json<ThreadListResponse>, ApiError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let threads = threads_of_project(&mut tx, &project_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(ThreadListResponse { threads }))
}

/// Writes the conclusion of a thread, replacing an earlier one. The thread
/// keeps taking posts.
#[utoipa::path(
    put,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/conclusion",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("thread_id" = String, Path, description = "The thread's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
    ),
    request_body = ConcludeThreadRequest,
    responses(
        (status = 200, description = "The thread with its conclusion", body = ThreadResponse),
        (status = 400, description = "The conclusion is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such thread in this project and workspace", body = ErrorBody),
    )
)]
pub async fn conclude_thread(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, thread_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<ThreadResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let thread_id = parse_uuid(&thread_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<ConcludeThreadRequest>(&body);
    let workspace_id = request.workspace_id;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    let thread = load_thread(&mut tx, &project_id, &thread_id, Some("FOR UPDATE")).await?;
    let conclusion = non_blank(new?.conclusion).ok_or_else(invalid)?;
    sqlx::query(
        "UPDATE threads SET conclusion = $2, concluded_at = clock_timestamp() WHERE id = $1::uuid",
    )
    .bind(&thread_id)
    .bind(&conclusion)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        &workspace_id,
        &project_id,
        "thread.concluded",
        &json!({
            "thread_id": thread_id,
            "feature_id": thread.feature_id,
        }),
    )
    .await?;
    let thread = load_thread(&mut tx, &project_id, &thread_id, None).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&workspace_id, &project_id);
    Ok(Json(ThreadResponse { thread }))
}

/// One thread with the posts the caller may read.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("thread_id" = String, Path, description = "The thread's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
    ),
    responses(
        (status = 200, description = "The thread", body = ThreadResponse),
        (status = 400, description = "An agent request without both `X-Omni-Run` and `X-Omni-Role`", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such thread in this project and workspace", body = ErrorBody),
    )
)]
pub async fn show_thread(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, thread_id)): Path<(String, String, String)>,
) -> Result<Json<ThreadResponse>, ApiError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let thread_id = parse_uuid(&thread_id).ok_or(ApiError::NotFound)?;
    let viewer = Viewer::of(&request.actor, &identity);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let mut thread = load_thread(&mut tx, &project_id, &thread_id, None).await?;
    let rows: Vec<PostRow> = bind_viewer(
        sqlx::query_as(&format!(
            "SELECT {POST_COLUMNS} FROM posts p WHERE p.thread_id = $1::uuid AND {} \
             ORDER BY p.created_at, p.id",
            visible("p", 2)
        ))
        .bind(&thread_id),
        &viewer,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let mut posts = rows
        .into_iter()
        .map(PostRow::into_body)
        .collect::<Result<Vec<_>, _>>()?;
    hide_unseen_links(&mut tx, &viewer, &mut posts).await?;
    thread.posts = Some(posts);
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(ThreadResponse { thread }))
}

/// The posts of a thread that the caller may read, as summaries without their
/// text, oldest first.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/digest",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("thread_id" = String, Path, description = "The thread's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
    ),
    responses(
        (status = 200, description = "The posts the caller may read", body = DigestResponse),
        (status = 400, description = "An agent request without both `X-Omni-Run` and `X-Omni-Role`", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such thread in this project and workspace", body = ErrorBody),
    )
)]
pub async fn digest(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, thread_id)): Path<(String, String, String)>,
) -> Result<Json<DigestResponse>, ApiError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let thread_id = parse_uuid(&thread_id).ok_or(ApiError::NotFound)?;
    let viewer = Viewer::of(&request.actor, &identity);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    load_thread(&mut tx, &project_id, &thread_id, None).await?;
    let rows: Vec<DigestRow> = bind_viewer(
        sqlx::query_as(&format!(
            "SELECT p.id::text AS id, p.author::text AS author, p.summary, {} AS created_at \
             FROM posts p WHERE p.thread_id = $1::uuid AND {} ORDER BY p.created_at, p.id",
            timestamp!("p.created_at"),
            visible("p", 2)
        ))
        .bind(&thread_id),
        &viewer,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let posts = rows
        .into_iter()
        .map(|row| {
            let author = serde_json::from_str(&row.author).map_err(|error| {
                eprintln!(
                    "omni-api: the author of post {} is unreadable: {error}",
                    row.id
                );
                ApiError::Unavailable
            })?;
            Ok(DigestEntry {
                id: row.id,
                author,
                summary: row.summary,
                created_at: row.created_at,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(DigestResponse {
        project_id,
        thread_id,
        posts,
    }))
}

/// One post of a project. A post that the caller may not read is answered
/// exactly like one that does not exist.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/posts/{post_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("post_id" = String, Path, description = "The post's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
    ),
    responses(
        (status = 200, description = "The post", body = SinglePostResponse),
        (status = 400, description = "An agent request without both `X-Omni-Run` and `X-Omni-Role`", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such post in this project and workspace, or the caller may not read it yet", body = ErrorBody),
    )
)]
pub async fn show_post(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, post_id)): Path<(String, String, String)>,
) -> Result<Json<SinglePostResponse>, ApiError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let post_id = parse_uuid(&post_id).ok_or(ApiError::NotFound)?;
    let viewer = Viewer::of(&request.actor, &identity);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row: PostRow = bind_viewer(
        sqlx::query_as(&format!(
            "SELECT {POST_COLUMNS} FROM posts p WHERE p.id = $1::uuid \
               AND EXISTS (SELECT 1 FROM threads t WHERE t.workspace_id = p.workspace_id \
                           AND t.id = p.thread_id AND t.project_id = $2::uuid \
                           AND t.deleted_at IS NULL) AND {}",
            visible("p", 3)
        ))
        .bind(&post_id)
        .bind(&project_id),
        &viewer,
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    let mut posts = [row.into_body()?];
    hide_unseen_links(&mut tx, &viewer, &mut posts).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let [post] = posts;
    Ok(Json(SinglePostResponse { project_id, post }))
}

/// Who writes a post.
enum Author {
    Agent(AgentIdentity),
    Human(String),
}

impl Author {
    fn json(&self) -> Value {
        match self {
            Self::Agent(identity) => {
                let mut author = identity.to_json();
                author["kind"] = json!("agent");
                author
            }
            Self::Human(user_id) => json!({ "kind": "human", "user_id": user_id }),
        }
    }
}

/// A post whose fields have the right form; what it points to is still to be
/// looked up.
struct Checked {
    summary: String,
    body: String,
    links: Vec<Value>,
    attachments: Vec<String>,
    confidence: Option<f64>,
    replaces: Option<String>,
}

fn invalid() -> ApiError {
    ApiError::bad_request("invalid_request")
}

/// A web address that a link may point to: http or https.
fn web_address(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["http://", "https://"]
        .iter()
        .any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len())
        && url.chars().count() <= MAX_URL_CHARS
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// A node address: a node ID, or `<instanceId>/<componentNodeId>`. Only its
/// form is checked, since the node may be in any version.
fn node_address(address: &str) -> bool {
    let mut parts = address.split('/');
    let (first, second) = (parts.next(), parts.next());
    first.is_some_and(|id| NodeId::from_str(id).is_ok())
        && second.is_none_or(|id| NodeId::from_str(id).is_ok())
        && parts.next().is_none()
}

/// The link in the form it is kept in, `None` when it is not one of the four
/// kinds with exactly their fields.
fn checked_link(link: &Value) -> Option<Value> {
    let object = link.as_object()?;
    let text = |key: &str| object.get(key)?.as_str();
    match (text("kind")?, object.len()) {
        ("post", 2) => Some(json!({ "kind": "post", "post_id": parse_uuid(text("post_id")?)? })),
        ("url", 2) => {
            let url = text("url")?;
            web_address(url).then(|| json!({ "kind": "url", "url": url }))
        }
        ("node", 3) => {
            let doc_id = parse_uuid(text("doc_id")?)?;
            let address = text("address")?;
            node_address(address)
                .then(|| json!({ "kind": "node", "doc_id": doc_id, "address": address }))
        }
        ("attachment", 2) => Some(
            json!({ "kind": "attachment", "attachment_id": parse_uuid(text("attachment_id")?)? }),
        ),
        _ => None,
    }
}

fn check(new: CreatePostRequest) -> Result<Checked, ApiError> {
    let summary = non_blank(new.summary)
        .filter(|summary| {
            summary.chars().count() <= MAX_SUMMARY_CHARS
                && !summary.contains(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}'])
        })
        .ok_or_else(invalid)?;
    let body = new.body.ok_or_else(invalid)?;
    let links = new.links.unwrap_or_default();
    let attachments = new.attachments.unwrap_or_default();
    if links.len() > MAX_REFERENCES || attachments.len() > MAX_REFERENCES {
        return Err(invalid());
    }
    let links = links
        .iter()
        .map(|link| checked_link(link).ok_or_else(invalid))
        .collect::<Result<Vec<_>, _>>()?;
    let attachments = attachments
        .iter()
        .map(|id| parse_uuid(id).ok_or_else(invalid))
        .collect::<Result<Vec<_>, _>>()?;
    let confidence = match new.confidence {
        Some(confidence) if !(0.0..=1.0).contains(&confidence) => return Err(invalid()),
        confidence => confidence,
    };
    let replaces = new
        .replaces
        .map(|id| parse_uuid(&id).ok_or_else(invalid))
        .transpose()?;
    Ok(Checked {
        summary,
        body,
        links,
        attachments,
        confidence,
        replaces,
    })
}

/// Whether the one-column query `sql` finds a row for the ID (and the project,
/// when it is given).
async fn found(
    tx: &mut Transaction<'static, Postgres>,
    sql: &str,
    id: &str,
    project_id: Option<&str>,
) -> Result<bool, ApiError> {
    let query = sqlx::query_scalar(sql).bind(id);
    let query = match project_id {
        Some(project_id) => query.bind(project_id),
        None => query,
    };
    query.fetch_one(&mut **tx).await.map_err(ApiError::database)
}

const DOC_IN_PROJECT: &str =
    "SELECT EXISTS (SELECT 1 FROM docs WHERE id = $1::uuid AND project_id = $2::uuid)";
const ATTACHMENT_IN_WORKSPACE: &str =
    "SELECT EXISTS (SELECT 1 FROM attachments WHERE id = $1::uuid)";

/// Checks that what the post points to exists: the posts and docs in its
/// project, the attachments in its workspace, and the post it replaces in its
/// thread by the same author.
async fn check_references(
    tx: &mut Transaction<'static, Postgres>,
    project_id: &str,
    thread_id: &str,
    author: &Author,
    post: &Checked,
) -> Result<(), ApiError> {
    for link in &post.links {
        if link["kind"] == "post" {
            // A post that the author may not read is refused like no post.
            let viewer = Viewer::of_author(author);
            let id = link["post_id"].as_str().ok_or_else(invalid)?;
            let readable: bool = bind_viewer(
                sqlx::query_as::<_, (bool,)>(&format!(
                    "SELECT EXISTS (SELECT 1 FROM posts p \
                     JOIN threads t ON t.workspace_id = p.workspace_id AND t.id = p.thread_id \
                     WHERE p.id = $1::uuid AND t.project_id = $2::uuid AND {})",
                    visible("p", 3)
                ))
                .bind(id)
                .bind(project_id),
                &viewer,
            )
            .fetch_one(&mut **tx)
            .await
            .map_err(ApiError::database)?
            .0;
            if !readable {
                return Err(invalid());
            }
            continue;
        }
        let target = match link["kind"].as_str() {
            Some("node") => Some((DOC_IN_PROJECT, &link["doc_id"], Some(project_id))),
            Some("attachment") => Some((ATTACHMENT_IN_WORKSPACE, &link["attachment_id"], None)),
            // A web address names nothing of ours.
            _ => None,
        };
        if let Some((sql, id, project)) = target {
            let id = id.as_str().ok_or_else(invalid)?;
            if !found(tx, sql, id, project).await? {
                return Err(invalid());
            }
        }
    }
    for id in &post.attachments {
        if !found(tx, ATTACHMENT_IN_WORKSPACE, id, None).await? {
            return Err(invalid());
        }
    }
    if let Some(replaces) = &post.replaces {
        let same_author: bool = match author {
            Author::Agent(identity) => sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM posts WHERE id = $1::uuid AND thread_id = $2::uuid \
                     AND author->>'kind' = 'agent' \
                     AND author->>'run' IS NOT DISTINCT FROM $3::text \
                     AND author->>'role' IS NOT DISTINCT FROM $4::text)",
            )
            .bind(replaces)
            .bind(thread_id)
            .bind(identity.run())
            .bind(identity.role())
            .fetch_one(&mut **tx)
            .await,
            Author::Human(user_id) => sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM posts WHERE id = $1::uuid AND thread_id = $2::uuid \
                     AND author->>'kind' = 'human' AND author->>'user_id' = $3::text)",
            )
            .bind(replaces)
            .bind(thread_id)
            .bind(user_id)
            .fetch_one(&mut **tx)
            .await,
        }
        .map_err(ApiError::database)?;
        if !same_author {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Records a post in the thread's phase and emits `post.created`.
async fn record_post(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    thread_id: &str,
    author: Author,
    new: Result<CreatePostRequest, ApiError>,
) -> Result<(StatusCode, Json<PostResponse>), ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    load_thread(&mut tx, project_id, thread_id, Some("FOR SHARE")).await?;
    let post = check(new?)?;
    check_references(&mut tx, project_id, thread_id, &author, &post).await?;
    let row: PostRow = sqlx::query_as(&format!(
        "INSERT INTO posts (workspace_id, thread_id, phase, summary, body, author, \
                            links, attachments, confidence, replaces) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6::text::jsonb, $7::text::jsonb, \
                 $8::text::jsonb, $9, $10::uuid) RETURNING {POST_COLUMNS}"
    ))
    .bind(workspace_id)
    .bind(thread_id)
    .bind(PHASE)
    .bind(&post.summary)
    .bind(&post.body)
    .bind(author.json().to_string())
    .bind(json!(post.links).to_string())
    .bind(json!(post.attachments).to_string())
    .bind(post.confidence)
    .bind(&post.replaces)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        workspace_id,
        project_id,
        "post.created",
        &json!({ "post_id": row.id, "thread_id": thread_id }),
    )
    .await?;
    let post = row.into_body()?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, project_id);
    Ok((StatusCode::CREATED, Json(PostResponse { post })))
}

/// Records a post in the thread's phase, as the agent or the human that calls.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/posts",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("thread_id" = String, Path, description = "The thread's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = CreatePostRequest,
    responses(
        (status = 201, description = "The new post", body = PostResponse),
        (status = 400, description = "The summary, body, links, attachments, confidence or replaced post is missing or not valid, or the agent sent no `X-Omni-Run` and `X-Omni-Role`", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such thread in this project and workspace", body = ErrorBody),
    )
)]
pub async fn create_post(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, thread_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<PostResponse>), ApiError> {
    require_identity(&request.actor, &identity)?;
    let author = match request.actor {
        Actor::Agent => Author::Agent(identity),
        Actor::Human { user_id, .. } => Author::Human(user_id),
    };
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let thread_id = parse_uuid(&thread_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreatePostRequest>(&body);
    record_post(
        &state,
        &request.workspace_id,
        &project_id,
        &thread_id,
        author,
        new,
    )
    .await
}
