//! Projects. A project holds the app level and the features of one app; the
//! docs, threads and comments of a scope hang off it. Agents create projects.
//! Nothing is deleted.
//!
//! The helpers that every route of a project shares live here too: reading a
//! JSON body and checking that the project exists.

use crate::AppState;
use crate::docs::Tx;
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
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
pub struct RenameProjectRequest {
    pub name: Option<String>,
}

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

/// Renames a project. Its ID, features, docs and threads stay.
#[utoipa::path(
    patch,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
    ),
    request_body = RenameProjectRequest,
    responses(
        (status = 200, description = "The renamed project", body = ProjectResponse),
        (status = 400, description = "The name is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
        (status = 409, description = "The workspace has another project of this name (`name_taken`)", body = ErrorBody),
    )
)]
pub async fn rename_project(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<ProjectResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<RenameProjectRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let old_name = lock_project(&mut tx, &project_id).await?;
    let name = non_blank(new?.name).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let renamed = sqlx::query_as(concat!(
        "UPDATE projects SET name = $2 WHERE id = $1::uuid RETURNING id::text, name, ",
        timestamp!("created_at"),
        " AS created_at"
    ))
    .bind(&project_id)
    .bind(&name)
    .fetch_one(&mut *tx)
    .await;
    let project: ProjectBody = match renamed {
        Ok(project) => project,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::conflict("name_taken"));
        }
        Err(e) => return Err(ApiError::database(e)),
    };
    let changed = project.name != old_name;
    if changed {
        live::emit(
            &mut tx,
            &request.workspace_id,
            &project_id,
            "project.renamed",
            &json!({ "project_id": project.id, "name": project.name }),
        )
        .await?;
    }
    tx.commit().await.map_err(ApiError::database)?;
    if changed {
        state.live.announce(&request.workspace_id, &project_id);
    }
    Ok(Json(ProjectResponse { project }))
}

/// Locks a project of the workspace and returns its name, 404 when there is none.
async fn lock_project(tx: &mut Tx, project_id: &str) -> Result<String, ApiError> {
    let found: Option<(String,)> =
        sqlx::query_as("SELECT name FROM projects WHERE id = $1::uuid FOR UPDATE")
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(ApiError::database)?;
    found.map(|(name,)| name).ok_or(ApiError::NotFound)
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
