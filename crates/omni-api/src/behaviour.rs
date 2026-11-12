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
use axum::extract::{Path, State};
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

#[derive(Serialize, ToSchema)]
pub struct NodeReadResponse {
    pub node: NodeBody,
    /// The nodes above it, the root first.
    pub chain: Vec<NodeBody>,
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
    version: i32,
    created_by: String,
    created_at: String,
    updated_at: String,
}

const NODE_COLUMNS: &str = concat!(
    "id::text, parent_id::text, slug, position, title, summary, situation, behaviour, status, ",
    "version, created_by::text, ",
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
    author: &'a Value,
}

/// Inserts a node as the last child of its parent and returns its id.
async fn insert_node(
    tx: &mut Transaction<'static, Postgres>,
    new: NewNode<'_>,
) -> Result<String, ApiError> {
    sqlx::query_scalar(
        "INSERT INTO behaviour_nodes (workspace_id, project_id, feature_id, parent_id, slug, \
         position, title, summary, created_by) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, \
         (SELECT coalesce(max(position) + 1, 0) FROM behaviour_nodes \
          WHERE feature_id = $3::uuid AND parent_id IS NOT DISTINCT FROM $4::uuid), \
         $6, $7, $8::text::jsonb) RETURNING id::text",
    )
    .bind(new.workspace_id)
    .bind(new.project_id)
    .bind(new.feature_id)
    .bind(new.parent_id)
    .bind(new.slug)
    .bind(new.title)
    .bind(new.summary)
    .bind(new.author.to_string())
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
        (status = 409, description = "A sibling has the slug (`slug_taken`)", body = ErrorBody),
        (status = 422, description = "A blank title (`invalid_title`) or a slug that does not fit `^[a-z0-9]+(-[a-z0-9]+)*$` and 80 characters (`invalid_slug`)", body = ErrorBody),
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
    ),
    responses(
        (status = 200, description = "Every node of the feature in tree order; empty when the feature has no tree", body = NodeListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace", body = ErrorBody),
    )
)]
pub async fn list_nodes(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
) -> Result<Json<NodeListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let nodes = tree
        .in_order()
        .into_iter()
        .map(|at| tree.summary(at))
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
    tx.commit().await.map_err(ApiError::database)?;
    let at = tree.find(&node_id).ok_or(ApiError::NotFound)?;
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
    Ok(Json(NodeReadResponse {
        node: tree.body(at),
        chain,
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
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale; checked before the rest. Else `slug_taken`", body = StoryConflictBody),
        (status = 422, description = "No field but `expected_version` (`no_changes`), a blank title (`invalid_title`), a slug that does not fit or one on the root (`invalid_slug`), or a status other than `open`, `done` and `out_of_scope` (`invalid_status`)", body = ErrorBody),
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

    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let current: Option<(i32, bool)> = sqlx::query_as(
        "SELECT version, parent_id IS NULL FROM behaviour_nodes \
         WHERE id = $1::uuid AND feature_id = $2::uuid AND project_id = $3::uuid FOR UPDATE",
    )
    .bind(&node_id)
    .bind(&feature_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (version, is_root) = current.ok_or(ApiError::NotFound)?;
    if i64::from(version) != patch.expected_version {
        return Err(StoryError::Stale(version));
    }
    if is_root && patch.slug.is_some() {
        return Err(invalid_field("invalid_slug").into());
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
    sqlx::query(
        "UPDATE behaviour_nodes SET title = COALESCE($2, title), slug = COALESCE($3, slug), \
         summary = COALESCE($4, summary), situation = COALESCE($5, situation), \
         behaviour = COALESCE($6, behaviour), status = COALESCE($7, status), \
         version = version + 1, updated_at = clock_timestamp() WHERE id = $1::uuid",
    )
    .bind(&node_id)
    .bind(&title)
    .bind(&patch.slug)
    .bind(&patch.summary)
    .bind(&patch.situation)
    .bind(&patch.behaviour)
    .bind(&patch.status)
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
