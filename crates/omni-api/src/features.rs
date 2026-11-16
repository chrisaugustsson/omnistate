//! Features: what a part can do. A feature belongs to one part and has a name
//! that is unique in its part ignoring case among the features that are not
//! deleted, and a description. Any member creates, renames and deletes
//! features, with a session or an API key. A delete is soft (`deleted_at`);
//! the name can be used again. The features of a deleted part are hidden with
//! it: its routes answer 404.

use crate::AppState;
use crate::docs::Tx;
use crate::live;
use crate::projects::{non_blank, parse_body};
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
pub struct FeatureBody {
    pub id: String,
    pub project_id: String,
    pub part_id: String,
    /// Unique in the part ignoring case; the omni CLI accepts it where it
    /// takes a feature.
    pub name: String,
    pub description: String,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct FeatureResponse {
    pub feature: FeatureBody,
}

#[derive(Serialize, ToSchema)]
pub struct FeatureListResponse {
    pub features: Vec<FeatureBody>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateFeatureRequest {
    pub name: Option<String>,
    /// Defaults to the empty text.
    pub description: Option<String>,
}

pub(crate) const FEATURE_COLUMNS: &str = concat!(
    "id::text, project_id::text, part_id::text, name, description, ",
    "to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at"
);

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

/// Locks a feature that is not deleted, on a part that is not deleted, of the
/// project, 404 when there is none.
async fn lock_feature(
    tx: &mut Tx,
    project_id: &str,
    part_id: &str,
    feature_id: &str,
) -> Result<(), ApiError> {
    lock_part(tx, project_id, part_id).await?;
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM features \
         WHERE id = $1::uuid AND part_id = $2::uuid AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(feature_id)
    .bind(part_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found.map(|_| ()).ok_or(ApiError::NotFound)
}

/// Creates a feature on a part.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("part_id" = String, Path, description = "The part's UUID"),
    ),
    request_body = CreateFeatureRequest,
    responses(
        (status = 201, description = "The new feature", body = FeatureResponse),
        (status = 400, description = "The name is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such part in this project and workspace, or it was deleted", body = ErrorBody),
        (status = 409, description = "The part has a feature of this name, ignoring case (`name_taken`)", body = ErrorBody),
    )
)]
pub async fn create_feature(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, part_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let part_id = parse_uuid(&part_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateFeatureRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    lock_part(&mut tx, &project_id, &part_id).await?;
    let new = new?;
    let name = non_blank(new.name).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let inserted = sqlx::query_as::<_, FeatureBody>(&format!(
        "INSERT INTO features (workspace_id, project_id, part_id, name, description) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5) RETURNING {FEATURE_COLUMNS}"
    ))
    .bind(&request.workspace_id)
    .bind(&project_id)
    .bind(&part_id)
    .bind(&name)
    .bind(new.description.unwrap_or_default())
    .fetch_one(&mut *tx)
    .await;
    let feature = match inserted {
        Ok(feature) => feature,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::conflict("name_taken"));
        }
        Err(e) => return Err(ApiError::database(e)),
    };
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "feature.created",
        &json!({ "feature_id": feature.id, "part_id": part_id, "name": feature.name }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(FeatureResponse { feature })).into_response())
}

/// The features of a part, in the order they were created.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("part_id" = String, Path, description = "The part's UUID"),
    ),
    responses(
        (status = 200, description = "The part's features", body = FeatureListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such part in this project and workspace, or it was deleted", body = ErrorBody),
    )
)]
pub async fn list_features(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, part_id)): Path<(String, String, String)>,
) -> Result<Json<FeatureListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let part_id = parse_uuid(&part_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let part: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM parts \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL",
    )
    .bind(&part_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    part.ok_or(ApiError::NotFound)?;
    let features = sqlx::query_as(&format!(
        "SELECT {FEATURE_COLUMNS} FROM features WHERE part_id = $1::uuid \
         AND deleted_at IS NULL ORDER BY created_at, id"
    ))
    .bind(&part_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(FeatureListResponse { features }))
}

/// One feature of a part.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features/{feature_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("part_id" = String, Path, description = "The part's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
    ),
    responses(
        (status = 200, description = "The feature", body = FeatureResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature on this part, or it or its part was deleted", body = ErrorBody),
    )
)]
pub async fn show_feature(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, part_id, feature_id)): Path<(String, String, String, String)>,
) -> Result<Json<FeatureResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let part_id = parse_uuid(&part_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let feature = sqlx::query_as(&format!(
        "SELECT {FEATURE_COLUMNS} FROM features \
         WHERE id = $1::uuid AND part_id = $2::uuid AND project_id = $3::uuid \
         AND deleted_at IS NULL \
         AND EXISTS (SELECT 1 FROM parts WHERE parts.id = features.part_id AND parts.deleted_at IS NULL)"
    ))
    .bind(&feature_id)
    .bind(&part_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(FeatureResponse { feature }))
}

#[derive(Deserialize, ToSchema)]
pub struct RenameFeatureRequest {
    /// The feature's new name; not blank, and not the name of another
    /// feature of the part, ignoring case.
    pub name: Option<String>,
}

/// Renames a feature. Its ID stays.
#[utoipa::path(
    patch,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features/{feature_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("part_id" = String, Path, description = "The part's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
    ),
    request_body = RenameFeatureRequest,
    responses(
        (status = 200, description = "The renamed feature", body = FeatureResponse),
        (status = 400, description = "The name is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature on this part, or it or its part was deleted", body = ErrorBody),
        (status = 409, description = "The part has a feature of this name, ignoring case (`name_taken`)", body = ErrorBody),
    )
)]
pub async fn rename_feature(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, part_id, feature_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<FeatureResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let part_id = parse_uuid(&part_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<RenameFeatureRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    lock_feature(&mut tx, &project_id, &part_id, &feature_id).await?;
    let name = non_blank(new?.name).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let renamed = sqlx::query_as::<_, FeatureBody>(&format!(
        "UPDATE features SET name = $2 WHERE id = $1::uuid RETURNING {FEATURE_COLUMNS}"
    ))
    .bind(&feature_id)
    .bind(&name)
    .fetch_one(&mut *tx)
    .await;
    let feature = match renamed {
        Ok(feature) => feature,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::conflict("name_taken"));
        }
        Err(e) => return Err(ApiError::database(e)),
    };
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "feature.renamed",
        &json!({ "feature_id": feature.id, "part_id": part_id, "name": feature.name }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(FeatureResponse { feature }))
}

/// Deletes a feature. Nothing is erased: the row gets `deleted_at` and every
/// list and read leaves it out. The feature's name can be used again.
#[utoipa::path(
    delete,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features/{feature_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("part_id" = String, Path, description = "The part's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
    ),
    responses(
        (status = 204, description = "The feature is deleted"),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature on this part, or it or its part was deleted already", body = ErrorBody),
    )
)]
pub async fn delete_feature(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, part_id, feature_id)): Path<(String, String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let part_id = parse_uuid(&part_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    lock_feature(&mut tx, &project_id, &part_id, &feature_id).await?;
    sqlx::query("UPDATE features SET deleted_at = clock_timestamp() WHERE id = $1::uuid")
        .bind(&feature_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "feature.deleted",
        &json!({ "feature_id": feature_id, "part_id": part_id }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(StatusCode::NO_CONTENT)
}
