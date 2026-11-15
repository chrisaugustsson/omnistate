//! Tracks (Q51, FR-52). A project holds global docs and tracks; a track
//! is the same thing one level deeper, with its own docs and threads, so
//! several tracks can be worked on at once. A track has a name that is
//! unique in its project ignoring case and a description, and no status (V7).
//! Agents create, rename and delete tracks. A delete is soft (`deleted_at`)
//! and takes the track's docs and threads with it.

use crate::AppState;
use crate::docs::{AgentIdentity, Tx, empty_head, log_change};
use crate::live;
use crate::projects::{non_blank, parse_body, require_project};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct TrackBody {
    pub id: String,
    pub project_id: String,
    /// Unique in the project ignoring case; the omni CLI accepts it where it
    /// takes a track.
    pub name: String,
    pub description: String,
    /// The part of the project the track changes.
    pub part_id: String,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct TrackResponse {
    pub track: TrackBody,
}

#[derive(Serialize, ToSchema)]
pub struct TrackListResponse {
    pub tracks: Vec<TrackBody>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateTrackRequest {
    pub name: Option<String>,
    /// Defaults to the empty text.
    pub description: Option<String>,
    /// The part of the project the track is on: an undeleted part of the same
    /// project. Missing or null is 422 `part_required`; any other value that
    /// does not name one is 422 `invalid_part`.
    pub part_id: Option<serde_json::Value>,
}

pub(crate) const TRACK_COLUMNS: &str = concat!(
    "id::text, project_id::text, name, description, part_id::text, to_char(created_at AT TIME ZONE 'UTC', ",
    "'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at"
);

/// Creates a track in a project.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tracks",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
    ),
    request_body = CreateTrackRequest,
    responses(
        (status = 201, description = "The new track", body = TrackResponse),
        (status = 400, description = "The name is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
        (status = 409, description = "The project has a track of this name, ignoring case", body = ErrorBody),
        (status = 422, description = "`part_required` for a missing or null `part_id`, `invalid_part` for one that is no undeleted part of the project", body = ErrorBody),
    )
)]
pub async fn create_track(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<TrackResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateTrackRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let new = new?;
    let name = non_blank(new.name).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let part_id = require_part(&mut tx, &project_id, new.part_id.as_ref()).await?;
    let inserted = sqlx::query_as::<_, TrackBody>(&format!(
        "INSERT INTO tracks (workspace_id, project_id, name, description, part_id) \
         VALUES ($1::uuid, $2::uuid, $3, $4, $5::uuid) RETURNING {TRACK_COLUMNS}"
    ))
    .bind(&request.workspace_id)
    .bind(&project_id)
    .bind(&name)
    .bind(new.description.unwrap_or_default())
    .bind(&part_id)
    .fetch_one(&mut *tx)
    .await;
    let track = match inserted {
        Ok(track) => track,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::conflict("name_taken"));
        }
        Err(e) => return Err(ApiError::database(e)),
    };
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "track.created",
        &json!({ "track_id": track.id, "name": track.name }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(TrackResponse { track })))
}

/// The ID of the undeleted part of the project that `part_id` names: 422
/// `part_required` when it is missing or null, 422 `invalid_part` when it is
/// not a UUID or names no such part.
async fn require_part(
    tx: &mut Tx,
    project_id: &str,
    part_id: Option<&serde_json::Value>,
) -> Result<String, ApiError> {
    let unprocessable = StatusCode::UNPROCESSABLE_ENTITY;
    let given = match part_id {
        None | Some(serde_json::Value::Null) => {
            return Err(ApiError::Refused(unprocessable, "part_required"));
        }
        Some(given) => given,
    };
    let invalid = || ApiError::Refused(unprocessable, "invalid_part");
    let id = given.as_str().and_then(parse_uuid).ok_or_else(invalid)?;
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM parts \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL",
    )
    .bind(&id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found.map(|(id,)| id).ok_or_else(invalid)
}

/// The tracks of a project, in the order they were created.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tracks",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
    ),
    responses(
        (status = 200, description = "The project's tracks", body = TrackListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn list_tracks(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
) -> Result<Json<TrackListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let tracks = sqlx::query_as(&format!(
        "SELECT {TRACK_COLUMNS} FROM tracks WHERE project_id = $1::uuid \
         AND deleted_at IS NULL ORDER BY created_at, id"
    ))
    .bind(&project_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(TrackListResponse { tracks }))
}

/// One track of a project.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("track_id" = String, Path, description = "The track's UUID"),
    ),
    responses(
        (status = 200, description = "The track", body = TrackResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such track in this project and workspace", body = ErrorBody),
    )
)]
pub async fn show_track(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, track_id)): Path<(String, String, String)>,
) -> Result<Json<TrackResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let track_id = parse_uuid(&track_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let track = sqlx::query_as(&format!(
        "SELECT {TRACK_COLUMNS} FROM tracks \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL"
    ))
    .bind(&track_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(TrackResponse { track }))
}

#[derive(Deserialize, ToSchema)]
pub struct RenameTrackRequest {
    /// The track's new name; not blank, and not the name of another track
    /// of the project, ignoring case.
    pub name: Option<String>,
}

/// Renames a track. Its ID, docs and threads stay.
#[utoipa::path(
    patch,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("track_id" = String, Path, description = "The track's UUID"),
    ),
    request_body = RenameTrackRequest,
    responses(
        (status = 200, description = "The renamed track", body = TrackResponse),
        (status = 400, description = "The name is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such track in this project and workspace, or it was deleted", body = ErrorBody),
        (status = 409, description = "The project has a track of this name, ignoring case (`name_taken`)", body = ErrorBody),
    )
)]
pub async fn rename_track(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, track_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<TrackResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let track_id = parse_uuid(&track_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<RenameTrackRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    lock_track(&mut tx, &project_id, &track_id).await?;
    let name = non_blank(new?.name).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let renamed = sqlx::query_as::<_, TrackBody>(&format!(
        "UPDATE tracks SET name = $2 WHERE id = $1::uuid RETURNING {TRACK_COLUMNS}"
    ))
    .bind(&track_id)
    .bind(&name)
    .fetch_one(&mut *tx)
    .await;
    let track = match renamed {
        Ok(track) => track,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::conflict("name_taken"));
        }
        Err(e) => return Err(ApiError::database(e)),
    };
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "track.renamed",
        &json!({ "track_id": track.id, "name": track.name }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(TrackResponse { track }))
}

#[derive(Deserialize, ToSchema)]
pub struct MoveTrackRequest {
    /// The part to put the track on; same rules as on create.
    pub part_id: Option<serde_json::Value>,
}

/// Puts a track on another part of its project. Moving it to the part it is
/// on changes nothing.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}/move",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("track_id" = String, Path, description = "The track's UUID"),
    ),
    request_body = MoveTrackRequest,
    responses(
        (status = 200, description = "The track on its part", body = TrackResponse),
        (status = 400, description = "The body is not readable", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such track in this project and workspace, or it was deleted", body = ErrorBody),
        (status = 422, description = "`part_required` for a missing or null `part_id`, `invalid_part` for one that is no undeleted part of the project; the track stays where it was", body = ErrorBody),
    )
)]
pub async fn move_track(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, track_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<TrackResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let track_id = parse_uuid(&track_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<MoveTrackRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    lock_track(&mut tx, &project_id, &track_id).await?;
    let part_id = require_part(&mut tx, &project_id, new?.part_id.as_ref()).await?;
    let track = sqlx::query_as::<_, TrackBody>(&format!(
        "UPDATE tracks SET part_id = $2::uuid WHERE id = $1::uuid RETURNING {TRACK_COLUMNS}"
    ))
    .bind(&track_id)
    .bind(&part_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "track.moved",
        &json!({ "track_id": track.id, "part_id": track.part_id }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(TrackResponse { track }))
}

/// Locks a track of the project that is not deleted, 404 when there is none.
async fn lock_track(tx: &mut Tx, project_id: &str, track_id: &str) -> Result<(), ApiError> {
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM tracks \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(track_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found.map(|_| ()).ok_or(ApiError::NotFound)
}

/// Deletes a track, and with it its docs and threads. Nothing is erased: the
/// rows get `deleted_at`, every list and read leaves them out (the comments of
/// a deleted doc with them), and each doc's log gets a row for its deletion.
/// The track's name can be used again.
#[utoipa::path(
    delete,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("track_id" = String, Path, description = "The track's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    responses(
        (status = 204, description = "The track, its docs and its threads are deleted"),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such track in this project and workspace, or it was deleted already", body = ErrorBody),
    )
)]
pub async fn delete_track(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, track_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let track_id = parse_uuid(&track_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    lock_track(&mut tx, &project_id, &track_id).await?;
    let docs: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id::text, head FROM docs \
         WHERE track_id = $1::uuid AND deleted_at IS NULL ORDER BY created_at, id FOR UPDATE",
    )
    .bind(&track_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let agent = identity.to_json();
    for (doc_id, head) in &docs {
        let after = head.clone().unwrap_or_else(empty_head);
        log_change(
            &mut tx,
            &request.workspace_id,
            doc_id,
            &agent,
            &json!([{ "op": "delete" }]),
            head.as_deref(),
            &after,
        )
        .await?;
    }
    for table in ["docs", "threads"] {
        sqlx::query(&format!(
            "UPDATE {table} SET deleted_at = clock_timestamp() \
             WHERE track_id = $1::uuid AND deleted_at IS NULL"
        ))
        .bind(&track_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    }
    sqlx::query("UPDATE tracks SET deleted_at = clock_timestamp() WHERE id = $1::uuid")
        .bind(&track_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "track.deleted",
        &json!({ "track_id": track_id }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(StatusCode::NO_CONTENT)
}
