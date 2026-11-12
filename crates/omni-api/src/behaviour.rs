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
    /// How many questions on the node have a current answer an agent gave on its own.
    pub default_answers: i64,
    /// Whether the next grill run should revisit the node.
    pub needs_pass: bool,
    /// When the node was marked, or `null`.
    pub needs_pass_since: Option<String>,
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
    /// Whether the next grill run should revisit the node.
    pub needs_pass: bool,
    /// When the node was marked, or `null`.
    pub needs_pass_since: Option<String>,
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
    /// The current answer, or `null` while the question is open.
    pub answer: Option<AnswerBody>,
    /// Every answer, oldest first; the last one is the current answer.
    pub answers: Vec<AnswerBody>,
}

/// An answer to a question.
#[derive(Serialize, ToSchema)]
pub struct AnswerBody {
    pub answer: String,
    /// `human`, or `agent` for an answer an agent gave on its own (an answer by default).
    pub given_by: String,
    /// Whoever made the call: `{kind: "agent", run, role, model, harness}` or `{kind: "human", user_id}`.
    #[schema(value_type = Object)]
    pub author: Value,
    /// When the answer was given.
    pub at: String,
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
    by: Option<String>,
    under: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct AskRequest {
    /// Not blank.
    pub question: String,
    /// Creates the question answered. Not blank.
    pub answer: Option<String>,
    /// With `answer`: `human` or `agent`; see `AnswerRequest`.
    pub by: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct AnswerRequest {
    /// The question's `version` as the caller read it.
    pub expected_version: i64,
    /// Not blank.
    pub answer: String,
    /// `human` or `agent`. A session always gives `human` and refuses `agent`. An API
    /// key gives `agent`, an answer by default, unless this says `human`: the agent
    /// wrote down what a person said.
    pub by: Option<String>,
    /// `true` to change the answer of an answered question: a new answer is stored
    /// and the earlier ones stay. An agent's own answer never replaces a person's
    /// (409 `answered_by_human`). Without it an answered question is 409
    /// `already_answered`; with it an open one is 409 `not_answered`.
    #[serde(default)]
    pub change: bool,
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
    answer: Option<String>,
    answers: String,
}

const QUESTION_COLUMNS: &str = concat!(
    "id::text, node_id::text, question, state, answered_by, asked_by::text, version, ",
    "to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at, ",
    "(SELECT jsonb_build_object('answer', a.answer, 'given_by', a.given_by, 'author', a.author, ",
    "'at', to_char(a.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'))::text ",
    "FROM behaviour_answers a WHERE a.question_id = behaviour_questions.id ",
    "ORDER BY a.created_at DESC, a.id DESC LIMIT 1) AS answer, ",
    "COALESCE((SELECT jsonb_agg(jsonb_build_object('answer', a.answer, 'given_by', a.given_by, ",
    "'author', a.author, 'at', to_char(a.created_at AT TIME ZONE 'UTC', ",
    "'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')) ORDER BY a.created_at, a.id) ",
    "FROM behaviour_answers a WHERE a.question_id = behaviour_questions.id), '[]'::jsonb)::text AS answers"
);

fn answer_of(value: &Value) -> Option<AnswerBody> {
    Some(AnswerBody {
        answer: value["answer"].as_str()?.to_string(),
        given_by: value["given_by"].as_str()?.to_string(),
        author: value["author"].clone(),
        at: value["at"].as_str()?.to_string(),
    })
}

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
            answer: self
                .answer
                .and_then(|text| answer_of(&serde_json::from_str(&text).ok()?)),
            answers: serde_json::from_str::<Vec<Value>>(&self.answers)
                .unwrap_or_default()
                .iter()
                .filter_map(answer_of)
                .collect(),
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
    needs_pass: Option<bool>,
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
    /// `false` clears the node's mark (an API key only, 403 with a session);
    /// `true` is 422: marks come from the rules.
    pub needs_pass: Option<bool>,
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
    needs_pass_since: Option<String>,
    created_by: String,
    created_at: String,
    updated_at: String,
}

const NODE_COLUMNS: &str = concat!(
    "id::text, parent_id::text, slug, position, title, summary, situation, behaviour, status, ",
    "same_as_id::text, handoff_id::text, version, ",
    "to_char(needs_pass_since AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS needs_pass_since, ",
    "created_by::text, ",
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
            needs_pass: row.needs_pass_since.is_some(),
            needs_pass_since: row.needs_pass_since.clone(),
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
            default_answers: 0,
            needs_pass: row.needs_pass_since.is_some(),
            needs_pass_since: row.needs_pass_since.clone(),
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

/// Marks a node, the nodes below it, or both, for another grill pass and announces
/// it once. Sets `needs_pass_since` where it is null; nothing else changes.
async fn mark(
    tx: &mut Transaction<'static, Postgres>,
    workspace_id: &str,
    project_id: &str,
    feature_id: &str,
    node_id: &str,
    itself: bool,
    below: bool,
) -> Result<(), ApiError> {
    sqlx::query(
        "WITH RECURSIVE sub AS (SELECT id FROM behaviour_nodes WHERE parent_id = $1::uuid \
         UNION ALL SELECT n.id FROM behaviour_nodes n JOIN sub ON n.parent_id = sub.id) \
         UPDATE behaviour_nodes SET needs_pass_since = clock_timestamp() \
         WHERE feature_id = $2::uuid AND needs_pass_since IS NULL \
         AND (($3 AND id = $1::uuid) OR ($4 AND id IN (SELECT id FROM sub)))",
    )
    .bind(node_id)
    .bind(feature_id)
    .bind(itself)
    .bind(below)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        tx,
        workspace_id,
        project_id,
        "behaviour.marked",
        &json!({ "feature_id": feature_id, "node_id": node_id, "below": below }),
    )
    .await?;
    Ok(())
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
        ("needs_pass" = Option<bool>, Query, description = "`true`: only the nodes marked for another grill pass"),
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
    let mut default_counts: HashMap<&str, i64> = HashMap::new();
    for question in &open {
        if question.state == "open" {
            *open_counts.entry(question.node_id.as_str()).or_default() += 1;
        } else if question.answered_by.as_deref() == Some("agent") {
            *default_counts.entry(question.node_id.as_str()).or_default() += 1;
        }
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
        .filter(|at| query.needs_pass != Some(true) || tree.rows[*at].needs_pass_since.is_some())
        .map(|at| {
            let mut summary = tree.summary(at);
            summary.open_questions = open_counts.get(summary.id.as_str()).copied().unwrap_or(0);
            summary.default_answers = default_counts
                .get(summary.id.as_str())
                .copied()
                .unwrap_or(0);
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
        (status = 403, description = "A session sent `needs_pass`: only an API key clears a mark", body = ErrorBody),
        (status = 422, description = "No field but `expected_version` (`no_changes`), `needs_pass: true` (`invalid_needs_pass`), a blank title (`invalid_title`), a slug that does not fit or one on the root (`invalid_slug`), a status other than `open`, `done` and `out_of_scope` (`invalid_status`), both pointers set (`two_pointers`), a pointer to the node itself (`pointer_to_itself`) or to a node that is not in the feature (`pointer_target_unknown`)", body = ErrorBody),
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
        ("needs_pass", patch.needs_pass.is_some()),
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
    if patch.needs_pass == Some(true) {
        return Err(invalid_field("invalid_needs_pass").into());
    }
    if patch.needs_pass.is_some() && matches!(request.actor, Actor::Human { .. }) {
        return Err(ApiError::Forbidden.into());
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
         needs_pass_since = CASE WHEN $10 THEN NULL ELSE needs_pass_since END, \
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
    .bind(patch.needs_pass == Some(false))
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
        (status = 201, description = "The new question: open (a node that was `done` is `open` again), or answered when the body has `answer` (the node's status is left alone)", body = QuestionResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`), or an API key without `X-Omni-Run` and `X-Omni-Role` (`identity_required`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such node in this feature, project and workspace", body = ErrorBody),
        (status = 422, description = "A blank question (`invalid_question`), a blank answer (`invalid_answer`), `by` without `answer`, a `by` other than `human` and `agent`, or `agent` from a session (`invalid_by`)", body = ErrorBody),
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
    let answer = match &ask.answer {
        Some(answer) => Some(checked_answer(answer, ask.by.as_deref(), &request.actor)?),
        None if ask.by.is_some() => return Err(invalid_field("invalid_by")),
        None => None,
    };
    let given_by = answer.as_ref().map(|(_, by)| *by);
    let question_id: String = sqlx::query_scalar(
        "INSERT INTO behaviour_questions (workspace_id, project_id, feature_id, node_id, \
         question, asked_by, state, answered_by) VALUES ($1::uuid, $2::uuid, $3::uuid, \
         $4::uuid, $5, $6::text::jsonb, CASE WHEN $7::text IS NULL THEN 'open' ELSE 'answered' END, $7) \
         RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(&project_id)
    .bind(&feature_id)
    .bind(&node_id)
    .bind(text)
    .bind(author.to_string())
    .bind(given_by)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    if let Some((text, by)) = &answer {
        insert_answer(
            &mut tx,
            workspace_id,
            &feature_id,
            &question_id,
            text,
            by,
            &author,
        )
        .await?;
    }
    let tree = Tree::load(&mut tx, &feature_id).await?;
    if status == "done" && answer.is_none() {
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
            "state": if answer.is_some() { "answered" } else { "open" },
        }),
    )
    .await?;
    if matches!(request.actor, Actor::Human { .. }) {
        mark(
            &mut tx,
            workspace_id,
            &project_id,
            &feature_id,
            &node_id,
            true,
            false,
        )
        .await?;
    }
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok((
        StatusCode::CREATED,
        Json(QuestionResponse {
            question: row.body(&tree),
        }),
    ))
}

/// The trimmed answer and who gave it. A session is always a person and refuses
/// `agent`; an API key is an agent giving its own answer unless `by` says `human`.
fn checked_answer<'a>(
    answer: &'a str,
    by: Option<&str>,
    actor: &Actor,
) -> Result<(&'a str, &'static str), ApiError> {
    let by = match by {
        None | Some("agent") if matches!(actor, Actor::Agent) => "agent",
        Some("human") => "human",
        Some("agent") if matches!(actor, Actor::Human { .. }) => {
            return Err(invalid_field("invalid_by"));
        }
        None => "human",
        Some(_) => return Err(invalid_field("invalid_by")),
    };
    let text = answer.trim();
    if text.is_empty() {
        return Err(invalid_field("invalid_answer"));
    }
    Ok((text, by))
}

async fn insert_answer(
    tx: &mut Transaction<'static, Postgres>,
    workspace_id: &str,
    feature_id: &str,
    question_id: &str,
    answer: &str,
    given_by: &str,
    author: &Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO behaviour_answers (workspace_id, feature_id, question_id, answer, \
         given_by, author) VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6::text::jsonb)",
    )
    .bind(workspace_id)
    .bind(feature_id)
    .bind(question_id)
    .bind(answer)
    .bind(given_by)
    .bind(author.to_string())
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(())
}

/// Answers an open question, or with `change` an answered one.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/questions/{question_id}/answers",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("question_id" = String, Path, description = "The question's UUID"),
    ),
    request_body = AnswerRequest,
    responses(
        (status = 201, description = "The question, answered, with its new `version`", body = QuestionResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`), or an API key without `X-Omni-Run` and `X-Omni-Role` (`identity_required`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such question in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`already_answered` (an answer without `change` on an answered question), `not_answered` (`change` on an open one), `conflict` with `current_version` when `expected_version` is stale, else `answered_by_human` (an agent's own answer on a question a person answered)", body = StoryConflictBody),
        (status = 422, description = "A blank answer (`invalid_answer`), a `by` other than `human` and `agent`, or `agent` from a session (`invalid_by`)", body = ErrorBody),
    )
)]
pub async fn answer_question(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, feature_id, question_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<QuestionResponse>), StoryError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let question_id = parse_uuid(&question_id).ok_or(ApiError::NotFound)?;
    let write = parse_body::<AnswerRequest>(&body);
    let author = author_of(&request.actor, &identity);
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let locked: Option<(String, String, i32, Option<String>)> = sqlx::query_as(
        "SELECT node_id::text, state, version, answered_by FROM behaviour_questions \
         WHERE id = $1::uuid AND feature_id = $2::uuid AND project_id = $3::uuid FOR UPDATE",
    )
    .bind(&question_id)
    .bind(&feature_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (node_id, current_state, version, answered_by) = locked.ok_or(ApiError::NotFound)?;
    let write = write?;
    let (text, by) = checked_answer(&write.answer, write.by.as_deref(), &request.actor)?;
    if write.change && current_state != "answered" {
        return Err(ApiError::conflict("not_answered").into());
    }
    if !write.change && current_state == "answered" {
        return Err(ApiError::conflict("already_answered").into());
    }
    if write.expected_version != i64::from(version) {
        return Err(StoryError::Stale(version));
    }
    if by == "agent" && answered_by.as_deref() == Some("human") {
        return Err(ApiError::conflict("answered_by_human").into());
    }
    insert_answer(
        &mut tx,
        workspace_id,
        &feature_id,
        &question_id,
        text,
        by,
        &author,
    )
    .await?;
    sqlx::query(
        "UPDATE behaviour_questions SET state = 'answered', answered_by = $2, \
         version = version + 1, updated_at = clock_timestamp() WHERE id = $1::uuid",
    )
    .bind(&question_id)
    .bind(by)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
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
            "state": "answered",
        }),
    )
    .await?;
    let by_person = matches!(request.actor, Actor::Human { .. });
    if by_person || write.change {
        mark(
            &mut tx,
            workspace_id,
            &project_id,
            &feature_id,
            &node_id,
            by_person,
            write.change,
        )
        .await?;
    }
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok((
        StatusCode::CREATED,
        Json(QuestionResponse {
            question: row.body(&tree),
        }),
    ))
}

/// One question of a feature's behaviour tree with every answer it has had.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/questions/{question_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("question_id" = String, Path, description = "The question's UUID"),
    ),
    responses(
        (status = 200, description = "The question, with `answers` oldest first", body = QuestionResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such question in this feature, project and workspace", body = ErrorBody),
    )
)]
pub async fn show_question(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id, question_id)): Path<(String, String, String, String)>,
) -> Result<Json<QuestionResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let question_id = parse_uuid(&question_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    let row = sqlx::query_as::<_, QuestionRow>(&format!(
        "SELECT {QUESTION_COLUMNS} FROM behaviour_questions \
         WHERE id = $1::uuid AND feature_id = $2::uuid"
    ))
    .bind(&question_id)
    .bind(&feature_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(QuestionResponse {
        question: row.body(&tree),
    }))
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
        ("by" = Option<String>, Query, description = "`agent` or `human`: only the answered questions whose current answer was given that way"),
        ("under" = Option<String>, Query, description = "Only the questions on this node and the nodes below it"),
    ),
    responses(
        (status = 200, description = "The questions in tree order of their nodes, then oldest first", body = QuestionListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace, or `under` is not a node of it", body = ErrorBody),
        (status = 422, description = "A `state` other than `open` and `answered` (`invalid_state`), or a `by` other than `agent` and `human` (`invalid_by`)", body = ErrorBody),
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
    if query
        .by
        .as_deref()
        .is_some_and(|by| !["agent", "human"].contains(&by))
    {
        return Err(invalid_field("invalid_by"));
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
            && query
                .by
                .as_deref()
                .is_none_or(|by| Some(by) == row.answered_by.as_deref())
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

/// A rule a node imposes beyond itself.
#[derive(Serialize, ToSchema)]
pub struct RuleBody {
    pub id: String,
    /// The node that imposes the rule.
    pub node_id: String,
    /// The path of that node; `""` for the root.
    pub node_path: String,
    pub text: String,
    /// `node` (the node and below), `story` (every node of its story) or `feature`.
    pub scope: String,
    /// `true` for a rule that is no longer in force; it stays readable.
    pub withdrawn: bool,
    /// `{kind: "agent", run, role, model, harness}` or `{kind: "human", user_id}`.
    #[schema(value_type = Object)]
    pub author: Value,
    /// 1 on creation, plus 1 on every accepted write.
    pub version: i32,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct RuleResponse {
    pub rule: RuleBody,
}

#[derive(Serialize, ToSchema)]
pub struct RuleListResponse {
    /// In tree order of their nodes, then oldest first; withdrawn rules included.
    pub rules: Vec<RuleBody>,
}

#[derive(Deserialize)]
pub struct RuleQuery {
    scope: Option<String>,
    under: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateRuleRequest {
    /// Not blank.
    pub text: String,
    /// `node` (the default), `story` or `feature`. `story` on the root is 422 `no_story`.
    pub scope: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct PatchRuleRequest {
    /// The rule's `version` as the caller read it.
    pub expected_version: i64,
    /// Not blank.
    pub text: Option<String>,
    /// `node`, `story` or `feature`. `story` on a rule of the root is 422 `no_story`.
    pub scope: Option<String>,
    /// `true` withdraws the rule, `false` puts it back in force.
    pub withdrawn: Option<bool>,
}

#[derive(sqlx::FromRow)]
struct RuleRow {
    id: String,
    node_id: String,
    text: String,
    scope: String,
    withdrawn: bool,
    author: String,
    version: i32,
    created_at: String,
}

const RULE_COLUMNS: &str = concat!(
    "id::text, node_id::text, text, scope, withdrawn_at IS NOT NULL AS withdrawn, author::text, version, ",
    "to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at"
);

impl RuleRow {
    fn body(self, tree: &Tree) -> RuleBody {
        let node_path = tree
            .find(&self.node_id)
            .map(|at| tree.place(at).0)
            .unwrap_or_default();
        RuleBody {
            id: self.id,
            node_id: self.node_id,
            node_path,
            text: self.text,
            scope: self.scope,
            withdrawn: self.withdrawn,
            author: serde_json::from_str(&self.author).unwrap_or(Value::Null),
            version: self.version,
            created_at: self.created_at,
        }
    }
}

fn scope_known(scope: &str) -> bool {
    ["node", "story", "feature"].contains(&scope)
}

/// Adds a rule on a node.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/nodes/{node_id}/rules",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("node_id" = String, Path, description = "The node's UUID"),
    ),
    request_body = CreateRuleRequest,
    responses(
        (status = 201, description = "The rule, at version 1", body = RuleResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`), or an API key without `X-Omni-Run` and `X-Omni-Role` (`identity_required`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such node in this feature, project and workspace", body = ErrorBody),
        (status = 422, description = "A blank text (`invalid_text`), a scope other than `node`, `story` and `feature` (`invalid_scope`), or `story` on the root (`no_story`)", body = ErrorBody),
    )
)]
pub async fn create_rule(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, feature_id, node_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<RuleResponse>), ApiError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let node_id = parse_uuid(&node_id).ok_or(ApiError::NotFound)?;
    let write = parse_body::<CreateRuleRequest>(&body);
    let author = author_of(&request.actor, &identity);
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let node: Option<(bool,)> = sqlx::query_as(
        "SELECT parent_id IS NULL FROM behaviour_nodes \
         WHERE id = $1::uuid AND feature_id = $2::uuid AND project_id = $3::uuid FOR UPDATE",
    )
    .bind(&node_id)
    .bind(&feature_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (is_root,) = node.ok_or(ApiError::NotFound)?;
    let write = write?;
    let text = write.text.trim();
    if text.is_empty() {
        return Err(invalid_field("invalid_text"));
    }
    let scope = write.scope.as_deref().unwrap_or("node");
    if !scope_known(scope) {
        return Err(invalid_field("invalid_scope"));
    }
    if is_root && scope == "story" {
        return Err(invalid_field("no_story"));
    }
    let rule_id: String = sqlx::query_scalar(
        "INSERT INTO behaviour_rules (workspace_id, project_id, feature_id, node_id, text, \
         scope, author) VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, $6, $7::text::jsonb) \
         RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(&project_id)
    .bind(&feature_id)
    .bind(&node_id)
    .bind(text)
    .bind(scope)
    .bind(author.to_string())
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    let row = sqlx::query_as::<_, RuleRow>(&format!(
        "SELECT {RULE_COLUMNS} FROM behaviour_rules WHERE id = $1::uuid"
    ))
    .bind(&rule_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        workspace_id,
        &project_id,
        "behaviour.rule_changed",
        &json!({ "feature_id": feature_id, "node_id": node_id, "rule_id": rule_id }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok((
        StatusCode::CREATED,
        Json(RuleResponse {
            rule: row.body(&tree),
        }),
    ))
}

/// Rewrites a rule, changes its scope, or withdraws it. A rule is never deleted.
#[utoipa::path(
    patch,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/rules/{rule_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("rule_id" = String, Path, description = "The rule's UUID"),
    ),
    request_body = PatchRuleRequest,
    responses(
        (status = 200, description = "The rule with its new `version`", body = RuleResponse),
        (status = 400, description = "The body is not JSON of the expected shape (`invalid_request`), or an API key without `X-Omni-Run` and `X-Omni-Role` (`identity_required`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such rule in this feature, project and workspace", body = ErrorBody),
        (status = 409, description = "`conflict` with `current_version` when `expected_version` is stale", body = StoryConflictBody),
        (status = 422, description = "Nothing to change (`no_changes`), a blank text (`invalid_text`), an unknown scope (`invalid_scope`), or `story` on a rule of the root (`no_story`)", body = ErrorBody),
    )
)]
pub async fn patch_rule(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    Path((_, project_id, feature_id, rule_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<RuleResponse>, StoryError> {
    require_identity(&request.actor, &identity)?;
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    let rule_id = parse_uuid(&rule_id).ok_or(ApiError::NotFound)?;
    let patch = parse_body::<PatchRuleRequest>(&body)?;
    if patch.text.is_none() && patch.scope.is_none() && patch.withdrawn.is_none() {
        return Err(invalid_field("no_changes").into());
    }
    let text = patch.text.as_ref().map(|text| text.trim().to_string());
    if text.as_ref().is_some_and(|text| text.is_empty()) {
        return Err(invalid_field("invalid_text").into());
    }
    if patch
        .scope
        .as_deref()
        .is_some_and(|scope| !scope_known(scope))
    {
        return Err(invalid_field("invalid_scope").into());
    }
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let current: Option<(String, i32, bool, String)> = sqlx::query_as(
        "SELECT r.node_id::text, r.version, n.parent_id IS NULL, r.scope \
         FROM behaviour_rules r JOIN behaviour_nodes n ON n.id = r.node_id \
         WHERE r.id = $1::uuid AND r.feature_id = $2::uuid AND r.project_id = $3::uuid \
         FOR UPDATE OF r",
    )
    .bind(&rule_id)
    .bind(&feature_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (node_id, version, is_root, scope_now) = current.ok_or(ApiError::NotFound)?;
    if i64::from(version) != patch.expected_version {
        return Err(StoryError::Stale(version));
    }
    let scope = patch.scope.as_deref().unwrap_or(&scope_now);
    if is_root && scope == "story" {
        return Err(invalid_field("no_story").into());
    }
    sqlx::query(
        "UPDATE behaviour_rules SET text = COALESCE($2, text), scope = $3, \
         withdrawn_at = CASE WHEN $4::boolean IS NULL THEN withdrawn_at \
         WHEN $4 THEN COALESCE(withdrawn_at, clock_timestamp()) ELSE NULL END, \
         version = version + 1, updated_at = clock_timestamp() WHERE id = $1::uuid",
    )
    .bind(&rule_id)
    .bind(text)
    .bind(scope)
    .bind(patch.withdrawn)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    let row = sqlx::query_as::<_, RuleRow>(&format!(
        "SELECT {RULE_COLUMNS} FROM behaviour_rules WHERE id = $1::uuid"
    ))
    .bind(&rule_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    live::emit(
        &mut tx,
        workspace_id,
        &project_id,
        "behaviour.rule_changed",
        &json!({ "feature_id": feature_id, "node_id": node_id, "rule_id": rule_id }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(workspace_id, &project_id);
    Ok(Json(RuleResponse {
        rule: row.body(&tree),
    }))
}

/// The rules of a feature's behaviour tree.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/rules",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("scope" = Option<String>, Query, description = "`node`, `story` or `feature`; every scope when left out"),
        ("under" = Option<String>, Query, description = "Only the rules imposed on this node and the nodes below it"),
    ),
    responses(
        (status = 200, description = "The rules in tree order of their nodes, then oldest first, withdrawn ones included and flagged", body = RuleListResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace, or `under` is not a node of it", body = ErrorBody),
        (status = 422, description = "A `scope` other than `node`, `story` and `feature` (`invalid_scope`)", body = ErrorBody),
    )
)]
pub async fn list_rules(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
    Query(query): Query<RuleQuery>,
) -> Result<Json<RuleListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    if query
        .scope
        .as_deref()
        .is_some_and(|scope| !scope_known(scope))
    {
        return Err(invalid_field("invalid_scope"));
    }
    let under = match &query.under {
        Some(id) => Some(parse_uuid(id).ok_or(ApiError::NotFound)?),
        None => None,
    };
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_feature(&mut tx, &project_id, &feature_id).await?;
    let tree = Tree::load(&mut tx, &feature_id).await?;
    let rows = sqlx::query_as::<_, RuleRow>(&format!(
        "SELECT {RULE_COLUMNS} FROM behaviour_rules WHERE feature_id = $1::uuid \
         AND ($2::text IS NULL OR scope = $2) ORDER BY created_at, id"
    ))
    .bind(&feature_id)
    .bind(&query.scope)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
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
    let mut by_node: HashMap<String, Vec<RuleRow>> = HashMap::new();
    for row in rows {
        by_node.entry(row.node_id.clone()).or_default().push(row);
    }
    let mut rules = Vec::new();
    for at in order {
        for row in by_node.remove(&tree.rows[at].id).unwrap_or_default() {
            rules.push(row.body(&tree));
        }
    }
    Ok(Json(RuleListResponse { rules }))
}
