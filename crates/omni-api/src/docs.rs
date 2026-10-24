//! Docs and their drafts. A doc belongs to a project; a change set edits it
//! through its own draft, with batches of operations (docs/formats/operations.md)
//! that the operation engine of omni-core applies all or nothing. A doc has a
//! head, the chunk of its content now (US-102): a new doc is readable at once
//! as the empty doc of its kind, and `create_app_doc` and `create_feature_doc`
//! make one with no change set. A doc started in a change set (`create_doc`)
//! has the empty head too, and gets that change set's draft; its first version
//! comes through the change set (Q18). Every write to a doc is a row of
//! `doc_changes`.
//!
//! A draft is kept in memory, keyed by workspace and draft, because an edit
//! must take well under 50 ms (FR-48). Postgres holds the truth: every applied
//! operation is a row of `draft_ops` (with the agent and the IDs the server
//! assigned), written in the transaction of the request, and a draft that is
//! not in memory (after a restart) is rebuilt by replaying its log. One lock
//! per draft orders the batches of agents that edit the same draft at once.

use crate::AppState;
use crate::boards;
use crate::changesets::{Scope, non_blank, parse_body, require_project};
use crate::claims::{self, ClaimBody};
use crate::design::{self, Library};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::versions::{self, Snapshot};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid};
use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use omni_core::design::tokens::TokenTree;
use omni_core::design::{DesignDoc, Node, NodeId};
use omni_core::hash::blake3_hex;
use omni_core::ops::{ApplyError, Conflict, EditDoc, Operation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use utoipa::ToSchema;

pub(crate) type Tx = Transaction<'static, Postgres>;

/// The columns of a doc; `created_at` is ISO 8601 text in UTC.
const DOC_COLUMNS: &str = "id::text, project_id::text, kind, pillar, title, \
     to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at, \
     mainline_version_id::text, feature_id::text, head";

const DRAFT_COLUMNS: &str = "id::text, doc_id::text, changeset_id::text, base_version_id::text";

/// The columns of a `DraftRow` for a query that names the draft table `d`.
pub(crate) const DRAFT_ROW_COLUMNS: &str = "d.id::text AS id, d.doc_id::text AS doc_id, \
     d.changeset_id::text AS changeset_id, d.base_version_id::text AS base_version_id, \
     d.content_version_id::text AS content_version_id, d.content_after_seq AS content_after_seq";

/// The alphabet of node IDs: Crockford's base32 in lowercase.
const NODE_ID_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct DraftBody {
    pub id: String,
    pub doc_id: String,
    pub changeset_id: String,
    /// The version the draft started from; `null` for a new doc, whose first
    /// version comes through this change set.
    pub base_version_id: Option<String>,
}

#[derive(sqlx::FromRow)]
pub(crate) struct DocRow {
    pub(crate) id: String,
    project_id: String,
    pub(crate) kind: String,
    pillar: String,
    title: String,
    created_at: String,
    pub(crate) mainline_version_id: Option<String>,
    pub(crate) feature_id: Option<String>,
    /// The chunk of the doc's content now; `None` for a doc that was started in a
    /// change set before heads existed and never merged, which reads as empty.
    pub(crate) head: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DocBody {
    pub id: String,
    pub project_id: String,
    /// `html`, `design` or `design-system`.
    pub kind: String,
    /// `spec`, `ui`, `architecture`, `db` or `research`.
    pub pillar: String,
    pub title: String,
    pub created_at: String,
    /// `null` until a change set that holds the doc's first version merges.
    pub mainline_version_id: Option<String>,
    /// The feature the doc belongs to; `null` for a global doc of the project.
    /// Fixed when a change set creates the doc.
    pub feature_id: Option<String>,
    /// The doc's drafts, one per change set, oldest first.
    pub drafts: Vec<DraftBody>,
    /// The doc's live claims, oldest first.
    pub claims: Vec<ClaimBody>,
}

#[derive(Serialize, ToSchema)]
pub struct DocResponse {
    pub doc: DocBody,
}

#[derive(Serialize, ToSchema)]
pub struct DocListResponse {
    pub docs: Vec<DocBody>,
}

#[derive(Serialize, ToSchema)]
pub struct CreatedDocResponse {
    pub doc: DocBody,
    /// The change set's draft of the new doc: empty, and based on no version.
    pub draft: DraftBody,
}

#[derive(Serialize, ToSchema)]
pub struct DraftResponse {
    pub draft: DraftBody,
}

#[derive(Serialize, ToSchema)]
pub struct DraftWithHtml {
    #[serde(flatten)]
    pub draft: DraftBody,
    /// The draft as HTML: `data-omni-id` and `data-rev` (the node's revision)
    /// on every block; empty for a design doc, whose content is `tags` and
    /// `pages`.
    pub html: String,
    /// A design doc's pages as tags, with every ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    /// A design doc's pages as JSON nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub pages: Option<Vec<Value>>,
    /// The design system that a design doc's instances resolve against:
    /// `{"doc_id", "source", "draft_id", "version_id"}`, where `source` is
    /// `draft` (the change set's draft of the design system, with its
    /// `draft_id`), `mainline` (the mainline version, with its `version_id`) or
    /// `null` (no components yet).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub design_system: Option<Value>,
    /// The components that a design doc's instances use, as resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub components: Option<Vec<Value>>,
    /// The token tree of the design system that a design doc's instances
    /// resolve against: the change set's draft of it, unsaved changes
    /// included, else the mainline version's. Absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub tokens: Option<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct DraftHtmlResponse {
    pub draft: DraftWithHtml,
}

#[derive(Serialize, ToSchema)]
pub struct MainlineHtmlResponse {
    pub doc_id: String,
    /// The version that was read. For the doc's head, the mainline version when
    /// the head is its content; `null` for a head that no version holds.
    pub version_id: Option<String>,
    /// The head or version as HTML with `data-omni-id`; the empty text for an
    /// empty doc, and for a design's head, whose content is `tags` and `pages`.
    pub html: String,
    /// A design's head as tags, one `<Page>` after the other; only for the
    /// head of a design.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    /// A design's head as JSON nodes, each page with its boards under
    /// `children`; only for the head of a design.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub pages: Option<Vec<Value>>,
    /// The token tree of the project's design system head, in every mode; only
    /// for the head of a design, and left out when the design system has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub tokens: Option<Value>,
    /// The components that the design's instances use, as the design system's
    /// head defines them; only for the head of a design.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub components: Option<Vec<Value>>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateDocRequest {
    /// `html`, `design` or `design-system`.
    pub kind: Option<String>,
    /// `spec`, `ui`, `architecture`, `db` or `research`; the design kinds only go in `ui`.
    pub pillar: Option<String>,
    pub title: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct EditRequest {
    /// A batch of operations: `insert`, `replace`, `set`, `move` and `delete`.
    #[schema(value_type = Vec<Object>)]
    pub ops: Vec<Value>,
    /// For a design: the ID of a board that every operation must stay inside
    /// (its node, and its parent for an insert or a move, are the board or in
    /// it). An ID that is no board of the doc is 404 `unknown_board`.
    pub board: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct OpResultBody {
    /// The node the operation inserted, replaced, set, moved or deleted.
    pub node: String,
    /// The node's revision after the operation.
    pub revision: u64,
    /// The IDs the server assigned to new nodes, in document order.
    pub created: Vec<String>,
    /// The nodes the operation removed, in document order.
    pub removed: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct EditResponse {
    /// One result per operation, in order.
    pub results: Vec<OpResultBody>,
}

#[derive(Serialize, ToSchema)]
pub struct ConflictDetails {
    /// The position of the operation in the batch, counting from 0.
    pub index: usize,
    pub node: String,
    /// The revision the operation listed.
    pub expected: u64,
    /// The revision the node has now.
    pub current_revision: u64,
    /// The node as it is now: HTML with `data-omni-id` for an HTML doc, tags
    /// with `rev` for a design.
    pub current_html: String,
}

#[derive(Serialize, ToSchema)]
pub struct ConflictBody {
    /// `conflict`
    pub error: String,
    pub conflict: ConflictDetails,
}

#[derive(Serialize, ToSchema)]
pub struct InvalidOperationBody {
    /// `invalid_operation`
    pub error: String,
    /// The position of the operation in the batch, counting from 0.
    pub index: usize,
    /// What is wrong; it names an ID that is not in the draft.
    pub message: String,
}

/// An error of an edit or a claim: the usual ones, a revision conflict, an
/// operation that the engine refuses, or a claim of another agent in the way.
/// Each of the last three changes nothing.
pub enum DocError {
    Api(ApiError),
    Conflict(Conflict),
    Invalid {
        index: usize,
        message: String,
    },
    /// A claim of another agent is in the way (US-023); nothing changed.
    Claimed {
        /// The operation that touches the claim; `None` for a claim request.
        index: Option<usize>,
        message: String,
        claim: Box<ClaimBody>,
    },
}

impl From<ApiError> for DocError {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for DocError {
    fn into_response(self) -> Response {
        match self {
            Self::Api(error) => error.into_response(),
            Self::Conflict(conflict) => (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "conflict",
                    "conflict": {
                        "index": conflict.index,
                        "node": conflict.node,
                        "expected": conflict.expected,
                        "current_revision": conflict.current_revision,
                        "current_html": conflict.current_html,
                    }
                })),
            )
                .into_response(),
            Self::Invalid { index, message } => (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "invalid_operation",
                    "index": index,
                    "message": message,
                })),
            )
                .into_response(),
            Self::Claimed {
                index,
                message,
                claim,
            } => {
                let mut body = json!({ "error": "claimed", "message": message, "claim": claim });
                if let Some(index) = index {
                    body["index"] = json!(index);
                }
                (StatusCode::CONFLICT, Json(body)).into_response()
            }
        }
    }
}

// The agent -----------------------------------------------------------------------

/// The agent identity that a request declares in `X-Omni-Run`, `X-Omni-Role`,
/// `X-Omni-Model` and `X-Omni-Harness`. The server records what the agent
/// declares and does not verify it; a header that is missing is `null`.
pub(crate) struct AgentIdentity {
    run: Option<String>,
    role: Option<String>,
    model: Option<String>,
    harness: Option<String>,
}

/// The longest identity value that is kept.
const MAX_IDENTITY_CHARS: usize = 200;

impl AgentIdentity {
    /// The run the agent declares, `None` when it declares none.
    pub(crate) fn run(&self) -> Option<&str> {
        self.run.as_deref()
    }

    /// The role the agent declares, `None` when it declares none.
    pub(crate) fn role(&self) -> Option<&str> {
        self.role.as_deref()
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "run": self.run,
            "role": self.role,
            "model": self.model,
            "harness": self.harness,
        })
    }
}

impl<S: Send + Sync> FromRequestParts<S> for AgentIdentity {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let header = |name: &str| {
            parts
                .headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.trim().chars().take(MAX_IDENTITY_CHARS).collect())
                .filter(|value: &String| !value.is_empty())
        };
        Ok(Self {
            run: header("x-omni-run"),
            role: header("x-omni-role"),
            model: header("x-omni-model"),
            harness: header("x-omni-harness"),
        })
    }
}

// Drafts in memory -----------------------------------------------------------------

/// A draft that is in memory: the doc under edit and the `seq` of the next
/// operation of its log. The draft of a design system doc holds no blocks: its
/// content is the token tree of the last import (`None` before one) and the
/// components of the last components import, kept in the log as `import_tokens`
/// and `import_components` operations and in a version as one chunk of JSON.
/// The draft of a design doc holds pages, which an `import_design` operation
/// of the log replaces and the operations of the log change node by node
/// (US-031); a version holds them as a manifest of chunks.
pub(crate) struct DraftState {
    pub(crate) doc: EditDoc,
    pub(crate) design_system: bool,
    pub(crate) design: bool,
    pub(crate) tokens: Option<TokenTree>,
    /// The components of a design system draft, in order.
    pub(crate) components: Vec<Node>,
    /// The pages of a design draft, in order, with the revision of each node:
    /// 1 for a node of the version the draft started from or one an import or
    /// an insert added, and one more for each later import or operation that
    /// changed the node's own content.
    pub(crate) design_doc: DesignDoc,
    pub(crate) next_seq: i64,
}

impl DraftState {
    /// What a version of the draft holds, with its node index. `None` for the
    /// draft of a design system that has neither tokens nor components yet.
    pub(crate) fn snapshot(&self) -> Option<Snapshot> {
        if self.design_system {
            Some(Snapshot::new(self.content()?, Vec::new()))
        } else if self.design {
            Some(boards::snapshot_of(self.design_doc.pages()))
        } else {
            Some(Snapshot::new(self.doc.to_html(), self.doc.index()))
        }
    }

    /// Whether the draft has the node.
    pub(crate) fn contains(&self, node: NodeId) -> bool {
        if self.design {
            self.design_doc.contains(node)
        } else {
            self.doc.contains(node)
        }
    }

    /// Whether the node can be claimed: a board of a design, a section of an
    /// HTML doc.
    pub(crate) fn can_claim(&self, node: NodeId) -> bool {
        if self.design {
            self.design_doc.is_board(node)
        } else {
            self.doc.is_section(node)
        }
    }

    /// The draft's nodes, for the claims on them.
    pub(crate) fn tree(&self) -> &dyn claims::Tree {
        if self.design {
            &self.design_doc
        } else {
            &self.doc
        }
    }

    /// The chunk text of a version of the draft as it is now.
    pub(crate) fn content(&self) -> Option<String> {
        if self.design_system {
            if self.tokens.is_none() && self.components.is_empty() {
                return None;
            }
            Some(design::design_system_text(
                self.tokens.as_ref(),
                &self.components,
            ))
        } else if self.design {
            Some(boards::content_of(self.design_doc.pages()))
        } else {
            Some(self.doc.to_html())
        }
    }
}

pub(crate) type Slot = Arc<tokio::sync::Mutex<Option<DraftState>>>;
pub(crate) type UsedIds = Arc<Mutex<HashSet<NodeId>>>;

/// The drafts that are in memory, and the node IDs of every project in use.
/// Held by the `AppState`; clones share it.
#[derive(Clone, Default)]
pub struct DraftStore {
    /// One slot per (workspace, draft). Its lock orders the batches of the
    /// draft, and it holds `None` until the draft is loaded, and again after a
    /// write that failed in the database, so the next request reloads it.
    drafts: Arc<Mutex<HashMap<(String, String), Slot>>>,
    /// The node IDs of each (workspace, project): IDs are unique within a
    /// project, so a new ID must be new to every doc of it.
    node_ids: Arc<tokio::sync::Mutex<HashMap<(String, String), UsedIds>>>,
}

impl DraftStore {
    pub(crate) fn slot(&self, workspace_id: &str, draft_id: &str) -> Slot {
        let mut drafts = self.drafts.lock().unwrap_or_else(|e| e.into_inner());
        drafts
            .entry((workspace_id.to_string(), draft_id.to_string()))
            .or_default()
            .clone()
    }

    /// The IDs in use in a project, read from the logs of its drafts the first
    /// time the project needs them.
    pub(crate) async fn used_ids(
        &self,
        tx: &mut Tx,
        workspace_id: &str,
        project_id: &str,
    ) -> Result<UsedIds, ApiError> {
        let mut projects = self.node_ids.lock().await;
        let key = (workspace_id.to_string(), project_id.to_string());
        if let Some(used) = projects.get(&key) {
            return Ok(used.clone());
        }
        let known: Vec<String> = sqlx::query_scalar(
            "SELECT unnest(o.created) FROM draft_ops o \
             JOIN drafts d ON d.workspace_id = o.workspace_id AND d.id = o.draft_id \
             JOIN docs x ON x.workspace_id = d.workspace_id AND x.id = d.doc_id \
             WHERE x.project_id = $1::uuid",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(ApiError::database)?;
        let used: HashSet<NodeId> = known.iter().filter_map(|id| id.parse().ok()).collect();
        let used = Arc::new(Mutex::new(used));
        projects.insert(key, used.clone());
        Ok(used)
    }
}

/// A fresh random node ID.
pub(crate) fn random_node_id() -> NodeId {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    // 256 is a multiple of 32, so every character is equally likely.
    let id: String = bytes
        .iter()
        .map(|byte| NODE_ID_ALPHABET[usize::from(*byte) % NODE_ID_ALPHABET.len()] as char)
        .collect();
    id.parse().expect("the alphabet gives a node ID")
}

/// The token tree that a saved design system version holds, `None` when it
/// has components only.
pub(crate) fn read_tokens(text: &str, what: &str) -> Result<Option<TokenTree>, ApiError> {
    design::read_design_system(text, what).map(|(tokens, _)| tokens)
}

/// Rebuilds a draft from its base version and its log. The log holds the IDs
/// that the server assigned, which the replay hands out again in the same order.
pub(crate) async fn load_state(
    state: &AppState,
    tx: &mut Tx,
    workspace_id: &str,
    draft: &DraftRow,
) -> Result<DraftState, ApiError> {
    let kind: String = sqlx::query_scalar("SELECT kind FROM docs WHERE id = $1::uuid")
        .bind(&draft.doc_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::database)?;
    let design_system = kind == "design-system";
    let design = kind == "design";
    // A rebased draft starts from the version the rebase saved, a draft that
    // was never rebased from its base.
    let start = draft
        .content_version_id
        .as_ref()
        .or(draft.base_version_id.as_ref());
    let mut doc = EditDoc::from_html("").expect("an empty doc parses");
    let mut tokens = None;
    let mut components = Vec::new();
    let mut pages = Vec::new();
    if let Some(base) = start {
        let chunk = versions::chunk_of_version(tx, &draft.doc_id, base).await?;
        let html = match (&chunk, design) {
            (Some(chunk), false) => versions::read_chunk(state, workspace_id, chunk).await?,
            _ => String::new(),
        };
        if design_system {
            if !html.is_empty() {
                let what = format!("the base version {base} of draft {}", draft.id);
                let (tree, nodes) = design::read_design_system(&html, &what)?;
                tokens = tree;
                components = nodes;
            }
        } else if design {
            if let Some(chunk) = &chunk {
                let what = format!("the base version {base} of draft {}", draft.id);
                pages = boards::read_pages(state, workspace_id, chunk, &what).await?;
            }
        } else {
            doc = EditDoc::from_html(&html).map_err(|e| {
                eprintln!(
                    "omni-api: the base version {base} of draft {} cannot be loaded: {e}",
                    draft.id
                );
                ApiError::Unavailable
            })?;
        }
    }
    let rows: Vec<(i64, String, Vec<String>)> = sqlx::query_as(
        "SELECT seq, op::text, created FROM draft_ops \
         WHERE draft_id = $1::uuid AND seq > $2 ORDER BY seq",
    )
    .bind(&draft.id)
    .bind(draft.content_after_seq)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let corrupt = |seq: i64, why: &str| {
        eprintln!(
            "omni-api: cannot replay operation {seq} of draft {}: {why}",
            draft.id
        );
        ApiError::Unavailable
    };
    let revisions = boards::initial_revisions(&pages);
    let mut design_doc = DesignDoc::new(pages, revisions);
    let mut next_seq = draft.content_after_seq + 1;
    for (seq, op, created) in rows {
        if design_system {
            if let Some(imported) = crate::tokens::imported_in(&op) {
                tokens = Some(
                    TokenTree::from_json(&imported).map_err(|e| corrupt(seq, &e.to_string()))?,
                );
                next_seq = seq + 1;
                continue;
            }
            if let Some(imported) = design::imported_components(&op) {
                components = imported.map_err(|e| corrupt(seq, &e))?;
                next_seq = seq + 1;
                continue;
            }
        }
        if design && let Some(imported) = design::imported_pages(&op) {
            let imported = imported.map_err(|e| corrupt(seq, &e))?;
            let revisions =
                boards::next_revisions(design_doc.pages(), design_doc.revisions(), &imported);
            design_doc = DesignDoc::new(imported, revisions);
            next_seq = seq + 1;
            continue;
        }
        let op: Operation = serde_json::from_str(&op).map_err(|e| corrupt(seq, &e.to_string()))?;
        let mut ids: VecDeque<NodeId> = created.iter().filter_map(|id| id.parse().ok()).collect();
        let mut ran_out = false;
        let mut next_id = || {
            ids.pop_front().unwrap_or_else(|| {
                ran_out = true;
                random_node_id()
            })
        };
        // The operations were checked against the design system when they were
        // applied, which the replay does not need again.
        let applied = if design {
            design_doc.apply(&[op], None, &mut next_id, &|_| Ok(()))
        } else {
            doc.apply(&[op], &mut next_id)
        };
        if let Err(error) = applied {
            return Err(corrupt(seq, &error.to_string()));
        }
        if ran_out {
            return Err(corrupt(seq, "it created more nodes than the log records"));
        }
        next_seq = seq + 1;
    }
    Ok(DraftState {
        doc,
        design_system,
        design,
        tokens,
        components,
        design_doc,
        next_seq,
    })
}

/// Loads the draft into its locked slot when it is not in memory.
pub(crate) async fn ensure_loaded(
    state: &AppState,
    workspace_id: &str,
    draft: &DraftRow,
    held: &mut Option<DraftState>,
) -> Result<(), ApiError> {
    if held.is_none() {
        let mut tx = state.workspace_tx(workspace_id).await?;
        *held = Some(load_state(state, &mut tx, workspace_id, draft).await?);
        tx.commit().await.map_err(ApiError::database)?;
    }
    Ok(())
}

// Looking things up ----------------------------------------------------------------

#[derive(sqlx::FromRow)]
pub(crate) struct DraftRow {
    pub(crate) id: String,
    pub(crate) doc_id: String,
    pub(crate) changeset_id: String,
    pub(crate) base_version_id: Option<String>,
    /// The version a rebase saved as the draft's content, `None` before one.
    pub(crate) content_version_id: Option<String>,
    /// The draft's content takes the operations of the log after this `seq`.
    pub(crate) content_after_seq: i64,
}

impl From<DraftRow> for DraftBody {
    fn from(row: DraftRow) -> Self {
        Self {
            id: row.id,
            doc_id: row.doc_id,
            changeset_id: row.changeset_id,
            base_version_id: row.base_version_id,
        }
    }
}

/// The draft's row as it is now. A request reads the row before it waits for
/// the draft's lock, and a rebase that held the lock meanwhile has rewritten
/// the row, so a request takes the row again once it holds the lock.
pub(crate) async fn current_draft(tx: &mut Tx, draft_id: &str) -> Result<DraftRow, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {DRAFT_ROW_COLUMNS} FROM drafts d WHERE d.id = $1::uuid"
    ))
    .bind(draft_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)
}

/// The draft that `changeset_id` has of `doc_id`, 404 when the change set, the
/// doc or the draft is not there (or not in this project).
pub(crate) async fn find_draft(
    tx: &mut Tx,
    project_id: &str,
    changeset_id: &str,
    doc_id: &str,
) -> Result<DraftRow, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {DRAFT_ROW_COLUMNS} \
         FROM drafts d \
         JOIN docs x ON x.workspace_id = d.workspace_id AND x.id = d.doc_id \
         JOIN changesets c ON c.workspace_id = d.workspace_id AND c.id = d.changeset_id \
         WHERE d.changeset_id = $1::uuid AND d.doc_id = $2::uuid \
           AND x.project_id = $3::uuid AND c.project_id = $3::uuid"
    ))
    .bind(changeset_id)
    .bind(doc_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)
}

pub(crate) async fn require_changeset(
    tx: &mut Tx,
    project_id: &str,
    changeset_id: &str,
) -> Result<(), ApiError> {
    changeset_feature(tx, project_id, changeset_id)
        .await
        .map(|_| ())
}

/// The feature that the change set belongs to, `None` for a project-level one;
/// 404 when the change set is not in this project or its feature is deleted.
async fn changeset_feature(
    tx: &mut Tx,
    project_id: &str,
    changeset_id: &str,
) -> Result<Option<String>, ApiError> {
    let found: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT c.feature_id::text FROM changesets c LEFT JOIN features f ON f.id = c.feature_id \
         WHERE c.id = $1::uuid AND c.project_id = $2::uuid AND f.deleted_at IS NULL",
    )
    .bind(changeset_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found
        .map(|(feature_id,)| feature_id)
        .ok_or(ApiError::NotFound)
}

/// A change set may draft the global docs and the docs of its own feature
/// (FR-54): a project-level one drafts global docs only. Another scope is 422
/// `doc_in_other_feature`.
async fn check_doc_scope(
    tx: &mut Tx,
    doc_feature: Option<&str>,
    changeset_id: &str,
) -> Result<(), ApiError> {
    let Some(doc_feature) = doc_feature else {
        return Ok(());
    };
    let same: bool = sqlx::query_scalar(
        "SELECT feature_id IS NOT DISTINCT FROM $2::uuid FROM changesets WHERE id = $1::uuid",
    )
    .bind(changeset_id)
    .bind(doc_feature)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    if same {
        Ok(())
    } else {
        Err(ApiError::Refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "doc_in_other_feature",
        ))
    }
}

/// The ID of the draft that `changeset_id` has of `doc_id`. A change set that
/// has none gets one, based on the doc's mainline version (409
/// `no_mainline_version` when the doc has none, so that only its own change set
/// has a draft of it), and `draft.opened` is emitted; two requests that open it
/// at once get the same draft. A doc of another feature than the change set's is
/// 422 `doc_in_other_feature`.
pub(crate) async fn ensure_draft(
    tx: &mut Tx,
    workspace_id: &str,
    project_id: &str,
    doc_id: &str,
    changeset_id: &str,
    mainline: Option<&str>,
) -> Result<String, ApiError> {
    let doc_feature: Option<String> =
        sqlx::query_scalar("SELECT feature_id::text FROM docs WHERE id = $1::uuid")
            .bind(doc_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(ApiError::database)?;
    check_doc_scope(tx, doc_feature.as_deref(), changeset_id).await?;
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM drafts WHERE doc_id = $1::uuid AND changeset_id = $2::uuid",
    )
    .bind(doc_id)
    .bind(changeset_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    if let Some(draft_id) = existing {
        return Ok(draft_id);
    }
    let Some(mainline) = mainline else {
        return Err(ApiError::conflict("no_mainline_version"));
    };
    // The unique key lets one of two requests make the draft, and the other
    // reads it.
    let inserted: Option<String> = sqlx::query_scalar(
        "INSERT INTO drafts (workspace_id, doc_id, changeset_id, base_version_id) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid) \
         ON CONFLICT (workspace_id, doc_id, changeset_id) DO NOTHING \
         RETURNING id::text",
    )
    .bind(workspace_id)
    .bind(doc_id)
    .bind(changeset_id)
    .bind(mainline)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    match inserted {
        Some(draft_id) => {
            live::emit(
                tx,
                workspace_id,
                project_id,
                "draft.opened",
                &json!({
                    "doc_id": doc_id,
                    "changeset_id": changeset_id,
                    "draft_id": draft_id,
                    "base_version_id": mainline,
                }),
            )
            .await?;
            Ok(draft_id)
        }
        None => sqlx::query_scalar(
            "SELECT id::text FROM drafts WHERE doc_id = $1::uuid AND changeset_id = $2::uuid",
        )
        .bind(doc_id)
        .bind(changeset_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::database),
    }
}

pub(crate) async fn find_doc(
    tx: &mut Tx,
    project_id: &str,
    doc_id: &str,
) -> Result<DocRow, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {DOC_COLUMNS} FROM docs \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL"
    ))
    .bind(doc_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)
}

async fn drafts_of(tx: &mut Tx, doc_id: &str) -> Result<Vec<DraftBody>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {DRAFT_COLUMNS} FROM drafts WHERE doc_id = $1::uuid ORDER BY created_at, id"
    ))
    .bind(doc_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// The docs of a project in the scope, oldest first, each as `show_doc` answers
/// it. Callers sweep the expired claims first.
pub(crate) async fn docs_of_project(
    state: &AppState,
    tx: &mut Tx,
    workspace_id: &str,
    project_id: &str,
    scope: Scope<'_>,
    pillar: Option<&str>,
) -> Result<Vec<DocBody>, ApiError> {
    let all = matches!(scope, Scope::Project);
    let feature = match scope {
        Scope::Feature(id) => Some(id),
        _ => None,
    };
    let rows: Vec<DocRow> = sqlx::query_as(&format!(
        "SELECT {DOC_COLUMNS} FROM docs WHERE project_id = $1::uuid AND deleted_at IS NULL \
         AND ($2::bool OR feature_id IS NOT DISTINCT FROM $3::uuid) \
         AND ($4::text IS NULL OR pillar = $4) ORDER BY created_at, id"
    ))
    .bind(project_id)
    .bind(all)
    .bind(feature)
    .bind(pillar)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let mut docs = Vec::with_capacity(rows.len());
    for row in rows {
        let drafts = drafts_of(tx, &row.id).await?;
        let claims = state.claims.live_of_doc(workspace_id, &row.id);
        docs.push(doc_body(row, drafts, claims));
    }
    Ok(docs)
}

fn doc_body(row: DocRow, drafts: Vec<DraftBody>, claims: Vec<ClaimBody>) -> DocBody {
    DocBody {
        id: row.id,
        project_id: row.project_id,
        kind: row.kind,
        pillar: row.pillar,
        title: row.title,
        created_at: row.created_at,
        mainline_version_id: row.mainline_version_id,
        feature_id: row.feature_id,
        drafts,
        claims,
    }
}

/// The five pillars of a scope (FR-5).
const PILLARS: [&str; 5] = ["spec", "ui", "architecture", "db", "research"];

// Routes ---------------------------------------------------------------------------

/// A new doc as the request describes it, once its kind, pillar and title are
/// checked.
struct NewDoc {
    kind: String,
    pillar: String,
    title: String,
}

/// Checks the body of a request that creates a doc: 400 `invalid_request` for
/// a missing or unknown kind, pillar or title, 422 `design_needs_ui_pillar` for
/// a design kind outside `ui` and 422 `design_system_is_global` for a design
/// system in a feature.
fn check_new_doc(new: CreateDocRequest, in_feature: bool) -> Result<NewDoc, ApiError> {
    let invalid = || ApiError::bad_request("invalid_request");
    let kind = non_blank(new.kind).ok_or_else(invalid)?;
    let pillar = non_blank(new.pillar).ok_or_else(invalid)?;
    let title = non_blank(new.title).ok_or_else(invalid)?;
    if !matches!(kind.as_str(), "html" | "design" | "design-system")
        || !PILLARS.contains(&pillar.as_str())
    {
        return Err(invalid());
    }
    if kind != "html" && pillar != "ui" {
        return Err(ApiError::Refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "design_needs_ui_pillar",
        ));
    }
    if kind == "design-system" && in_feature {
        return Err(ApiError::Refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "design_system_is_global",
        ));
    }
    Ok(NewDoc {
        kind,
        pillar,
        title,
    })
}

fn design_system_exists() -> ApiError {
    ApiError::conflict("design_system_exists")
}

/// A project has one design system doc, whichever change set started it and
/// whether or not it merged: 409 `design_system_exists` when a second one is
/// asked for. The unique index of the migration settles two requests that
/// start one at once.
async fn require_no_design_system(
    tx: &mut Tx,
    project_id: &str,
    new: &NewDoc,
) -> Result<(), ApiError> {
    if new.kind != "design-system" {
        return Ok(());
    }
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM docs \
         WHERE project_id = $1::uuid AND kind = 'design-system' AND deleted_at IS NULL",
    )
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    match found {
        Some(_) => Err(design_system_exists()),
        None => Ok(()),
    }
}

/// What the head of an empty doc of `kind` holds: the empty text for an HTML
/// doc or a design system, a manifest of one page without boards for a design.
fn empty_snapshot(kind: &str) -> Snapshot {
    if kind != "design" {
        return Snapshot::new(String::new(), Vec::new());
    }
    let mut ids = HashSet::new();
    let mut next = || loop {
        let id = random_node_id();
        if ids.insert(id) {
            return id;
        }
    };
    let page = omni_core::design::tags::parse_all(r#"<Page name="Page 1"/>"#)
        .expect("the tag of an empty page parses")
        .remove(0)
        .assign_ids(&mut next);
    boards::snapshot_of(&[page])
}

/// The head of an empty doc of `kind`, with what its chunks hold, which the
/// caller lists in the chunk index in the transaction that writes the doc.
struct EmptyHead {
    /// The name of the head's chunk, the BLAKE3 hash of its text.
    name: String,
    snapshot: Snapshot,
    /// Whether the chunks are in RustFS.
    stored: bool,
}

/// Stores the chunks of an empty doc of `kind`. With `required` false, a
/// storage that cannot be reached leaves the chunks unstored and the doc with a
/// null head, which reads as the empty doc all the same: a doc started in a
/// change set does not need the storage until its first version.
async fn store_empty_head(
    state: &AppState,
    workspace_id: &str,
    kind: &str,
    required: bool,
) -> Result<EmptyHead, ApiError> {
    let snapshot = empty_snapshot(kind);
    let name = blake3_hex(snapshot.html.as_bytes());
    match versions::store_snapshot(state, workspace_id, &snapshot).await {
        Ok(_) => Ok(EmptyHead {
            name,
            snapshot,
            stored: true,
        }),
        Err(_) if !required => Ok(EmptyHead {
            name,
            snapshot,
            stored: false,
        }),
        Err(error) => Err(error),
    }
}

/// Adds the row of `doc_changes` for a write to a doc, in the transaction of the
/// write: the next `seq` of the doc, the agent, the operations, and the head
/// before and after. A write to one doc is serialised by the lock on the doc's
/// row, or by its being new.
pub(crate) async fn log_change(
    tx: &mut Tx,
    workspace_id: &str,
    doc_id: &str,
    agent: &Value,
    operations: &Value,
    head_before: Option<&str>,
    head_after: &str,
) -> Result<(), ApiError> {
    log_change_of_boards(
        tx,
        workspace_id,
        doc_id,
        agent,
        operations,
        head_before,
        head_after,
        None,
    )
    .await
}

/// Like `log_change`, for a write to a design: `boards` lists each board whose
/// chunk the write changed as `{board, before, after}`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn log_change_of_boards(
    tx: &mut Tx,
    workspace_id: &str,
    doc_id: &str,
    agent: &Value,
    operations: &Value,
    head_before: Option<&str>,
    head_after: &str,
    boards: Option<&Value>,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO doc_changes \
         (workspace_id, doc_id, seq, agent, operations, head_before, head_after, boards) \
         VALUES ($1::uuid, $2::uuid, \
                 (SELECT COALESCE(MAX(seq), 0) + 1 FROM doc_changes WHERE doc_id = $2::uuid), \
                 $3::text::jsonb, $4::text::jsonb, $5, $6, $7::text::jsonb)",
    )
    .bind(workspace_id)
    .bind(doc_id)
    .bind(agent.to_string())
    .bind(operations.to_string())
    .bind(head_before)
    .bind(head_after)
    .bind(boards.map(Value::to_string))
    .execute(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    Ok(())
}

/// Makes `versions[i]` the mainline version of `docs[i]` and its chunk the doc's
/// head, in the transaction of a merge whose docs are locked. A version that
/// removes its doc leaves the doc with the empty head of its kind. Each doc
/// whose head changes gets a row of `doc_changes`.
pub(crate) async fn set_mainlines(
    state: &AppState,
    tx: &mut Tx,
    workspace_id: &str,
    docs: &[&str],
    versions: &[&str],
) -> Result<(), ApiError> {
    #[derive(sqlx::FromRow)]
    struct Merging {
        doc_id: String,
        kind: String,
        head: Option<String>,
        chunk: Option<String>,
        version_id: String,
    }
    let merging: Vec<Merging> = sqlx::query_as(
        "SELECT d.id::text AS doc_id, d.kind, d.head, v.chunk, v.id::text AS version_id \
         FROM unnest($1::text[]::uuid[], $2::text[]::uuid[]) AS a (doc, version) \
         JOIN docs d ON d.id = a.doc JOIN versions v ON v.id = a.version \
         ORDER BY d.id",
    )
    .bind(docs)
    .bind(versions)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    let agent = json!({ "run": null, "role": "merge", "model": null, "harness": null });
    for doc in merging {
        let head = match doc.chunk {
            Some(chunk) => chunk,
            None => {
                let empty = store_empty_head(state, workspace_id, &doc.kind, true).await?;
                versions::index_snapshot(tx, workspace_id, &empty.name, &empty.snapshot).await?;
                empty.name
            }
        };
        sqlx::query(
            "UPDATE docs SET mainline_version_id = $2::uuid, head = $3 WHERE id = $1::uuid",
        )
        .bind(&doc.doc_id)
        .bind(&doc.version_id)
        .bind(&head)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::database)?;
        if doc.head.as_deref() != Some(head.as_str()) {
            log_change(
                tx,
                workspace_id,
                &doc.doc_id,
                &agent,
                &json!([{ "op": "merge", "version": doc.version_id }]),
                doc.head.as_deref(),
                &head,
            )
            .await?;
        }
    }
    Ok(())
}

/// Writes a new doc with its head: the row, the head's chunks in the chunk
/// index, the first row of `doc_changes` and the `doc.created` event, in the
/// transaction of the request. `changeset_id` is the change set that starts the
/// doc, `None` for a doc created live.
#[allow(clippy::too_many_arguments)]
async fn insert_doc(
    tx: &mut Tx,
    workspace_id: &str,
    project_id: &str,
    feature_id: Option<&str>,
    changeset_id: Option<&str>,
    new: &NewDoc,
    empty: &EmptyHead,
    agent: &Value,
) -> Result<DocRow, ApiError> {
    let row: DocRow = sqlx::query_as(&format!(
        "INSERT INTO docs (workspace_id, project_id, feature_id, kind, pillar, title, head) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, $7) RETURNING {DOC_COLUMNS}"
    ))
    .bind(workspace_id)
    .bind(project_id)
    .bind(feature_id)
    .bind(&new.kind)
    .bind(&new.pillar)
    .bind(&new.title)
    .bind(empty.stored.then_some(empty.name.as_str()))
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| {
        if error
            .as_database_error()
            .is_some_and(|e| e.is_unique_violation())
        {
            design_system_exists()
        } else {
            ApiError::database(error)
        }
    })?;
    if empty.stored {
        versions::index_snapshot(tx, workspace_id, &empty.name, &empty.snapshot).await?;
    }
    log_change(
        tx,
        workspace_id,
        &row.id,
        agent,
        &json!([{
            "op": "create",
            "kind": new.kind,
            "pillar": new.pillar,
            "title": new.title,
            "feature_id": feature_id,
        }]),
        None,
        &empty.name,
    )
    .await?;
    live::emit(
        tx,
        workspace_id,
        project_id,
        "doc.created",
        &json!({
            "doc_id": row.id,
            "changeset_id": changeset_id,
            "feature_id": feature_id,
            "kind": new.kind,
            "pillar": new.pillar,
            "title": new.title,
        }),
    )
    .await?;
    Ok(row)
}

/// Starts a new doc in a change set. The doc has the empty head of its kind
/// and no mainline version: its first version comes through the change set,
/// which gets an empty draft of it. The doc belongs to the change set's feature
/// (global for a project-level change set) and keeps that scope.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
    ),
    request_body = CreateDocRequest,
    responses(
        (status = 201, description = "The new doc and the change set's draft of it", body = CreatedDocResponse),
        (status = 400, description = "The kind, pillar or title is missing or not allowed", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
        (status = 409, description = "The project has a design system doc already (`design_system_exists`)", body = ErrorBody),
        (status = 422, description = "A design or design-system doc outside the `ui` pillar (`design_needs_ui_pillar`), or a design system in a change set that belongs to a feature, which is always global (`design_system_is_global`)", body = ErrorBody),
    )
)]
pub async fn create_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<CreatedDocResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<CreateDocRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let feature_id = changeset_feature(&mut tx, &project_id, &changeset_id).await?;
    let new = check_new_doc(new?, feature_id.is_some())?;
    require_no_design_system(&mut tx, &project_id, &new).await?;
    tx.commit().await.map_err(ApiError::database)?;

    let empty = store_empty_head(&state, &request.workspace_id, &new.kind, false).await?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row = insert_doc(
        &mut tx,
        &request.workspace_id,
        &project_id,
        feature_id.as_deref(),
        Some(&changeset_id),
        &new,
        &empty,
        &identity.to_json(),
    )
    .await?;
    let draft: DraftBody = sqlx::query_as(&format!(
        "INSERT INTO drafts (workspace_id, doc_id, changeset_id) \
         VALUES ($1::uuid, $2::uuid, $3::uuid) RETURNING {DRAFT_COLUMNS}"
    ))
    .bind(&request.workspace_id)
    .bind(&row.id)
    .bind(&changeset_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    let doc = doc_body(
        row,
        vec![DraftBody {
            id: draft.id.clone(),
            doc_id: draft.doc_id.clone(),
            changeset_id: draft.changeset_id.clone(),
            base_version_id: None,
        }],
        Vec::new(),
    );
    Ok((StatusCode::CREATED, Json(CreatedDocResponse { doc, draft })))
}

/// Creates a doc at the app level of a project or in one of its features,
/// without a change set. The doc is readable at once: its head is the empty
/// doc of its kind, and its creation is the first row of its log.
async fn create_live_doc(
    state: &AppState,
    request: &AgentRequest,
    identity: &AgentIdentity,
    project_id: &str,
    feature_id: Option<&str>,
    body: &Bytes,
) -> Result<(StatusCode, Json<DocResponse>), ApiError> {
    let new = parse_body::<CreateDocRequest>(body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, project_id).await?;
    if let Some(feature_id) = feature_id {
        let found: Option<(String,)> = sqlx::query_as(
            "SELECT id::text FROM features \
             WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL",
        )
        .bind(feature_id)
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        found.ok_or(ApiError::NotFound)?;
    }
    let new = check_new_doc(new?, feature_id.is_some())?;
    require_no_design_system(&mut tx, project_id, &new).await?;
    tx.commit().await.map_err(ApiError::database)?;

    let empty = store_empty_head(state, &request.workspace_id, &new.kind, true).await?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    if let Some(feature_id) = feature_id {
        // A feature that is deleted in between takes no new doc.
        let found: Option<(String,)> = sqlx::query_as(
            "SELECT id::text FROM features \
             WHERE id = $1::uuid AND deleted_at IS NULL FOR SHARE",
        )
        .bind(feature_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        found.ok_or(ApiError::NotFound)?;
    }
    let row = insert_doc(
        &mut tx,
        &request.workspace_id,
        project_id,
        feature_id,
        None,
        &new,
        &empty,
        &identity.to_json(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, project_id);
    let doc = doc_body(row, Vec::new(), Vec::new());
    Ok((StatusCode::CREATED, Json(DocResponse { doc })))
}

/// Creates a doc at the app level of a project, without a change set. The doc
/// is readable at once, as the empty doc of its kind.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = CreateDocRequest,
    responses(
        (status = 201, description = "The new doc", body = DocResponse),
        (status = 400, description = "The kind, pillar or title is missing or not allowed", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
        (status = 409, description = "The project has a design system doc already (`design_system_exists`)", body = ErrorBody),
        (status = 422, description = "A design or design-system doc outside the `ui` pillar (`design_needs_ui_pillar`)", body = ErrorBody),
    )
)]
pub async fn create_app_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<DocResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    create_live_doc(&state, &request, &identity, &project_id, None, &body).await
}

/// Creates a doc in a feature of a project, without a change set. The doc is
/// readable at once, as the empty doc of its kind.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/docs",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature_id" = String, Path, description = "The feature's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = CreateDocRequest,
    responses(
        (status = 201, description = "The new doc", body = DocResponse),
        (status = 400, description = "The kind, pillar or title is missing or not allowed", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such feature in this project and workspace", body = ErrorBody),
        (status = 422, description = "A design or design-system doc outside the `ui` pillar (`design_needs_ui_pillar`), or a design system in a feature, which is always global (`design_system_is_global`)", body = ErrorBody),
    )
)]
pub async fn create_feature_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, feature_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<DocResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature_id = parse_uuid(&feature_id).ok_or(ApiError::NotFound)?;
    create_live_doc(
        &state,
        &request,
        &identity,
        &project_id,
        Some(&feature_id),
        &body,
    )
    .await
}

/// The docs of a project, oldest first, each as `GET .../docs/{doc_id}` answers it.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("feature" = Option<String>, Query, description = "A feature's UUID lists that feature's docs only; `global` lists the project's global docs only; without it, every doc of the project"),
        ("pillar" = Option<String>, Query, description = "`spec`, `ui`, `architecture`, `db` or `research` lists that pillar's docs only; without it, every pillar"),
    ),
    responses(
        (status = 200, description = "The project's docs", body = DocListResponse),
        (status = 400, description = "`feature` is neither `global` nor a UUID, or `pillar` is none of the five pillars", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn list_docs(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<DocListResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let feature = match query.get("feature").map(String::as_str) {
        None => None,
        Some("global") => Some(None),
        Some(id) => Some(Some(
            parse_uuid(id).ok_or_else(|| ApiError::bad_request("invalid_request"))?,
        )),
    };
    let scope = match &feature {
        None => Scope::Project,
        Some(None) => Scope::Global,
        Some(Some(id)) => Scope::Feature(id),
    };
    let pillar = match query.get("pillar").map(String::as_str) {
        None => None,
        Some(pillar) if PILLARS.contains(&pillar) => Some(pillar),
        Some(_) => return Err(ApiError::bad_request("invalid_request")),
    };
    claims::sweep(&state, &request.workspace_id).await;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let docs = docs_of_project(
        &state,
        &mut tx,
        &request.workspace_id,
        &project_id,
        scope,
        pillar,
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(DocListResponse { docs }))
}

/// A doc's metadata, its mainline version, its drafts and its claims.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
    ),
    responses(
        (status = 200, description = "The doc", body = DocResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace", body = ErrorBody),
    )
)]
pub async fn show_doc(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
) -> Result<Json<DocResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    claims::sweep(&state, &request.workspace_id).await;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row = find_doc(&mut tx, &project_id, &doc_id).await?;
    let drafts = drafts_of(&mut tx, &row.id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let claims = state.claims.live_of_doc(&request.workspace_id, &row.id);
    Ok(Json(DocResponse {
        doc: doc_body(row, drafts, claims),
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct RenameDocRequest {
    /// The doc's new title; not blank.
    pub title: Option<String>,
}

/// The name in the log of a head that is empty: the chunk of the empty text.
/// A doc that has no head yet is logged with it.
pub(crate) fn empty_head() -> String {
    blake3_hex(b"")
}

/// Locks a doc of the project that is not deleted; 404 when there is none.
/// Returns its head.
async fn lock_doc(tx: &mut Tx, project_id: &str, doc_id: &str) -> Result<Option<String>, ApiError> {
    let found: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT head FROM docs \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(doc_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)?;
    found.map(|(head,)| head).ok_or(ApiError::NotFound)
}

/// Renames a doc. Its ID, head and comments stay; the rename is one row of the
/// doc's log, with the head before and after the same.
#[utoipa::path(
    patch,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = RenameDocRequest,
    responses(
        (status = 200, description = "The renamed doc", body = DocResponse),
        (status = 400, description = "The title is missing or blank", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, or it was deleted", body = ErrorBody),
    )
)]
pub async fn rename_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<DocResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let new = parse_body::<RenameDocRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let head = lock_doc(&mut tx, &project_id, &doc_id).await?;
    let title = non_blank(new?.title).ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    sqlx::query("UPDATE docs SET title = $2 WHERE id = $1::uuid")
        .bind(&doc_id)
        .bind(&title)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    log_change(
        &mut tx,
        &request.workspace_id,
        &doc_id,
        &identity.to_json(),
        &json!([{ "op": "rename", "title": title }]),
        head.as_deref(),
        &head.clone().unwrap_or_else(empty_head),
    )
    .await?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "doc.renamed",
        &json!({ "doc_id": doc_id, "title": title }),
    )
    .await?;
    let row = find_doc(&mut tx, &project_id, &doc_id).await?;
    let drafts = drafts_of(&mut tx, &row.id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    let claims = state.claims.live_of_doc(&request.workspace_id, &row.id);
    Ok(Json(DocResponse {
        doc: doc_body(row, drafts, claims),
    }))
}

/// Deletes a doc. Nothing is erased: the row gets `deleted_at`, the doc and its
/// comments leave every list and read, and the deletion is one row of the
/// doc's log. A deleted doc takes no more writes.
#[utoipa::path(
    delete,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    responses(
        (status = 204, description = "The doc is deleted"),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, or it was deleted already", body = ErrorBody),
    )
)]
pub async fn delete_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let head = lock_doc(&mut tx, &project_id, &doc_id).await?;
    sqlx::query("UPDATE docs SET deleted_at = clock_timestamp() WHERE id = $1::uuid")
        .bind(&doc_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    log_change(
        &mut tx,
        &request.workspace_id,
        &doc_id,
        &identity.to_json(),
        &json!([{ "op": "delete" }]),
        head.as_deref(),
        &head.clone().unwrap_or_else(empty_head),
    )
    .await?;
    live::emit(
        &mut tx,
        &request.workspace_id,
        &project_id,
        "doc.deleted",
        &json!({ "doc_id": doc_id }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&request.workspace_id, &project_id);
    Ok(StatusCode::NO_CONTENT)
}

/// A doc's head as HTML with `data-omni-id`, or the version named by
/// `?version=` (a version has no `data-rev`). The head is read, never the
/// mainline version: a new doc reads as the empty doc. A design's head answers
/// its `pages` and `tags` and no HTML.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/html",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("version" = Option<String>, Query, description = "The version to read instead of the head"),
    ),
    responses(
        (status = 200, description = "The head's or version's HTML", body = MainlineHtmlResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc, or the doc has no such version", body = ErrorBody),
    )
)]
pub async fn read_doc_html(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<MainlineHtmlResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let row = find_doc(&mut tx, &project_id, &doc_id).await?;
    let Some(version) = query.get("version") else {
        let version_id = head_version(&mut tx, &row).await?;
        tx.commit().await.map_err(ApiError::database)?;
        return read_head(&state, &request.workspace_id, row, version_id)
            .await
            .map(Json);
    };
    let version_id = parse_uuid(version).ok_or(ApiError::NotFound)?;
    let chunk = versions::chunk_of_version(&mut tx, &row.id, &version_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    // A version that holds no content (the removal of the doc) reads as empty.
    let html = match chunk {
        Some(chunk) => versions::read_chunk(&state, &request.workspace_id, &chunk).await?,
        None => String::new(),
    };
    Ok(Json(MainlineHtmlResponse {
        doc_id: row.id,
        version_id: Some(version_id),
        html,
        tags: None,
        pages: None,
        tokens: None,
        components: None,
    }))
}

/// The version that the doc's head is the content of: the mainline version
/// when it holds the very chunk of the head, else none.
pub(crate) async fn head_version(tx: &mut Tx, doc: &DocRow) -> Result<Option<String>, ApiError> {
    let (Some(mainline), Some(head)) = (&doc.mainline_version_id, &doc.head) else {
        return Ok(None);
    };
    sqlx::query_scalar("SELECT id::text FROM versions WHERE id = $1::uuid AND chunk = $2")
        .bind(mainline)
        .bind(head)
        .fetch_optional(&mut **tx)
        .await
        .map_err(ApiError::database)
}

/// A doc's head as the read of the doc answers it.
async fn read_head(
    state: &AppState,
    workspace_id: &str,
    row: DocRow,
    version_id: Option<String>,
) -> Result<MainlineHtmlResponse, ApiError> {
    let mut answer = MainlineHtmlResponse {
        doc_id: row.id,
        version_id,
        html: String::new(),
        tags: None,
        pages: None,
        tokens: None,
        components: None,
    };
    if row.kind == "design" {
        let pages = match &row.head {
            Some(head) => {
                let what = format!("the head of doc {}", answer.doc_id);
                boards::read_pages(state, workspace_id, head, &what).await?
            }
            None => Vec::new(),
        };
        // The design resolves against the design system's head as it is now.
        let system = design::system_head_of_project(state, workspace_id, &row.project_id).await?;
        answer.tags = Some(design::tags_of(&pages));
        answer.pages = Some(design::nodes_values(&pages));
        answer.tokens = system.tokens_json();
        answer.components = Some(system.components_used_by(&pages));
    } else if let Some(head) = &row.head {
        answer.html = versions::read_chunk(state, workspace_id, head).await?;
    }
    Ok(answer)
}

/// Opens a doc in a change set: makes the change set's draft from the doc's
/// mainline version, or returns the draft the change set already has.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/open",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
    ),
    responses(
        (status = 200, description = "The draft the change set already had", body = DraftResponse),
        (status = 201, description = "The new draft, based on the doc's mainline version", body = DraftResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set or doc in this project and workspace", body = ErrorBody),
        (status = 409, description = "The doc has no mainline version, so only its own change set has it (`no_mainline_version`)", body = ErrorBody),
    )
)]
pub async fn open_draft(
    State(state): State<AppState>,
    request: AgentRequest,
    Path((_, project_id, changeset_id, doc_id)): Path<(String, String, String, String)>,
) -> Result<(StatusCode, Json<DraftResponse>), ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_changeset(&mut tx, &project_id, &changeset_id).await?;
    let doc = find_doc(&mut tx, &project_id, &doc_id).await?;
    check_doc_scope(&mut tx, doc.feature_id.as_deref(), &changeset_id).await?;
    let existing = sqlx::query_as::<_, DraftBody>(&format!(
        "SELECT {DRAFT_COLUMNS} FROM drafts WHERE doc_id = $1::uuid AND changeset_id = $2::uuid"
    ))
    .bind(&doc_id)
    .bind(&changeset_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    if let Some(draft) = existing {
        tx.commit().await.map_err(ApiError::database)?;
        return Ok((StatusCode::OK, Json(DraftResponse { draft })));
    }
    let Some(mainline) = doc.mainline_version_id else {
        return Err(ApiError::conflict("no_mainline_version"));
    };
    // Two requests may open the same doc at once: the unique key lets one make
    // the draft, and the other reads it.
    let inserted: Option<DraftBody> = sqlx::query_as(&format!(
        "INSERT INTO drafts (workspace_id, doc_id, changeset_id, base_version_id) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid) \
         ON CONFLICT (workspace_id, doc_id, changeset_id) DO NOTHING \
         RETURNING {DRAFT_COLUMNS}"
    ))
    .bind(&request.workspace_id)
    .bind(&doc_id)
    .bind(&changeset_id)
    .bind(&mainline)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (status, draft) = match inserted {
        Some(draft) => {
            live::emit(
                &mut tx,
                &request.workspace_id,
                &project_id,
                "draft.opened",
                &json!({
                    "doc_id": doc_id,
                    "changeset_id": changeset_id,
                    "draft_id": draft.id,
                    "base_version_id": mainline,
                }),
            )
            .await?;
            (StatusCode::CREATED, draft)
        }
        None => {
            let draft = sqlx::query_as::<_, DraftBody>(&format!(
                "SELECT {DRAFT_COLUMNS} FROM drafts \
                 WHERE doc_id = $1::uuid AND changeset_id = $2::uuid"
            ))
            .bind(&doc_id)
            .bind(&changeset_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::database)?;
            (StatusCode::OK, draft)
        }
    };
    tx.commit().await.map_err(ApiError::database)?;
    if status == StatusCode::CREATED {
        state.live.announce(&request.workspace_id, &project_id);
    }
    Ok((status, Json(DraftResponse { draft })))
}

/// A change set's draft of a doc, as HTML with `data-omni-id` and `data-rev` on
/// every block.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
    ),
    responses(
        (status = 200, description = "The draft with its HTML", body = DraftHtmlResponse),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "The change set has no draft of this doc in this project and workspace", body = ErrorBody),
    )
)]
pub async fn read_draft(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, changeset_id, doc_id)): Path<(String, String, String, String)>,
) -> Result<Json<DraftHtmlResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let draft = find_draft(&mut tx, &project_id, &changeset_id, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;

    let slot = state.drafts.slot(&request.workspace_id, &draft.id);
    let mut held = slot.lock().await;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let draft = current_draft(&mut tx, &draft.id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    ensure_loaded(&state, &request.workspace_id, &draft, &mut held).await?;
    let html = held
        .as_ref()
        .map(|loaded| loaded.doc.to_html_with_revisions())
        .unwrap_or_default();
    // A design doc's draft holds pages, which resolve against the change set's
    // design system once this draft's lock is let go.
    let pages = held
        .as_ref()
        .filter(|loaded| loaded.design)
        .map(|loaded| loaded.design_doc.pages().to_vec());
    drop(held);
    let view = match pages {
        Some(pages) => Some(
            design::view_of_draft(
                &state,
                &request.workspace_id,
                &project_id,
                &changeset_id,
                &pages,
            )
            .await?,
        ),
        None => None,
    };
    let (tags, pages, design_system, components, tokens) = match view {
        Some(view) => (
            Some(view.tags),
            Some(view.pages),
            Some(view.design_system),
            Some(view.components),
            view.tokens,
        ),
        None => (None, None, None, None, None),
    };
    Ok(Json(DraftHtmlResponse {
        draft: DraftWithHtml {
            draft: draft.into(),
            html,
            tags,
            pages,
            design_system,
            components,
            tokens,
        },
    }))
}

/// What an edit needs, gathered before the draft is locked.
struct Edit {
    workspace_id: String,
    project_id: String,
    draft: DraftRow,
    agent: Value,
    /// The operations as the agent sent them, kept in the draft's log.
    sent: Vec<Value>,
    ops: Vec<Operation>,
    /// The board that a design batch must stay inside.
    board: Option<String>,
    /// The components that the instances of a design batch are checked
    /// against.
    library: Library,
}

/// Applies a batch of operations to a change set's draft, all or nothing. Each
/// applied operation is logged with the agent identity and emits `op.applied`;
/// a batch that is refused changes, logs and emits nothing. A design doc's
/// operations take tags of the tag format as content, and the server assigns
/// the IDs of the new nodes; `board` keeps a batch inside one board.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/ops",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = EditRequest,
    responses(
        (status = 200, description = "The batch applied; one result per operation", body = EditResponse),
        (status = 400, description = "The body is not a batch (`invalid_request`), `board` is given for a doc that is no design (`not_a_design`), or an operation is refused (`invalid_operation`, naming an unknown ID; for a design also an insert whose tags carry an `id`, a placement the tags do not allow, a property the node's tag does not take, or an operation outside `board`)", body = InvalidOperationBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "The change set has no draft of this doc in this project and workspace, or `board` is no board of the design (`unknown_board`)", body = ErrorBody),
        (status = 409, description = "A node is at another revision than the operation expected (`conflict`), or an operation touches a claim of another agent (`claimed`); nothing changed", body = ConflictBody),
    )
)]
pub async fn edit_draft(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id, doc_id)): Path<(String, String, String, String)>,
    body: Bytes,
) -> Result<Json<EditResponse>, DocError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let batch = parse_body::<EditRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let draft = find_draft(&mut tx, &project_id, &changeset_id, &doc_id).await?;
    tx.commit().await.map_err(ApiError::database)?;

    let batch = batch?;
    let (sent, board) = (batch.ops, batch.board);
    claims::sweep(&state, &request.workspace_id).await;
    let mut ops = Vec::with_capacity(sent.len());
    for (index, op) in sent.iter().enumerate() {
        match serde_json::from_value::<Operation>(op.clone()) {
            Ok(op) => ops.push(op),
            Err(error) => {
                return Err(DocError::Invalid {
                    index,
                    message: error.to_string(),
                });
            }
        }
    }
    // The components come first, under no lock of this draft, so that no two
    // draft locks are ever held together. Only a batch that places or changes
    // an instance needs them.
    let library = if places_instances(&sent) {
        design::resolve(&state, &request.workspace_id, &project_id, &changeset_id)
            .await?
            .library
    } else {
        Library::default()
    };
    let edit = Edit {
        workspace_id: request.workspace_id,
        project_id,
        draft,
        agent: identity.to_json(),
        sent,
        ops,
        board,
        library,
    };
    // A client that gives up must not leave the draft in memory ahead of its
    // log, so the batch runs to its end whatever happens to this request.
    let state_for_task = state.clone();
    let results = tokio::spawn(async move { apply_batch(&state_for_task, edit).await })
        .await
        .map_err(|e| {
            eprintln!("omni-api: an edit failed: {e}");
            ApiError::Unavailable
        })??;
    Ok(Json(EditResponse { results }))
}

/// Applies a batch of operations to the head of an HTML doc or a design, all or
/// nothing, with no change set. The batch runs under the lock on the doc's row,
/// so batches to one doc are applied one after the other, each to the head the
/// one before made. The new head of an HTML doc is a chunk in RustFS that holds
/// the doc as HTML with `data-omni-id` and `data-rev`, which is where a node's
/// revision is kept; the new head of a design is a manifest that lists the
/// chunk of every page and board and the revisions, and only the boards the
/// batch changed get a new chunk (`design::edit_head`). Nothing about a head is
/// held in memory. A batch that is refused changes, logs and emits nothing; an
/// applied one adds one row of `doc_changes` and emits `doc.changed`.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/ops",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The doc's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = EditRequest,
    responses(
        (status = 200, description = "The batch applied; one result per operation", body = EditResponse),
        (status = 400, description = "The body is not a batch (`invalid_request`), the doc is a design system (`not_an_html_doc`), or an operation is refused (`invalid_operation`, naming an unknown ID, or for a design an instance of a component the design system lacks)", body = InvalidOperationBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, or `board` is no board of the design (`unknown_board`)", body = ErrorBody),
        (status = 409, description = "A node is at another revision than the operation expected (`conflict`); nothing changed", body = ConflictBody),
    )
)]
pub async fn edit_doc(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, doc_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<EditResponse>, DocError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let workspace_id = request.workspace_id;
    let batch = parse_body::<EditRequest>(&body);

    // The lock comes first and the batch holds it to its commit, and the only
    // connection it uses is its own.
    let mut tx = state.workspace_tx(&workspace_id).await?;
    let locked: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT kind, head FROM docs \
         WHERE id = $1::uuid AND project_id = $2::uuid AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(&doc_id)
    .bind(&project_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    let (kind, head_before) = locked.ok_or(ApiError::NotFound)?;
    let batch = batch?;
    let (sent, board) = (batch.ops, batch.board);
    if kind != "html" && kind != "design" {
        return Err(ApiError::bad_request("not_an_html_doc").into());
    }
    let mut ops = Vec::with_capacity(sent.len());
    for (index, op) in sent.iter().enumerate() {
        match serde_json::from_value::<Operation>(op.clone()) {
            Ok(op) => ops.push(op),
            Err(error) => {
                return Err(DocError::Invalid {
                    index,
                    message: error.to_string(),
                });
            }
        }
    }

    if kind == "design" {
        let agent = identity.to_json();
        let results = design::edit_head(
            &state,
            &mut tx,
            &workspace_id,
            &project_id,
            &doc_id,
            head_before.as_deref(),
            &agent,
            sent,
            ops,
            board,
        )
        .await?;
        tx.commit().await.map_err(ApiError::database)?;
        if !results.is_empty() {
            state.live.announce(&workspace_id, &project_id);
        }
        return Ok(Json(EditResponse { results }));
    }

    let html = match &head_before {
        Some(head) => versions::read_chunk(&state, &workspace_id, head).await?,
        None => String::new(),
    };
    let mut doc = EditDoc::from_html(&html).map_err(|e| {
        eprintln!("omni-api: the head of doc {doc_id} cannot be loaded: {e}");
        ApiError::Unavailable
    })?;
    let mut used: HashSet<NodeId> = doc.index().iter().map(|node| node.id).collect();
    let mut new_ids = || loop {
        let id = random_node_id();
        if used.insert(id) {
            return id;
        }
    };
    let applied = match doc.apply(&ops, &mut new_ids) {
        Ok(applied) => applied,
        Err(ApplyError::Conflict(conflict)) => return Err(DocError::Conflict(conflict)),
        Err(ApplyError::Invalid { index, message }) => {
            return Err(DocError::Invalid { index, message });
        }
    };
    let results = results_of(&applied);
    if results.is_empty() {
        return Ok(Json(EditResponse { results }));
    }

    let snapshot = Snapshot::new(doc.to_html_with_revisions(), doc.index());
    let head_after = versions::store_text(&state, &workspace_id, &snapshot.html).await?;
    versions::index_snapshot(&mut tx, &workspace_id, &head_after, &snapshot).await?;
    sqlx::query("UPDATE docs SET head = $2 WHERE id = $1::uuid")
        .bind(&doc_id)
        .bind(&head_after)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    log_change(
        &mut tx,
        &workspace_id,
        &doc_id,
        &identity.to_json(),
        &Value::Array(sent),
        head_before.as_deref(),
        &head_after,
    )
    .await?;
    live::emit(
        &mut tx,
        &workspace_id,
        &project_id,
        "doc.changed",
        &json!({ "doc_id": doc_id, "head": head_after }),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&workspace_id, &project_id);
    Ok(Json(EditResponse { results }))
}

/// The results of an applied batch as the API answers them.
pub(crate) fn results_of(applied: &omni_core::ops::Applied) -> Vec<OpResultBody> {
    applied
        .ops
        .iter()
        .map(|result| OpResultBody {
            node: result.node.to_string(),
            revision: result.revision,
            created: result.created.iter().map(NodeId::to_string).collect(),
            removed: result.removed.iter().map(NodeId::to_string).collect(),
        })
        .collect()
}

/// Whether a batch places an instance of a component or changes one, whose
/// component, properties and overrides must fit the design system.
fn places_instances(sent: &[Value]) -> bool {
    sent.iter().any(|op| match op["op"].as_str() {
        Some("insert" | "replace") => op["content"]
            .as_str()
            .is_some_and(|content| content.contains("<Instance")),
        Some("set") => op["properties"].as_object().is_some_and(|properties| {
            ["component", "props", "overrides"]
                .iter()
                .any(|name| properties.contains_key(*name))
        }),
        _ => false,
    })
}

async fn apply_batch(state: &AppState, edit: Edit) -> Result<Vec<OpResultBody>, DocError> {
    let Edit {
        workspace_id,
        project_id,
        draft,
        agent,
        sent,
        ops,
        board,
        library,
    } = edit;
    let slot = state.drafts.slot(&workspace_id, &draft.id);
    let mut held = slot.lock().await;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    let draft = current_draft(&mut tx, &draft.id).await?;
    if held.is_none() {
        *held = Some(load_state(state, &mut tx, &workspace_id, &draft).await?);
    }
    let used = state
        .drafts
        .used_ids(&mut tx, &workspace_id, &project_id)
        .await?;
    let loaded = held.as_mut().expect("the draft was just loaded");
    let first_seq = loaded.next_seq;

    let scope = match board {
        None => None,
        Some(_) if !loaded.design => return Err(ApiError::bad_request("not_a_design").into()),
        Some(board) => Some(
            board
                .parse::<NodeId>()
                .ok()
                .filter(|board| loaded.design_doc.is_board(*board))
                .ok_or(ApiError::Refused(StatusCode::NOT_FOUND, "unknown_board"))?,
        ),
    };

    let live = state.claims.live_in_draft(&workspace_id, &draft.id);
    if let Some((index, claim)) = claims::blocked(loaded.tree(), &ops, &live, &agent) {
        return Err(state.claims.refusal(Some(index), claim));
    }

    let mut new_ids = || loop {
        let id = random_node_id();
        if used.lock().unwrap_or_else(|e| e.into_inner()).insert(id) {
            return id;
        }
    };
    let applied = if loaded.design {
        let check = |nodes: &[Node]| design::check_instances(nodes, &library);
        loaded.design_doc.apply(&ops, scope, &mut new_ids, &check)
    } else {
        loaded.doc.apply(&ops, &mut new_ids)
    };
    let applied = match applied {
        Ok(applied) => applied,
        Err(ApplyError::Conflict(conflict)) => return Err(DocError::Conflict(conflict)),
        Err(ApplyError::Invalid { index, message }) => {
            return Err(DocError::Invalid { index, message });
        }
    };

    let results = results_of(&applied);
    let persisted = log_and_emit(
        &mut tx,
        &workspace_id,
        &project_id,
        &draft,
        &agent,
        first_seq,
        &sent,
        &results,
    )
    .await;
    let committed = match persisted {
        Ok(()) => tx.commit().await.map_err(ApiError::database),
        Err(error) => Err(error),
    };
    match committed {
        Ok(()) => {
            held.as_mut().expect("the draft is loaded").next_seq += results.len() as i64;
            // The holder's claims are renewed before the draft's lock is let
            // go, and only the ones that are still live.
            if !results.is_empty() {
                state.claims.touch(&workspace_id, &draft.id, &agent);
            }
            drop(held);
            if !results.is_empty() {
                state.live.announce(&workspace_id, &project_id);
            }
            Ok(results)
        }
        Err(error) => {
            // The draft in memory has the batch and the database does not: the
            // next request loads the draft again from its log.
            *held = None;
            Err(error.into())
        }
    }
}

/// Writes one row of the draft's log for each applied operation and emits its
/// `op.applied`, in the transaction of the batch.
#[allow(clippy::too_many_arguments)]
async fn log_and_emit(
    tx: &mut Tx,
    workspace_id: &str,
    project_id: &str,
    draft: &DraftRow,
    agent: &Value,
    first_seq: i64,
    sent: &[Value],
    results: &[OpResultBody],
) -> Result<(), ApiError> {
    for (offset, (op, result)) in sent.iter().zip(results).enumerate() {
        let seq = first_seq + offset as i64;
        sqlx::query(
            "INSERT INTO draft_ops \
             (workspace_id, draft_id, seq, op, agent, node, revision, created, removed) \
             VALUES ($1::uuid, $2::uuid, $3, $4::text::jsonb, $5::text::jsonb, $6, $7, $8, $9)",
        )
        .bind(workspace_id)
        .bind(&draft.id)
        .bind(seq)
        .bind(op.to_string())
        .bind(agent.to_string())
        .bind(&result.node)
        .bind(result.revision as i64)
        .bind(&result.created)
        .bind(&result.removed)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::database)?;
        live::emit(
            tx,
            workspace_id,
            project_id,
            "op.applied",
            &json!({
                "doc_id": draft.doc_id,
                "changeset_id": draft.changeset_id,
                "draft_id": draft.id,
                "node": result.node,
                "revision": result.revision,
                "created": result.created,
                "removed": result.removed,
                "agent": agent,
            }),
        )
        .await?;
    }
    Ok(())
}
