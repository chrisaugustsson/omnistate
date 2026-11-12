//! The behaviour tree. A feature has one tree of nodes: the root stands for
//! the feature, its children are user stories and the nodes below them are
//! situations. The level and the path of a node are worked out from the
//! parents and the slugs, never stored. Every write stores who made it and adds
//! 1 to the node's `version`. Adding a node emits `behaviour.node_created`, changing one
//! `behaviour.node_changed`.

use crate::AppState;
use crate::docs::AgentIdentity;
use crate::live;
use crate::projects::{parse_body, require_project};
use crate::session::{ApiError, ErrorBody};
use crate::stories::{StoryConflictBody, StoryError};
use crate::threads::require_identity;
use crate::workspace::{Actor, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;
use utoipa::ToSchema;

/// The longest slug a person may give.
const MAX_SLUG: usize = 80;
/// The longest slug that is derived from a title.
const DERIVED_SLUG: usize = 60;

/// The node a pointer leads to.
#[derive(Serialize, ToSchema)]
pub struct PointerBody {
    pub id: String,
    pub path: String,
}

/// The node a pointer leads to, as the read of a node gives it.
#[derive(Serialize, ToSchema)]
pub struct PointerDetailBody {
    pub id: String,
    pub path: String,
    pub title: String,
    pub summary: String,
}

/// Keeps `null` apart from a missing field: `Some(None)` is `null`.
fn nullable<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

/// A node as the list of a feature's nodes gives it: no text.
#[derive(Serialize, ToSchema)]
pub struct NodeSummaryBody {
    pub id: String,
    /// `null` for the root.
    pub parent_id: Option<String>,
    /// The slugs from the story down, joined by `/`; `""` for the root.
    pub path: String,
    /// `null` for the root.
    pub slug: Option<String>,
    /// The order among siblings.
    pub position: i32,
    /// 0 for the root, 1 for a user story, 2 and more for a situation.
    pub level: i32,
    pub title: String,
    pub summary: String,
    /// `open`, `done` or `out_of_scope`.
    pub status: String,
    /// 1 on creation, plus 1 on every accepted write.
    pub version: i32,
    /// The node that describes the same situation, or `null`.
    pub same_as: Option<PointerBody>,
    /// The story, or the node in one, that the user carries on in, or `null`.
    pub handoff: Option<PointerBody>,
    /// How many questions on the node are open.
    pub open_questions: i64,
}

#[derive(Serialize, ToSchema)]
pub struct NodeBody {
    pub id: String,
    pub parent_id: Option<String>,
    pub path: String,
    pub slug: Option<String>,
    pub position: i32,
    pub level: i32,
    pub title: String,
    pub summary: String,
    /// `open`, `done` or `out_of_scope`.
    pub status: String,
    pub version: i32,
    pub situation: String,
    pub behaviour: String,
    pub same_as: Option<PointerDetailBody>,
    pub handoff: Option<PointerDetailBody>,
    /// `{kind: "agent", run, role, model, harness}` or `{kind: "human", user_id}`.
    #[schema(value_type = Object)]
    pub created_by: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct NodeResponse {
    pub node: NodeBody,
}

/// A direct child of a node, as the read of the node gives it.
#[derive(Serialize, ToSchema)]
pub struct NodeChildBody {
    pub id: String,
    pub path: String,
    pub slug: Option<String>,
    pub title: String,
    pub summary: String,
    /// `open`, `done` or `out_of_scope`.
    pub status: String,
    pub same_as: Option<PointerBody>,
    pub handoff: Option<PointerBody>,
}

#[derive(Serialize, ToSchema)]
pub struct NodeReadResponse {
    pub node: NodeBody,
    /// The nodes above it, the root first.
    pub chain: Vec<NodeBody>,
    /// The direct children, by position.
    pub children: Vec<NodeChildBody>,
    /// The node's own questions, oldest first.
    pub questions: Vec<QuestionBody>,
}

/// A question asked on a node, open or answered.
#[derive(Serialize, ToSchema)]
pub struct QuestionBody {
    pub id: String,
    pub node_id: String,
    /// The path of the node; `""` for the root.
    pub node_path: String,
    pub question: String,
    /// `open` or `answered`.
    pub state: String,
    /// Who gave the current answer: `human`, `agent`, or `null` while open.
    pub answered_by: Option<String>,
    /// `{kind: "agent", run, role, model, harness}` or `{kind: "human", user_id}`.
    #[schema(value_type = Object)]
    pub asked_by: Value,
    /// 1 on creation, plus 1 on every accepted write.
    pub version: i32,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct QuestionResponse {
    pub question: QuestionBody,
}

#[derive(Serialize, ToSchema)]
pub struct QuestionListResponse {
    /// In tree order of their nodes, then oldest first.
    pub questions: Vec<QuestionBody>,
}

#[derive(Deserialize)]
pub struct QuestionQuery {
    state: Option<String>,
    under: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct AskRequest {
    /// Not blank.
    pub question: String,
}

#[derive(sqlx::FromRow)]
struct QuestionRow {
    id: String,
    node_id: String,
    question: String,
    state: String,
    answered_by: Option<String>,
    asked_by: String,
    version: i32,
    created_at: String,
}

const QUESTION_COLUMNS: &str = concat!(
    "id::text, node_id::text, question, state, answered_by, asked_by::text, version, ",
    "to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at"
);

impl QuestionRow {
    fn body(self, tree: &Tree) -> QuestionBody {
        let node_path = tree
            .find(&self.node_id)
            .map(|at| tree.place(at).0)
            .unwrap_or_default();
        QuestionBody {
            id: self.id,
            node_id: self.node_id,
            node_path,
            question: self.question,
            state: self.state,
            answered_by: self.answered_by,
            asked_by: serde_json::from_str(&self.asked_by).unwrap_or(Value::Null),
            version: self.version,
            created_at: self.created_at,
        }
    }
}

/// The questions of a feature, oldest first.
async fn load_questions(
    tx: &mut Transaction<'static, Postgres>,
    feature_id: &str,
) -> Result<Vec<QuestionRow>, ApiError> {
    sqlx::query_as::<_, QuestionRow>(&format!(
        "SELECT {QUESTION_COLUMNS} FROM behaviour_questions WHERE feature_id = $1::uuid \
         ORDER BY created_at, id"
    ))
    .bind(feature_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}

#[derive(Deserialize)]
pub struct ListQuery {
    under: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct NodeListResponse {
    /// Every node of the feature in tree order: depth first, siblings by position.
    pub nodes: Vec<NodeSummaryBody>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateNodeRequest {
    /// Not blank.
    pub title: String,
    /// Lowercase words joined by `-`, at most 80 characters, unique among the
    /// siblings. Derived from the title when left out.
    pub slug: Option<String>,
    /// One sentence. Defaults to empty.
    pub summary: Option<String>,
    /// The node's parent. A child of the root when left out.
    pub parent_id: Option<String>,
    /// A node of the feature that describes the same situation.
    pub same_as: Option<String>,
    /// A node of the feature, a story or one in a story, that the user carries on in.
    pub handoff: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct PatchNodeRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// Not blank.
    pub title: Option<String>,
    /// As on creation; the root has none.
    pub slug: Option<String>,
    pub summary: Option<String>,
    pub situation: Option<String>,
    pub behaviour: Option<String>,
    /// `open`, `done` or `out_of_scope`.
    pub status: Option<String>,
    /// A node of the feature; `null` clears it.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<String>)]
    pub same_as: Option<Option<String>>,
    /// A node of the feature; `null` clears it.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<String>)]
    pub handoff: Option<Option<String>>,
}

#[derive(Deserialize, ToSchema)]
pub struct MoveNodeRequest {
    /// The `version` the caller last read.
    pub expected_version: i64,
    /// The node's new parent; the node becomes its last child.
    pub parent_id: String,
}

#[derive(sqlx::FromRow)]
struct NodeRow {
    id: String,
    parent_id: Option<String>,
    slug: Option<String>,
    position: i32,
    title: String,
    summary: String,
    situation: String,
    behaviour: String,
    status: String,
    same_as_id: Option<String>,
    handoff_id: Option<String>,
    version: i32,
    created_by: String,
    created_at: String,
    updated_at: String,
}

const NODE_COLUMNS: &str = concat!(
    "id::text, parent_id::text, slug, position, title, summary, situation, behaviour, status, ",
    "same_as_id::text, handoff_id::text, version, created_by::text, ",
    "to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at, ",
    "to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at"
);

/// The nodes of a feature with their paths and levels.
struct Tree {
    rows: Vec<NodeRow>,
    /// Index into `rows` by node id.
    index: HashMap<String, usize>,
}

impl Tree {
    async fn load(
        tx: &mut Transaction<'static, Postgres>,
        feature_id: &str,
    ) -> Result<Self, ApiError> {
        let rows = sqlx::query_as::<_, NodeRow>(&format!(
            "SELECT {NODE_COLUMNS} FROM behaviour_nodes WHERE feature_id = $1::uuid \
             ORDER BY position, id"
        ))
        .bind(feature_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(ApiError::database)?;
        let index = rows
            .iter()
            .enumerate()
            .map(|(at, row)| (row.id.clone(), at))
            .collect();
        Ok(Self { rows, index })
    }

    fn find(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    /// The path and the level of a node.
    fn place(&self, at: usize) -> (String, i32) {
        let mut slugs = Vec::new();
        let mut level = 0;
        let mut row = &self.rows[at];
        while let Some(parent) = row.parent_id.as_deref().and_then(|id| self.find(id)) {
            slugs.push(row.slug.clone().unwrap_or_default());
            level += 1;
            row = &self.rows[parent];
        }
        slugs.reverse();
        (slugs.join("/"), level)
    }

    fn pointer(&self, id: &Option<String>) -> Option<PointerBody> {
        let at = self.find(id.as_deref()?)?;
        Some(PointerBody {
            id: self.rows[at].id.clone(),
            path: self.place(at).0,
        })
    }

    fn pointer_detail(&self, id: &Option<String>) -> Option<PointerDetailBody> {
        let at = self.find(id.as_deref()?)?;
        Some(PointerDetailBody {
            id: self.rows[at].id.clone(),
            path: self.place(at).0,
            title: self.rows[at].title.clone(),
            summary: self.rows[at].summary.clone(),
        })
    }

    fn body(&self, at: usize) -> NodeBody {
        let (path, level) = self.place(at);
        let row = &self.rows[at];
        NodeBody {
            id: row.id.clone(),
            parent_id: row.parent_id.clone(),
            path,
            slug: row.slug.clone(),
            position: row.position,
            level,
            title: row.title.clone(),
            summary: row.summary.clone(),
            status: row.status.clone(),
            version: row.version,
            situation: row.situation.clone(),
            behaviour: row.behaviour.clone(),
            same_as: self.pointer_detail(&row.same_as_id),
            handoff: self.pointer_detail(&row.handoff_id),
            created_by: serde_json::from_str(&row.created_by).unwrap_or(Value::Null),
            created_at: row.created_at.clone(),
            updated_at: row.updated_at.clone(),
        }
    }

    fn summary(&self, at: usize) -> NodeSummaryBody {
        let (path, level) = self.place(at);
        let row = &self.rows[at];
        NodeSummaryBody {
            id: row.id.clone(),
            parent_id: row.parent_id.clone(),
            path,
            slug: row.slug.clone(),
            position: row.position,
            level,
            title: row.title.clone(),
            summary: row.summary.clone(),
            status: row.status.clone(),
            version: row.version,
            same_as: self.pointer(&row.same_as_id),
            handoff: self.pointer(&row.handoff_id),
            open_questions: 0,
        }
    }

    fn child(&self, at: usize) -> NodeChildBody {
        let (path, _) = self.place(at);
        let row = &self.rows[at];
        NodeChildBody {
            id: row.id.clone(),
            path,
            slug: row.slug.clone(),
            title: row.title.clone(),
            summary: row.summary.clone(),
            status: row.status.clone(),
            same_as: self.pointer(&row.same_as_id),
            handoff: self.pointer(&row.handoff_id),
        }
    }

    /// The id a pointer in a request names: a node of the feature other than `from`.
    fn target(&self, given: &str, from: Option<&str>) -> Result<String, ApiError> {
        let id = parse_uuid(given).filter(|id| self.find(id).is_some());
        match id {
            Some(id) if Some(id.as_str()) == from => Err(invalid_field("pointer_to_itself")),
            Some(id) => Ok(id),
            None => Err(invalid_field("pointer_target_unknown")),
        }
    }

    /// The indexes of the nodes in tree order: depth first, siblings by position.
    fn in_order(&self) -> Vec<usize> {
        let mut children: HashMap<Option<&str>, Vec<usize>> = HashMap::new();
        for (at, row) in self.rows.iter().enumerate() {
            children
                .entry(row.parent_id.as_deref())
                .or_default()
                .push(at);
        }
        let mut order = Vec::with_capacity(self.rows.len());
        let mut stack: Vec<usize> = children
            .get(&None)
            .map(|roots| roots.iter().rev().copied().collect())
            .unwrap_or_default();
        while let Some(at) = stack.pop() {
            order.push(at);
            if let Some(below) = children.get(&Some(self.rows[at].id.as_str())) {
                stack.extend(below.iter().rev());
            }
        }
        order
    }
}

/// 422 with a stable code that names the field.
fn invalid_field(code: &'static str) -> ApiError {
    ApiError::Refused(StatusCode::UNPROCESSABLE_ENTITY, code)
}

/// Who writes, as the JSON that is stored.
fn author_of(actor: &Actor, identity: &AgentIdentity) -> Value {
    match actor {
        Actor::Agent => {
            let mut author = identity.to_json();
            author["kind"] = json!("agent");
            author
        }
        Actor::Human { user_id, .. } => json!({ "kind": "human", "user_id": user_id }),
    }
}

fn slug_fits(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= MAX_SLUG
        && slug.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// A slug from a title: lowercase, every run of other characters becomes `-`,
/// cut to 60 characters.
fn slug_from(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let slug = &slug[..slug.len().min(DERIVED_SLUG)];
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() { "node" } else { slug }.to_string()
}

/// The feature's name, with its row locked so that two first writes do not
/// make two roots. 404 when the feature is not one of the project.
async fn lock_feature(
    tx: &mut Transaction<'static, Postgres>,
    project_id: &str,
    feature_id: &str,
) -> Result<String, ApiError> {
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT name FROM features \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(feature_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found.map(|(name,)| name).ok_or(ApiError::NotFound)
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

struct NewNode<'a> {
    workspace_id: &'a str,
    project_id: &'a str,
    feature_id: &'a str,
    parent_id: Option<&'a str>,
    slug: Option<&'a str>,
    title: &'a str,
    summary: &'a str,
    same_as_id: Option<&'a str>,
    handoff_id: Option<&'a str>,
    author: &'a Value,
}

/// Inserts a node as the last child of its parent and returns its id.
async fn insert_node(
    tx: &mut Transaction<'static, Postgres>,
    new: NewNode<'_>,
) -> Result<String, ApiError> {
    sqlx::query_scalar(
        "INSERT INTO behaviour_nodes (workspace_id, project_id, feature_id, parent_id, slug, \
         position, title, summary, created_by, same_as_id, handoff_id) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, \
         (SELECT coalesce(max(position) + 1, 0) FROM behaviour_nodes \
          WHERE feature_id = $3::uuid AND parent_id IS NOT DISTINCT FROM $4::uuid), \
         $6, $7, $8::text::jsonb, $9::uuid, $10::uuid) RETURNING id::text",
    )
    .bind(new.workspace_id)
    .bind(new.project_id)
    .bind(new.feature_id)
    .bind(new.parent_id)
    .bind(new.slug)
    .bind(new.title)
    .bind(new.summary)
    .bind(new.author.to_string())
    .bind(new.same_as_id)
    .bind(new.handoff_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// Adds a node to a feature's behaviour tree.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/nodes",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
    ),
    request_body = CreateNodeRequest,
    responses(
        (status = 201, description = "The new node, the last child of its parent. The first node of a feature also creates the root, titled with the feature's name", body = NodeResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`), or an API key without `X-Omni-Run` and `X-Omni-Role` (`identity_required`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace, or no such parent in the feature", body = ErrorBody),
        (status = 409, description = "A sibling has the slug (`slug_taken`), or the parent has a pointer (`points_elsewhere`)", body = ErrorBody),
        (status = 422, description = "A blank title (`invalid_title`), a slug that does not fit `^[a-z0-9]+(-[a-z0-9]+)*$` and 80 characters (`invalid_slug`), both pointers (`two_pointers`), or a pointer to a node that is not in the feature (`pointer_target_unknown`)", body = ErrorBody),
    )
)]
pub async fn create_node(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<NodeResponse>), ApiError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateNodeRequest>(&body);
    let author = author_of(&request.actor, &identity);
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    let feature_name = lock_feature(&mut tx, &project_id, &feature_id).await?;
    let new = new?;

    let title = new.title.trim().to_string();
    if title.is_empty() {
        return Err(invalid_field("invalid_title"));
    }
    if new.slug.as_deref().is_some_and(|slug| !slug_fits(slug)) {
        return Err(invalid_field("invalid_slug"));
    }
    if new.same_as.is_some() && new.handoff.is_some() {
        return Err(invalid_field("two_pointers"));
    }
    let mut tree = Tree::load(&mut tx, &feature_id).await?;
    let mut events = Vec::new();
    let root = match tree.rows.iter().position(|row| row.parent_id.is_none()) {
        Some(at) => tree.rows[at].id.clone(),
        None => {
            if new.parent_id.is_some() {
                return Err(ApiError::NotFound);
            }
            let root_id = insert_node(
                &mut tx,
                NewNode {
                    workspace_id,
                    project_id: &project_id,
                    feature_id: &feature_id,
                    parent_id: None,
                    slug: None,
                    title: &feature_name,
                    summary: "",
                    same_as_id: None,
                    handoff_id: None,
                    author: &author,
                },
            )
            .await?;
            events.push(json!({
                "feature_id": feature_id, "node_id": root_id, "parent_id": null, "path": "",
            }));
            root_id
        }
    };
    let parent_id = match &new.parent_id {
        Some(given) => {
            let given = parse_uuid(given).ok_or(ApiError::NotFound)?;
            if !tree.index.contains_key(&given) {
                return Err(ApiError::NotFound);
            }
            given
        }
        None => root,
    };
    if tree
        .find(&parent_id)
        .is_some_and(|at| tree.rows[at].same_as_id.is_some() || tree.rows[at].handoff_id.is_some())
    {
        return Err(ApiError::conflict("points_elsewhere"));
    }
    let same_as_id = new
        .same_as
        .as_deref()
        .map(|given| tree.target(given, None))
        .transpose()?;
    let handoff_id = new
        .handoff
        .as_deref()
        .map(|given| tree.target(given, None))
        .transpose()?;
    let siblings: Vec<&str> = tree
        .rows
        .iter()
        .filter(|row| row.parent_id.as_deref() == Some(parent_id.as_str()))
        .filter_map(|row| row.slug.as_deref())
        .collect();
    let slug = match &new.slug {
        Some(slug) if siblings.contains(&slug.as_str()) => {
            return Err(ApiError::conflict("slug_taken"));
        }
        Some(slug) => slug.clone(),
        None => {
            let base = slug_from(&title);
            let mut slug = base.clone();
            let mut n = 2;
            while siblings.contains(&slug.as_str()) {
                slug = format!("{base}-{n}");
                n += 1;
            }
            slug
        }
    };
    let node_id = insert_node(
        &mut tx,
        NewNode {
            workspace_id,
            project_id: &project_id,
            feature_id: &feature_id,
            parent_id: Some(&parent_id),
            slug: Some(&slug),
            title: &title,
            summary: new.summary.as_deref().unwrap_or_default(),
            same_as_id: same_as_id.as_deref(),
            handoff_id: handoff_id.as_deref(),
            author: &author,
        },
    )
    .await?;
    tree = Tree::load(&mut tx, &feature_id).await?;
    let at = tree.find(&node_id).ok_or(ApiError::Unavailable)?;
    let node = tree.body(at);
    events.push(json!({
        "feature_id": feature_id, "node_id": node.id, "parent_id": node.parent_id,
        "path": node.path,
    }));
    for data in &events {
        live::emit(
            &mut tx,
            workspace_id,
            &project_id,
            "behaviour.node_created",
            data,
        )
        .await?;
    }
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(NodeResponse { node })))
}

/// The nodes of a feature's behaviour tree, without their text.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("under" = Option<String>, Query, description = "Only this node and the nodes below it"),
    ),
    responses(
        (status = 200, description = "Every node of the feature in tree order; empty when the feature has no tree", body = NodeListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace, or `under` is not a node of it", body = ErrorBody),
    )
)]
pub async fn list_nodes(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
    Query(query): Query<ListQuery>,
) -> Result<Json<NodeListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let under = match &query.under {
        Some(id) => Some(parse_uuid(id).ok_or(ApiError::NotFound)?),
        None => None,
    };
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    let open = load_questions(&mut tx, &feature_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let mut open_counts: HashMap<&str, i64> = HashMap::new();
    for question in open.iter().filter(|q| q.state == "open") {
        *open_counts.entry(question.node_id.as_str()).or_default() += 1;
    }
    let mut order = tree.in_order();
    if let Some(under) = under {
        let top = tree.find(&under).ok_or(ApiError::NotFound)?;
        let start = order.iter().position(|at| *at == top).unwrap_or(0);
        let level = tree.place(top).1;
        let below = order[start + 1..]
            .iter()
            .take_while(|at| tree.place(**at).1 > level)
            .count();
        order = order[start..=start + below].to_vec();
    }
    let nodes = order
        .into_iter()
        .map(|at| {
            let mut summary = tree.summary(at);
            summary.open_questions = open_counts.get(summary.id.as_str()).copied().unwrap_or(0);
            summary
        })
        .collect();
    Ok(Json(NodeListResponse { nodes }))
}

/// One node with the nodes above it.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/nodes/{node_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("node_id" = String, Path, description = "The node's UUID"),
    ),
    responses(
        (status = 200, description = "The node and the nodes above it, the root first", body = NodeReadResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such node in this feature, project and workspace", body = ErrorBody),
    )
)]
pub async fn show_node(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id, node_id)): Path<(String, String, String, String)>,
) -> Result<Json<NodeReadResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let node_id = parse_uuid(&node_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    let all = load_questions(&mut tx, &feature_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let at = tree.find(&node_id).ok_or(ApiError::NotFound)?;
    let questions = all
        .into_iter()
        .filter(|question| question.node_id == node_id)
        .map(|question| question.body(&tree))
        .collect();
    let mut chain = Vec::new();
    let mut above = tree.rows[at]
        .parent_id
        .as_deref()
        .and_then(|id| tree.find(id));
    while let Some(parent) = above {
        chain.push(tree.body(parent));
        above = tree.rows[parent]
            .parent_id
            .as_deref()
            .and_then(|id| tree.find(id));
    }
    chain.reverse();
    let id = tree.rows[at].id.as_str();
    let children = tree
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.parent_id.as_deref() == Some(id))
        .map(|(child, _)| tree.child(child))
        .collect();
    Ok(Json(NodeReadResponse {
        node: tree.body(at),
        chain,
        children,
        questions,
    }))
}

/// Rewrites a node's text or sets its status.
#[utoipa::path(
    patch,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/nodes/{node_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("node_id" = String, Path, description = "The node's UUID"),
    ),
    request_body = PatchNodeRequest,
    responses(
        (status = 200, description = "The node with its new `version`", body = NodeResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`), or an API key without `X-Omni-Run` and `X-Omni-Role` (`identity_required`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such node in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; checked before the rest. Else `slug_taken`, `has_children` (a pointer set on a node that has children), or `open_questions` (status `done` while a question on the node is open)", body = StoryConflictBody),
        (status = 422, description = "No field but `expected_version` (`no_changes`), a blank title (`invalid_title`), a slug that does not fit or one on the root (`invalid_slug`), a status other than `open`, `done` and `out_of_scope` (`invalid_status`), both pointers set (`two_pointers`), a pointer to the node itself (`pointer_to_itself`) or to a node that is not in the feature (`pointer_target_unknown`)", body = ErrorBody),
    )
)]
pub async fn patch_node(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, feature_id, node_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<NodeResponse>, StoryError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let node_id = parse_uuid(&node_id).ok_or(ApiError::NotFound)?;
    let patch = parse_body::<PatchNodeRequest>(&body)?;
    let mut fields: Vec<&str> = Vec::new();
    for (name, present) in [
        ("title", patch.title.is_some()),
        ("slug", patch.slug.is_some()),
        ("summary", patch.summary.is_some()),
        ("situation", patch.situation.is_some()),
        ("behaviour", patch.behaviour.is_some()),
        ("status", patch.status.is_some()),
        ("same_as", patch.same_as.is_some()),
        ("handoff", patch.handoff.is_some()),
    ] {
        if present {
            fields.push(name);
        }
    }
    if fields.is_empty() {
        return Err(invalid_field("no_changes").into());
    }
    let title = patch.title.as_ref().map(|title| title.trim().to_string());
    if title.as_ref().is_some_and(|title| title.is_empty()) {
        return Err(invalid_field("invalid_title").into());
    }
    if patch.slug.as_deref().is_some_and(|slug| !slug_fits(slug)) {
        return Err(invalid_field("invalid_slug").into());
    }
    if patch
        .status
        .as_deref()
        .is_some_and(|status| !["open", "done", "out_of_scope"].contains(&status))
    {
        return Err(invalid_field("invalid_status").into());
    }
    if matches!(patch.same_as, Some(Some(_))) && matches!(patch.handoff, Some(Some(_))) {
        return Err(invalid_field("two_pointers").into());
    }

    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let current: Option<(i32, bool, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT version, parent_id IS NULL, same_as_id::text, handoff_id::text \
         FROM behaviour_nodes \
         WHERE id = $1::uuid AND feature_id = $2::uuid AND project_id = $3::uuid FOR UPDATE",
    )
    .bind(&node_id)
    .bind(&feature_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (version, is_root, same_as_now, handoff_now) = current.ok_or(ApiError::NotFound)?;
    if i64::from(version) != patch.expected_version {
        return Err(StoryError::Stale(version));
    }
    if is_root && patch.slug.is_some() {
        return Err(invalid_field("invalid_slug").into());
    }
    if patch.status.as_deref() == Some("done") {
        let (open,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM behaviour_questions \
             WHERE node_id = $1::uuid AND state = 'open')",
        )
        .bind(&node_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        if open {
            return Err(ApiError::conflict("open_questions").into());
        }
    }
    if let Some(slug) = &patch.slug {
        let (taken,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM behaviour_nodes WHERE slug = $2 AND id <> $1::uuid \
             AND parent_id = (SELECT parent_id FROM behaviour_nodes WHERE id = $1::uuid))",
        )
        .bind(&node_id)
        .bind(slug)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        if taken {
            return Err(ApiError::conflict("slug_taken").into());
        }
    }
    let tree = Tree::load(&mut tx, &feature_id).await?;
    let resolve = |given: &Option<Option<String>>, now: Option<String>| match given {
        None => Ok(now),
        Some(None) => Ok(None),
        Some(Some(given)) => tree.target(given, Some(&node_id)).map(Some),
    };
    let same_as = resolve(&patch.same_as, same_as_now)?;
    let handoff = resolve(&patch.handoff, handoff_now)?;
    if same_as.is_some() && handoff.is_some() {
        return Err(invalid_field("two_pointers").into());
    }
    let sets_pointer =
        matches!(patch.same_as, Some(Some(_))) || matches!(patch.handoff, Some(Some(_)));
    if sets_pointer
        && tree
            .rows
            .iter()
            .any(|row| row.parent_id.as_deref() == Some(node_id.as_str()))
    {
        return Err(ApiError::conflict("has_children").into());
    }
    sqlx::query(
        "UPDATE behaviour_nodes SET title = COALESCE($2, title), slug = COALESCE($3, slug), \
         summary = COALESCE($4, summary), situation = COALESCE($5, situation), \
         behaviour = COALESCE($6, behaviour), status = COALESCE($7, status), \
         same_as_id = $8::uuid, handoff_id = $9::uuid, \
         version = version + 1, updated_at = clock_timestamp() WHERE id = $1::uuid",
    )
    .bind(&node_id)
    .bind(&title)
    .bind(&patch.slug)
    .bind(&patch.summary)
    .bind(&patch.situation)
    .bind(&patch.behaviour)
    .bind(&patch.status)
    .bind(&same_as)
    .bind(&handoff)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    let at = tree.find(&node_id).ok_or(ApiError::Unavailable)?;
    let node = tree.body(at);
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "behaviour.node_changed",
        &json!({
            "feature_id": feature_id, "node_id": node.id, "path": node.path, "fields": fields,
        }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(NodeResponse { node }))
}

/// Moves a node, with everything below it, to be the last child of another node.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/nodes/{node_id}/move",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("node_id" = String, Path, description = "The node's UUID"),
    ),
    request_body = MoveNodeRequest,
    responses(
        (status = 200, description = "The node with its new `path`, `position` and `version`", body = NodeResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`), or an API key without `X-Omni-Run` and `X-Omni-Role` (`identity_required`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such node, or no such parent, in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; else `cycle` (the parent is the node or below it) `slug_taken` (the parent has a child with the node's slug) or `points_elsewhere` (the parent has a pointer)", body = StoryConflictBody),
        (status = 422, description = "The node is the root (`root_cannot_move`)", body = ErrorBody),
    )
)]
pub async fn move_node(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, feature_id, node_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<NodeResponse>, StoryError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let node_id = parse_uuid(&node_id).ok_or(ApiError::NotFound)?;
    let step = parse_body::<MoveNodeRequest>(&body)?;
    let parent_id = parse_uuid(&step.parent_id).ok_or(ApiError::NotFound)?;

    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let current: Option<(i32, bool, Option<String>)> = sqlx::query_as(
        "SELECT version, parent_id IS NULL, slug FROM behaviour_nodes \
         WHERE id = $1::uuid AND feature_id = $2::uuid AND project_id = $3::uuid FOR UPDATE",
    )
    .bind(&node_id)
    .bind(&feature_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (version, is_root, slug) = current.ok_or(ApiError::NotFound)?;
    if i64::from(version) != step.expected_version {
        return Err(StoryError::Stale(version));
    }
    if is_root {
        return Err(invalid_field("root_cannot_move").into());
    }
    let tree = Tree::load(&mut tx, &feature_id).await?;
    if tree.find(&parent_id).is_none() {
        return Err(ApiError::NotFound.into());
    }
    if tree
        .find(&parent_id)
        .is_some_and(|at| tree.rows[at].same_as_id.is_some() || tree.rows[at].handoff_id.is_some())
    {
        return Err(ApiError::conflict("points_elsewhere").into());
    }
    let mut above = Some(parent_id.as_str());
    while let Some(id) = above {
        if id == node_id {
            return Err(ApiError::conflict("cycle").into());
        }
        above = tree
            .find(id)
            .and_then(|at| tree.rows[at].parent_id.as_deref());
    }
    let (taken,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM behaviour_nodes WHERE parent_id = $1::uuid \
         AND slug = $2 AND id <> $3::uuid)",
    )
    .bind(&parent_id)
    .bind(&slug)
    .bind(&node_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    if taken {
        return Err(ApiError::conflict("slug_taken").into());
    }
    sqlx::query(
        "UPDATE behaviour_nodes SET parent_id = $2::uuid, \
         position = (SELECT COALESCE(MAX(position) + 1, 0) FROM behaviour_nodes \
                     WHERE parent_id = $2::uuid AND id <> $1::uuid), \
         version = version + 1, updated_at = clock_timestamp() WHERE id = $1::uuid",
    )
    .bind(&node_id)
    .bind(&parent_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    let at = tree.find(&node_id).ok_or(ApiError::Unavailable)?;
    let node = tree.body(at);
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "behaviour.node_changed",
        &json!({
            "feature_id": feature_id, "node_id": node.id, "path": node.path,
            "fields": ["parent_id", "position"],
        }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(Json(NodeResponse { node }))
}

/// Asks a question, or leaves a remark, on a node.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/nodes/{node_id}/questions",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("node_id" = String, Path, description = "The node's UUID"),
    ),
    request_body = AskRequest,
    responses(
        (status = 201, description = "The new open question. A node that was `done` is `open` again", body = QuestionResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`), or an API key without `X-Omni-Run` and `X-Omni-Role` (`identity_required`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such node in this feature, project and workspace", body = ErrorBody),
        (status = 422, description = "A blank question (`invalid_question`)", body = ErrorBody),
    )
)]
pub async fn ask_question(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, feature_id, node_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<QuestionResponse>), ApiError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let node_id = parse_uuid(&node_id).ok_or(ApiError::NotFound)?;
    let ask = parse_body::<AskRequest>(&body);
    let author = author_of(&request.actor, &identity);
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let node: Option<(String,)> = sqlx::query_as(
        "SELECT status FROM behaviour_nodes \
         WHERE id = $1::uuid AND feature_id = $2::uuid AND project_id = $3::uuid FOR UPDATE",
    )
    .bind(&node_id)
    .bind(&feature_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (status,) = node.ok_or(ApiError::NotFound)?;
    let ask = ask?;
    let text = ask.question.trim();
    if text.is_empty() {
        return Err(invalid_field("invalid_question"));
    }
    let question_id: String = sqlx::query_scalar(
        "INSERT INTO behaviour_questions (workspace_id, project_id, feature_id, node_id, \
         question, asked_by) VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, $6::text::jsonb) \
         RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(&project_id)
    .bind(&feature_id)
    .bind(&node_id)
    .bind(text)
    .bind(author.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    if status == "done" {
        sqlx::query(
            "UPDATE behaviour_nodes SET status = 'open', version = version + 1, \
             updated_at = clock_timestamp() WHERE id = $1::uuid",
        )
        .bind(&node_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        let path = tree.find(&node_id).map(|at| tree.place(at).0);
        live::emit(
            &mut tx,
            workspace_id,
            &project_id,
            "behaviour.node_changed",
            &json!({
                "feature_id": feature_id, "node_id": node_id, "path": path,
                "fields": ["status"],
            }),
        )
        .await?;
    }
    let row = sqlx::query_as::<_, QuestionRow>(&format!(
        "SELECT {QUESTION_COLUMNS} FROM behaviour_questions WHERE id = $1::uuid"
    ))
    .bind(&question_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        workspace_id,
        &project_id,
        "behaviour.question_changed",
        &json!({
            "feature_id": feature_id, "node_id": node_id, "question_id": question_id,
            "state": "open",
        }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok((
        StatusCode::CREATED,
        Json(QuestionResponse {
            question: row.body(&tree),
        }),
    ))
}

/// The questions of a feature's behaviour tree.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/questions",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("state" = Option<String>, Query, description = "`open` or `answered`; both when left out"),
        ("under" = Option<String>, Query, description = "Only the questions on this node and the nodes below it"),
    ),
    responses(
        (status = 200, description = "The questions in tree order of their nodes, then oldest first", body = QuestionListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace, or `under` is not a node of it", body = ErrorBody),
        (status = 422, description = "A `state` other than `open` and `answered` (`invalid_state`)", body = ErrorBody),
    )
)]
pub async fn list_questions(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
    Query(query): Query<QuestionQuery>,
) -> Result<Json<QuestionListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    if query
        .state
        .as_deref()
        .is_some_and(|state| !["open", "answered"].contains(&state))
    {
        return Err(invalid_field("invalid_state"));
    }
    let under = match &query.under {
        Some(id) => Some(parse_uuid(id).ok_or(ApiError::NotFound)?),
        None => None,
    };
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    let rows = load_questions(&mut tx, &feature_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let mut order = tree.in_order();
    if let Some(under) = under {
        let top = tree.find(&under).ok_or(ApiError::NotFound)?;
        let start = order.iter().position(|at| *at == top).unwrap_or(0);
        let level = tree.place(top).1;
        let below = order[start + 1..]
            .iter()
            .take_while(|at| tree.place(**at).1 > level)
            .count();
        order = order[start..=start + below].to_vec();
    }
    let mut by_node: HashMap<String, Vec<QuestionRow>> = HashMap::new();
    for row in rows {
        if query
            .state
            .as_deref()
            .is_none_or(|state| state == row.state)
        {
            by_node.entry(row.node_id.clone()).or_default().push(row);
        }
    }
    let mut questions = Vec::new();
    for at in order {
        for row in by_node.remove(&tree.rows[at].id).unwrap_or_default() {
            questions.push(row.body(&tree));
        }
    }
    Ok(Json(QuestionListResponse { questions }))
}
