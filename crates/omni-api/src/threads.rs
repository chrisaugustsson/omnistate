//! Threads and phases (US-033). A thread is a question that agents work out in
//! a change set, with the criteria for a good answer. Its phases, numbered from
//! 1, are open or sealed and the last one is the current phase; an agent starts
//! the next one. A thread belongs to its project and outlives its change set:
//! it keeps taking posts and phases after the change set merges, so builders
//! can raise problems with the plan. Starting a thread emits `thread.created`
//! and starting a later phase emits `phase.started`; the thread's first phase
//! is announced by `thread.created` itself.
//!
//! A thread takes posts (US-034): typed messages with a one-line summary and a
//! Markdown body, recorded in the thread's current phase and never edited or
//! deleted. Only the human posts directives, on a route of its own.
//!
//! Sealed phases (US-035): an agent identity (run and role) may read a post
//! when the human wrote it, when its phase is not the thread's current one,
//! when its phase is open, or when the identity has posted in that phase; the
//! human reads every post. The rule is one SQL predicate, `visible`, which
//! every read path uses: the thread, the digest, a single post, the links of
//! a post and the live events of a socket. Nothing is cached, so the next read
//! after the agent posted or a phase started shows what was hidden. A decision
//! proposed in a sealed phase is sealed too (US-056): `visible_decision` is
//! the same rule applied to the phase the decision was proposed in.

use crate::AppState;
use crate::changesets::{non_blank, parse_body};
use crate::decisions::current_decision;
use crate::docs::AgentIdentity;
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{Actor, AgentRequest, HumanRequest, WorkspaceRequest, parse_uuid};
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
/// The post types, as written. `directive` is the human's.
const POST_TYPES: [&str; 8] = [
    "note",
    "finding",
    "position",
    "objection",
    "support",
    "question",
    "directive",
    "verdict",
];
/// How many links, and how many attachments, one post may carry.
const MAX_REFERENCES: usize = 100;
/// The longest web address in a link, in characters.
const MAX_URL_CHARS: usize = 2048;

/// One phase of a thread.
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct PhaseBody {
    /// From 1; the thread's last phase is its current one.
    pub number: i32,
    /// A sealed phase hides the posts of other agents from an agent that has
    /// not posted in it yet (US-034); an open phase does not.
    pub sealed: bool,
    pub started_at: String,
    /// The distinct agent identities that have posted in this phase, in the
    /// order of their first post there. Told to every caller, also in a
    /// sealed phase: it says who has posted, never what (US-036). The human is
    /// never in it.
    #[sqlx(skip)]
    pub posted_by: Vec<PosterBody>,
}

/// An agent identity that has posted in a phase.
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct PosterBody {
    pub run: Option<String>,
    pub role: Option<String>,
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
    /// The thread's current decision (US-037): the newest, which is not
    /// superseded, as the decision routes answer it, so with at least its `id`
    /// and `status`; `null` while the thread has none. An agent gets only the
    /// posts of it that it may read.
    #[schema(value_type = Option<Object>)]
    pub decision: Option<Value>,
    /// The posts the caller may read, oldest first. Only in the answer to a
    /// read of the thread; an agent does not get what a sealed phase hides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<PostBody>>,
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
    /// The kind of post: `note`, `finding`, `position`, `objection`, `support`,
    /// `question`, `directive` or `verdict`; required. Only the human posts a
    /// `directive`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
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

#[derive(Deserialize, ToSchema)]
pub struct CreateDirectiveRequest {
    /// One line of at most 200 characters; required.
    pub summary: Option<String>,
    /// The Markdown text; required.
    pub body: Option<String>,
    /// As for a post.
    #[schema(value_type = Option<Vec<Object>>)]
    pub links: Option<Vec<Value>>,
    /// As for a post.
    pub attachments: Option<Vec<String>>,
}

#[derive(Serialize, ToSchema)]
pub struct PostBody {
    pub id: String,
    pub thread_id: String,
    /// The number of the thread's phase that the post was made in.
    pub phase: i32,
    #[serde(rename = "type")]
    pub kind: String,
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
    pub phase: i32,
    #[serde(rename = "type")]
    pub kind: String,
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
    phase: i32,
    #[sqlx(rename = "type")]
    kind: String,
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

    pub(crate) fn is_human(&self) -> bool {
        self.human
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

/// The SQL condition that the viewer may read the decision of the alias
/// `decision` (US-056, Q50): a decision belongs to the phase it was proposed
/// in and follows the rule of the posts of that phase, except that no decision
/// is the human's. The parameters are those of `visible`.
pub(crate) fn visible_decision(decision: &str, first: usize) -> String {
    let human = first;
    format!("(${human}::bool OR {})", phase_readable(decision, first))
}

/// The part of the rule of sealed phases that depends on the phase of the row
/// of the alias `row` (a post or a decision, both have `workspace_id`,
/// `thread_id` and `phase`): the phase is not the current one, it is open, or
/// the viewer has posted in it.
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
    phase: i32,
    #[sqlx(rename = "type")]
    kind: String,
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
            phase: self.phase,
            kind: self.kind,
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

pub(crate) const POST_COLUMNS: &str = concat!(
    "id::text, thread_id::text, phase, type, author::text, summary, body, links::text, \
     attachments::text, confidence, replaces::text, ",
    timestamp!("created_at"),
    " AS created_at"
);

/// The phases of a thread, oldest first.
async fn phases_of(
    tx: &mut Transaction<'static, Postgres>,
    thread_id: &str,
) -> Result<Vec<PhaseBody>, ApiError> {
    let mut phases: Vec<PhaseBody> = sqlx::query_as(&format!(
        "SELECT {PHASE_COLUMNS} FROM thread_phases WHERE thread_id = $1::uuid ORDER BY number"
    ))
    .bind(thread_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    // Whoever posted, whatever the viewer may read: the identities only.
    let posters: Vec<(i32, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT p.phase, p.author->>'run', p.author->>'role' FROM posts p \
         WHERE p.thread_id = $1::uuid AND p.author->>'kind' = 'agent' \
         GROUP BY p.phase, p.author->>'run', p.author->>'role' \
         ORDER BY p.phase, min(p.created_at), min(p.id::text)",
    )
    .bind(thread_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    for (number, run, role) in posters {
        if let Some(phase) = phases.iter_mut().find(|phase| phase.number == number) {
            phase.posted_by.push(PosterBody { run, role });
        }
    }
    Ok(phases)
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
        decision: None,
        posts: None,
    })
}

/// The threads of a project, oldest first, each as `show_thread` answers the
/// human but without its posts.
pub(crate) async fn threads_of_project(
    tx: &mut Transaction<'static, Postgres>,
    project_id: &str,
) -> Result<Vec<ThreadBody>, ApiError> {
    let ids: Vec<(String,)> = sqlx::query_as(
        "SELECT id::text FROM threads WHERE project_id = $1::uuid ORDER BY created_at, id",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let viewer = Viewer::human();
    let mut threads = Vec::with_capacity(ids.len());
    for (id,) in ids {
        let mut thread = load_thread(tx, project_id, &id, None).await?;
        thread.decision = current_decision(tx, &viewer, &id).await?;
        threads.push(thread);
    }
    Ok(threads)
}

/// The questions that agents asked in a project, oldest first, in every phase
/// (the human reads sealed phases too), until a post of the human links to the
/// question with `{"kind": "post", "post_id"}`.
pub(crate) async fn unanswered_questions(
    tx: &mut Transaction<'static, Postgres>,
    project_id: &str,
) -> Result<Vec<PostBody>, ApiError> {
    let rows: Vec<PostRow> = sqlx::query_as(&format!(
        "SELECT {POST_COLUMNS} FROM posts p \
         WHERE p.type = 'question' AND p.author->>'kind' = 'agent' \
           AND EXISTS (SELECT 1 FROM threads t WHERE t.workspace_id = p.workspace_id \
                       AND t.id = p.thread_id AND t.project_id = $1::uuid) \
           AND NOT EXISTS (SELECT 1 FROM posts r WHERE r.workspace_id = p.workspace_id \
                           AND r.author->>'kind' = 'human' \
                           AND r.links @> jsonb_build_array(jsonb_build_object( \
                               'kind', 'post', 'post_id', p.id::text))) \
         ORDER BY p.created_at, p.id"
    ))
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    rows.into_iter().map(PostRow::into_body).collect()
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

/// One thread with its phases and the posts the caller may read.
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
    thread.decision = current_decision(&mut tx, &viewer, &thread_id).await?;
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
            "SELECT p.id::text AS id, p.phase, p.type, p.author::text AS author, p.summary, {} \
                 AS created_at \
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
                phase: row.phase,
                kind: row.kind,
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
                           AND t.id = p.thread_id AND t.project_id = $2::uuid) AND {}",
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
    identity: AgentIdentity,
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
    let mut thread = load_thread(&mut tx, &project_id, &thread_id, None).await?;
    thread.decision = current_decision(&mut tx, &Viewer::agent(&identity), &thread_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(ThreadResponse { thread })))
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
    kind: String,
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
    let kind = new
        .kind
        .filter(|kind| POST_TYPES.contains(&kind.as_str()))
        .ok_or_else(invalid)?;
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
        kind,
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

/// Records a post in the thread's current phase and emits `post.created`. The
/// thread takes posts whatever the status of its change set.
async fn record_post(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    thread_id: &str,
    author: Author,
    new: Result<CreatePostRequest, ApiError>,
) -> Result<(StatusCode, Json<PostResponse>), ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    let thread = load_thread(&mut tx, project_id, thread_id, Some("FOR SHARE")).await?;
    let post = check(new?)?;
    check_references(&mut tx, project_id, thread_id, &author, &post).await?;
    let phase = thread.phases.last().map_or(1, |phase| phase.number);
    let row: PostRow = sqlx::query_as(&format!(
        "INSERT INTO posts (workspace_id, thread_id, phase, type, summary, body, author, \
                            links, attachments, confidence, replaces) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, $7::text::jsonb, $8::text::jsonb, \
                 $9::text::jsonb, $10, $11::uuid) RETURNING {POST_COLUMNS}"
    ))
    .bind(workspace_id)
    .bind(thread_id)
    .bind(phase)
    .bind(&post.kind)
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
        &json!({
            "post_id": row.id,
            "thread_id": thread_id,
            "changeset_id": thread.changeset_id,
            "phase": phase,
            "type": post.kind,
        }),
    )
    .await?;
    let post = row.into_body()?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, project_id);
    Ok((StatusCode::CREATED, Json(PostResponse { post })))
}

/// Whether a request body asks for a post of type `directive`.
fn asks_for_a_directive(body: &Bytes) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .is_some_and(|body| body["type"] == "directive")
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
        (status = 400, description = "The type, summary, body, links, attachments, confidence or replaced post is missing or not valid, or the agent sent no `X-Omni-Run` and `X-Omni-Role`", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "An API key posted a `directive`; only the human does", body = ErrorBody),
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
        Actor::Agent if asks_for_a_directive(&body) => return Err(ApiError::Forbidden),
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

/// Records a directive of the human in the thread's current phase. Every agent
/// request inside the thread's change set is answered with the directives that
/// the agent has not been given yet.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/directives",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("thread_id" = String, Path, description = "The thread's UUID"),
    ),
    request_body = CreateDirectiveRequest,
    responses(
        (status = 201, description = "The new post, of type `directive`", body = PostResponse),
        (status = 400, description = "The summary or body is missing or not valid", body = ErrorBody),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such thread in this project and workspace", body = ErrorBody),
    )
)]
pub async fn create_directive(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, thread_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<PostResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let thread_id = parse_uuid(&thread_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateDirectiveRequest>(&body).map(|directive| CreatePostRequest {
        kind: Some("directive".to_string()),
        summary: directive.summary,
        body: directive.body,
        links: directive.links,
        attachments: directive.attachments,
        confidence: None,
        replaces: None,
    });
    record_post(
        &state,
        &request.workspace_id,
        &project_id,
        &thread_id,
        Author::Human(request.user_id),
        new,
    )
    .await
}
