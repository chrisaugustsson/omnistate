//! Projects and change sets. A project holds the change sets of one body of
//! work; a change set moves along the lifecycle of FR-20 and keeps every move
//! in its status history. Agents create projects and change sets and move them
//! (submit, reopen, abandon); only the human rejects, and must give a reason.
//! Nothing is deleted: an abandoned change set stays listed and shown.
//!
//! The human approves a change set in review (US-025). Approving records the
//! latest saved version of each changed doc and merges them at once when every
//! doc's base is still its mainline version, else the change set needs a rebase
//! (US-026). The server, not a request, makes both of those moves, so its
//! history entries carry the actor `system`.

use crate::AppState;
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, HumanRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;
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

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct ProjectBody {
    pub id: String,
    /// Unique in the workspace; the omni CLI accepts it where it takes a project.
    pub name: String,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct ProjectResponse {
    pub project: ProjectBody,
}

#[derive(Serialize, ToSchema)]
pub struct ProjectListResponse {
    pub projects: Vec<ProjectBody>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateProjectRequest {
    pub name: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateChangesetRequest {
    pub title: Option<String>,
    /// Defaults to the empty text.
    pub description: Option<String>,
    /// The run (such as a UI research run) that the change set belongs to.
    pub run_tag: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct RejectRequest {
    /// Why the change set is rejected; required.
    pub reason: Option<String>,
}

/// One status change of a change set. The entry that creates it has no
/// `from_status`.
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct HistoryEntry {
    pub from_status: Option<String>,
    pub to_status: String,
    /// `agent` or `human`, or `system` when the server moved the change set
    /// itself (saving a version clears an approval).
    pub actor: String,
    /// Only set when the human rejected the change set.
    pub reason: Option<String>,
    pub at: String,
}

/// The version of a doc that the human approved.
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct ApprovedVersion {
    pub doc_id: String,
    pub version_id: String,
}

#[derive(Serialize, ToSchema)]
pub struct ChangesetBody {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: String,
    /// `open`, `in_review`, `approved`, `rejected`, `needs_rebase`, `merged` or `abandoned`.
    pub status: String,
    pub run_tag: Option<String>,
    pub created_at: String,
    /// The status changes, oldest first.
    pub history: Vec<HistoryEntry>,
    /// What the human approved: the latest version saved in the change set
    /// for each doc that has one, removals included. Empty while the change
    /// set has no approval (saving a version clears it).
    pub approved_versions: Vec<ApprovedVersion>,
}

#[derive(Serialize, ToSchema)]
pub struct ChangesetResponse {
    pub changeset: ChangesetBody,
}

#[derive(Serialize, ToSchema)]
pub struct ChangesetListResponse {
    pub changesets: Vec<ChangesetBody>,
}

/// A history entry with the change set it belongs to, for the list.
#[derive(sqlx::FromRow)]
struct ListedEntry {
    changeset_id: String,
    #[sqlx(flatten)]
    entry: HistoryEntry,
}

/// An approved version with the change set it belongs to, for the list.
#[derive(sqlx::FromRow)]
struct ListedApproval {
    changeset_id: String,
    #[sqlx(flatten)]
    approved: ApprovedVersion,
}

#[derive(sqlx::FromRow)]
struct ChangesetRow {
    id: String,
    project_id: String,
    title: String,
    description: String,
    status: String,
    run_tag: Option<String>,
    created_at: String,
}

impl ChangesetRow {
    fn into_body(
        self,
        history: Vec<HistoryEntry>,
        approved_versions: Vec<ApprovedVersion>,
    ) -> ChangesetBody {
        ChangesetBody {
            id: self.id,
            project_id: self.project_id,
            title: self.title,
            description: self.description,
            status: self.status,
            run_tag: self.run_tag,
            created_at: self.created_at,
            history,
            approved_versions,
        }
    }
}

const CHANGESET_COLUMNS: &str = concat!(
    "id::text, project_id::text, title, description, status, run_tag, ",
    timestamp!("created_at"),
    " AS created_at"
);

const HISTORY_COLUMNS: &str = concat!(
    "from_status, to_status, actor, reason, ",
    timestamp!("created_at"),
    " AS at"
);

/// The JSON body of a request, or 400 `invalid_request` when it is not JSON of
/// the expected shape. An empty body counts as `{}`.
pub(crate) fn parse_body<T: DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    let bytes: &[u8] = if body.iter().all(u8::is_ascii_whitespace) {
        b"{}"
    } else {
        body
    };
    serde_json::from_slice(bytes).map_err(|_| ApiError::bad_request("invalid_request"))
}

/// The text without its surrounding whitespace, `None` when nothing is left.
pub(crate) fn non_blank(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Creates a project.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects",
    params(("workspace_id" = String, Path, description = "The workspace's UUID")),
    request_body = CreateProjectRequest,
    responses(
        (status = 201, description = "The new project", body = ProjectResponse),
        (status = 400, description = "The name is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "The caller cannot reach this workspace", body = ErrorBody),
        (status = 409, description = "The workspace has a project of this name", body = ErrorBody),
    )
)]
pub async fn create_project(
    State(state): State<AppState>,
    request: AgentRequest,
    body: Bytes,
) -> Result<(StatusCode, Json<ProjectResponse>), ApiError> {
    let name = non_blank(parse_body::<CreateProjectRequest>(&body)?.name)
        .ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let inserted = sqlx::query_as(concat!(
        "INSERT INTO projects (workspace_id, name) VALUES ($1::uuid, $2) \
         RETURNING id::text, name, ",
        timestamp!("created_at"),
        " AS created_at"
    ))
    .bind(&request.workspace_id)
    .bind(&name)
    .fetch_one(&mut *tx)
    .await;
    let project = match inserted {
        Ok(project) => project,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::conflict("name_taken"));
        }
        Err(e) => return Err(ApiError::database(e)),
    };
    tx.commit().await.map_err(ApiError::database)?;
    Ok((StatusCode::CREATED, Json(ProjectResponse { project })))
}

/// The projects of the workspace, oldest first.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects",
    params(("workspace_id" = String, Path, description = "The workspace's UUID")),
    responses(
        (status = 200, description = "The workspace's projects", body = ProjectListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "The caller cannot reach this workspace", body = ErrorBody),
    )
)]
pub async fn list_projects(
    State(state): State<AppState>,
    request: WorkspaceRequest,
) -> Result<Json<ProjectListResponse>, ApiError> {
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let projects = sqlx::query_as(concat!(
        "SELECT id::text, name, ",
        timestamp!("created_at"),
        " AS created_at FROM projects ORDER BY created_at, id"
    ))
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(ProjectListResponse { projects }))
}

/// Fails with 404 unless the project exists in the transaction's workspace
/// (row-level security hides the projects of other workspaces).
pub(crate) async fn require_project(
    tx: &mut Transaction<'static, Postgres>,
    project_id: &str,
) -> Result<(), ApiError> {
    let found: Option<(String,)> =
        sqlx::query_as("SELECT id::text FROM projects WHERE id = $1::uuid")
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(ApiError::database)?;
    found.map(|_| ()).ok_or(ApiError::NotFound)
}

async fn history_of(
    tx: &mut Transaction<'static, Postgres>,
    changeset_id: &str,
) -> Result<Vec<HistoryEntry>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {HISTORY_COLUMNS} FROM changeset_status_history \
         WHERE changeset_id = $1::uuid ORDER BY created_at, id"
    ))
    .bind(changeset_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}

async fn approved_of(
    tx: &mut Transaction<'static, Postgres>,
    changeset_id: &str,
) -> Result<Vec<ApprovedVersion>, ApiError> {
    sqlx::query_as(
        "SELECT doc_id::text AS doc_id, version_id::text AS version_id FROM approvals \
         WHERE changeset_id = $1::uuid AND NOT cleared ORDER BY created_at, id",
    )
    .bind(changeset_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// One move of a change set along the lifecycle, as the history keeps it.
struct StatusChange<'a> {
    project_id: &'a str,
    changeset_id: &'a str,
    /// `None` for the creation.
    from: Option<&'a str>,
    to: &'a str,
    actor: &'a str,
    reason: Option<&'a str>,
}

/// Records a status change in the history and emits its live event,
/// `changeset.<status>`. Callers announce the event once they have committed.
async fn insert_history(
    tx: &mut Transaction<'static, Postgres>,
    workspace_id: &str,
    change: StatusChange<'_>,
) -> Result<(), ApiError> {
    let StatusChange {
        project_id,
        changeset_id,
        from,
        to,
        actor,
        reason,
    } = change;
    sqlx::query(
        "INSERT INTO changeset_status_history \
         (workspace_id, changeset_id, from_status, to_status, actor, reason) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
    )
    .bind(workspace_id)
    .bind(changeset_id)
    .bind(from)
    .bind(to)
    .bind(actor)
    .bind(reason)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let mut data = json!({
        "changeset_id": changeset_id,
        "from_status": from,
        "status": to,
        "actor": actor,
    });
    if let Some(reason) = reason {
        data["reason"] = json!(reason);
    }
    live::emit(
        tx,
        workspace_id,
        project_id,
        &format!("changeset.{to}"),
        &data,
    )
    .await?;
    Ok(())
}

/// Creates a change set, `open`, in a project.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
    ),
    request_body = CreateChangesetRequest,
    responses(
        (status = 201, description = "The new change set", body = ChangesetResponse),
        (status = 400, description = "The title is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn create_changeset(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<ChangesetResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateChangesetRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let new = new?;
    let title = non_blank(new.title).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let run_tag = non_blank(new.run_tag);
    let row: ChangesetRow = sqlx::query_as(&format!(
        "INSERT INTO changesets (workspace_id, project_id, title, description, run_tag) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5) RETURNING {CHANGESET_COLUMNS}"
    ))
    .bind(&request.workspace_id)
    .bind(&project_id)
    .bind(&title)
    .bind(new.description.unwrap_or_default())
    .bind(run_tag)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let creation = StatusChange {
        project_id: &project_id,
        changeset_id: &row.id,
        from: None,
        to: &row.status,
        actor: "agent",
        reason: None,
    };
    insert_history(&mut tx, &request.workspace_id, creation).await?;
    let history = history_of(&mut tx, &row.id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok((
        StatusCode::CREATED,
        Json(ChangesetResponse {
            changeset: row.into_body(history, Vec::new()),
        }),
    ))
}

/// The change sets of a project, oldest first, whatever their status.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
    ),
    responses(
        (status = 200, description = "The project's change sets, abandoned ones included", body = ChangesetListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn list_changesets(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
) -> Result<Json<ChangesetListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let rows: Vec<ChangesetRow> = sqlx::query_as(&format!(
        "SELECT {CHANGESET_COLUMNS} FROM changesets WHERE project_id = $1::uuid \
         ORDER BY created_at, id"
    ))
    .bind(&project_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let entries: Vec<ListedEntry> = sqlx::query_as(&format!(
        "SELECT changeset_id::text AS changeset_id, {HISTORY_COLUMNS} \
         FROM changeset_status_history \
         WHERE changeset_id IN (SELECT id FROM changesets WHERE project_id = $1::uuid) \
         ORDER BY created_at, id"
    ))
    .bind(&project_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let approvals: Vec<ListedApproval> = sqlx::query_as(
        "SELECT changeset_id::text AS changeset_id, doc_id::text AS doc_id, \
         version_id::text AS version_id FROM approvals \
         WHERE NOT cleared AND changeset_id IN (SELECT id FROM changesets WHERE project_id = $1::uuid) \
         ORDER BY created_at, id",
    )
    .bind(&project_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let mut histories: HashMap<String, Vec<HistoryEntry>> = HashMap::new();
    for listed in entries {
        histories
            .entry(listed.changeset_id)
            .or_default()
            .push(listed.entry);
    }
    let mut approved: HashMap<String, Vec<ApprovedVersion>> = HashMap::new();
    for listed in approvals {
        approved
            .entry(listed.changeset_id)
            .or_default()
            .push(listed.approved);
    }
    let changesets = rows
        .into_iter()
        .map(|row| {
            let history = histories.remove(&row.id).unwrap_or_default();
            let approved = approved.remove(&row.id).unwrap_or_default();
            row.into_body(history, approved)
        })
        .collect();
    Ok(Json(ChangesetListResponse { changesets }))
}

/// One change set with its status history.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
    ),
    responses(
        (status = 200, description = "The change set", body = ChangesetResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
    )
)]
pub async fn show_changeset(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
) -> Result<Json<ChangesetResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row: ChangesetRow = sqlx::query_as(&format!(
        "SELECT {CHANGESET_COLUMNS} FROM changesets WHERE id = $1::uuid AND project_id = $2::uuid"
    ))
    .bind(&changeset_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    let history = history_of(&mut tx, &row.id).await?;
    let approved = approved_of(&mut tx, &row.id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(ChangesetResponse {
        changeset: row.into_body(history, approved),
    }))
}

/// The lifecycle actions of this story and the statuses each may start from
/// (FR-20). The first status that matches decides the new one.
#[derive(Clone, Copy)]
enum Action {
    Submit,
    Reject,
    Reopen,
    Abandon,
}

impl Action {
    /// The status the action leads to from `status`, `None` when it may not
    /// start there.
    fn target(self, status: &str) -> Option<&'static str> {
        match (self, status) {
            (Self::Submit, "open") => Some("in_review"),
            (Self::Reject, "in_review") => Some("rejected"),
            (Self::Reopen, "rejected") => Some("open"),
            (Self::Abandon, "open" | "in_review" | "rejected") => Some("abandoned"),
            _ => None,
        }
    }
}

/// Moves a change set along the lifecycle and records the move. The row is
/// locked first, so of several requests that race for the same change set one
/// makes the move and the rest find the new status and are refused. This is
/// the place where a status changes by request: the live events of US-017 are
/// emitted from here (through `insert_history`).
async fn transition(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    changeset_id: &str,
    action: Action,
    actor: &str,
    reason: Option<&str>,
) -> Result<Json<ChangesetResponse>, ApiError> {
    let project_id = parse_uuid(project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(changeset_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(workspace_id).await?;
    let status: String = sqlx::query_scalar(
        "SELECT status FROM changesets WHERE id = $1::uuid AND project_id = $2::uuid FOR UPDATE",
    )
    .bind(&changeset_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    if matches!(action, Action::Reject) && reason.is_none() {
        return Err(ApiError::bad_request("reason_required"));
    }
    let to = action
        .target(&status)
        .ok_or_else(|| ApiError::conflict("invalid_transition"))?;
    sqlx::query("UPDATE changesets SET status = $2 WHERE id = $1::uuid")
        .bind(&changeset_id)
        .bind(to)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    let change = StatusChange {
        project_id: &project_id,
        changeset_id: &changeset_id,
        from: Some(&status),
        to,
        actor,
        reason,
    };
    insert_history(&mut tx, workspace_id, change).await?;
    let row: ChangesetRow = sqlx::query_as(&format!(
        "SELECT {CHANGESET_COLUMNS} FROM changesets WHERE id = $1::uuid"
    ))
    .bind(&changeset_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let history = history_of(&mut tx, &changeset_id).await?;
    let approved = approved_of(&mut tx, &changeset_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok(Json(ChangesetResponse {
        changeset: row.into_body(history, approved),
    }))
}

/// Saving a version while a change set is `approved` or `needs_rebase` clears
/// the approval: the change set moves back to `in_review`, in the transaction of
/// the save, with the actor `system` (a rebase, US-026, saves its versions
/// without calling this). The row is locked first, as in `transition`. Any
/// other status stays as it is.
pub(crate) async fn clear_approval(
    tx: &mut Transaction<'static, Postgres>,
    workspace_id: &str,
    project_id: &str,
    changeset_id: &str,
) -> Result<(), ApiError> {
    let status: String =
        sqlx::query_scalar("SELECT status FROM changesets WHERE id = $1::uuid FOR UPDATE")
            .bind(changeset_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(ApiError::database)?
            .ok_or(ApiError::NotFound)?;
    if !matches!(status.as_str(), "approved" | "needs_rebase") {
        return Ok(());
    }
    sqlx::query("UPDATE changesets SET status = 'in_review' WHERE id = $1::uuid")
        .bind(changeset_id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::database)?;
    sqlx::query(
        "UPDATE approvals SET cleared = true WHERE changeset_id = $1::uuid AND NOT cleared",
    )
    .bind(changeset_id)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let change = StatusChange {
        project_id,
        changeset_id,
        from: Some(&status),
        to: "in_review",
        actor: "system",
        reason: None,
    };
    insert_history(tx, workspace_id, change).await
}

/// Sends an `open` change set to review.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/submit",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
    ),
    responses(
        (status = 200, description = "The change set, now `in_review`", body = ChangesetResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
        (status = 409, description = "The change set is not `open`", body = ErrorBody),
    )
)]
pub async fn submit(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
) -> Result<Json<ChangesetResponse>, ApiError> {
    transition(
        &state,
        &request.workspace_id,
        &project_id,
        &changeset_id,
        Action::Submit,
        "agent",
        None,
    )
    .await
}

/// Rejects a change set that is in review. The reason is required and kept in
/// the history.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reject",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
    ),
    request_body = RejectRequest,
    responses(
        (status = 200, description = "The change set, now `rejected`", body = ChangesetResponse),
        (status = 400, description = "The reason is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
        (status = 409, description = "The change set is not `in_review`", body = ErrorBody),
    )
)]
pub async fn reject(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<ChangesetResponse>, ApiError> {
    // A body that is not JSON has no reason either.
    let reason = parse_body::<RejectRequest>(&body)
        .ok()
        .and_then(|rejection| non_blank(rejection.reason));
    transition(
        &state,
        &request.workspace_id,
        &project_id,
        &changeset_id,
        Action::Reject,
        "human",
        reason.as_deref(),
    )
    .await
}

/// A doc that the change set changes, with the version that the human sees
/// and what its base is compared with.
#[derive(sqlx::FromRow)]
struct MergeCandidate {
    mainline_version_id: Option<String>,
    base_version_id: Option<String>,
}

/// Approves a change set that is in review: records the latest saved version
/// of each changed doc, then merges them when every doc's base is still its
/// mainline version, or moves the change set to `needs_rebase` when one is not
/// (nothing merges then). The change set row is locked first, so only one
/// request decides its fate, and the docs are locked in ID order, so of two
/// change sets on the same base that are approved at once the first merges and
/// the second finds the new mainline and needs a rebase. Both moves are made by
/// the server (actor `system`) in the transaction of the approval.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/approve",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
    ),
    responses(
        (status = 200, description = "The change set, `merged` or `needs_rebase`", body = ChangesetResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
        (status = 409, description = "The change set is not `in_review`", body = ErrorBody),
    )
)]
pub async fn approve(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
) -> Result<Json<ChangesetResponse>, ApiError> {
    let workspace_id = &request.workspace_id;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(workspace_id).await?;
    // NO KEY UPDATE keeps out the other moves of this change set but not the
    // saves of versions, whose foreign keys only share the row.
    let status: String = sqlx::query_scalar(
        "SELECT status FROM changesets WHERE id = $1::uuid AND project_id = $2::uuid \
         FOR NO KEY UPDATE",
    )
    .bind(&changeset_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    if status != "in_review" {
        return Err(ApiError::conflict("invalid_transition"));
    }

    // The latest version saved for each doc, in doc order.
    let approved: Vec<ApprovedVersion> = sqlx::query_as(
        "SELECT DISTINCT ON (doc_id) doc_id::text AS doc_id, id::text AS version_id \
         FROM versions WHERE changeset_id = $1::uuid \
         ORDER BY doc_id, created_at DESC, id DESC",
    )
    .bind(&changeset_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let docs: Vec<&str> = approved.iter().map(|a| a.doc_id.as_str()).collect();
    let versions: Vec<&str> = approved.iter().map(|a| a.version_id.as_str()).collect();
    sqlx::query(
        "INSERT INTO approvals (workspace_id, changeset_id, doc_id, version_id) \
         SELECT $1::uuid, $2::uuid, doc, version \
         FROM unnest($3::text[]::uuid[], $4::text[]::uuid[]) AS a (doc, version)",
    )
    .bind(workspace_id)
    .bind(&changeset_id)
    .bind(&docs)
    .bind(&versions)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    move_status(
        &mut tx,
        workspace_id,
        &project_id,
        &changeset_id,
        "in_review",
        "approved",
        "human",
    )
    .await?;

    // The docs stay locked until the commit, so no other merge slips in
    // between this check and the new mainline versions.
    let candidates: Vec<MergeCandidate> = sqlx::query_as(
        "SELECT d.mainline_version_id::text AS mainline_version_id, \
                dr.base_version_id::text AS base_version_id \
         FROM docs d \
         LEFT JOIN drafts dr ON dr.workspace_id = d.workspace_id AND dr.doc_id = d.id \
                            AND dr.changeset_id = $2::uuid \
         WHERE d.id = ANY($1::text[]::uuid[]) ORDER BY d.id FOR NO KEY UPDATE OF d",
    )
    .bind(&docs)
    .bind(&changeset_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let outdated = candidates
        .iter()
        .any(|c| c.base_version_id != c.mainline_version_id);
    let to = if outdated {
        "needs_rebase"
    } else {
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
    };
    move_status(
        &mut tx,
        workspace_id,
        &project_id,
        &changeset_id,
        "approved",
        to,
        "system",
    )
    .await?;

    let row: ChangesetRow = sqlx::query_as(&format!(
        "SELECT {CHANGESET_COLUMNS} FROM changesets WHERE id = $1::uuid"
    ))
    .bind(&changeset_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let history = history_of(&mut tx, &changeset_id).await?;
    let approved = approved_of(&mut tx, &changeset_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok(Json(ChangesetResponse {
        changeset: row.into_body(history, approved),
    }))
}

/// Sets the status of a change set whose row the transaction has locked and
/// records the move.
async fn move_status(
    tx: &mut Transaction<'static, Postgres>,
    workspace_id: &str,
    project_id: &str,
    changeset_id: &str,
    from: &str,
    to: &str,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE changesets SET status = $2 WHERE id = $1::uuid")
        .bind(changeset_id)
        .bind(to)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::database)?;
    let change = StatusChange {
        project_id,
        changeset_id,
        from: Some(from),
        to,
        actor,
        reason: None,
    };
    insert_history(tx, workspace_id, change).await
}

/// Reopens a rejected change set so the agent can rework it.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reopen",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
    ),
    responses(
        (status = 200, description = "The change set, `open` again", body = ChangesetResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
        (status = 409, description = "The change set is not `rejected`", body = ErrorBody),
    )
)]
pub async fn reopen(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
) -> Result<Json<ChangesetResponse>, ApiError> {
    transition(
        &state,
        &request.workspace_id,
        &project_id,
        &changeset_id,
        Action::Reopen,
        "agent",
        None,
    )
    .await
}

/// Abandons a change set. It is marked, never deleted.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/abandon",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
    ),
    responses(
        (status = 200, description = "The change set, now `abandoned`", body = ChangesetResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
        (status = 409, description = "The change set is already abandoned", body = ErrorBody),
    )
)]
pub async fn abandon(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
) -> Result<Json<ChangesetResponse>, ApiError> {
    transition(
        &state,
        &request.workspace_id,
        &project_id,
        &changeset_id,
        Action::Abandon,
        "agent",
        None,
    )
    .await
}
