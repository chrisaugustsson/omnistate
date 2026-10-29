//! The project home (US-040): the reviewer's page of one project in a single
//! read. It lists the project's docs, change sets and threads, and what waits
//! for the human: the decisions to approve, the change sets in review, the
//! comments whose latest reply is an agent's, and the agents' questions that no
//! post of the human has answered yet.
//!
//! A feature (Q51) is a level below the project: the home lists the features
//! with their counts, and every item that waits for the human carries the
//! `feature_id` of its change set (a comment's of its doc), so the web app can
//! show the project and each feature on their own while `docs`, `changesets`
//! and `threads` stay complete.
//!
//! The scope pages (US-112) show the app level and each feature with their last
//! activity, each doc with when and by which agent role it was last written
//! and how many of its comments wait, and each thread with its post count and
//! the time of its newest post.
//!
//! Every item is the body that the API's own read of it answers, so the web
//! app shows one shape per entity. The route is the human's: a sealed phase
//! stays sealed for agents, and the home lists questions of every phase. The
//! page reloads the home when the project's live events come; the changes that
//! move an item onto or off a list emit those events already.

use crate::AppState;
use crate::changesets::{ChangesetBody, Scope, changesets_of_project, require_project};
use crate::claims;
use crate::comments::{CommentBody, finish, waiting_of_project};
use crate::decisions::{DecisionBody, proposed_of_project};
use crate::docs::{DocBody, Tx, docs_of_project};
use crate::features::{FEATURE_COLUMNS, FeatureBody};
use crate::session::{ApiError, ErrorBody};
use crate::threads::{PostBody, ThreadBody, Viewer, threads_of_project, unanswered_questions};
use crate::workspace::{HumanRequest, parse_uuid};
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;

/// What waits for the human, each list oldest first.
#[derive(Serialize, ToSchema)]
pub struct WaitingBody {
    /// The decisions whose status is `proposed`.
    pub decisions: Vec<DecisionBody>,
    /// The change sets whose status is `in_review`.
    pub changesets: Vec<ChangesetBody>,
    /// The comments whose latest reply is an agent's.
    pub comments: Vec<CommentBody>,
    /// The `question` posts of agents that no post of the human links to as a
    /// reply, in every phase.
    pub questions: Vec<PostBody>,
}

/// A feature of the project with what it holds.
#[derive(Serialize, ToSchema)]
pub struct FeatureSummary {
    pub feature: FeatureBody,
    /// The docs of the feature; the global docs that its change sets draft are
    /// not counted.
    pub docs: i64,
    /// The feature's change sets that are open, in review, approved or need a
    /// rebase.
    pub open_changesets: i64,
    /// The items of `waiting` that belong to the feature.
    pub waiting: i64,
    /// The time of the latest write to one of the feature's docs (a row of
    /// `doc_changes`: create, edit, rename or delete) or its threads (a
    /// thread's creation, a post or a conclusion); comments and replies do not
    /// count. `null` while there is none. The API always sends it.
    #[schema(required = false)]
    pub last_activity: Option<String>,
}

/// The app level of the project.
#[derive(Serialize, ToSchema)]
pub struct AppSummary {
    /// The time of the latest write to an app-level doc or thread, counted as
    /// a feature's `last_activity` is; `null` while there is none.
    pub last_activity: Option<String>,
}

/// A doc of the project with what the scope pages show of it.
#[derive(Serialize, ToSchema)]
pub struct HomeDoc {
    #[serde(flatten)]
    pub doc: DocBody,
    /// When the doc was last written: the time of its newest row of
    /// `doc_changes`, or its creation for a doc that has none. The API always
    /// sends it.
    #[schema(required = false)]
    pub updated_at: String,
    /// The role of the agent that made that write; empty when it declared none.
    #[schema(required = false)]
    pub last_writer_role: String,
    /// The doc's comments whose latest reply is an agent's.
    #[schema(required = false)]
    pub waiting: i64,
}

/// A thread of the project with what the scope pages show of its posts.
#[derive(Serialize, ToSchema)]
pub struct HomeThread {
    #[serde(flatten)]
    pub thread: ThreadBody,
    /// How many posts the thread has.
    #[schema(required = false)]
    pub post_count: i64,
    /// When its newest post was made; `null` for a thread without posts.
    #[schema(required = false)]
    pub last_post_at: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct HomeResponse {
    pub project_id: String,
    /// Every feature of the project that is not deleted, oldest first. The
    /// API always sends it; it is not marked required so that the web app's
    /// reader still accepts a home from before features.
    #[schema(required = false)]
    pub features: Vec<FeatureSummary>,
    /// The app level. The API always sends it.
    #[schema(required = false)]
    pub app: Option<AppSummary>,
    /// Every doc of the project, oldest first, as `GET /docs/{doc_id}` answers it.
    pub docs: Vec<HomeDoc>,
    /// Every change set of the project, oldest first, with its status.
    pub changesets: Vec<ChangesetBody>,
    /// Every thread of the project, oldest first, without its posts.
    pub threads: Vec<HomeThread>,
    pub waiting: WaitingBody,
}

/// The project's home: its docs, change sets and threads, and what waits for
/// the human. Only the human reads it.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/home",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
    ),
    responses(
        (status = 200, description = "The project's home", body = HomeResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn show(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id)): Path<(String, String)>,
) -> Result<Json<HomeResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    claims::sweep(&state, &request.workspace_id).await;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let features: Vec<FeatureBody> = sqlx::query_as(&format!(
        "SELECT {FEATURE_COLUMNS} FROM features WHERE project_id = $1::uuid \
         AND deleted_at IS NULL ORDER BY created_at, id"
    ))
    .bind(&project_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let docs = docs_of_project(
        &state,
        &mut tx,
        &request.workspace_id,
        &project_id,
        Scope::Project,
        None,
    )
    .await?;
    let changesets = changesets_of_project(&mut tx, &Viewer::human(), &project_id).await?;
    let threads = threads_of_project(&mut tx, &Viewer::human(), &project_id).await?;
    let decisions = proposed_of_project(&mut tx, &Viewer::human(), &project_id).await?;
    let waiting_comments = waiting_of_project(&mut tx, &project_id).await?;
    let questions = unanswered_questions(&mut tx, &project_id).await?;
    let written = last_writes(&mut tx, &project_id).await?;
    let activity = last_activity(&mut tx, &project_id).await?;
    let post_stats = post_stats(&mut tx, &project_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let comments = finish(&state, &request.workspace_id, &project_id, waiting_comments).await?;
    let in_review: Vec<ChangesetBody> = changesets
        .iter()
        .filter(|changeset| changeset.status == "in_review")
        .cloned()
        .collect();
    let features = features
        .into_iter()
        .map(|feature| {
            let own = Some(feature.id.as_str());
            let open =
                |status: &str| matches!(status, "open" | "in_review" | "approved" | "needs_rebase");
            let waiting = decisions
                .iter()
                .filter(|d| d.feature_id.as_deref() == own)
                .count()
                + in_review
                    .iter()
                    .filter(|c| c.feature_id.as_deref() == own)
                    .count()
                + comments
                    .iter()
                    .filter(|c| c.feature_id.as_deref() == own)
                    .count()
                + questions
                    .iter()
                    .filter(|q| q.feature_id.as_deref() == own)
                    .count();
            FeatureSummary {
                docs: docs
                    .iter()
                    .filter(|d| d.feature_id.as_deref() == own)
                    .count() as i64,
                open_changesets: changesets
                    .iter()
                    .filter(|c| c.feature_id.as_deref() == own && open(&c.status))
                    .count() as i64,
                waiting: waiting as i64,
                last_activity: activity.get(Some(feature.id.as_str())),
                feature,
            }
        })
        .collect();
    let docs = docs
        .into_iter()
        .map(|doc| {
            let (updated_at, last_writer_role) = written
                .get(&doc.id)
                .cloned()
                .unwrap_or_else(|| (doc.created_at.clone(), String::new()));
            let waiting = comments.iter().filter(|c| c.doc_id == doc.id).count() as i64;
            HomeDoc {
                doc,
                updated_at,
                last_writer_role,
                waiting,
            }
        })
        .collect();
    let threads = threads
        .into_iter()
        .map(|thread| {
            let (post_count, last_post_at) =
                post_stats.get(&thread.id).cloned().unwrap_or_default();
            HomeThread {
                thread,
                post_count,
                last_post_at,
            }
        })
        .collect();
    Ok(Json(HomeResponse {
        project_id,
        features,
        app: Some(AppSummary {
            last_activity: activity.get(None),
        }),
        docs,
        changesets,
        threads,
        waiting: WaitingBody {
            decisions,
            changesets: in_review,
            comments,
            questions,
        },
    }))
}

/// The time and the agent's role of the newest row of `doc_changes` of each
/// doc of the project, by doc ID.
async fn last_writes(
    tx: &mut Tx,
    project_id: &str,
) -> Result<HashMap<String, (String, String)>, ApiError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(&format!(
        "SELECT DISTINCT ON (c.doc_id) c.doc_id::text, {AT}, COALESCE(c.agent->>'role', '') \
         FROM doc_changes c JOIN docs d ON d.workspace_id = c.workspace_id AND d.id = c.doc_id \
         WHERE d.project_id = $1::uuid ORDER BY c.doc_id, c.seq DESC"
    ))
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(rows
        .into_iter()
        .map(|(doc, at, role)| (doc, (at, role)))
        .collect())
}

/// The latest write of each scope of the project: a row of `doc_changes` of
/// one of the scope's docs (a deleted doc's too), and a thread's creation,
/// post or conclusion.
struct Activity(HashMap<Option<String>, String>);

impl Activity {
    fn get(&self, feature: Option<&str>) -> Option<String> {
        self.0.get(&feature.map(str::to_string)).cloned()
    }
}

/// A timestamp column `at` as ISO 8601 text in UTC (the format of the bodies).
const AT: &str = "to_char(c.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')";

async fn last_activity(tx: &mut Tx, project_id: &str) -> Result<Activity, ApiError> {
    let rows: Vec<(Option<String>, String)> = sqlx::query_as(
        "SELECT scope, to_char(max(at) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM ( \
           SELECT d.feature_id::text AS scope, c.created_at AS at \
             FROM doc_changes c JOIN docs d ON d.workspace_id = c.workspace_id AND d.id = c.doc_id \
             WHERE d.project_id = $1::uuid \
           UNION ALL \
           SELECT feature_id::text, created_at FROM threads WHERE project_id = $1::uuid \
           UNION ALL \
           SELECT feature_id::text, concluded_at FROM threads \
             WHERE project_id = $1::uuid AND concluded_at IS NOT NULL \
           UNION ALL \
           SELECT t.feature_id::text, p.created_at \
             FROM posts p JOIN threads t ON t.workspace_id = p.workspace_id AND t.id = p.thread_id \
             WHERE t.project_id = $1::uuid \
         ) writes GROUP BY scope",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(Activity(rows.into_iter().collect()))
}

/// The number of posts and the time of the newest post of each thread of the
/// project that has posts, by thread ID.
async fn post_stats(
    tx: &mut Tx,
    project_id: &str,
) -> Result<HashMap<String, (i64, Option<String>)>, ApiError> {
    let rows: Vec<(String, i64, Option<String>)> = sqlx::query_as(
        "SELECT p.thread_id::text, count(*), \
                to_char(max(p.created_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM posts p JOIN threads t ON t.workspace_id = p.workspace_id AND t.id = p.thread_id \
         WHERE t.project_id = $1::uuid GROUP BY p.thread_id",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(rows
        .into_iter()
        .map(|(thread, count, at)| (thread, (count, at)))
        .collect())
}
