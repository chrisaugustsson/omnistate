//! The change set review (US-046): what the reviewer needs to approve or reject
//! a change set, in a single read. It lists every doc the change set changes
//! with the node changes from the version it is based on to the latest version
//! saved in the change set, and the decisions the change set carries out.
//!
//! The conflicts of the change set's rebases come from the existing conflicts
//! read (US-026) and the content of the versions from the version reads, so
//! this route holds only what no other read answers. The route is the human's.

use crate::AppState;
use crate::changesets::{ChangesetBody, load_changeset};
use crate::decisions::{DecisionBody, load_decision};
use crate::session::{ApiError, ErrorBody};
use crate::threads::Viewer;
use crate::versions::{MovedNode, NodeEntry, compare, find_version, nodes_of_version};
use crate::workspace::{HumanRequest, parse_uuid};
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use utoipa::ToSchema;

/// The node changes of one doc, as the diff route lists them.
#[derive(Serialize, ToSchema)]
pub struct ReviewChanges {
    /// The nodes only the new version has; all of them for a new doc.
    pub added: Vec<NodeEntry>,
    /// The nodes only the base has; all of them for a removal.
    pub removed: Vec<NodeEntry>,
    /// The nodes of both whose own content differs, as in the new version.
    pub changed: Vec<NodeEntry>,
    /// The nodes of both whose place differs.
    pub moved: Vec<MovedNode>,
}

/// One doc that the change set changes.
#[derive(Serialize, ToSchema)]
pub struct ReviewedDoc {
    pub doc_id: String,
    /// `html`, `design` or `design-system`.
    pub kind: String,
    pub pillar: String,
    pub title: String,
    /// The version that the change set's draft is based on: the mainline
    /// version it was opened from or last rebased onto; `null` for a doc that
    /// the change set created.
    pub base_version_id: Option<String>,
    /// The latest version of the doc saved in the change set, the one that an
    /// approval covers.
    pub version_id: String,
    /// Whether that version removes the doc.
    pub removed: bool,
    pub changes: ReviewChanges,
}

#[derive(Serialize, ToSchema)]
pub struct ReviewResponse {
    pub changeset: ChangesetBody,
    /// Every doc with a version saved in the change set, oldest doc first.
    pub docs: Vec<ReviewedDoc>,
    /// The decisions linked to the change set, in the order they were linked,
    /// with their status now.
    pub decisions: Vec<DecisionBody>,
}

#[derive(sqlx::FromRow)]
struct ReviewedRow {
    doc_id: String,
    kind: String,
    pillar: String,
    title: String,
    base_version_id: Option<String>,
    version_id: String,
    removed: bool,
}

/// The review of a change set: the docs it changes and how, and its linked
/// decisions. Only the human reads it.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/review",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
    ),
    responses(
        (status = 200, description = "The change set review", body = ReviewResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
    )
)]
pub async fn show(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
) -> Result<Json<ReviewResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM changesets WHERE id = $1::uuid AND project_id = $2::uuid",
    )
    .bind(&changeset_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    found.ok_or(ApiError::NotFound)?;
    let changeset = load_changeset(&mut tx, &Viewer::human(), &changeset_id).await?;
    let rows: Vec<ReviewedRow> = sqlx::query_as(
        "SELECT d.id::text AS doc_id, d.kind, d.pillar, d.title, \
                dr.base_version_id::text AS base_version_id, \
                v.id::text AS version_id, v.removed \
         FROM docs d \
         JOIN LATERAL (SELECT id, removed FROM versions \
                       WHERE doc_id = d.id AND changeset_id = $2::uuid \
                       ORDER BY created_at DESC, id DESC LIMIT 1) v ON true \
         LEFT JOIN drafts dr ON dr.workspace_id = d.workspace_id AND dr.doc_id = d.id \
                            AND dr.changeset_id = $2::uuid \
         WHERE d.project_id = $1::uuid ORDER BY d.created_at, d.id",
    )
    .bind(&project_id)
    .bind(&changeset_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let mut docs = Vec::with_capacity(rows.len());
    for row in rows {
        let before = match &row.base_version_id {
            Some(base) => {
                let base = find_version(&mut tx, &project_id, base).await?;
                nodes_of_version(&mut tx, &base).await?
            }
            None => Vec::new(),
        };
        let version = find_version(&mut tx, &project_id, &row.version_id).await?;
        let after = nodes_of_version(&mut tx, &version).await?;
        let diff = compare(
            row.doc_id.clone(),
            row.base_version_id.clone().unwrap_or_default(),
            row.version_id.clone(),
            before,
            after,
        );
        docs.push(ReviewedDoc {
            doc_id: row.doc_id,
            kind: row.kind,
            pillar: row.pillar,
            title: row.title,
            base_version_id: row.base_version_id,
            version_id: row.version_id,
            removed: row.removed,
            changes: ReviewChanges {
                added: diff.added,
                removed: diff.removed,
                changed: diff.changed,
                moved: diff.moved,
            },
        });
    }
    let mut decisions = Vec::with_capacity(changeset.decisions.len());
    for linked in &changeset.decisions {
        decisions.push(load_decision(&mut tx, &Viewer::human(), &project_id, &linked.id).await?);
    }
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(ReviewResponse {
        changeset,
        docs,
        decisions,
    }))
}
