//! Decisions (US-037). A thread ends in a decision record (Q20) that an agent
//! proposes and only the human approves or rejects: the direction, the options
//! with their scores and why each lost, the assumptions, the open questions,
//! the dissent and the posts the decision relies on. The schema is the one of
//! `omni decision propose` (docs/formats/decision.md).
//!
//! A thread has at most one current decision, its newest: a new proposal
//! supersedes the earlier decision whatever its status. The statuses are
//! `proposed`, then `approved` or `rejected` (a rejection keeps its reason),
//! then `superseded`. A change set can be linked to the decisions it carries
//! out and cannot be approved before they are (Q5, `changesets::approve`).
//!
//! Proposing, superseding, approving and rejecting emit `decision.proposed`,
//! `decision.superseded`, `decision.approved` and `decision.rejected`, each
//! with `{decision_id, thread_id, changeset_id, status}`. What an agent reads
//! of a decision keeps to the sealed phases (US-035): the IDs of posts that it
//! may not read are left out of `posts`.
//!
//! A decision belongs to the phase its thread was in when it was proposed. One
//! proposed in a sealed phase is sealed too (US-056, Q50): an agent identity
//! that has not posted in that phase reads it nowhere, and it looks exactly
//! like no decision (404 by ID, `null` on the thread, left out of a change
//! set's `decisions`, no live event) until the identity posts there or a later
//! phase starts. The human reads every decision, and so does every agent one of
//! an open phase.

use crate::AppState;
use crate::changesets::ChangesetResponse;
use crate::changesets::{load_changeset, non_blank, parse_body};
use crate::docs::AgentIdentity;
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::threads::{Viewer, bind_viewer, load_thread, visible, visible_decision};
use crate::workspace::{AgentRequest, HumanRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{Postgres, Transaction};
use utoipa::ToSchema;

/// A timestamp column as ISO 8601 text in UTC.
const CREATED_AT: &str =
    "to_char(d.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')";

/// One option that the thread considered.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionOption {
    /// Not blank, and no two options have the same name.
    pub name: String,
    /// One number for each criterion, keyed by the criterion's text, and
    /// nothing else.
    #[schema(value_type = Object)]
    pub scores: Map<String, Value>,
    /// Why the option lost: text that is not blank for every option but the
    /// chosen one, `null` (or left out) for the chosen one.
    pub lost_because: Option<String>,
}

/// Who disagrees with the decision and why.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Dissent {
    pub by: String,
    pub text: String,
}

/// The decision file of `omni decision propose`. Every field is required and
/// no other field is accepted.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionFile {
    /// The chosen direction; not blank.
    pub direction: String,
    /// The name of the option that won.
    pub chosen: String,
    /// What the options were scored on: at least one, not blank, all different.
    pub criteria: Vec<String>,
    /// At least one option.
    pub options: Vec<DecisionOption>,
    pub assumptions: Vec<String>,
    pub open_questions: Vec<String>,
    pub dissent: Vec<Dissent>,
    /// IDs of posts of the thread that the proposing agent identity may read.
    pub posts: Vec<String>,
}

/// A decision as the API answers it: the proposed fields as they were sent,
/// with its status and where it belongs.
#[derive(Serialize, ToSchema)]
pub struct DecisionBody {
    pub id: String,
    pub project_id: String,
    pub thread_id: String,
    /// The change set of the thread.
    pub changeset_id: String,
    /// The feature of the change set; `null` for a project-level change set.
    pub feature_id: Option<String>,
    /// `proposed`, `approved`, `rejected` or `superseded`.
    pub status: String,
    /// The agent that proposed it: `{"kind": "agent", "run", "role", "model",
    /// "harness"}`.
    #[schema(value_type = Object)]
    pub author: Value,
    pub direction: String,
    pub chosen: String,
    pub criteria: Vec<String>,
    pub options: Vec<DecisionOption>,
    pub assumptions: Vec<String>,
    pub open_questions: Vec<String>,
    pub dissent: Vec<Dissent>,
    /// The posts the decision relies on. An agent gets only those it may read.
    pub posts: Vec<String>,
    /// Why the human rejected it; kept after the decision is superseded.
    pub rejection_reason: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct DecisionResponse {
    pub decision: DecisionBody,
}

#[derive(Deserialize, ToSchema)]
pub struct RejectDecisionRequest {
    /// Why the decision is rejected; required, not blank.
    pub reason: Option<String>,
}

#[derive(sqlx::FromRow)]
struct DecisionRow {
    id: String,
    project_id: String,
    thread_id: String,
    changeset_id: String,
    feature_id: Option<String>,
    status: String,
    content: String,
    author: String,
    rejection_reason: Option<String>,
    created_at: String,
}

impl DecisionRow {
    fn into_body(self) -> Result<DecisionBody, ApiError> {
        let unreadable = |what: &str, error: serde_json::Error| {
            eprintln!(
                "omni-api: the {what} of decision {} are unreadable: {error}",
                self.id
            );
            ApiError::Unavailable
        };
        let file: DecisionFile =
            serde_json::from_str(&self.content).map_err(|e| unreadable("fields", e))?;
        let author = serde_json::from_str(&self.author).map_err(|e| unreadable("author", e))?;
        Ok(DecisionBody {
            id: self.id,
            project_id: self.project_id,
            thread_id: self.thread_id,
            changeset_id: self.changeset_id,
            feature_id: self.feature_id,
            status: self.status,
            author,
            direction: file.direction,
            chosen: file.chosen,
            criteria: file.criteria,
            options: file.options,
            assumptions: file.assumptions,
            open_questions: file.open_questions,
            dissent: file.dissent,
            posts: file.posts,
            rejection_reason: self.rejection_reason,
            created_at: self.created_at,
        })
    }
}

/// The columns and tables of a decision read; the project and the change set
/// are those of the decision's thread.
fn select(condition: &str) -> String {
    format!(
        "SELECT d.id::text AS id, t.project_id::text AS project_id, \
                d.thread_id::text AS thread_id, t.changeset_id::text AS changeset_id, \
                (SELECT c.feature_id::text FROM changesets c \
                  WHERE c.workspace_id = t.workspace_id AND c.id = t.changeset_id) AS feature_id, \
                d.status, d.content::text AS content, d.author::text AS author, \
                d.rejection_reason, {CREATED_AT} AS created_at \
         FROM decisions d JOIN threads t ON t.workspace_id = d.workspace_id AND t.id = d.thread_id \
         WHERE {condition}"
    )
}

fn invalid() -> ApiError {
    ApiError::bad_request("invalid_request")
}

/// Whether no two of the texts are the same.
fn all_different<'a>(texts: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = std::collections::HashSet::new();
    texts.into_iter().all(|text| seen.insert(text))
}

/// The decision file with every rule of the schema applied, `invalid_request`
/// for one that breaks a rule. The IDs of its posts come back in lowercase.
fn check(mut file: DecisionFile) -> Result<DecisionFile, ApiError> {
    let blank = |text: &str| text.trim().is_empty();
    if blank(&file.direction) {
        return Err(invalid());
    }
    if file.criteria.is_empty()
        || file.criteria.iter().any(|criterion| blank(criterion))
        || !all_different(file.criteria.iter().map(String::as_str))
    {
        return Err(invalid());
    }
    if file.options.is_empty()
        || file.options.iter().any(|option| blank(&option.name))
        || !all_different(file.options.iter().map(|option| option.name.as_str()))
        || !file.options.iter().any(|option| option.name == file.chosen)
    {
        return Err(invalid());
    }
    for option in &mut file.options {
        let scored = option.scores.len() == file.criteria.len()
            && file
                .criteria
                .iter()
                .all(|criterion| option.scores.get(criterion).is_some_and(Value::is_number));
        let reason_fits = if option.name == file.chosen {
            option.lost_because.is_none()
        } else {
            option
                .lost_because
                .as_deref()
                .is_some_and(|why| !blank(why))
        };
        if !scored || !reason_fits {
            return Err(invalid());
        }
    }
    let texts = file
        .assumptions
        .iter()
        .chain(&file.open_questions)
        .chain(file.dissent.iter().flat_map(|d| [&d.by, &d.text]));
    if texts.into_iter().any(|text| blank(text)) {
        return Err(invalid());
    }
    file.posts = file
        .posts
        .iter()
        .map(|id| parse_uuid(id).ok_or_else(invalid))
        .collect::<Result<_, _>>()?;
    Ok(file)
}

/// The data of the four decision events.
fn event_data(decision_id: &str, thread_id: &str, changeset_id: &str, status: &str) -> Value {
    json!({
        "decision_id": decision_id,
        "thread_id": thread_id,
        "changeset_id": changeset_id,
        "status": status,
    })
}

/// Fails with `invalid_request` unless every post is one of the thread's that
/// the viewer may read.
async fn require_readable_posts(
    tx: &mut Transaction<'static, Postgres>,
    viewer: &Viewer,
    thread_id: &str,
    posts: &[String],
) -> Result<(), ApiError> {
    let mut wanted: Vec<&String> = posts.iter().collect();
    wanted.sort();
    wanted.dedup();
    let found: i64 = bind_viewer(
        sqlx::query_as::<_, (i64,)>(&format!(
            "SELECT count(*) FROM posts p WHERE p.id = ANY($1::text[]::uuid[]) \
             AND p.thread_id = $2::uuid AND {}",
            visible("p", 3)
        ))
        .bind(&wanted)
        .bind(thread_id),
        viewer,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .0;
    if found == wanted.len() as i64 {
        Ok(())
    } else {
        Err(invalid())
    }
}

/// Leaves out of the decision's posts the IDs of posts that the viewer may not
/// read.
async fn hide_unseen_posts(
    tx: &mut Transaction<'static, Postgres>,
    viewer: &Viewer,
    decision: &mut DecisionBody,
) -> Result<(), ApiError> {
    if decision.posts.is_empty() {
        return Ok(());
    }
    let seen: Vec<String> = bind_viewer(
        sqlx::query_as::<_, (String,)>(&format!(
            "SELECT p.id::text FROM posts p WHERE p.id = ANY($1::text[]::uuid[]) \
             AND p.thread_id = $2::uuid AND {}",
            visible("p", 3)
        ))
        .bind(&decision.posts)
        .bind(&decision.thread_id),
        viewer,
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .into_iter()
    .map(|(id,)| id)
    .collect();
    decision.posts.retain(|id| seen.contains(id));
    Ok(())
}

/// The decision with this ID in the project as `viewer` reads it, 404 when the
/// project has none or the viewer may not read it yet.
pub(crate) async fn load_decision(
    tx: &mut Transaction<'static, Postgres>,
    viewer: &Viewer,
    project_id: &str,
    decision_id: &str,
) -> Result<DecisionBody, ApiError> {
    let row: DecisionRow = bind_viewer(
        sqlx::query_as(&select(&format!(
            "d.id = $1::uuid AND t.project_id = $2::uuid AND {}",
            visible_decision("d", 3)
        )))
        .bind(decision_id)
        .bind(project_id),
        viewer,
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    let mut decision = row.into_body()?;
    hide_unseen_posts(tx, viewer, &mut decision).await?;
    Ok(decision)
}

/// The decisions of a project that are `proposed`, oldest first, as `viewer`
/// reads them.
pub(crate) async fn proposed_of_project(
    tx: &mut Transaction<'static, Postgres>,
    viewer: &Viewer,
    project_id: &str,
) -> Result<Vec<DecisionBody>, ApiError> {
    let rows: Vec<DecisionRow> = bind_viewer(
        sqlx::query_as(&format!(
            "{} ORDER BY d.created_at, d.id",
            select(&format!(
                "t.project_id = $1::uuid AND d.status = 'proposed' AND {}",
                visible_decision("d", 2)
            ))
        ))
        .bind(project_id),
        viewer,
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let mut decisions = Vec::with_capacity(rows.len());
    for row in rows {
        let mut decision = row.into_body()?;
        hide_unseen_posts(tx, viewer, &mut decision).await?;
        decisions.push(decision);
    }
    Ok(decisions)
}

/// The thread's current decision as `viewer` reads it, as the JSON of a
/// `DecisionBody`: the one that is not superseded, `None` while the thread has
/// none or the viewer may not read it yet.
pub(crate) async fn current_decision(
    tx: &mut Transaction<'static, Postgres>,
    viewer: &Viewer,
    thread_id: &str,
) -> Result<Option<Value>, ApiError> {
    let row: Option<DecisionRow> = bind_viewer(
        sqlx::query_as(&select(&format!(
            "d.thread_id = $1::uuid AND d.status <> 'superseded' AND {}",
            visible_decision("d", 2)
        )))
        .bind(thread_id),
        viewer,
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let Some(row) = row else { return Ok(None) };
    let mut decision = row.into_body()?;
    hide_unseen_posts(tx, viewer, &mut decision).await?;
    let decision = serde_json::to_value(&decision).map_err(|error| {
        eprintln!(
            "omni-api: decision {} cannot be written: {error}",
            decision.id
        );
        ApiError::Unavailable
    })?;
    Ok(Some(decision))
}

/// Proposes a decision for a thread. The thread's earlier decision, whatever
/// its status, is superseded.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/decisions",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("thread_id" = String, Path, description = "The thread's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = DecisionFile,
    responses(
        (status = 201, description = "The new decision, `proposed`", body = DecisionResponse),
        (status = 400, description = "The body breaks the schema of docs/formats/decision.md, or relies on a post that is not one of the thread's that the agent may read", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such thread in this project and workspace", body = ErrorBody),
    )
)]
pub async fn propose(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, thread_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<DecisionResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let thread_id = parse_uuid(&thread_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<DecisionFile>(&body);
    let workspace_id = request.workspace_id;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    // The thread's row is locked until the end, so of several proposals that
    // are made at once each supersedes the one before.
    let thread = load_thread(&mut tx, &project_id, &thread_id, Some("FOR UPDATE")).await?;
    // Only a thread of a change set has a decision.
    let changeset_id = thread.changeset_id.clone().ok_or(ApiError::NotFound)?;
    let file = check(new?)?;
    let viewer = Viewer::agent(&identity);
    require_readable_posts(&mut tx, &viewer, &thread_id, &file.posts).await?;
    // The decision belongs to the phase the thread is in now; the lock on the
    // thread keeps a new phase out until the decision is stored.
    let phase = thread
        .phases
        .iter()
        .flatten()
        .last()
        .map_or(1, |phase| phase.number);

    let superseded: Vec<(String,)> = sqlx::query_as(
        "UPDATE decisions SET status = 'superseded' \
         WHERE thread_id = $1::uuid AND status <> 'superseded' RETURNING id::text",
    )
    .bind(&thread_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let mut author = identity.to_json();
    author["kind"] = json!("agent");
    let decision_id: String = sqlx::query_scalar(
        "INSERT INTO decisions (workspace_id, thread_id, phase, content, author) \
         VALUES ($1::uuid, $2::uuid, $3, $4::text::jsonb, $5::text::jsonb) RETURNING id::text",
    )
    .bind(&workspace_id)
    .bind(&thread_id)
    .bind(phase)
    .bind(json!(file).to_string())
    .bind(author.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    for (earlier,) in &superseded {
        let data = event_data(earlier, &thread_id, &changeset_id, "superseded");
        live::emit(
            &mut tx,
            &workspace_id,
            &project_id,
            "decision.superseded",
            &data,
        )
        .await?;
    }
    let data = event_data(&decision_id, &thread_id, &changeset_id, "proposed");
    live::emit(
        &mut tx,
        &workspace_id,
        &project_id,
        "decision.proposed",
        &data,
    )
    .await?;
    // The proposer is told what it proposed even if it has not posted in a
    // sealed phase, where a later read of the decision is 404 until it does.
    let mut decision = load_decision(&mut tx, &Viewer::human(), &project_id, &decision_id).await?;
    hide_unseen_posts(&mut tx, &viewer, &mut decision).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(DecisionResponse { decision })))
}

/// One decision of a project, a superseded one included.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("decision_id" = String, Path, description = "The decision's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
    ),
    responses(
        (status = 200, description = "The decision; an agent gets only the posts it may read", body = DecisionResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such decision in this project and workspace, or one that the agent may not read yet (proposed in a sealed phase in which it has not posted)", body = ErrorBody),
    )
)]
pub async fn show(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, decision_id)): Path<(String, String, String)>,
) -> Result<Json<DecisionResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let decision_id = parse_uuid(&decision_id).ok_or(ApiError::NotFound)?;
    let viewer = Viewer::of(&request.actor, &identity);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let decision = load_decision(&mut tx, &viewer, &project_id, &decision_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(DecisionResponse { decision }))
}

/// What the human decides about a proposed decision.
enum Verdict {
    Approve,
    /// With the reason, not blank.
    Reject(String),
}

/// Approves or rejects a decision that is still `proposed`. The decision's row
/// is locked first, so of two requests that race for it, or a request and a
/// new proposal that supersedes it, one wins and the other is refused.
async fn decide(
    state: &AppState,
    request: &HumanRequest,
    project_id: &str,
    decision_id: &str,
    verdict: Result<Verdict, ApiError>,
) -> Result<Json<DecisionResponse>, ApiError> {
    let project_id = parse_uuid(project_id).ok_or(ApiError::NotFound)?;
    let decision_id = parse_uuid(decision_id).ok_or(ApiError::NotFound)?;
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    let (status, thread_id, changeset_id): (String, String, String) = sqlx::query_as(
        "SELECT d.status, d.thread_id::text, t.changeset_id::text \
         FROM decisions d JOIN threads t ON t.workspace_id = d.workspace_id AND t.id = d.thread_id \
         WHERE d.id = $1::uuid AND t.project_id = $2::uuid FOR UPDATE OF d",
    )
    .bind(&decision_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    let verdict = verdict?;
    if status != "proposed" {
        return Err(ApiError::conflict("invalid_transition"));
    }
    let (to, event, reason) = match &verdict {
        Verdict::Approve => ("approved", "decision.approved", None),
        Verdict::Reject(reason) => ("rejected", "decision.rejected", Some(reason.as_str())),
    };
    sqlx::query("UPDATE decisions SET status = $2, rejection_reason = $3 WHERE id = $1::uuid")
        .bind(&decision_id)
        .bind(to)
        .bind(reason)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    let data = event_data(&decision_id, &thread_id, &changeset_id, to);
    live::emit(&mut tx, workspace_id, &project_id, event, &data).await?;
    let decision = load_decision(&mut tx, &Viewer::human(), &project_id, &decision_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok(Json(DecisionResponse { decision }))
}

/// Approves a proposed decision. Only the human does.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}/approve",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("decision_id" = String, Path, description = "The decision's UUID"),
    ),
    responses(
        (status = 200, description = "The decision, now `approved`", body = DecisionResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such decision in this project and workspace", body = ErrorBody),
        (status = 409, description = "The decision is not `proposed`", body = ErrorBody),
    )
)]
pub async fn approve(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, decision_id)): Path<(String, String, String)>,
) -> Result<Json<DecisionResponse>, ApiError> {
    decide(
        &state,
        &request,
        &project_id,
        &decision_id,
        Ok(Verdict::Approve),
    )
    .await
}

/// Rejects a proposed decision, with the reason. Only the human does.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}/reject",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("decision_id" = String, Path, description = "The decision's UUID"),
    ),
    request_body = RejectDecisionRequest,
    responses(
        (status = 200, description = "The decision, now `rejected`, with its reason", body = DecisionResponse),
        (status = 400, description = "The reason is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such decision in this project and workspace", body = ErrorBody),
        (status = 409, description = "The decision is not `proposed`", body = ErrorBody),
    )
)]
pub async fn reject(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, decision_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<DecisionResponse>, ApiError> {
    let verdict = parse_body::<RejectDecisionRequest>(&body).and_then(|rejection| {
        non_blank(rejection.reason)
            .map(Verdict::Reject)
            .ok_or_else(invalid)
    });
    decide(&state, &request, &project_id, &decision_id, verdict).await
}

/// Links a change set to a decision that it carries out. The change set cannot
/// be approved while a linked decision is not approved.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/decisions/{decision_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("decision_id" = String, Path, description = "The decision's UUID, of any thread of the project"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
    ),
    responses(
        (status = 200, description = "The change set with its linked decisions; linking a decision again changes nothing", body = ChangesetResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set or decision in this project and workspace, or a decision that the agent may not read yet (a sealed phase)", body = ErrorBody),
    )
)]
pub async fn link(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id, decision_id)): Path<(String, String, String, String)>,
) -> Result<Json<ChangesetResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let decision_id = parse_uuid(&decision_id).ok_or(ApiError::NotFound)?;
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    // FOR SHARE keeps out an approval of the change set that is deciding
    // right now, which would not see the new link.
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM changesets WHERE id = $1::uuid AND project_id = $2::uuid FOR SHARE",
    )
    .bind(&changeset_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    found.ok_or(ApiError::NotFound)?;
    // A decision that the agent may not read yet is no decision to it.
    let viewer = Viewer::agent(&identity);
    let found: Option<(String,)> = bind_viewer(
        sqlx::query_as(&format!(
            "SELECT d.id::text FROM decisions d \
             JOIN threads t ON t.workspace_id = d.workspace_id AND t.id = d.thread_id \
             WHERE d.id = $1::uuid AND t.project_id = $2::uuid AND {}",
            visible_decision("d", 3)
        ))
        .bind(&decision_id)
        .bind(&project_id),
        &viewer,
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    found.ok_or(ApiError::NotFound)?;
    sqlx::query(
        "INSERT INTO changeset_decisions (workspace_id, changeset_id, decision_id) \
         VALUES ($1::uuid, $2::uuid, $3::uuid) \
         ON CONFLICT (workspace_id, changeset_id, decision_id) DO NOTHING",
    )
    .bind(workspace_id)
    .bind(&changeset_id)
    .bind(&decision_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let changeset = load_changeset(&mut tx, &viewer, &changeset_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(ChangesetResponse { changeset }))
}
