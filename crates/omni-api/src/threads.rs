//! Threads and phases (US-033). A thread is a question that agents work out in
//! a change set, with the criteria for a good answer. Its phases, numbered from
//! 1, are open or sealed and the last one is the current phase; an agent starts
//! the next one. A thread belongs to its project and outlives its change set:
//! it keeps taking posts and phases after the change set merges, so builders
//! can raise problems with the plan. Starting a thread emits `thread.created`
//! and starting a later phase emits `phase.started`; the thread's first phase
//! is announced by `thread.created` itself.
//!
//! The post route here is the smallest one that lets a thread take posts: it
//! records a post in the thread's current phase. The rules of a post (types,
//! sealed phases, what an agent may see) come with US-034.

use crate::AppState;
use crate::changesets::{non_blank, parse_body};
use crate::docs::AgentIdentity;
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{Actor, AgentRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
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

/// One phase of a thread.
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct PhaseBody {
    /// From 1; the thread's last phase is its current one.
    pub number: i32,
    /// A sealed phase hides the posts of other agents from an agent that has
    /// not posted in it yet (US-034); an open phase does not.
    pub sealed: bool,
    pub started_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct ThreadBody {
    pub id: String,
    pub project_id: String,
    pub changeset_id: String,
    pub question: String,
    /// The decision criteria, in the order the agent gave them.
    pub criteria: Vec<String>,
    /// The run of the agent that started the thread.
    pub run_tag: Option<String>,
    pub created_at: String,
    /// The phases, oldest first.
    pub phases: Vec<PhaseBody>,
}

#[derive(Serialize, ToSchema)]
pub struct ThreadResponse {
    pub thread: ThreadBody,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateThreadRequest {
    /// The question to work out; required.
    pub question: Option<String>,
    /// The decision criteria, one text each; required, and may be empty.
    pub criteria: Option<Vec<String>>,
    /// Whether the first phase is sealed. Defaults to `false`.
    pub sealed: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct StartPhaseRequest {
    /// Whether the new phase is sealed; required.
    pub sealed: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreatePostRequest {
    /// The kind of post; required.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// One line of at most 200 characters; required.
    pub summary: Option<String>,
    /// The Markdown text; required.
    pub body: Option<String>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct PostBody {
    pub id: String,
    pub thread_id: String,
    /// The number of the thread's phase that the post was made in.
    pub phase: i32,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    pub kind: String,
    pub summary: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct PostResponse {
    pub post: PostBody,
}

#[derive(sqlx::FromRow)]
struct ThreadRow {
    id: String,
    project_id: String,
    changeset_id: String,
    question: String,
    criteria: String,
    run_tag: Option<String>,
    created_at: String,
}

const THREAD_COLUMNS: &str = concat!(
    "id::text, project_id::text, changeset_id::text, question, criteria::text, run_tag, ",
    timestamp!("created_at"),
    " AS created_at"
);

const PHASE_COLUMNS: &str = concat!(
    "number, sealed, ",
    timestamp!("started_at"),
    " AS started_at"
);

const POST_COLUMNS: &str = concat!(
    "id::text, thread_id::text, phase, type, summary, body, ",
    timestamp!("created_at"),
    " AS created_at"
);

/// The phases of a thread, oldest first.
async fn phases_of(
    tx: &mut Transaction<'static, Postgres>,
    thread_id: &str,
) -> Result<Vec<PhaseBody>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {PHASE_COLUMNS} FROM thread_phases WHERE thread_id = $1::uuid ORDER BY number"
    ))
    .bind(thread_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// The thread as the API answers it, 404 unless the project has it. With
/// `lock`, the thread's row is locked until the transaction ends, so of
/// several requests that start a phase or post one at a time, each sees the
/// phases the others made.
async fn load_thread(
    tx: &mut Transaction<'static, Postgres>,
    project_id: &str,
    thread_id: &str,
    lock: Option<&str>,
) -> Result<ThreadBody, ApiError> {
    let row: ThreadRow = sqlx::query_as(&format!(
        "SELECT {THREAD_COLUMNS} FROM threads WHERE id = $1::uuid AND project_id = $2::uuid {}",
        lock.unwrap_or("")
    ))
    .bind(thread_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    let phases = phases_of(tx, &row.id).await?;
    let criteria = serde_json::from_str(&row.criteria).map_err(|e| {
        eprintln!(
            "omni-api: the criteria of thread {} are no list: {e}",
            row.id
        );
        ApiError::Unavailable
    })?;
    Ok(ThreadBody {
        id: row.id,
        project_id: row.project_id,
        changeset_id: row.changeset_id,
        question: row.question,
        criteria,
        run_tag: row.run_tag,
        created_at: row.created_at,
        phases,
    })
}

async fn insert_phase(
    tx: &mut Transaction<'static, Postgres>,
    workspace_id: &str,
    thread_id: &str,
    number: i32,
    sealed: bool,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO thread_phases (workspace_id, thread_id, number, sealed) \
         VALUES ($1::uuid, $2::uuid, $3, $4)",
    )
    .bind(workspace_id)
    .bind(thread_id)
    .bind(number)
    .bind(sealed)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(())
}

/// Starts a thread in a change set. Its first phase is open unless `sealed`.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/threads",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run; the thread's run tag"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = CreateThreadRequest,
    responses(
        (status = 201, description = "The new thread, with its first phase", body = ThreadResponse),
        (status = 400, description = "The question is missing or blank, or the criteria are not a list of texts", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
    )
)]
pub async fn create_thread(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<ThreadResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateThreadRequest>(&body);
    let workspace_id = request.workspace_id;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM changesets WHERE id = $1::uuid AND project_id = $2::uuid",
    )
    .bind(&changeset_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    found.ok_or(ApiError::NotFound)?;
    let new = new?;
    let question =
        non_blank(new.question).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let criteria = new
        .criteria
        .filter(|criteria| {
            criteria
                .iter()
                .all(|criterion| !criterion.trim().is_empty())
        })
        .ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let sealed = new.sealed.unwrap_or(false);

    let thread_id: String = sqlx::query_scalar(
        "INSERT INTO threads (workspace_id, project_id, changeset_id, question, criteria, run_tag) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5::text::jsonb, $6) RETURNING id::text",
    )
    .bind(&workspace_id)
    .bind(&project_id)
    .bind(&changeset_id)
    .bind(&question)
    .bind(json!(criteria).to_string())
    .bind(identity.run())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    insert_phase(&mut tx, &workspace_id, &thread_id, 1, sealed).await?;
    live::emit(
        &mut tx,
        &workspace_id,
        &project_id,
        "thread.created",
        &json!({
            "thread_id": thread_id,
            "changeset_id": changeset_id,
            "question": question,
            "phase": 1,
            "sealed": sealed,
        }),
    )
    .await?;
    let thread = load_thread(&mut tx, &project_id, &thread_id, None).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(ThreadResponse { thread })))
}

/// One thread with its phases.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("thread_id" = String, Path, description = "The thread's UUID"),
    ),
    responses(
        (status = 200, description = "The thread", body = ThreadResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such thread in this project and workspace", body = ErrorBody),
    )
)]
pub async fn show_thread(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, thread_id)): Path<(String, String, String)>,
) -> Result<Json<ThreadResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let thread_id = parse_uuid(&thread_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let thread = load_thread(&mut tx, &project_id, &thread_id, None).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(ThreadResponse { thread }))
}

/// Starts the thread's next phase, which becomes the current one.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/phases",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("thread_id" = String, Path, description = "The thread's UUID"),
    ),
    request_body = StartPhaseRequest,
    responses(
        (status = 201, description = "The thread with the new phase appended", body = ThreadResponse),
        (status = 400, description = "`sealed` is missing or not a boolean", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such thread in this project and workspace", body = ErrorBody),
    )
)]
pub async fn start_phase(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, thread_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<ThreadResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let thread_id = parse_uuid(&thread_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<StartPhaseRequest>(&body);
    let workspace_id = request.workspace_id;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    let thread = load_thread(&mut tx, &project_id, &thread_id, Some("FOR UPDATE")).await?;
    let sealed = new?
        .sealed
        .ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let number = thread.phases.last().map_or(1, |phase| phase.number + 1);
    insert_phase(&mut tx, &workspace_id, &thread_id, number, sealed).await?;
    live::emit(
        &mut tx,
        &workspace_id,
        &project_id,
        "phase.started",
        &json!({
            "thread_id": thread_id,
            "changeset_id": thread.changeset_id,
            "phase": number,
            "sealed": sealed,
        }),
    )
    .await?;
    let thread = load_thread(&mut tx, &project_id, &thread_id, None).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(ThreadResponse { thread })))
}

/// Records a post in the thread's current phase, as the agent or the human
/// that calls. The thread takes posts whatever the status of its change set.
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
        (status = 400, description = "The type, summary or body is missing, or the summary is blank or longer than 200 characters", body = ErrorBody),
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
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let thread_id = parse_uuid(&thread_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreatePostRequest>(&body);
    let workspace_id = request.workspace_id;
    let author: Value = match request.actor {
        Actor::Agent => {
            let mut author = identity.to_json();
            author["kind"] = json!("agent");
            author
        }
        Actor::Human { user_id, .. } => json!({ "kind": "human", "user_id": user_id }),
    };
    let mut tx = state.workspace_tx(&workspace_id).await?;
    let thread = load_thread(&mut tx, &project_id, &thread_id, Some("FOR SHARE")).await?;
    let new = new?;
    let kind = non_blank(new.kind);
    let summary = non_blank(new.summary)
        .filter(|summary| summary.chars().count() <= MAX_SUMMARY_CHARS && !summary.contains('\n'));
    let (Some(kind), Some(summary), Some(text)) = (kind, summary, new.body) else {
        return Err(ApiError::bad_request("invalid_request"));
    };
    let phase = thread.phases.last().map_or(1, |phase| phase.number);
    let post: PostBody = sqlx::query_as(&format!(
        "INSERT INTO posts (workspace_id, thread_id, phase, type, summary, body, author) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, $7::text::jsonb) RETURNING {POST_COLUMNS}"
    ))
    .bind(&workspace_id)
    .bind(&thread_id)
    .bind(phase)
    .bind(&kind)
    .bind(&summary)
    .bind(&text)
    .bind(author.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok((StatusCode::CREATED, Json(PostResponse { post })))
}
