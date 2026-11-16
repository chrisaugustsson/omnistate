//! Decisions (D7, D16). A decision is an append-only record on a track: the
//! question, what was decided, why, who recorded it and when. It may be about
//! one feature of the track's project, and it may replace an earlier decision
//! of the project, which stays as it was. There is no edit and no delete.
//! The track's owner records decisions, like every other write under a track
//! (`require_track_writer`); everyone reads them.
//!
//! Recording emits `decision.recorded`.

use crate::AppState;
use crate::comments::author_of;
use crate::docs::{AgentIdentity, Tx};
use crate::live;
use crate::projects::{non_blank, parse_body};
use crate::session::{ApiError, ErrorBody};
use crate::tracks::require_track_writer;
use crate::workspace::{WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct DecisionBody {
    pub id: String,
    pub project_id: String,
    pub track_id: String,
    /// The feature the decision is about; null when it is about none.
    pub feature_id: Option<String>,
    pub question: String,
    pub decision: String,
    pub why: String,
    /// `{"kind": "human", "user_id"}` or `{"kind": "agent", "run", "role",
    /// "model", "harness"}`.
    #[schema(value_type = Object)]
    pub author: Value,
    /// The earlier decision this one replaces; null when it replaces none.
    pub replaces_decision_id: Option<String>,
    /// The latest decision that replaces this one; null while none does.
    pub replaced_by_decision_id: Option<String>,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct DecisionRow {
    id: String,
    project_id: String,
    track_id: String,
    feature_id: Option<String>,
    question: String,
    decision: String,
    why: String,
    author: String,
    replaces_decision_id: Option<String>,
    replaced_by_decision_id: Option<String>,
    created_at: String,
}

impl From<DecisionRow> for DecisionBody {
    fn from(row: DecisionRow) -> Self {
        Self {
            id: row.id,
            project_id: row.project_id,
            track_id: row.track_id,
            feature_id: row.feature_id,
            question: row.question,
            decision: row.decision,
            why: row.why,
            author: serde_json::from_str(&row.author).unwrap_or(Value::Null),
            replaces_decision_id: row.replaces_decision_id,
            replaced_by_decision_id: row.replaced_by_decision_id,
            created_at: row.created_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct DecisionResponse {
    pub decision: DecisionBody,
}

#[derive(Serialize, ToSchema)]
pub struct DecisionListResponse {
    pub decisions: Vec<DecisionBody>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateDecisionRequest {
    /// What was asked. Not blank.
    pub question: Option<String>,
    /// What was decided. Not blank.
    pub decision: Option<String>,
    /// Why. Not blank.
    pub why: Option<String>,
    /// A feature of the track's project the decision is about. Another
    /// project's feature is 422 `feature_not_in_project`, an unknown ID 404.
    /// Without it, a decision that replaces another takes that one's feature.
    #[schema(value_type = Option<String>)]
    pub feature_id: Option<Value>,
    /// A decision of the same project that this one replaces. Another
    /// project's decision is 422 `replaces_not_in_project`, an unknown ID 404.
    #[schema(value_type = Option<String>)]
    pub replaces_decision_id: Option<Value>,
}

const DECISION_COLUMNS: &str = concat!(
    "d.id::text, d.project_id::text, d.track_id::text, d.feature_id::text, d.question, ",
    "d.decision, d.why, d.author::text AS author, d.replaces_decision_id::text, ",
    "(SELECT r.id::text FROM decisions r WHERE r.replaces_decision_id = d.id \
      ORDER BY r.created_at DESC, r.id DESC LIMIT 1) AS replaced_by_decision_id, ",
    "to_char(d.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at"
);

/// Checks that a track of the project exists and is not deleted, 404 when
/// not; `lock` holds the track's row for the rest of the transaction.
async fn find_track(
    tx: &mut Tx,
    project_id: &str,
    track_id: &str,
    lock: bool,
) -> Result<(), ApiError> {
    let found: Option<(String,)> = sqlx::query_as(&format!(
        "SELECT id::text FROM tracks \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL{}",
        if lock { " FOR UPDATE" } else { "" }
    ))
    .bind(track_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found.map(|_| ()).ok_or(ApiError::NotFound)
}

/// The id a body field names, `None` when it is missing or null; a value that
/// is no UUID names nothing: 404.
fn named_id(given: Option<Value>) -> Result<Option<String>, ApiError> {
    match given {
        None | Some(Value::Null) => Ok(None),
        Some(given) => given
            .as_str()
            .and_then(parse_uuid)
            .map(Some)
            .ok_or(ApiError::NotFound),
    }
}

/// Records a decision on a track.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}/decisions",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("track_id" = String, Path, description = "The track's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = CreateDecisionRequest,
    responses(
        (status = 201, description = "The new decision", body = DecisionResponse),
        (status = 400, description = "The question, the decision or the why is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "`not_track_owner`: the caller does not own the track", body = ErrorBody),
        (status = 404, description = "No such track in this project and workspace, or no such feature or decision", body = ErrorBody),
        (status = 422, description = "`feature_not_in_project` or `replaces_not_in_project`: the feature or the decision is of another project", body = ErrorBody),
    )
)]
pub async fn create_decision(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, track_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<DecisionResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let track_id = parse_uuid(&track_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateDecisionRequest>(&body);
    let author = author_of(&request.actor, &identity);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    find_track(&mut tx, &project_id, &track_id, true).await?;
    require_track_writer(&mut tx, &request.who(), &track_id).await?;
    let new = new?;
    let (Some(question), Some(decision), Some(why)) = (
        non_blank(new.question),
        non_blank(new.decision),
        non_blank(new.why),
    ) else {
        return Err(ApiError::bad_request("invalid_request"));
    };
    let unprocessable = StatusCode::UNPROCESSABLE_ENTITY;
    let mut feature_id = named_id(new.feature_id)?;
    if let Some(feature) = &feature_id {
        let found: Option<(String,)> = sqlx::query_as(
            "SELECT project_id::text FROM features WHERE id = $1::uuid AND deleted_at IS NULL",
        )
        .bind(feature)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        let (of_project,) = found.ok_or(ApiError::NotFound)?;
        if of_project != project_id {
            return Err(ApiError::Refused(unprocessable, "feature_not_in_project"));
        }
    }
    let replaces = named_id(new.replaces_decision_id)?;
    if let Some(replaces) = &replaces {
        let found: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT project_id::text, feature_id::text FROM decisions WHERE id = $1::uuid",
        )
        .bind(replaces)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        let (of_project, of_feature) = found.ok_or(ApiError::NotFound)?;
        if of_project != project_id {
            return Err(ApiError::Refused(unprocessable, "replaces_not_in_project"));
        }
        if feature_id.is_none() {
            feature_id = of_feature;
        }
    }
    let id: String = sqlx::query_scalar(
        "INSERT INTO decisions (workspace_id, project_id, track_id, feature_id, question, \
         decision, why, author, replaces_decision_id) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, $6, $7, $8::text::jsonb, $9::uuid) \
         RETURNING id::text",
    )
    .bind(&request.workspace_id)
    .bind(&project_id)
    .bind(&track_id)
    .bind(&feature_id)
    .bind(&question)
    .bind(&decision)
    .bind(&why)
    .bind(author.to_string())
    .bind(&replaces)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let recorded: DecisionRow = sqlx::query_as(&format!(
        "SELECT {DECISION_COLUMNS} FROM decisions d WHERE d.id = $1::uuid"
    ))
    .bind(&id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "decision.recorded",
        &json!({ "decision_id": id, "track_id": track_id, "feature_id": feature_id }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok((
        StatusCode::CREATED,
        Json(DecisionResponse {
            decision: recorded.into(),
        }),
    ))
}

/// The decisions of a track, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}/decisions",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("track_id" = String, Path, description = "The track's UUID"),
    ),
    responses(
        (status = 200, description = "The track's decisions, newest first", body = DecisionListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such track in this project and workspace, or it was deleted", body = ErrorBody),
    )
)]
pub async fn list_track_decisions(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, track_id)): Path<(String, String, String)>,
) -> Result<Json<DecisionListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let track_id = parse_uuid(&track_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    find_track(&mut tx, &project_id, &track_id, false).await?;
    let rows: Vec<DecisionRow> = sqlx::query_as(&format!(
        "SELECT {DECISION_COLUMNS} FROM decisions d WHERE d.track_id = $1::uuid \
         ORDER BY d.created_at DESC, d.id DESC"
    ))
    .bind(&track_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(DecisionListResponse {
        decisions: rows.into_iter().map(DecisionBody::from).collect(),
    }))
}

/// The decisions about a feature, newest first, from every track.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features/{feature_id}/decisions",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("part_id" = String, Path, description = "The part's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
    ),
    responses(
        (status = 200, description = "The feature's decisions, newest first", body = DecisionListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature on this part, or it or its part was deleted", body = ErrorBody),
    )
)]
pub async fn list_feature_decisions(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, part_id, feature_id)): Path<(String, String, String, String)>,
) -> Result<Json<DecisionListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let part_id = parse_uuid(&part_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT f.id::text FROM features f JOIN parts p ON p.id = f.part_id \
         WHERE f.id = $1::uuid AND f.part_id = $2::uuid AND f.project_id = $3::uuid \
         AND f.deleted_at IS NULL AND p.deleted_at IS NULL",
    )
    .bind(&feature_id)
    .bind(&part_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    found.ok_or(ApiError::NotFound)?;
    let rows: Vec<DecisionRow> = sqlx::query_as(&format!(
        "SELECT {DECISION_COLUMNS} FROM decisions d WHERE d.feature_id = $1::uuid \
         ORDER BY d.created_at DESC, d.id DESC"
    ))
    .bind(&feature_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(DecisionListResponse {
        decisions: rows.into_iter().map(DecisionBody::from).collect(),
    }))
}
