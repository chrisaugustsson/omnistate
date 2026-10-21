//! The project home (US-040): the reviewer's page of one project in a single
//! read. It lists the project's docs, change sets and threads, and what waits
//! for the human: the decisions to approve, the change sets in review, the
//! comments that an agent marked addressed, and the agents' questions that no
//! post of the human has answered yet.
//!
//! Every item is the body that the API's own read of it answers, so the web
//! app shows one shape per entity. The route is the human's: a sealed phase
//! stays sealed for agents, and the home lists questions of every phase. The
//! page reloads the home when the project's live events come; the changes that
//! move an item onto or off a list emit those events already.

use crate::AppState;
use crate::changesets::{ChangesetBody, changesets_of_project, require_project};
use crate::claims;
use crate::comments::{CommentBody, addressed_of_project};
use crate::decisions::{DecisionBody, proposed_of_project};
use crate::docs::{DocBody, docs_of_project};
use crate::session::{ApiError, ErrorBody};
use crate::threads::{PostBody, ThreadBody, Viewer, threads_of_project, unanswered_questions};
use crate::workspace::{HumanRequest, parse_uuid};
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use utoipa::ToSchema;

/// What waits for the human, each list oldest first.
#[derive(Serialize, ToSchema)]
pub struct WaitingBody {
    /// The decisions whose status is `proposed`.
    pub decisions: Vec<DecisionBody>,
    /// The change sets whose status is `in_review`.
    pub changesets: Vec<ChangesetBody>,
    /// The comments whose status is `addressed`.
    pub comments: Vec<CommentBody>,
    /// The `question` posts of agents that no post of the human links to as a
    /// reply, in every phase.
    pub questions: Vec<PostBody>,
}

#[derive(Serialize, ToSchema)]
pub struct HomeResponse {
    pub project_id: String,
    /// Every doc of the project, oldest first, as `GET /docs/{doc_id}` answers it.
    pub docs: Vec<DocBody>,
    /// Every change set of the project, oldest first, with its status.
    pub changesets: Vec<ChangesetBody>,
    /// Every thread of the project, oldest first, without its posts.
    pub threads: Vec<ThreadBody>,
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
    let docs = docs_of_project(&state, &mut tx, &request.workspace_id, &project_id).await?;
    let changesets = changesets_of_project(&mut tx, &project_id).await?;
    let threads = threads_of_project(&mut tx, &project_id).await?;
    let decisions = proposed_of_project(&mut tx, &Viewer::human(), &project_id).await?;
    let comments = addressed_of_project(&mut tx, &project_id).await?;
    let questions = unanswered_questions(&mut tx, &project_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let in_review = changesets
        .iter()
        .filter(|changeset| changeset.status == "in_review")
        .cloned()
        .collect();
    Ok(Json(HomeResponse {
        project_id,
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
