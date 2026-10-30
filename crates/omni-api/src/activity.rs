//! The activity list of a doc (US-111): what changed in it, who wrote it and
//! when. Every write to a doc is one row of `doc_changes` (FR-11), and an entry
//! of the list is that row, read back.
//!
//! - an HTML doc's entry lists the HTML of each element the write acted on,
//!   before and after, found by loading the head before and the head after;
//! - a design's entry lists the boards whose chunk the write changed, which
//!   `doc_changes.boards` holds, as the `seq` they can be read at
//!   (`/boards/{board}?at=<seq>`, the head that write `seq` left);
//! - a design system's entry lists the components that differ between the two
//!   heads, as the `seq` they can be read at (`/components/{component}?at=`),
//!   and the tokens that were added, changed or removed.

use crate::AppState;
use crate::boards;
use crate::chunks::read_chunk;
use crate::design::{self, read_design_system, tags_of};
use crate::docs::{empty_head, find_doc};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::extract::{Path, Query, State};
use omni_core::design::NodeId;
use omni_core::ops::EditDoc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use utoipa::ToSchema;

/// How many entries a page of the list holds when `?limit=` is not given.
const DEFAULT_LIMIT: i64 = 50;
/// The most entries a page of the list may hold.
const MAX_LIMIT: i64 = 200;

#[derive(Serialize, Deserialize, Default, ToSchema)]
#[serde(default)]
pub struct ActivityAgent {
    pub run: Option<String>,
    pub role: Option<String>,
    pub model: Option<String>,
    pub harness: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ActivityEntry {
    /// The write's number in the doc: 1 for its creation, rising by one with
    /// every write, with no gaps.
    pub seq: i64,
    /// `create`, `edit`, `rename` or `delete`.
    pub kind: String,
    /// The agent as it declared itself; not verified.
    pub agent: ActivityAgent,
    /// The time of the write, ISO 8601 in UTC.
    pub created_at: String,
    /// One line about the write, such as `3 nodes changed, 1 inserted`.
    pub summary: String,
}

#[derive(Serialize, ToSchema)]
pub struct ActivityListResponse {
    /// The doc's writes, newest first.
    pub entries: Vec<ActivityEntry>,
    /// The `before` that lists the next, older page; `null` on the last page.
    pub next_before: Option<i64>,
}

/// One thing a write changed. Which one an item is shows in its key: `node`
/// for an HTML doc, `board` for a design, `component` for a design system.
#[derive(Serialize, ToSchema)]
#[serde(untagged)]
pub enum ActivityItem {
    /// An element of an HTML doc, as outer HTML with `data-omni-id`. `before`
    /// is `null` for an inserted element and `after` for a deleted one. The
    /// creation or deletion of a doc is one item whose `node` is `null` and
    /// whose HTML is the whole doc.
    Node {
        node: Option<String>,
        before: Option<String>,
        after: Option<String>,
    },
    /// A board of a design. `before_at` and `after_at` are the `seq` to read
    /// the board at with `/boards/{board}?at=`, `null` for a board the write
    /// added (`before_at`) or removed (`after_at`).
    Board {
        board: String,
        before_at: Option<i64>,
        after_at: Option<i64>,
    },
    /// A component of a design system, to read at `/components/{component}?at=`
    /// as boards are.
    Component {
        component: String,
        before_at: Option<i64>,
        after_at: Option<i64>,
    },
}

#[derive(Serialize, ToSchema)]
pub struct TitleChange {
    pub before: String,
    pub after: String,
}

#[derive(Serialize, ToSchema)]
pub struct TokensChange {
    /// DTCG JSON of the tokens the write changed or removed, as they were, in
    /// their groups and with every mode; `null` for a design system that was
    /// just made. For a deleted design system, every token.
    #[schema(value_type = Option<Object>)]
    pub before: Option<Value>,
    /// DTCG JSON of the tokens the write added or changed, as they are now;
    /// `null` for a design system that was deleted.
    #[schema(value_type = Option<Object>)]
    pub after: Option<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct ActivityDetail {
    #[serde(flatten)]
    pub entry: ActivityEntry,
    /// What the write changed, in the order it touched it; empty for a rename.
    pub items: Vec<ActivityItem>,
    /// The old and new title; only on a rename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<TitleChange>,
    /// The tokens the write changed; only on a write to a design system that
    /// changed a token, made it or deleted it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokensChange>,
}

// Reading the log ---------------------------------------------------------------------

/// A row of `doc_changes` with its JSON as text.
#[derive(sqlx::FromRow)]
struct Change {
    seq: i64,
    agent: String,
    operations: String,
    head_before: Option<String>,
    head_after: String,
    boards: Option<String>,
    results: Option<String>,
    created_at: String,
}

const CHANGE_COLUMNS: &str = "seq, agent::text AS agent, operations::text AS operations, \
     head_before, head_after, boards::text AS boards, results::text AS results, \
     to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at";

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

impl Change {
    fn operations(&self) -> Vec<Value> {
        match json_of(&self.operations) {
            Value::Array(operations) => operations,
            _ => Vec::new(),
        }
    }

    /// A doc's creation, rename and deletion are each logged as one operation
    /// without a `node`; a node `delete` names its node and is an edit.
    fn kind(&self) -> &'static str {
        let operations = self.operations();
        let Some(first) = operations.first() else {
            return "edit";
        };
        if first.get("node").is_some() {
            return "edit";
        }
        match first["op"].as_str() {
            Some("create") => "create",
            Some("rename") => "rename",
            Some("delete") => "delete",
            _ => "edit",
        }
    }
}

/// The titles a doc has had: its creation and each rename, as (seq, title).
async fn titles_of(tx: &mut crate::docs::Tx, doc_id: &str) -> Result<Vec<(i64, String)>, ApiError> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT seq, COALESCE(operations->0->>'title', '') FROM doc_changes \
         WHERE doc_id = $1::uuid AND operations->0->>'op' IN ('create', 'rename') ORDER BY seq",
    )
    .bind(doc_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(rows)
}

/// The title the doc had before write `seq`.
fn title_before(titles: &[(i64, String)], seq: i64) -> String {
    titles
        .iter()
        .rev()
        .find(|(at, _)| *at < seq)
        .map(|(_, title)| title.clone())
        .unwrap_or_default()
}

fn title_after(change: &Change) -> String {
    change
        .operations()
        .first()
        .and_then(|operation| operation["title"].as_str())
        .unwrap_or_default()
        .to_string()
}

// Summaries ---------------------------------------------------------------------------

fn noun(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn line(parts: Vec<String>) -> String {
    if parts.is_empty() {
        "No changes".to_string()
    } else {
        parts.join(", ")
    }
}

/// The summary of a batch of operations on an HTML doc: the distinct nodes it
/// changed, the nodes it inserted, deleted and moved. A node that is deleted
/// counts as deleted, one that is changed (even if it is moved too) as changed.
fn html_summary(operations: &[Value]) -> String {
    let (mut inserted, mut changed, mut moved, mut deleted) =
        (0, BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for operation in operations {
        let node = operation["node"].as_str().unwrap_or_default();
        match operation["op"].as_str() {
            Some("insert") => inserted += 1,
            Some("replace" | "set") => {
                changed.insert(node);
            }
            Some("move") => {
                moved.insert(node);
            }
            Some("delete") => {
                deleted.insert(node);
            }
            _ => {}
        }
    }
    let changed: BTreeSet<_> = changed.difference(&deleted).copied().collect();
    let moved: BTreeSet<_> = moved
        .difference(&deleted)
        .copied()
        .filter(|node| !changed.contains(node))
        .collect();
    let mut parts = Vec::new();
    if !changed.is_empty() {
        parts.push(format!("{} changed", noun(changed.len(), "node", "nodes")));
    }
    for (count, what) in [
        (inserted, "inserted"),
        (deleted.len(), "deleted"),
        (moved.len(), "moved"),
    ] {
        if count > 0 {
            parts.push(format!("{count} {what}"));
        }
    }
    line(parts)
}

fn design_summary(boards: &[Value]) -> String {
    let (mut changed, mut added, mut removed) = (0, 0, 0);
    for board in boards {
        match (board["before"].is_null(), board["after"].is_null()) {
            (true, _) => added += 1,
            (_, true) => removed += 1,
            _ => changed += 1,
        }
    }
    let mut parts = Vec::new();
    if changed > 0 {
        parts.push(format!("{} changed", noun(changed, "board", "boards")));
    }
    if added > 0 {
        parts.push(format!("{added} added"));
    }
    if removed > 0 {
        parts.push(format!("{removed} removed"));
    }
    line(parts)
}

// The design system -------------------------------------------------------------------

/// What a design system's head holds, as the activity compares it: the JSON of
/// each component by ID, and each token (an object with a `$value`) by its
/// path through its groups.
#[derive(Default, Clone)]
struct SystemContent {
    components: Vec<(String, Value)>,
    tokens: BTreeMap<Vec<String>, Value>,
    /// The `$` properties of each group (such as its `$type`) by its path.
    groups: BTreeMap<Vec<String>, Map<String, Value>>,
}

fn collect_tokens(path: &mut Vec<String>, group: &Map<String, Value>, content: &mut SystemContent) {
    for (name, child) in group {
        let Some(object) = child.as_object().filter(|_| !name.starts_with('$')) else {
            continue;
        };
        path.push(name.clone());
        if object.contains_key("$value") {
            content.tokens.insert(path.clone(), child.clone());
        } else {
            let properties: Map<String, Value> = object
                .iter()
                .filter(|(key, _)| key.starts_with('$'))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            content.groups.insert(path.clone(), properties);
            collect_tokens(path, object, content);
        }
        path.pop();
    }
}

/// The tokens as a DTCG tree that holds only them, in their groups, each group
/// with its `$` properties as `groups` holds them.
fn token_tree<'a>(
    tokens: impl IntoIterator<Item = (&'a Vec<String>, &'a Value)>,
    groups: &BTreeMap<Vec<String>, Map<String, Value>>,
) -> Value {
    let mut root = Map::new();
    for (path, token) in tokens {
        let Some((name, path_groups)) = path.split_last() else {
            continue;
        };
        let mut level = &mut root;
        for (depth, group) in path_groups.iter().enumerate() {
            level = level
                .entry(group.clone())
                .or_insert_with(|| {
                    Value::Object(
                        groups
                            .get(&path_groups[..=depth])
                            .cloned()
                            .unwrap_or_default(),
                    )
                })
                .as_object_mut()
                .expect("a group is an object");
        }
        level.insert(name.clone(), token.clone());
    }
    Value::Object(root)
}

impl SystemContent {
    fn parse(text: &str, doc_id: &str) -> Result<Self, ApiError> {
        let what = format!("the head of the design system {doc_id}");
        let (tokens, components) = read_design_system(text, &what)?;
        let mut content = SystemContent::default();
        if let Some(tree) = tokens
            && let Value::Object(root) = tree.to_json()
        {
            collect_tokens(&mut Vec::new(), &root, &mut content);
        }
        content.components = components
            .iter()
            .map(|component| {
                (
                    component.id().to_string(),
                    serde_json::to_value(component).expect("a node serializes to JSON"),
                )
            })
            .collect();
        Ok(content)
    }
}

/// The difference between two heads of a design system.
struct SystemDiff {
    /// The components that are in only one head or differ: its ID, whether the
    /// head before has it and whether the head after has it.
    components: Vec<(String, bool, bool)>,
    /// The paths of the tokens that were added, changed or removed.
    tokens: Vec<Vec<String>>,
    before: Value,
    after: Value,
}

fn diff_systems(before: &SystemContent, after: &SystemContent) -> SystemDiff {
    let was: HashMap<&str, &Value> = before
        .components
        .iter()
        .map(|(id, json)| (id.as_str(), json))
        .collect();
    let is: HashMap<&str, &Value> = after
        .components
        .iter()
        .map(|(id, json)| (id.as_str(), json))
        .collect();
    let mut components = Vec::new();
    for (id, json) in &after.components {
        match was.get(id.as_str()) {
            Some(old) if *old == json => {}
            old => components.push((id.clone(), old.is_some(), true)),
        }
    }
    for (id, _) in &before.components {
        if !is.contains_key(id.as_str()) {
            components.push((id.clone(), true, false));
        }
    }
    let paths: BTreeSet<&Vec<String>> = before.tokens.keys().chain(after.tokens.keys()).collect();
    let mut tokens = Vec::new();
    let (mut was_tokens, mut is_tokens) = (Vec::new(), Vec::new());
    for path in paths {
        let (old, new) = (before.tokens.get(path), after.tokens.get(path));
        if old == new {
            continue;
        }
        tokens.push(path.clone());
        if let Some(old) = old {
            was_tokens.push((path, old));
        }
        if let Some(new) = new {
            is_tokens.push((path, new));
        }
    }
    SystemDiff {
        components,
        tokens,
        before: token_tree(was_tokens, &before.groups),
        after: token_tree(is_tokens, &after.groups),
    }
}

fn system_summary(diff: &SystemDiff) -> String {
    let mut parts = Vec::new();
    if !diff.components.is_empty() {
        parts.push(format!(
            "{} changed",
            noun(diff.components.len(), "component", "components")
        ));
    }
    if !diff.tokens.is_empty() {
        parts.push(format!(
            "{} changed",
            noun(diff.tokens.len(), "token", "tokens")
        ));
    }
    line(parts)
}

/// Reads the heads of a design system, each chunk once.
struct SystemReader<'a> {
    state: &'a AppState,
    workspace_id: &'a str,
    doc_id: &'a str,
    read: HashMap<Option<String>, SystemContent>,
}

impl<'a> SystemReader<'a> {
    fn new(state: &'a AppState, workspace_id: &'a str, doc_id: &'a str) -> Self {
        Self {
            state,
            workspace_id,
            doc_id,
            read: HashMap::new(),
        }
    }

    async fn head(&mut self, chunk: Option<&str>) -> Result<SystemContent, ApiError> {
        let key = chunk.map(str::to_string);
        if let Some(content) = self.read.get(&key) {
            return Ok(content.clone());
        }
        let content = match chunk {
            Some(chunk) if chunk != empty_head() => {
                let text = read_chunk(self.state, self.workspace_id, chunk).await?;
                SystemContent::parse(&text, self.doc_id)?
            }
            _ => SystemContent::default(),
        };
        self.read.insert(key, content.clone());
        Ok(content)
    }

    async fn diff(&mut self, change: &Change) -> Result<SystemDiff, ApiError> {
        let before = self.head(change.head_before.as_deref()).await?;
        let after = self.head(Some(&change.head_after)).await?;
        Ok(diff_systems(&before, &after))
    }
}

// HTML --------------------------------------------------------------------------------

async fn html_doc(
    state: &AppState,
    workspace_id: &str,
    doc_id: &str,
    chunk: Option<&str>,
) -> Result<EditDoc, ApiError> {
    let html = match chunk {
        Some(chunk) if chunk != empty_head() => read_chunk(state, workspace_id, chunk).await?,
        _ => String::new(),
    };
    EditDoc::from_html(&html).map_err(|e| {
        eprintln!("omni-api: the head {chunk:?} of doc {doc_id} cannot be loaded: {e}");
        ApiError::Unavailable
    })
}

/// The element of each node an operation of the write acted on, once each, in
/// the order the operations first touched them.
fn html_items(change: &Change, before: &EditDoc, after: &EditDoc) -> Vec<ActivityItem> {
    let operations = change.operations();
    let results = change.results.as_deref().map(json_of);
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        let id = results
            .as_ref()
            .and_then(|results| results[index]["node"].as_str())
            .or_else(|| operation["node"].as_str());
        let Some(id) = id else { continue };
        let Ok(node) = id.parse::<NodeId>() else {
            continue;
        };
        if !seen.insert(node) {
            continue;
        }
        let (was, is) = (before.node_html(node), after.node_html(node));
        if was.is_none() && is.is_none() {
            continue;
        }
        items.push(ActivityItem::Node {
            node: Some(id.to_string()),
            before: was,
            after: is,
        });
    }
    items
}

// The list ----------------------------------------------------------------------------

fn limit_of(query: &HashMap<String, String>) -> Result<i64, ApiError> {
    match query.get("limit") {
        None => Ok(DEFAULT_LIMIT),
        Some(limit) => limit
            .parse::<i64>()
            .ok()
            .filter(|limit| (1..=MAX_LIMIT).contains(limit))
            .ok_or_else(|| ApiError::bad_request("invalid_request")),
    }
}

fn before_of(query: &HashMap<String, String>) -> Result<Option<i64>, ApiError> {
    match query.get("before") {
        None => Ok(None),
        Some(before) => before
            .parse::<i64>()
            .ok()
            .filter(|before| *before >= 1)
            .map(Some)
            .ok_or_else(|| ApiError::bad_request("invalid_request")),
    }
}

fn entry_of(change: &Change, summary: String) -> ActivityEntry {
    ActivityEntry {
        seq: change.seq,
        kind: change.kind().to_string(),
        agent: serde_json::from_str(&change.agent).unwrap_or_default(),
        created_at: change.created_at.clone(),
        summary,
    }
}

/// The summary of a write that is not an edit.
fn plain_summary(change: &Change, titles: &[(i64, String)]) -> Option<String> {
    match change.kind() {
        "create" => Some("Created".to_string()),
        "delete" => Some("Deleted".to_string()),
        "rename" => Some(format!(
            "Renamed from '{}' to '{}'",
            title_before(titles, change.seq),
            title_after(change)
        )),
        _ => None,
    }
}

/// The summary of a write to a doc of `doc_kind`.
async fn summary_of(
    change: &Change,
    doc_kind: &str,
    titles: &[(i64, String)],
    system: &mut SystemReader<'_>,
) -> Result<String, ApiError> {
    if let Some(summary) = plain_summary(change, titles) {
        return Ok(summary);
    }
    Ok(match doc_kind {
        "design" => {
            let boards = json_of(change.boards.as_deref().unwrap_or("[]"));
            design_summary(boards.as_array().map_or(&[][..], Vec::as_slice))
        }
        "design-system" => system_summary(&system.diff(change).await?),
        _ => html_summary(&change.operations()),
    })
}

/// Lists the writes to a doc, newest first, a page at a time: one entry for
/// each row of the doc's log, with the agent's identity, the time and a
/// one-line summary.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/activity",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("limit" = Option<i64>, Query, description = "The most entries to list: 1 to 200, 50 by default"),
        ("before" = Option<i64>, Query, description = "List only the entries with a `seq` below this one: the `next_before` of the page before"),
    ),
    responses(
        (status = 200, description = "The doc's writes, newest first", body = ActivityListResponse),
        (status = 400, description = "`limit` or `before` is not a number in range (`invalid_request`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, or it was deleted", body = ErrorBody),
    )
)]
pub async fn list_activity(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<ActivityListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let (limit, before) = (limit_of(&query)?, before_of(&query)?);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let doc = find_doc(&mut tx, &project_id, &doc_id).await?;
    let mut changes: Vec<Change> = sqlx::query_as(&format!(
        "SELECT {CHANGE_COLUMNS} FROM doc_changes \
         WHERE doc_id = $1::uuid AND ($2::bigint IS NULL OR seq < $2) ORDER BY seq DESC LIMIT $3"
    ))
    .bind(&doc_id)
    .bind(before)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let titles = titles_of(&mut tx, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let more = changes.len() as i64 > limit;
    changes.truncate(limit as usize);

    let mut system = SystemReader::new(&state, &request.workspace_id, &doc_id);
    let mut entries = Vec::with_capacity(changes.len());
    for change in &changes {
        let summary = summary_of(change, &doc.kind, &titles, &mut system).await?;
        entries.push(entry_of(change, summary));
    }
    let next_before = more
        .then(|| changes.last().map(|change| change.seq))
        .flatten();
    Ok(Json(ActivityListResponse {
        entries,
        next_before,
    }))
}

// One entry ---------------------------------------------------------------------------

fn board_item(board: &str, before: bool, after: bool, seq: i64) -> ActivityItem {
    ActivityItem::Board {
        board: board.to_string(),
        before_at: before.then_some(seq - 1),
        after_at: after.then_some(seq),
    }
}

/// Answers one write to a doc: the entry of the list and what the write
/// changed. For an HTML doc each element the write acted on, as HTML before
/// and after. For a design the changed boards and the `seq` to read each at.
/// For a design system the changed components and the `seq` to read each at,
/// and the tokens that changed.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/activity/{seq}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("seq" = i64, Path, description = "The write's number in the doc"),
    ),
    responses(
        (status = 200, description = "The write", body = ActivityDetail),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, it was deleted, or it has no write with this `seq`", body = ErrorBody),
    )
)]
pub async fn show_activity(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, doc_id, seq)): Path<(String, String, String, String)>,
) -> Result<Json<ActivityDetail>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let seq = seq
        .parse::<i64>()
        .ok()
        .filter(|seq| *seq >= 1)
        .ok_or(ApiError::NotFound)?;
    let workspace_id = &request.workspace_id;
    let mut tx = state.workspace_tx(workspace_id).await?;
    let doc = find_doc(&mut tx, &project_id, &doc_id).await?;
    let change: Change = sqlx::query_as(&format!(
        "SELECT {CHANGE_COLUMNS} FROM doc_changes WHERE doc_id = $1::uuid AND seq = $2"
    ))
    .bind(&doc_id)
    .bind(seq)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    let titles = titles_of(&mut tx, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;

    let kind = change.kind();
    let mut detail = ActivityDetail {
        entry: entry_of(&change, String::new()),
        items: Vec::new(),
        title: None,
        tokens: None,
    };
    let creation = kind == "create";
    let deletion = kind == "delete";
    // The head the creation made, or the one the deletion removed.
    let whole = if deletion {
        change.head_before.as_deref()
    } else {
        Some(change.head_after.as_str())
    };
    match (kind, doc.kind.as_str()) {
        ("rename", _) => {
            detail.title = Some(TitleChange {
                before: title_before(&titles, seq),
                after: title_after(&change),
            });
        }
        ("create" | "delete", "html") => {
            let html = html_doc(&state, workspace_id, &doc_id, whole)
                .await?
                .to_html();
            let (before, after) = if creation {
                (None, Some(html))
            } else {
                (Some(html), None)
            };
            detail.items.push(ActivityItem::Node {
                node: None,
                before,
                after,
            });
        }
        ("create" | "delete", "design") => {
            let boards = match whole {
                Some(chunk) => boards::board_ids_of(&state, workspace_id, chunk).await?,
                None => Vec::new(),
            };
            detail.items = boards
                .iter()
                .map(|board| board_item(board, deletion, creation, seq))
                .collect();
        }
        ("create" | "delete", _) => {
            let content = SystemReader::new(&state, workspace_id, &doc_id)
                .head(whole)
                .await?;
            detail.items = content
                .components
                .iter()
                .map(|(id, _)| ActivityItem::Component {
                    component: id.clone(),
                    before_at: deletion.then_some(seq - 1),
                    after_at: creation.then_some(seq),
                })
                .collect();
            let full = Some(token_tree(content.tokens.iter(), &content.groups));
            detail.tokens = Some(if creation {
                TokensChange {
                    before: None,
                    after: full,
                }
            } else {
                TokensChange {
                    before: full,
                    after: None,
                }
            });
        }
        (_, "html") => {
            let before =
                html_doc(&state, workspace_id, &doc_id, change.head_before.as_deref()).await?;
            let after = html_doc(&state, workspace_id, &doc_id, Some(&change.head_after)).await?;
            detail.items = html_items(&change, &before, &after);
        }
        (_, "design") => {
            let boards = json_of(change.boards.as_deref().unwrap_or("[]"));
            detail.items = boards
                .as_array()
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .filter_map(|board| {
                    Some(board_item(
                        board["board"].as_str()?,
                        !board["before"].is_null(),
                        !board["after"].is_null(),
                        seq,
                    ))
                })
                .collect();
        }
        (_, _) => {
            let diff = SystemReader::new(&state, workspace_id, &doc_id)
                .diff(&change)
                .await?;
            detail.items = diff
                .components
                .iter()
                .map(|(id, before, after)| ActivityItem::Component {
                    component: id.clone(),
                    before_at: before.then_some(seq - 1),
                    after_at: after.then_some(seq),
                })
                .collect();
            if !diff.tokens.is_empty() {
                detail.tokens = Some(TokensChange {
                    before: Some(diff.before),
                    after: Some(diff.after),
                });
            }
        }
    }
    let mut system = SystemReader::new(&state, workspace_id, &doc_id);
    detail.entry.summary = summary_of(&change, &doc.kind, &titles, &mut system).await?;
    Ok(Json(detail))
}

// The chunk a write left ---------------------------------------------------------------

/// The head that write `seq` of a doc left, `None` when the doc has no such
/// write. This is what `?at=<seq>` reads.
pub(crate) async fn head_at(
    tx: &mut crate::docs::Tx,
    doc_id: &str,
    seq: &str,
) -> Result<String, ApiError> {
    let seq = seq
        .parse::<i64>()
        .ok()
        .filter(|seq| *seq >= 1)
        .ok_or(ApiError::NotFound)?;
    sqlx::query_scalar("SELECT head_after FROM doc_changes WHERE doc_id = $1::uuid AND seq = $2")
        .bind(doc_id)
        .bind(seq)
        .fetch_optional(&mut **tx)
        .await
        .map_err(ApiError::database)?
        .ok_or(ApiError::NotFound)
}

// One component -----------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
pub struct ComponentBody {
    /// The project's design system doc.
    pub doc_id: String,
    pub component_id: String,
    /// The `seq` that was read; `null` for the head.
    pub at: Option<i64>,
    /// The component and its subtree in the tag format.
    pub tags: String,
    /// The same component as a JSON node.
    #[schema(value_type = Object)]
    pub node: Value,
}

/// Reads one component of the project's design system: its head, or with
/// `?at=<seq>` the component as it was right after write `seq` of the design
/// system. Only the component's own content is versioned: the tokens and
/// components it refers to are those of the head.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/components/{component_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("component_id" = String, Path, description = "The component's node ID"),
        ("at" = Option<i64>, Query, description = "Read the component as it was right after this write of the design system's activity, instead of its head"),
    ),
    responses(
        (status = 200, description = "The component", body = ComponentBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "The project has no design system, the design system has no write `at`, or the head or that write has no such component (`unknown_component`)", body = ErrorBody),
    )
)]
pub async fn read_component(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, component_id)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<ComponentBody>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let (doc_id, head): (String, Option<String>) = sqlx::query_as(
        "SELECT id::text, head FROM docs \
         WHERE project_id = $1::uuid AND kind = 'design-system' AND deleted_at IS NULL",
    )
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    let (at, chunk) = match query.get("at") {
        Some(at) => {
            let chunk = head_at(&mut tx, &doc_id, at).await?;
            (at.parse::<i64>().ok(), Some(chunk))
        }
        None => (None, head),
    };
    tx.commit().await.map_err(ApiError::database)?;
    let unknown = || ApiError::Refused(axum::http::StatusCode::NOT_FOUND, "unknown_component");
    let component = component_id.parse::<NodeId>().map_err(|_| unknown())?;
    let system = design::read_system_head(
        &state,
        &request.workspace_id,
        &doc_id,
        chunk.as_deref().filter(|chunk| *chunk != empty_head()),
    )
    .await?;
    let node = system
        .library
        .into_components()
        .into_iter()
        .find(|node| *node.id() == component)
        .ok_or_else(unknown)?;
    Ok(Json(ComponentBody {
        doc_id,
        component_id: component.to_string(),
        at,
        tags: tags_of(std::slice::from_ref(&node)),
        node: serde_json::to_value(&node).expect("a node serializes to JSON"),
    }))
}
