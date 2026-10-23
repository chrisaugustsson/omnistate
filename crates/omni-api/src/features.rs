//! Features (Q51, FR-52). A project holds global docs and features; a feature
//! is the same thing one level deeper, with its own docs, change sets and
//! threads, so several features can be worked on at once. A feature has a name
//! that is unique in its project ignoring case, a description and a status,
//! `active` until the human marks it done. Agents create features. Features
//! are never deleted (FR-11).

use crate::AppState;
use crate::changesets::{non_blank, parse_body, require_project};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, HumanRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct FeatureBody {
    pub id: String,
    pub project_id: String,
    /// Unique in the project ignoring case; the omni CLI accepts it where it
    /// takes a feature.
    pub name: String,
    pub description: String,
    /// `active` or `done`.
    pub status: String,
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

const FEATURE_COLUMNS: &str = concat!(
    "id::text, project_id::text, name, description, status, to_char(created_at AT TIME ZONE 'UTC', ",
    "'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at"
);

/// Creates a feature, `active`, in a project.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
    ),
    request_body = CreateFeatureRequest,
    responses(
        (status = 201, description = "The new feature", body = FeatureResponse),
        (status = 400, description = "The name is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
        (status = 409, description = "The project has a feature of this name, ignoring case", body = ErrorBody),
    )
)]
pub async fn create_feature(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<FeatureResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateFeatureRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let new = new?;
    let name = non_blank(new.name).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let inserted = sqlx::query_as::<_, FeatureBody>(&format!(
        "INSERT INTO features (workspace_id, project_id, name, description) \
         VALUES ($1::uuid, $2::uuid, $3, $4) RETURNING {FEATURE_COLUMNS}"
    ))
    .bind(&request.workspace_id)
    .bind(&project_id)
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
        &json!({ "feature_id": feature.id, "name": feature.name }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(FeatureResponse { feature })))
}

/// The features of a project, in the order they were created.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
    ),
    responses(
        (status = 200, description = "The project's features, done ones included", body = FeatureListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn list_features(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
) -> Result<Json<FeatureListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let features = sqlx::query_as(&format!(
        "SELECT {FEATURE_COLUMNS} FROM features WHERE project_id = $1::uuid \
         ORDER BY created_at, id"
    ))
    .bind(&project_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(FeatureListResponse { features }))
}

/// One feature of a project.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
    ),
    responses(
        (status = 200, description = "The feature", body = FeatureResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace", body = ErrorBody),
    )
)]
pub async fn show_feature(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
) -> Result<Json<FeatureResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let feature = sqlx::query_as(&format!(
        "SELECT {FEATURE_COLUMNS} FROM features WHERE id = $1::uuid AND project_id = $2::uuid"
    ))
    .bind(&feature_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(FeatureResponse { feature }))
}

/// Which way a feature's status moves: the status it must have, the status it
/// gets, the conflict code when it has the other one, and the live event.
struct Move {
    from: &'static str,
    to: &'static str,
    refused: &'static str,
    event: &'static str,
}

const DONE: Move = Move {
    from: "active",
    to: "done",
    refused: "already_done",
    event: "feature.done",
};

const REOPEN: Move = Move {
    from: "done",
    to: "active",
    refused: "not_done",
    event: "feature.reopened",
};

async fn set_status(
    state: AppState,
    request: HumanRequest,
    (project_id, feature_id): (String, String),
    step: Move,
) -> Result<Json<FeatureResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let current: FeatureBody = sqlx::query_as(&format!(
        "SELECT {FEATURE_COLUMNS} FROM features WHERE id = $1::uuid AND project_id = $2::uuid \
         FOR UPDATE"
    ))
    .bind(&feature_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    if current.status != step.from {
        return Err(ApiError::conflict(step.refused));
    }
    let feature: FeatureBody = sqlx::query_as(&format!(
        "UPDATE features SET status = $2 WHERE id = $1::uuid RETURNING {FEATURE_COLUMNS}"
    ))
    .bind(&feature_id)
    .bind(step.to)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        step.event,
        &json!({ "feature_id": feature.id }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(FeatureResponse { feature }))
}

/// Marks an active feature done. Only the human does this. A done feature
/// takes no new change sets (FR-55); its docs, versions, threads and comments
/// stay readable, and change sets already open in it keep their lifecycle.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/done",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
    ),
    responses(
        (status = 200, description = "The feature, now done", body = FeatureResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; the human only", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace", body = ErrorBody),
        (status = 409, description = "The feature is already done (`already_done`)", body = ErrorBody),
    )
)]
pub async fn done(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
) -> Result<Json<FeatureResponse>, ApiError> {
    set_status(state, request, (project_id, feature_id), DONE).await
}

/// Sets a done feature back to active so it takes change sets again. Only the
/// human does this.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/reopen",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
    ),
    responses(
        (status = 200, description = "The feature, active again", body = FeatureResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; the human only", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace", body = ErrorBody),
        (status = 409, description = "The feature is not done (`not_done`)", body = ErrorBody),
    )
)]
pub async fn reopen(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
) -> Result<Json<FeatureResponse>, ApiError> {
    set_status(state, request, (project_id, feature_id), REOPEN).await
}
