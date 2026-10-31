//! Stories. A story is a work item of a feature: its criteria, the stories
//! that block it, the files it changes, a priority and a status. The key a
//! person reads is `US-<number>`, counted per project. Every write adds 1 to
//! the story's `version`. Creating a story emits `story.created`.

use crate::AppState;
use crate::live;
use crate::projects::{parse_body, require_project};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use utoipa::ToSchema;

const STATUSES: [&str; 6] = [
    "backlog",
    "implementing",
    "review",
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
    /// `backlog`, `implementing`, `review`, `testing`, `done` or `cancelled`.
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
