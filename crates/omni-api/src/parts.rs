//! Parts: the apps and services of a project. A part has a name that is
//! unique in its project ignoring case among the parts that are not deleted,
//! a kind (`app` or `service`) and a description. Any member creates, renames
//! and deletes parts, with a session or an API key. A delete is soft
//! (`deleted_at`); the name can be used again.

use crate::AppState;
use crate::docs::Tx;
use crate::live;
use crate::projects::{non_blank, parse_body, require_project};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct PartBody {
    pub id: String,
    pub project_id: String,
    /// Unique in the project ignoring case; the omni CLI accepts it where it
    /// takes a part.
    pub name: String,
    /// `app` or `service`.
    pub kind: String,
    pub description: String,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct PartResponse {
    pub part: PartBody,
}

#[derive(Serialize, ToSchema)]
pub struct PartListResponse {
    pub parts: Vec<PartBody>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreatePartRequest {
    pub name: Option<String>,
    /// `app` or `service`.
    pub kind: Option<String>,
    /// Defaults to the empty text.
    pub description: Option<String>,
}

const PART_COLUMNS: &str = concat!(
    "id::text, project_id::text, name, kind, description, to_char(created_at AT TIME ZONE 'UTC', ",
    "'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at"
);

#[derive(Serialize, ToSchema)]
pub struct InvalidKindBody {
    /// `invalid_kind`
    pub error: String,
    /// `kind must be app or service`
    pub message: String,
}

/// 422 `invalid_kind` for a kind that is missing or not `app` or `service`.
fn invalid_kind() -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(InvalidKindBody {
            error: "invalid_kind".to_string(),
            message: "kind must be app or service".to_string(),
        }),
    )
        .into_response()
}

/// Creates a part in a project.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
    ),
    request_body = CreatePartRequest,
    responses(
        (status = 201, description = "The new part", body = PartResponse),
        (status = 400, description = "The name is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
        (status = 409, description = "The project has a part of this name, ignoring case (`name_taken`)", body = ErrorBody),
        (status = 422, description = "A kind that is missing or not `app` or `service` (`invalid_kind`); `message` says so", body = InvalidKindBody),
    )
)]
pub async fn create_part(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreatePartRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let new = new?;
    let name = non_blank(new.name).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let Some(kind) = new.kind.filter(|kind| kind == "app" || kind == "service") else {
        return Ok(invalid_kind());
    };
    let inserted = sqlx::query_as::<_, PartBody>(&format!(
        "INSERT INTO parts (workspace_id, project_id, name, kind, description) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5) RETURNING {PART_COLUMNS}"
    ))
    .bind(&request.workspace_id)
    .bind(&project_id)
    .bind(&name)
    .bind(&kind)
    .bind(new.description.unwrap_or_default())
    .fetch_one(&mut *tx)
    .await;
    let part = match inserted {
        Ok(part) => part,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::conflict("name_taken"));
        }
        Err(e) => return Err(ApiError::database(e)),
    };
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "part.created",
        &json!({ "part_id": part.id, "name": part.name, "kind": part.kind }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(PartResponse { part })).into_response())
}

/// The parts of a project, in the order they were created.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
    ),
    responses(
        (status = 200, description = "The project's parts", body = PartListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn list_parts(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
) -> Result<Json<PartListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let parts = sqlx::query_as(&format!(
        "SELECT {PART_COLUMNS} FROM parts WHERE project_id = $1::uuid \
         AND deleted_at IS NULL ORDER BY created_at, id"
    ))
    .bind(&project_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(PartListResponse { parts }))
}

/// One part of a project.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("part_id" = String, Path, description = "The part's UUID"),
    ),
    responses(
        (status = 200, description = "The part", body = PartResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such part in this project and workspace, or it was deleted", body = ErrorBody),
    )
)]
pub async fn show_part(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, part_id)): Path<(String, String, String)>,
) -> Result<Json<PartResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let part_id = parse_uuid(&part_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let part = sqlx::query_as(&format!(
        "SELECT {PART_COLUMNS} FROM parts \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL"
    ))
    .bind(&part_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(PartResponse { part }))
}

#[derive(Deserialize, ToSchema)]
pub struct RenamePartRequest {
    /// The part's new name; not blank, and not the name of another part of
    /// the project, ignoring case.
    pub name: Option<String>,
}

/// Renames a part. Its ID stays.
#[utoipa::path(
    patch,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("part_id" = String, Path, description = "The part's UUID"),
    ),
    request_body = RenamePartRequest,
    responses(
        (status = 200, description = "The renamed part", body = PartResponse),
        (status = 400, description = "The name is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such part in this project and workspace, or it was deleted", body = ErrorBody),
        (status = 409, description = "The project has a part of this name, ignoring case (`name_taken`)", body = ErrorBody),
    )
)]
pub async fn rename_part(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, part_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<PartResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let part_id = parse_uuid(&part_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<RenamePartRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    lock_part(&mut tx, &project_id, &part_id).await?;
    let name = non_blank(new?.name).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let renamed = sqlx::query_as::<_, PartBody>(&format!(
        "UPDATE parts SET name = $2 WHERE id = $1::uuid RETURNING {PART_COLUMNS}"
    ))
    .bind(&part_id)
    .bind(&name)
    .fetch_one(&mut *tx)
    .await;
    let part = match renamed {
        Ok(part) => part,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::conflict("name_taken"));
        }
        Err(e) => return Err(ApiError::database(e)),
    };
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "part.renamed",
        &json!({ "part_id": part.id, "name": part.name }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(PartResponse { part }))
}

/// Locks a part of the project that is not deleted, 404 when there is none.
async fn lock_part(tx: &mut Tx, project_id: &str, part_id: &str) -> Result<(), ApiError> {
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM parts \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(part_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found.map(|_| ()).ok_or(ApiError::NotFound)
}

/// Deletes a part. Nothing is erased: the row gets `deleted_at` and every
/// list and read leaves it out. The part's name can be used again.
#[utoipa::path(
    delete,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("part_id" = String, Path, description = "The part's UUID"),
    ),
    responses(
        (status = 204, description = "The part is deleted"),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such part in this project and workspace, or it was deleted already", body = ErrorBody),
    )
)]
pub async fn delete_part(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, part_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let part_id = parse_uuid(&part_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    lock_part(&mut tx, &project_id, &part_id).await?;
    sqlx::query("UPDATE parts SET deleted_at = clock_timestamp() WHERE id = $1::uuid")
        .bind(&part_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "part.deleted",
        &json!({ "part_id": part_id }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(StatusCode::NO_CONTENT)
}
