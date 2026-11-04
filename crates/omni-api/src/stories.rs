//! Stories. A story is a work item of a feature: its criteria, the stories
//! that block it, the files it changes, a priority and a status. The key a
//! person reads is `US-<number>`, counted per project. Every write adds 1 to
//! the story's `version`. Creating a story emits `story.created`, editing one
//! `story.changed` and cancelling one `story.cancelled`.

use crate::AppState;
use crate::live;
use crate::projects::{parse_body, require_project};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, HumanRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use utoipa::ToSchema;

const STATUSES: [&str; 8] = [
    "backlog",
    "implementing",
    "implemented",
    "review",
    "reviewed",
    "testing",
    "done",
    "cancelled",
];

#[derive(Serialize, ToSchema)]
pub struct CriterionBody {
    pub text: String,
    pub ticked: bool,
    /// Whether a human has to check the criterion.
    pub human_check: bool,
    /// Whether the human check is still to be done.
    pub pending: bool,
}

#[derive(Serialize, ToSchema)]
pub struct StoryBody {
    pub id: String,
    pub project_id: String,
    pub feature_id: String,
    /// Counted per project.
    pub number: i32,
    /// `US-<number>`.
    pub key: String,
    pub title: String,
    pub text: String,
    pub topic: String,
    pub criteria: Vec<CriterionBody>,
    /// IDs of the stories of the feature that block this one.
    pub blocked_by: Vec<String>,
    pub files_planned: Vec<String>,
    pub files_changed: Vec<String>,
    /// 0 to 3; 1 is the highest and 0 is reserved for fix stories.
    pub priority: i32,
    /// `backlog`, `implementing`, `implemented`, `review`, `reviewed`, `testing`, `done` or `cancelled`.
    pub status: String,
    pub blocked: bool,
    /// `{run, stage, at}` while a run holds the story, otherwise `null`.
    #[schema(value_type = Option<Object>)]
    pub holder: Option<Value>,
    #[schema(value_type = Vec<Object>)]
    pub findings: Vec<Value>,
    #[schema(value_type = Vec<Object>)]
    pub decisions: Vec<Value>,
    /// `{doc_id, node_id, label}` or `{thread_id, label}`.
    #[schema(value_type = Vec<Object>)]
    pub links: Vec<Value>,
    /// 1 on creation, plus 1 on every write.
    pub version: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct StoryResponse {
    pub story: StoryBody,
}

#[derive(Serialize, ToSchema)]
pub struct StoryListResponse {
    /// The feature's stories, by priority and then number.
    pub stories: Vec<StoryBody>,
}

#[derive(Deserialize, ToSchema)]
pub struct NewCriterion {
    pub text: String,
    /// Defaults to `false`.
    pub human_check: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateStoryRequest {
    /// Not blank.
    pub title: String,
    pub text: String,
    pub topic: String,
    pub criteria: Vec<NewCriterion>,
    /// IDs of stories of the same feature. Defaults to none.
    pub blocked_by: Option<Vec<String>>,
    /// Defaults to none.
    pub files_planned: Option<Vec<String>>,
    /// 0 to 3. Defaults to 3.
    pub priority: Option<i64>,
    /// `{doc_id, node_id, label}` or `{thread_id, label}`, stored as given.
    /// Defaults to none.
    #[schema(value_type = Option<Vec<Object>>)]
    pub links: Option<Vec<Value>>,
    /// Honoured only when the project has no story yet: the first story gets
    /// the number `number_from + 1`. Not negative.
    pub number_from: Option<i64>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
    pickable: Option<String>,
}

#[derive(sqlx::FromRow)]
struct StoryRow {
    id: String,
    project_id: String,
    feature_id: String,
    number: i32,
    title: String,
    text: String,
    topic: String,
    criteria: String,
    blocked_by: String,
    files_planned: String,
    files_changed: String,
    priority: i32,
    status: String,
    blocked: bool,
    holder: Option<String>,
    findings: String,
    decisions: String,
    links: String,
    version: i32,
    created_at: String,
    updated_at: String,
}

const STORY_COLUMNS: &str = concat!(
    "id::text, project_id::text, feature_id::text, number, title, text, topic, ",
    "criteria::text, blocked_by::text, files_planned::text, files_changed::text, priority, ",
    "status, blocked, holder::text, findings::text, decisions::text, links::text, version, ",
    "to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at, ",
    "to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at"
);

fn list_of<T: serde::de::DeserializeOwned>(text: &str) -> Vec<T> {
    serde_json::from_str(text).unwrap_or_default()
}

impl StoryRow {
    fn into_body(self) -> StoryBody {
        let criteria: Vec<Value> = list_of(&self.criteria);
        let flag = |c: &Value, name: &str| c[name].as_bool().unwrap_or(false);
        StoryBody {
            key: format!("US-{}", self.number),
            criteria: criteria
                .iter()
                .map(|c| CriterionBody {
                    text: c["text"].as_str().unwrap_or_default().to_string(),
                    ticked: flag(c, "ticked"),
                    human_check: flag(c, "human_check"),
                    pending: flag(c, "pending"),
                })
                .collect(),
            blocked_by: list_of(&self.blocked_by),
            files_planned: list_of(&self.files_planned),
            files_changed: list_of(&self.files_changed),
            holder: self
                .holder
                .and_then(|holder| serde_json::from_str(&holder).ok()),
            findings: list_of(&self.findings),
            decisions: list_of(&self.decisions),
            links: list_of(&self.links),
            id: self.id,
            project_id: self.project_id,
            feature_id: self.feature_id,
            number: self.number,
            title: self.title,
            text: self.text,
            topic: self.topic,
            priority: self.priority,
            status: self.status,
            blocked: self.blocked,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// 422 with a stable code that names the field.
fn invalid_field(code: &'static str) -> ApiError {
    ApiError::Refused(StatusCode::UNPROCESSABLE_ENTITY, code)
}

/// Whether a link has one of the two shapes: `{doc_id, node_id, label}` or
/// `{thread_id, label}`, each a text.
fn link_fits(link: &Value) -> bool {
    let has_text = |name: &str| link[name].is_string();
    link.is_object()
        && has_text("label")
        && ((has_text("doc_id") && has_text("node_id")) || has_text("thread_id"))
}

/// A feature of the project that is not deleted, 404 when there is none.
async fn require_feature(
    tx: &mut Transaction<'static, Postgres>,
    project_id: &str,
    feature_id: &str,
) -> Result<(), ApiError> {
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM features \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL FOR SHARE",
    )
    .bind(feature_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found.map(|_| ()).ok_or(ApiError::NotFound)
}

/// Creates a story in a feature.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
    ),
    request_body = CreateStoryRequest,
    responses(
        (status = 201, description = "The new story", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace", body = ErrorBody),
        (status = 422, description = "A blank title (`invalid_title`), a priority outside 0 to 3 (`invalid_priority`), a link of neither shape (`invalid_links`), a `blocked_by` id that is not a story of the feature (`invalid_blocked_by`) or a negative `number_from` (`invalid_number_from`)", body = ErrorBody),
    )
)]
pub async fn create_story(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<StoryResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateStoryRequest>(&body);
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    // The project row is locked so that two stories do not get one number.
    let locked: Option<(String,)> =
        sqlx::query_as("SELECT id::text FROM projects WHERE id = $1::uuid FOR UPDATE")
            .bind(&project_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(ApiError::database)?;
    locked.ok_or(ApiError::NotFound)?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let new = new?;

    let title = new.title.trim().to_string();
    if title.is_empty() {
        return Err(invalid_field("invalid_title"));
    }
    let priority = new.priority.unwrap_or(3);
    if !(0..=3).contains(&priority) {
        return Err(invalid_field("invalid_priority"));
    }
    let links = new.links.unwrap_or_default();
    if !links.iter().all(link_fits) {
        return Err(invalid_field("invalid_links"));
    }
    if new.number_from.is_some_and(|from| from < 0) {
        return Err(invalid_field("invalid_number_from"));
    }
    let mut blocked_by: Vec<String> = Vec::new();
    for id in new.blocked_by.unwrap_or_default() {
        let id = parse_uuid(&id).ok_or_else(|| invalid_field("invalid_blocked_by"))?;
        if !blocked_by.contains(&id) {
            blocked_by.push(id);
        }
    }
    if !blocked_by.is_empty() {
        let (found,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM stories WHERE feature_id = $1::uuid AND id::text = ANY($2)",
        )
        .bind(&feature_id)
        .bind(&blocked_by)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        if found != blocked_by.len() as i64 {
            return Err(invalid_field("invalid_blocked_by"));
        }
    }

    let (highest,): (Option<i32>,) =
        sqlx::query_as("SELECT max(number) FROM stories WHERE project_id = $1::uuid")
            .bind(&project_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::database)?;
    let number = match highest {
        Some(highest) => i64::from(highest) + 1,
        None => new.number_from.unwrap_or(0) + 1,
    };
    let number = i32::try_from(number).map_err(|_| invalid_field("invalid_number_from"))?;

    let criteria: Vec<Value> = new
        .criteria
        .iter()
        .map(|c| {
            json!({
                "text": c.text,
                "ticked": false,
                "human_check": c.human_check.unwrap_or(false),
                "pending": false,
            })
        })
        .collect();
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "INSERT INTO stories (workspace_id, project_id, feature_id, number, title, text, topic, \
         criteria, blocked_by, files_planned, priority, links) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, $7, $8::jsonb, $9::jsonb, $10::jsonb, \
         $11, $12::jsonb) RETURNING {STORY_COLUMNS}"
    ))
    .bind(workspace_id)
    .bind(&project_id)
    .bind(&feature_id)
    .bind(number)
    .bind(&title)
    .bind(&new.text)
    .bind(&new.topic)
    .bind(Value::from(criteria).to_string())
    .bind(json!(blocked_by).to_string())
    .bind(json!(new.files_planned.unwrap_or_default()).to_string())
    .bind(priority as i32)
    .bind(Value::from(links).to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let story = row.into_body();
    live::emit(
        &mut tx,
        workspace_id,
        &project_id,
        "story.created",
        &json!({ "story_id": story.id, "key": story.key, "feature_id": story.feature_id }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(StoryResponse { story })))
}

/// The stories of a feature, by priority and then number.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("status" = Option<String>, Query, description = "Only the stories with this status; every status, `cancelled` included, when left out"),
        ("pickable" = Option<String>, Query, description = "`true` lists only the backlog stories a run can pick now: not blocked, every blocker done or cancelled, and no file shared with a story in implementing or review (`files_changed` when not empty, else `files_planned`). Ordered by priority, then by open dependants (most first), then number. `false` or absent gives the normal list"),
    ),
    responses(
        (status = 200, description = "The feature's stories", body = StoryListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace", body = ErrorBody),
        (status = 422, description = "An unknown status (`invalid_status`) or a `pickable` other than `true` or `false` (`invalid_pickable`)", body = ErrorBody),
    )
)]
pub async fn list_stories(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
    Query(query): Query<ListQuery>,
) -> Result<Json<StoryListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    if let Some(status) = &query.status
        && !STATUSES.contains(&status.as_str())
    {
        return Err(invalid_field("invalid_status"));
    }
    let pickable = match query.pickable.as_deref() {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err(invalid_field("invalid_pickable")),
    };
    // The pickable rule looks at the whole feature, so the status filter is
    // applied afterwards then.
    let status = if pickable { None } else { query.status.clone() };
    let rows = sqlx::query_as::<_, StoryRow>(&format!(
        "SELECT {STORY_COLUMNS} FROM stories \
         WHERE feature_id = $1::uuid AND ($2::text IS NULL OR status = $2) \
         ORDER BY priority, number"
    ))
    .bind(&feature_id)
    .bind(&status)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    let mut stories: Vec<StoryBody> = rows.into_iter().map(StoryRow::into_body).collect();
    if pickable {
        stories = pickable_stories(stories, query.status.as_deref());
    }
    Ok(Json(StoryListResponse { stories }))
}

/// The files a story occupies: what it changed, else what it plans to change.
fn story_files(story: &StoryBody) -> &[String] {
    if story.files_changed.is_empty() {
        &story.files_planned
    } else {
        &story.files_changed
    }
}

/// The backlog stories that can be worked now, in picking order: priority,
/// then open dependants (most first), then number. `stories` is every story of
/// the feature, already ordered by priority and number.
fn pickable_stories(stories: Vec<StoryBody>, status: Option<&str>) -> Vec<StoryBody> {
    if status.is_some_and(|status| status != "backlog") {
        return Vec::new();
    }
    let closed = |story: &StoryBody| story.status == "done" || story.status == "cancelled";
    let busy: Vec<&String> = stories
        .iter()
        .filter(|story| story.status == "implementing" || story.status == "review")
        .flat_map(|story| story_files(story).iter())
        .collect();
    let dependants = |story: &StoryBody| {
        stories
            .iter()
            .filter(|other| !closed(other) && other.blocked_by.contains(&story.id))
            .count()
    };
    let mut picked: Vec<(usize, usize)> = stories
        .iter()
        .enumerate()
        .filter(|(_, story)| {
            story.status == "backlog"
                && !story.blocked
                && story.blocked_by.iter().all(|id| {
                    stories
                        .iter()
                        .find(|other| &other.id == id)
                        .is_some_and(closed)
                })
                && !story_files(story).iter().any(|path| busy.contains(&path))
        })
        .map(|(index, story)| (dependants(story), index))
        .collect();
    picked.sort_by_key(|(open, index)| {
        let story = &stories[*index];
        (story.priority, std::cmp::Reverse(*open), story.number)
    });
    let order: Vec<usize> = picked.into_iter().map(|(_, index)| index).collect();
    let mut slots: Vec<Option<StoryBody>> = stories.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect()
}

/// One story of a feature.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    responses(
        (status = 200, description = "The story", body = StoryResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
    )
)]
pub async fn show_story(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
) -> Result<Json<StoryResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "SELECT {STORY_COLUMNS} FROM stories \
         WHERE id = $1::uuid AND feature_id = $2::uuid AND project_id = $3::uuid"
    ))
    .bind(&story_id)
    .bind(&feature_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(StoryResponse {
        story: row.into_body(),
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct PatchStoryRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// Backlog only. Not blank.
    pub title: Option<String>,
    /// Backlog only.
    pub text: Option<String>,
    /// Backlog only.
    pub topic: Option<String>,
    /// Backlog only. Replaces the list; every criterion starts unticked.
    pub criteria: Option<Vec<NewCriterion>>,
    /// IDs of other stories of the same feature. Any open status.
    pub blocked_by: Option<Vec<String>>,
    /// Backlog only.
    pub files_planned: Option<Vec<String>>,
    /// 0 to 3. Any open status.
    pub priority: Option<i64>,
    /// `{doc_id, node_id, label}` or `{thread_id, label}`. Any open status.
    #[schema(value_type = Option<Vec<Object>>)]
    pub links: Option<Vec<Value>>,
}

#[derive(Deserialize, ToSchema)]
pub struct CancelStoryRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
}

#[derive(Serialize, ToSchema)]
pub struct StoryConflictBody {
    /// `conflict`
    pub error: String,
    /// The version the story has now.
    pub current_version: i32,
}

/// An error of a story write: the usual ones, or a stale `expected_version`.
pub enum StoryError {
    Api(ApiError),
    Stale(i32),
    Held(Value),
    NotWaiting { status: String, stage: String },
    CriteriaOpen(Vec<usize>),
}

impl From<ApiError> for StoryError {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for StoryError {
    fn into_response(self) -> Response {
        match self {
            Self::Api(error) => error.into_response(),
            Self::Stale(current_version) => (
                StatusCode::CONFLICT,
                Json(json!({ "error": "conflict", "current_version": current_version })),
            )
                .into_response(),
            Self::Held(holder) => (
                StatusCode::CONFLICT,
                Json(json!({ "error": "held", "holder": holder })),
            )
                .into_response(),
            Self::NotWaiting { status, stage } => (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "not_waiting",
                    "status": status,
                    "stage": stage,
                    "message": format!("a story in {status} cannot be claimed for {stage}"),
                })),
            )
                .into_response(),
            Self::CriteriaOpen(open) => (
                StatusCode::CONFLICT,
                Json(json!({ "error": "criteria_open", "open": open })),
            )
                .into_response(),
        }
    }
}

/// The story with a lock on its row, 404 when the feature has none.
async fn lock_story(
    tx: &mut Transaction<'static, Postgres>,
    project_id: &str,
    feature_id: &str,
    story_id: &str,
) -> Result<StoryRow, ApiError> {
    sqlx::query_as::<_, StoryRow>(&format!(
        "SELECT {STORY_COLUMNS} FROM stories \
         WHERE id = $1::uuid AND feature_id = $2::uuid AND project_id = $3::uuid FOR UPDATE"
    ))
    .bind(story_id)
    .bind(feature_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)
}

fn is_closed(status: &str) -> bool {
    status == "done" || status == "cancelled"
}

/// Changes a story's text, priority, blockers or links.
#[utoipa::path(
    patch,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = PatchStoryRequest,
    responses(
        (status = 200, description = "The story with its new `version`", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; checked before the rest. Else `not_in_backlog` (a text field of a story outside backlog) or `not_open` (any field of a done or cancelled story); the request is refused whole", body = StoryConflictBody),
        (status = 422, description = "No field but `expected_version` (`no_changes`), a blank title (`invalid_title`), a priority outside 0 to 3 (`invalid_priority`), links of neither shape (`invalid_links`) or a `blocked_by` that is not a list of other stories of the feature (`invalid_blocked_by`)", body = ErrorBody),
    )
)]
pub async fn patch_story(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let patch = parse_body::<PatchStoryRequest>(&body)?;
    let mut fields: Vec<&str> = Vec::new();
    let mut restricted = false;
    for (name, present, in_backlog) in [
        ("title", patch.title.is_some(), true),
        ("text", patch.text.is_some(), true),
        ("topic", patch.topic.is_some(), true),
        ("criteria", patch.criteria.is_some(), true),
        ("blocked_by", patch.blocked_by.is_some(), false),
        ("files_planned", patch.files_planned.is_some(), true),
        ("priority", patch.priority.is_some(), false),
        ("links", patch.links.is_some(), false),
    ] {
        if present {
            fields.push(name);
            restricted |= in_backlog;
        }
    }
    if fields.is_empty() {
        return Err(invalid_field("no_changes").into());
    }
    let title = patch.title.as_ref().map(|title| title.trim().to_string());
    if title.as_ref().is_some_and(|title| title.is_empty()) {
        return Err(invalid_field("invalid_title").into());
    }
    if patch
        .priority
        .is_some_and(|priority| !(0..=3).contains(&priority))
    {
        return Err(invalid_field("invalid_priority").into());
    }
    if patch
        .links
        .as_ref()
        .is_some_and(|links| !links.iter().all(link_fits))
    {
        return Err(invalid_field("invalid_links").into());
    }
    let blocked_by = match &patch.blocked_by {
        None => None,
        Some(ids) => {
            let mut parsed: Vec<String> = Vec::new();
            for id in ids {
                let id = parse_uuid(id).ok_or_else(|| invalid_field("invalid_blocked_by"))?;
                if id == story_id {
                    return Err(invalid_field("invalid_blocked_by").into());
                }
                if !parsed.contains(&id) {
                    parsed.push(id);
                }
            }
            Some(parsed)
        }
    };

    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let current = lock_story(&mut tx, &project_id, &feature_id, &story_id).await?;
    if let Some(ids) = blocked_by.as_ref().filter(|ids| !ids.is_empty()) {
        let (found,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM stories WHERE feature_id = $1::uuid AND id::text = ANY($2)",
        )
        .bind(&feature_id)
        .bind(ids)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        if found != ids.len() as i64 {
            return Err(invalid_field("invalid_blocked_by").into());
        }
    }
    if i64::from(current.version) != patch.expected_version {
        return Err(StoryError::Stale(current.version));
    }
    if is_closed(&current.status) {
        return Err(ApiError::conflict("not_open").into());
    }
    if restricted && current.status != "backlog" {
        return Err(ApiError::conflict("not_in_backlog").into());
    }

    let criteria = patch.criteria.as_ref().map(|criteria| {
        Value::from(
            criteria
                .iter()
                .map(|c| {
                    json!({
                        "text": c.text,
                        "ticked": false,
                        "human_check": c.human_check.unwrap_or(false),
                        "pending": false,
                    })
                })
                .collect::<Vec<Value>>(),
        )
        .to_string()
    });
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET \
         title = COALESCE($2, title), text = COALESCE($3, text), topic = COALESCE($4, topic), \
         criteria = COALESCE($5::jsonb, criteria), blocked_by = COALESCE($6::jsonb, blocked_by), \
         files_planned = COALESCE($7::jsonb, files_planned), priority = COALESCE($8, priority), \
         links = COALESCE($9::jsonb, links), version = version + 1, updated_at = clock_timestamp() \
         WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .bind(&title)
    .bind(&patch.text)
    .bind(&patch.topic)
    .bind(&criteria)
    .bind(blocked_by.map(|ids| json!(ids).to_string()))
    .bind(patch.files_planned.map(|files| json!(files).to_string()))
    .bind(patch.priority.map(|priority| priority as i32))
    .bind(patch.links.map(|links| Value::from(links).to_string()))
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let story = row.into_body();
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "story.changed",
        &json!({ "story_id": story.id, "key": story.key, "fields": fields }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(StoryResponse { story }))
}

/// Cancels a story: it leaves the board and no longer blocks other stories.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/cancel",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = CancelStoryRequest,
    responses(
        (status = 200, description = "The cancelled story, its holder cleared", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale, or `not_open` when the story is done or cancelled", body = StoryConflictBody),
    )
)]
pub async fn cancel_story(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let cancel = parse_body::<CancelStoryRequest>(&body)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let current = lock_story(&mut tx, &project_id, &feature_id, &story_id).await?;
    if i64::from(current.version) != cancel.expected_version {
        return Err(StoryError::Stale(current.version));
    }
    if is_closed(&current.status) {
        return Err(ApiError::conflict("not_open").into());
    }
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET status = 'cancelled', holder = NULL, version = version + 1, \
         updated_at = clock_timestamp() WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let story = row.into_body();
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "story.cancelled",
        &json!({ "story_id": story.id, "key": story.key }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(StoryResponse { story }))
}

#[derive(Deserialize, ToSchema)]
pub struct ClaimStoryRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The run that takes the story.
    pub run: String,
    /// `implementing`, `review` or `testing`.
    pub stage: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ReleaseStoryRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The run that releases; any run may release, so it is only named.
    #[allow(dead_code)]
    pub run: String,
}

#[derive(Deserialize, ToSchema)]
pub struct MoveStoryRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The run that holds the story.
    pub run: String,
    /// `backlog`, `implementing`, `implemented`, `review`, `reviewed`, `testing` or `done`.
    pub to: String,
}

#[derive(Serialize, ToSchema)]
pub struct StoryHeldBody {
    /// `conflict`, `not_open`, `held`, `not_waiting`, `not_holder` or `criteria_open`
    pub error: String,
    /// The version the story has now, with `conflict`.
    pub current_version: Option<i32>,
    /// The holder, with `held`.
    #[schema(value_type = Option<Object>)]
    pub holder: Option<Value>,
    /// The story's status, with `not_waiting`.
    pub status: Option<String>,
    /// The stage asked for, with `not_waiting`.
    pub stage: Option<String>,
    /// What went wrong, with `not_waiting`.
    pub message: Option<String>,
    /// The 0-based indexes of the unticked criteria, with `criteria_open`.
    pub open: Option<Vec<usize>>,
}

/// The run that holds the story, if any.
fn holder_run(story: &StoryBody) -> Option<&str> {
    story.holder.as_ref()?.get("run")?.as_str()
}

async fn finish_story_write(
    state: &AppState,
    mut tx: Transaction<'static, Postgres>,
    request: &AgentRequest,
    project_id: &str,
    row: StoryRow,
    fields: &[&str],
) -> Result<Json<StoryResponse>, StoryError> {
    let story = row.into_body();
    live::emit(
        &mut tx,
        &request.workspace_id,
        project_id,
        "story.changed",
        &json!({ "story_id": story.id, "key": story.key, "fields": fields }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, project_id);
    Ok(Json(StoryResponse { story }))
}

/// Takes a story for a run, or moves the run's claim to another stage.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/claim",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = ClaimStoryRequest,
    responses(
        (status = 200, description = "The story, held by the run, its status the stage", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "A session, not an agent key", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; else `not_open` (done or cancelled) or `held` with the `holder` when another run holds it; else `not_waiting` with the story's `status` and the `stage` when the story is not in the waiting state of that stage (backlog for implementing, implemented for review, reviewed for testing)", body = StoryHeldBody),
        (status = 422, description = "A stage other than implementing, review or testing (`invalid_stage`)", body = ErrorBody),
    )
)]
pub async fn claim_story(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let claim = parse_body::<ClaimStoryRequest>(&body)?;
    if !["implementing", "review", "testing"].contains(&claim.stage.as_str()) {
        return Err(invalid_field("invalid_stage").into());
    }
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let current = lock_story(&mut tx, &project_id, &feature_id, &story_id).await?;
    if i64::from(current.version) != claim.expected_version {
        return Err(StoryError::Stale(current.version));
    }
    if is_closed(&current.status) {
        return Err(ApiError::conflict("not_open").into());
    }
    let current = current.into_body();
    if let Some(holder) = &current.holder
        && holder_run(&current) != Some(claim.run.as_str())
    {
        return Err(StoryError::Held(holder.clone()));
    }
    let waiting = match claim.stage.as_str() {
        "implementing" => "backlog",
        "review" => "implemented",
        _ => "reviewed",
    };
    if current.status != waiting {
        return Err(StoryError::NotWaiting {
            status: current.status.clone(),
            stage: claim.stage.clone(),
        });
    }
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET status = $2, holder = jsonb_build_object('run', $3::text, \
         'stage', $2::text, 'at', to_char(clock_timestamp() AT TIME ZONE 'UTC', \
         'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')), version = version + 1, \
         updated_at = clock_timestamp() WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .bind(&claim.stage)
    .bind(&claim.run)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    finish_story_write(
        &state,
        tx,
        &request,
        &project_id,
        row,
        &["status", "holder"],
    )
    .await
}

/// Clears a story's holder, whoever holds it; a working story returns to its waiting state.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/release",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = ReleaseStoryRequest,
    responses(
        (status = 200, description = "The story without a holder; a story in implementing, review or testing returns to its waiting state (backlog, implemented, reviewed), one already waiting or closed keeps its status", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "A session, not an agent key", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale", body = StoryConflictBody),
    )
)]
pub async fn release_story(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let release = parse_body::<ReleaseStoryRequest>(&body)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let current = lock_story(&mut tx, &project_id, &feature_id, &story_id).await?;
    if i64::from(current.version) != release.expected_version {
        return Err(StoryError::Stale(current.version));
    }
    let working = matches!(
        current.status.as_str(),
        "implementing" | "review" | "testing"
    );
    if current.holder.is_none() && !working {
        return Ok(Json(StoryResponse {
            story: current.into_body(),
        }));
    }
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET holder = NULL, status = CASE status WHEN 'implementing' THEN 'backlog' WHEN 'review' THEN 'implemented' \
         WHEN 'testing' THEN 'reviewed' ELSE status END, version = version + 1, \
         updated_at = clock_timestamp() WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    finish_story_write(
        &state,
        tx,
        &request,
        &project_id,
        row,
        &["status", "holder"],
    )
    .await
}

/// Moves a story to a column; only the run that holds it may.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/move",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = MoveStoryRequest,
    responses(
        (status = 200, description = "The story in its new column; the holder's stage follows, and `done` or `backlog` clear the holder", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "A session, not an agent key", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; else `not_open`, `not_holder` (the run does not hold the story) or `criteria_open` with the 0-based indexes in `open` when moving to done", body = StoryHeldBody),
        (status = 422, description = "A column other than backlog, implementing, implemented, review, reviewed, testing or done (`invalid_status`)", body = ErrorBody),
    )
)]
pub async fn move_story(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let step = parse_body::<MoveStoryRequest>(&body)?;
    if ![
        "backlog",
        "implementing",
        "implemented",
        "review",
        "reviewed",
        "testing",
        "done",
    ]
    .contains(&step.to.as_str())
    {
        return Err(invalid_field("invalid_status").into());
    }
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let current = lock_story(&mut tx, &project_id, &feature_id, &story_id).await?;
    if i64::from(current.version) != step.expected_version {
        return Err(StoryError::Stale(current.version));
    }
    if is_closed(&current.status) {
        return Err(ApiError::conflict("not_open").into());
    }
    let current = current.into_body();
    if holder_run(&current) != Some(step.run.as_str()) {
        return Err(ApiError::conflict("not_holder").into());
    }
    if step.to == "done" {
        let open: Vec<usize> = current
            .criteria
            .iter()
            .enumerate()
            .filter(|(_, criterion)| !criterion.ticked)
            .map(|(index, _)| index)
            .collect();
        if !open.is_empty() {
            return Err(StoryError::CriteriaOpen(open));
        }
    }
    let keeps_holder = ["implementing", "review", "testing"].contains(&step.to.as_str());
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET status = $2, \
         holder = CASE WHEN $3 THEN jsonb_set(holder, '{{stage}}', to_jsonb($2::text)) END, \
         version = version + 1, updated_at = clock_timestamp() \
         WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .bind(&step.to)
    .bind(keeps_holder)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    finish_story_write(
        &state,
        tx,
        &request,
        &project_id,
        row,
        &["status", "holder"],
    )
    .await
}

#[derive(Deserialize, ToSchema)]
pub struct TickRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The run that holds the story.
    pub run: String,
    /// The 0-based index of the criterion.
    pub criterion: i64,
    pub ticked: bool,
    /// Whether the human check is still to be done; left as it is when absent.
    pub pending: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct FindingRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The run that holds the story.
    pub run: String,
    /// `implementing`, `review` or `testing`.
    pub stage: String,
    pub text: String,
    pub files: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct ResolveFindingRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The run that holds the story.
    pub run: String,
}

#[derive(Deserialize, ToSchema)]
pub struct DecisionRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The run that holds the story; any run when nothing holds it.
    pub run: String,
    pub question: String,
    pub choice: String,
    /// `agent` or `human`.
    pub by: String,
}

#[derive(Deserialize, ToSchema)]
pub struct FilesRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The run that holds the story.
    pub run: String,
    /// Replaces the files the story changed.
    pub files_changed: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct BlockStoryRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The run that blocks or unblocks; any run may, so it is only named.
    #[allow(dead_code)]
    pub run: String,
}

#[derive(Deserialize, ToSchema)]
pub struct CheckCriterionRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The 0-based index of the criterion.
    pub criterion: i64,
}

/// Opens a write of a run on a story: the story with a lock, once the version
/// is current and, when a `run` is given, once the run is the holder's or
/// nothing holds the story.
async fn open_run_write(
    state: &AppState,
    workspace_id: &str,
    ids: [&str; 3],
    expected_version: i64,
    run: Option<&str>,
) -> Result<(Transaction<'static, Postgres>, StoryBody), StoryError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    let current = lock_story(&mut tx, ids[0], ids[1], ids[2]).await?;
    if i64::from(current.version) != expected_version {
        return Err(StoryError::Stale(current.version));
    }
    let current = current.into_body();
    if let Some(run) = run
        && current.holder.is_some()
        && holder_run(&current) != Some(run)
    {
        return Err(ApiError::conflict("not_holder").into());
    }
    Ok((tx, current))
}

fn criterion_index(criterion: i64, story: &StoryBody) -> Result<usize, StoryError> {
    usize::try_from(criterion)
        .ok()
        .filter(|index| *index < story.criteria.len())
        .ok_or_else(|| invalid_field("invalid_criterion").into())
}

/// Ticks or unticks a criterion, and sets whether its human check is pending.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/tick",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = TickRequest,
    responses(
        (status = 200, description = "The story with the criterion ticked", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "A session, not an agent key", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; else `not_holder` when the story has a holder and it is not `run`", body = StoryConflictBody),
        (status = 422, description = "A criterion index outside the story's criteria (`invalid_criterion`)", body = ErrorBody),
    )
)]
pub async fn tick_criterion(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let write = parse_body::<TickRequest>(&body)?;
    let (mut tx, current) = open_run_write(
        &state,
        &request.workspace_id,
        [&project_id, &feature_id, &story_id],
        write.expected_version,
        Some(&write.run),
    )
    .await?;
    let index = criterion_index(write.criterion, &current)?;
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET criteria = jsonb_set(CASE WHEN $3::bool IS NULL THEN criteria \
         ELSE jsonb_set(criteria, ARRAY[$2::text, 'pending'], to_jsonb($3::bool)) END, \
         ARRAY[$2::text, 'ticked'], to_jsonb($4::bool)), version = version + 1, \
         updated_at = clock_timestamp() WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .bind(index.to_string())
    .bind(write.pending)
    .bind(write.ticked)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    finish_story_write(&state, tx, &request, &project_id, row, &["criteria"]).await
}

/// Appends a finding to a story.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/findings",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = FindingRequest,
    responses(
        (status = 200, description = "The story with the finding appended, not resolved", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "A session, not an agent key", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; else `not_holder` when the story has a holder and it is not `run`", body = StoryConflictBody),
        (status = 422, description = "A stage other than implementing, review or testing (`invalid_stage`)", body = ErrorBody),
    )
)]
pub async fn add_finding(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let write = parse_body::<FindingRequest>(&body)?;
    if !["implementing", "review", "testing"].contains(&write.stage.as_str()) {
        return Err(invalid_field("invalid_stage").into());
    }
    let (mut tx, _) = open_run_write(
        &state,
        &request.workspace_id,
        [&project_id, &feature_id, &story_id],
        write.expected_version,
        Some(&write.run),
    )
    .await?;
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET findings = findings || jsonb_build_array(jsonb_build_object(\
         'stage', $2::text, 'text', $3::text, 'files', $4::text::jsonb, 'resolved', false)), \
         version = version + 1, updated_at = clock_timestamp() \
         WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .bind(&write.stage)
    .bind(&write.text)
    .bind(json!(write.files).to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    finish_story_write(&state, tx, &request, &project_id, row, &["findings"]).await
}

/// Marks a finding of a story resolved.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/findings/{index}/resolve",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
        ("index" = usize, Path, description = "The 0-based index of the finding"),
    ),
    request_body = ResolveFindingRequest,
    responses(
        (status = 200, description = "The story with the finding resolved", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "A session, not an agent key", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace, or no such finding", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; else `not_holder` when the story has a holder and it is not `run`", body = StoryConflictBody),
    )
)]
pub async fn resolve_finding(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, feature_id, story_id, index)): Path<(
        String,
        String,
        String,
        String,
        usize,
    )>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let write = parse_body::<ResolveFindingRequest>(&body)?;
    let (mut tx, current) = open_run_write(
        &state,
        &request.workspace_id,
        [&project_id, &feature_id, &story_id],
        write.expected_version,
        Some(&write.run),
    )
    .await?;
    if index >= current.findings.len() {
        return Err(ApiError::NotFound.into());
    }
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET findings = jsonb_set(findings, ARRAY[$2::text, 'resolved'], 'true'), \
         version = version + 1, updated_at = clock_timestamp() \
         WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .bind(index.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    finish_story_write(&state, tx, &request, &project_id, row, &["findings"]).await
}

/// Appends a decision to a story.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/decisions",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = DecisionRequest,
    responses(
        (status = 200, description = "The story with the decision appended", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "A session, not an agent key", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; else `not_holder` when the story has a holder and it is not `run`", body = StoryConflictBody),
        (status = 422, description = "`by` other than agent or human (`invalid_by`)", body = ErrorBody),
    )
)]
pub async fn add_decision(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let write = parse_body::<DecisionRequest>(&body)?;
    if !["agent", "human"].contains(&write.by.as_str()) {
        return Err(invalid_field("invalid_by").into());
    }
    let (mut tx, _) = open_run_write(
        &state,
        &request.workspace_id,
        [&project_id, &feature_id, &story_id],
        write.expected_version,
        Some(&write.run),
    )
    .await?;
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET decisions = decisions || jsonb_build_array(jsonb_build_object(\
         'question', $2::text, 'choice', $3::text, 'by', $4::text)), \
         version = version + 1, updated_at = clock_timestamp() \
         WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .bind(&write.question)
    .bind(&write.choice)
    .bind(&write.by)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    finish_story_write(&state, tx, &request, &project_id, row, &["decisions"]).await
}

/// Replaces the files a story changed.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/files",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = FilesRequest,
    responses(
        (status = 200, description = "The story with its changed files replaced", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "A session, not an agent key", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; else `not_holder` when the story has a holder and it is not `run`", body = StoryConflictBody),
    )
)]
pub async fn set_files(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let write = parse_body::<FilesRequest>(&body)?;
    let (mut tx, _) = open_run_write(
        &state,
        &request.workspace_id,
        [&project_id, &feature_id, &story_id],
        write.expected_version,
        Some(&write.run),
    )
    .await?;
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET files_changed = $2::text::jsonb, version = version + 1, \
         updated_at = clock_timestamp() WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .bind(json!(write.files_changed).to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    finish_story_write(&state, tx, &request, &project_id, row, &["files_changed"]).await
}

/// Blocks a story on a question to the human.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/block",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = BlockStoryRequest,
    responses(
        (status = 200, description = "The story, blocked; a story in implementing, review or testing returns to its waiting state (backlog, implemented, reviewed) with no holder, one already waiting keeps its status", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "A session, not an agent key", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; else `not_holder` when the story has a holder and it is not `run`", body = StoryConflictBody),
    )
)]
pub async fn block_story(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let write = parse_body::<BlockStoryRequest>(&body)?;
    let (mut tx, _) = open_run_write(
        &state,
        &request.workspace_id,
        [&project_id, &feature_id, &story_id],
        write.expected_version,
        None,
    )
    .await?;
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET blocked = true, holder = NULL, status = CASE status WHEN 'implementing' THEN 'backlog' WHEN 'review' THEN 'implemented' \
         WHEN 'testing' THEN 'reviewed' ELSE status END, version = version + 1, \
         updated_at = clock_timestamp() WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    finish_story_write(
        &state,
        tx,
        &request,
        &project_id,
        row,
        &["blocked", "status", "holder"],
    )
    .await
}

/// Lifts the block of a story.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/unblock",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = BlockStoryRequest,
    responses(
        (status = 200, description = "The story, not blocked", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key", body = ErrorBody),
        (status = 403, description = "A session, not an agent key", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; else `not_holder` when the story has a holder and it is not `run`", body = StoryConflictBody),
    )
)]
pub async fn unblock_story(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let write = parse_body::<BlockStoryRequest>(&body)?;
    let (mut tx, _) = open_run_write(
        &state,
        &request.workspace_id,
        [&project_id, &feature_id, &story_id],
        write.expected_version,
        None,
    )
    .await?;
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET blocked = false, version = version + 1, \
         updated_at = clock_timestamp() WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    finish_story_write(&state, tx, &request, &project_id, row, &["blocked"]).await
}

/// Clears the pending human check of a criterion, in any status.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories/{story_id}/check",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("story_id" = String, Path, description = "The story's UUID"),
    ),
    request_body = CheckCriterionRequest,
    responses(
        (status = 200, description = "The story with the criterion's human check cleared", body = StoryResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "An agent key, not a session", body = ErrorBody),
        (status = 404, description = "No such story in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale", body = StoryConflictBody),
        (status = 422, description = "A criterion index outside the story's criteria (`invalid_criterion`)", body = ErrorBody),
    )
)]
pub async fn check_criterion(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, feature_id, story_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<StoryResponse>, StoryError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let story_id = parse_uuid(&story_id).ok_or(ApiError::NotFound)?;
    let write = parse_body::<CheckCriterionRequest>(&body)?;
    let (mut tx, current) = open_run_write(
        &state,
        &request.workspace_id,
        [&project_id, &feature_id, &story_id],
        write.expected_version,
        None,
    )
    .await?;
    let index = criterion_index(write.criterion, &current)?;
    let row = sqlx::query_as::<_, StoryRow>(&format!(
        "UPDATE stories SET criteria = jsonb_set(criteria, ARRAY[$2::text, 'pending'], 'false'), \
         version = version + 1, updated_at = clock_timestamp() \
         WHERE id = $1::uuid RETURNING {STORY_COLUMNS}"
    ))
    .bind(&story_id)
    .bind(index.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let request = AgentRequest {
        workspace_id: request.workspace_id,
    };
    finish_story_write(&state, tx, &request, &project_id, row, &["criteria"]).await
}
