//! Claims. An agent claims a section (HTML docs) or a board (design docs) of
//! one draft while it works on it, and no other
//! agent identity can claim or change that subtree meanwhile. Nothing is
//! locked in the database: a claim is advisory and short, so claims are kept
//! in memory, next to the drafts they belong to, and are gone after a restart.
//!
//! The identity of a claim's holder is the run and role of its request (the
//! `X-Omni-Run` and `X-Omni-Role` headers), not its API key, model or harness.
//! A claim lasts a TTL (10 minutes unless `AppState::with_claim_ttl` says
//! otherwise) after the holder's last applied operation in the draft. Expiry
//! does not wait for a request: a background sweep, started by the router,
//! ends the claims whose time is up every second and emits `claim.expired`
//! for them, and a request that looks at claims does the same first.
//!
//! Taking a claim, releasing it and renewing it by an applied batch all happen
//! while the draft's slot is locked (see docs.rs), so a claim and an edit of
//! the same draft never interleave. Expiry only ends claims that are over
//! already, so it needs no lock. Taking and releasing write their live event
//! in a transaction of their own, and a claim only changes if that
//! transaction committed (a failed release puts the claim back before the
//! draft's lock is let go).

use crate::AppState;
use crate::changesets::parse_body;
use crate::docs::{
    AgentIdentity, DRAFT_ROW_COLUMNS, DocError, DraftRow, Tx, current_draft, ensure_loaded,
    require_changeset,
};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use omni_core::design::{DesignDoc, NodeId};
use omni_core::ops::{EditDoc, Operation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use utoipa::ToSchema;

/// How long a claim lasts without an operation from its holder.
const DEFAULT_TTL: Duration = Duration::from_secs(10 * 60);

/// How often the background sweep ends the claims whose time is up.
const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

/// A claim as the API shows it.
#[derive(Clone, Serialize, ToSchema)]
pub struct ClaimBody {
    /// The claim's UUID (v7).
    pub id: String,
    pub doc_id: String,
    pub draft_id: String,
    pub changeset_id: String,
    /// The claimed board or section.
    pub node: String,
    /// The holder: `run`, `role`, `model` and `harness`, each `null` when the
    /// request did not declare it.
    #[schema(value_type = Object)]
    pub agent: Value,
    pub taken_at: String,
    /// The holder's last operation in the draft (or `taken_at`) plus the TTL.
    pub expires_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct ClaimResponse {
    pub claim: ClaimBody,
}

#[derive(Deserialize, ToSchema)]
pub struct ClaimRequest {
    /// The ID of the board or section.
    pub node: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ClaimedBody {
    /// `claimed`
    pub error: String,
    /// The position of the operation in the batch, counting from 0; absent for
    /// a request to claim.
    pub index: Option<usize>,
    /// Names the holder, as `<run>/<role>`, and the claimed node.
    pub message: String,
    /// The claim in the way.
    pub claim: ClaimBody,
}

/// A claim in memory.
#[derive(Clone)]
pub(crate) struct ClaimRecord {
    id: String,
    workspace_id: String,
    project_id: String,
    doc_id: String,
    draft_id: String,
    changeset_id: String,
    node: NodeId,
    agent: Value,
    taken_at: SystemTime,
    /// The holder's last operation in the draft, or when the claim was taken.
    last_op: SystemTime,
}

impl ClaimRecord {
    fn expires_at(&self, ttl: Duration) -> SystemTime {
        self.last_op + ttl
    }

    fn event_data(&self) -> Value {
        json!({
            "claim_id": self.id,
            "doc_id": self.doc_id,
            "draft_id": self.draft_id,
            "changeset_id": self.changeset_id,
            "node": self.node.to_string(),
            "agent": self.agent,
        })
    }
}

/// The claims that are held, shared by every clone of the `AppState`.
#[derive(Clone)]
pub(crate) struct ClaimStore {
    ttl: Duration,
    records: Arc<Mutex<Vec<ClaimRecord>>>,
}

impl Default for ClaimStore {
    fn default() -> Self {
        Self::new(DEFAULT_TTL)
    }
}

impl ClaimStore {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            records: Arc::default(),
        }
    }

    fn records(&self) -> std::sync::MutexGuard<'_, Vec<ClaimRecord>> {
        self.records.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn live(&self, record: &ClaimRecord, now: SystemTime) -> bool {
        record.expires_at(self.ttl) > now
    }

    fn body(&self, record: &ClaimRecord) -> ClaimBody {
        ClaimBody {
            id: record.id.clone(),
            doc_id: record.doc_id.clone(),
            draft_id: record.draft_id.clone(),
            changeset_id: record.changeset_id.clone(),
            node: record.node.to_string(),
            agent: record.agent.clone(),
            taken_at: rfc3339(record.taken_at),
            expires_at: rfc3339(record.expires_at(self.ttl)),
        }
    }

    /// The live claims of a draft, oldest first.
    pub(crate) fn live_in_draft(&self, workspace_id: &str, draft_id: &str) -> Vec<ClaimRecord> {
        let now = SystemTime::now();
        self.records()
            .iter()
            .filter(|r| r.workspace_id == workspace_id && r.draft_id == draft_id)
            .filter(|r| self.live(r, now))
            .cloned()
            .collect()
    }

    /// The live claims of a doc, in all its drafts, oldest first.
    pub(crate) fn live_of_doc(&self, workspace_id: &str, doc_id: &str) -> Vec<ClaimBody> {
        let now = SystemTime::now();
        self.records()
            .iter()
            .filter(|r| r.workspace_id == workspace_id && r.doc_id == doc_id)
            .filter(|r| self.live(r, now))
            .map(|r| self.body(r))
            .collect()
    }

    /// An operation of `agent` was applied in the draft: its claims there that
    /// are still live last another TTL. A claim whose time is up stays over,
    /// even if nobody has ended it yet.
    pub(crate) fn touch(&self, workspace_id: &str, draft_id: &str, agent: &Value) {
        let now = SystemTime::now();
        for record in self.records().iter_mut() {
            if record.workspace_id == workspace_id
                && record.draft_id == draft_id
                && same_agent(&record.agent, agent)
                && record.expires_at(self.ttl) > now
            {
                record.last_op = now;
            }
        }
    }

    /// The refusal for a claim of another agent that is in the way.
    pub(crate) fn refusal(&self, index: Option<usize>, claim: ClaimRecord) -> DocError {
        let body = self.body(&claim);
        DocError::Claimed {
            index,
            message: format!(
                "{} is claimed by {} until {}",
                body.node,
                holder(&claim.agent),
                body.expires_at
            ),
            claim: Box::new(body),
        }
    }

    /// Takes the claims whose time is up out of the store: those of one
    /// workspace, or of all.
    fn take_expired(&self, workspace_id: Option<&str>) -> Vec<ClaimRecord> {
        let now = SystemTime::now();
        let mut records = self.records();
        let (live, expired) = std::mem::take(&mut *records).into_iter().partition(|r| {
            workspace_id.is_some_and(|workspace| r.workspace_id != workspace) || self.live(r, now)
        });
        *records = live;
        expired
    }

    fn restore(&self, claims: Vec<ClaimRecord>) {
        self.records().extend(claims);
    }
}

/// Whether two agent identities are the same agent: the same run and role.
/// The model and harness are recorded but do not tell agents apart.
pub(crate) fn same_agent(a: &Value, b: &Value) -> bool {
    a["run"] == b["run"] && a["role"] == b["role"]
}

/// `<run>/<role>`, with `?` for what the agent did not declare.
fn holder(agent: &Value) -> String {
    let part = |name: &str| agent[name].as_str().unwrap_or("?").to_string();
    format!("{}/{}", part("run"), part("role"))
}

/// Formats a time as ISO 8601 text in UTC, like the times that Postgres gives.
fn rfc3339(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let in_day = seconds.rem_euclid(86_400);
    // Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60,
        since.subsec_micros()
    )
}

// Which operations a claim stops ---------------------------------------------------

/// The nodes of a draft that claims are on: an HTML doc's sections or a
/// design's boards, and what they sit in.
pub(crate) trait Tree {
    fn contains(&self, node: NodeId) -> bool;

    /// The parent of a node: `Some(None)` for a top-level node, `None` when
    /// the node is not in the draft.
    fn parent(&self, node: NodeId) -> Option<Option<NodeId>>;
}

impl Tree for EditDoc {
    fn contains(&self, node: NodeId) -> bool {
        EditDoc::contains(self, node)
    }

    fn parent(&self, node: NodeId) -> Option<Option<NodeId>> {
        EditDoc::parent(self, node)
    }
}

impl Tree for DesignDoc {
    fn contains(&self, node: NodeId) -> bool {
        DesignDoc::contains(self, node)
    }

    fn parent(&self, node: NodeId) -> Option<Option<NodeId>> {
        DesignDoc::parent(self, node)
    }
}

/// A node and its ancestors, the node first.
fn ancestry(doc: &dyn Tree, node: NodeId) -> Vec<NodeId> {
    let mut chain = Vec::new();
    let mut at = Some(node);
    while let Some(id) = at {
        chain.push(id);
        at = doc.parent(id).flatten();
    }
    chain
}

/// The first operation of the batch that another agent's claim stops, with the
/// claim. A claim stops every operation that changes, moves or deletes a node in
/// its subtree (the claimed node too), that inserts or moves a node into it, or
/// that replaces or deletes an ancestor of the claimed node, which would take the
/// claimed nodes along.
pub(crate) fn blocked(
    doc: &dyn Tree,
    ops: &[Operation],
    claims: &[ClaimRecord],
    agent: &Value,
) -> Option<(usize, ClaimRecord)> {
    let others: Vec<&ClaimRecord> = claims
        .iter()
        .filter(|claim| !same_agent(&claim.agent, agent) && doc.contains(claim.node))
        .collect();
    if others.is_empty() {
        return None;
    }
    let inside = |node: NodeId, claim: &ClaimRecord| ancestry(doc, node).contains(&claim.node);
    let above = |node: NodeId, claim: &ClaimRecord| ancestry(doc, claim.node).contains(&node);
    for (index, op) in ops.iter().enumerate() {
        let stopped = others.iter().find(|claim| match op {
            Operation::Insert { parent, .. } => parent.is_some_and(|p| inside(p, claim)),
            Operation::Set { node, .. } => inside(*node, claim),
            Operation::Replace { node, .. } | Operation::Delete { node, .. } => {
                inside(*node, claim) || above(*node, claim)
            }
            Operation::Move { node, parent, .. } => {
                inside(*node, claim) || parent.is_some_and(|p| inside(p, claim))
            }
        });
        if let Some(claim) = stopped {
            return Some((index, (*claim).clone()));
        }
    }
    None
}

/// Whether claiming `node` collides with `claim`: the same node, an ancestor
/// of it or a descendant of it.
fn overlaps(doc: &dyn Tree, node: NodeId, claim: &ClaimRecord) -> bool {
    ancestry(doc, node).contains(&claim.node) || ancestry(doc, claim.node).contains(&node)
}

// Expiry ---------------------------------------------------------------------------

/// Ends the claims of the workspace whose time is up and emits `claim.expired`
/// for each. A claim stays if its event cannot be written.
pub(crate) async fn sweep(state: &AppState, workspace_id: &str) {
    let expired = state.claims.take_expired(Some(workspace_id));
    expire(state, workspace_id, expired).await;
}

async fn expire(state: &AppState, workspace_id: &str, expired: Vec<ClaimRecord>) {
    if expired.is_empty() {
        return;
    }
    if emit_all(state, workspace_id, &expired, "claim.expired")
        .await
        .is_err()
    {
        state.claims.restore(expired);
    }
}

/// Ends the claims of every workspace whose time is up.
async fn sweep_all(state: &AppState) {
    let mut expired = state.claims.take_expired(None);
    while let Some(first) = expired.first() {
        let workspace_id = first.workspace_id.clone();
        let (here, rest) = expired
            .into_iter()
            .partition(|claim| claim.workspace_id == workspace_id);
        expired = rest;
        expire(state, &workspace_id, here).await;
    }
}

/// Starts the background sweep, which ends claims on time whether or not any
/// request comes in. It needs a Tokio runtime and does nothing without one.
pub(crate) fn start_sweep(state: AppState) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            sweep_all(&state).await;
        }
    });
}

/// Emits `event` for each claim in one transaction and announces it.
async fn emit_all(
    state: &AppState,
    workspace_id: &str,
    claims: &[ClaimRecord],
    event: &str,
) -> Result<(), ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    for claim in claims {
        let mut data = claim.event_data();
        if event == "claim.expired" {
            data["expires_at"] = json!(rfc3339(claim.expires_at(state.claims.ttl)));
        }
        live::emit(&mut tx, workspace_id, &claim.project_id, event, &data).await?;
    }
    tx.commit().await.map_err(ApiError::database)?;
    let mut projects: Vec<&str> = claims.iter().map(|c| c.project_id.as_str()).collect();
    projects.dedup();
    for project in projects {
        state.live.announce(workspace_id, project);
    }
    Ok(())
}

// Routes ---------------------------------------------------------------------------

/// The drafts of a change set, oldest first.
async fn drafts_of_changeset(
    tx: &mut Tx,
    project_id: &str,
    changeset_id: &str,
) -> Result<Vec<DraftRow>, ApiError> {
    sqlx::query_as(&format!(
        "SELECT {DRAFT_ROW_COLUMNS} \
         FROM drafts d \
         JOIN changesets c ON c.workspace_id = d.workspace_id AND c.id = d.changeset_id \
         WHERE d.changeset_id = $1::uuid AND c.project_id = $2::uuid \
         ORDER BY d.created_at, d.id"
    ))
    .bind(changeset_id)
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// Reads the node that a claim request names: 400 when there is none or it is
/// not a node ID.
fn requested_node(request: Result<ClaimRequest, ApiError>) -> Result<NodeId, ApiError> {
    request?
        .node
        .and_then(|node| node.parse().ok())
        .ok_or_else(|| ApiError::bad_request("invalid_request"))
}

/// Claims a board or section of the change set's draft that has it, for the
/// agent identity of the request. Claiming a node that the same identity holds
/// already returns that claim.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/claims",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = ClaimRequest,
    responses(
        (status = 201, description = "The new claim", body = ClaimResponse),
        (status = 200, description = "The agent held the claim already", body = ClaimResponse),
        (status = 400, description = "The body has no node ID, or the node is not a board or a section", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace, or none of its drafts has the node", body = ErrorBody),
        (status = 409, description = "Another agent claims the node, an ancestor of it or a descendant of it (`claimed`)", body = ClaimedBody),
    )
)]
pub async fn take_claim(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<ClaimResponse>), DocError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let wanted = parse_body::<ClaimRequest>(&body);
    let workspace_id = request.workspace_id;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    require_changeset(&mut tx, &project_id, &changeset_id).await?;
    let drafts = drafts_of_changeset(&mut tx, &project_id, &changeset_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let node = requested_node(wanted)?;
    let agent = identity.to_json();
    sweep(&state, &workspace_id).await;

    // Node IDs are unique within the project, so at most one draft of the
    // change set has the node. Its slot stays locked while the claim is taken.
    let mut found = None;
    for draft in drafts {
        let mut held = state
            .drafts
            .slot(&workspace_id, &draft.id)
            .lock_owned()
            .await;
        let mut tx = state.workspace_tx(&workspace_id).await?;
        let draft = current_draft(&mut tx, &draft.id).await?;
        tx.commit().await.map_err(ApiError::database)?;
        ensure_loaded(&state, &workspace_id, &draft, &mut held).await?;
        if held.as_ref().is_some_and(|loaded| loaded.contains(node)) {
            found = Some((draft, held));
            break;
        }
    }
    let (draft, held) = found.ok_or(ApiError::NotFound)?;
    let loaded = held.as_ref().expect("the draft is loaded");
    if !loaded.can_claim(node) {
        return Err(ApiError::bad_request("invalid_request").into());
    }
    let doc = loaded.tree();

    let live = state.claims.live_in_draft(&workspace_id, &draft.id);
    if let Some(mine) = live
        .iter()
        .find(|claim| claim.node == node && same_agent(&claim.agent, &agent))
    {
        return Ok((
            StatusCode::OK,
            Json(ClaimResponse {
                claim: state.claims.body(mine),
            }),
        ));
    }
    if let Some(theirs) = live
        .iter()
        .find(|claim| !same_agent(&claim.agent, &agent) && overlaps(doc, node, claim))
    {
        return Err(state.claims.refusal(None, theirs.clone()));
    }

    let mut tx = state.workspace_tx(&workspace_id).await?;
    let id: String = sqlx::query_scalar("SELECT uuid_v7()::text")
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    let now = SystemTime::now();
    let record = ClaimRecord {
        id,
        workspace_id: workspace_id.clone(),
        project_id: project_id.clone(),
        doc_id: draft.doc_id,
        draft_id: draft.id,
        changeset_id,
        node,
        agent,
        taken_at: now,
        last_op: now,
    };
    live::emit(
        &mut tx,
        &workspace_id,
        &project_id,
        "claim.taken",
        &record.event_data(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;
    let claim = state.claims.body(&record);
    state.claims.records().push(record);
    drop(held);
    state.live.announce(&workspace_id, &project_id);
    Ok((StatusCode::CREATED, Json(ClaimResponse { claim })))
}

/// Ends the claim of the agent identity of the request on a node.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/claims/release",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = ClaimRequest,
    responses(
        (status = 200, description = "The claim that ended", body = ClaimResponse),
        (status = 400, description = "The body has no node ID", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace, or no claim on the node", body = ErrorBody),
        (status = 409, description = "Another agent holds the claim, which stays (`claimed`)", body = ClaimedBody),
    )
)]
pub async fn release_claim(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<ClaimResponse>, DocError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let wanted = parse_body::<ClaimRequest>(&body);
    let workspace_id = request.workspace_id;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    require_changeset(&mut tx, &project_id, &changeset_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let node = requested_node(wanted)?;
    let agent = identity.to_json();
    sweep(&state, &workspace_id).await;

    // The draft's lock is held until the release has committed or failed, so
    // an operation sent meanwhile waits and then sees the claim as it ends up.
    let draft_id = state
        .claims
        .records()
        .iter()
        .find(|r| {
            r.workspace_id == workspace_id && r.changeset_id == changeset_id && r.node == node
        })
        .map(|r| r.draft_id.clone())
        .ok_or(ApiError::NotFound)?;
    let _held = state
        .drafts
        .slot(&workspace_id, &draft_id)
        .lock_owned()
        .await;
    sweep(&state, &workspace_id).await;

    let record = {
        let mut records = state.claims.records();
        let at = records
            .iter()
            .position(|r| {
                r.workspace_id == workspace_id && r.changeset_id == changeset_id && r.node == node
            })
            .ok_or(ApiError::NotFound)?;
        if !same_agent(&records[at].agent, &agent) {
            return Err(state.claims.refusal(None, records[at].clone()));
        }
        records.remove(at)
    };
    if let Err(error) = emit_all(
        &state,
        &workspace_id,
        std::slice::from_ref(&record),
        "claim.released",
    )
    .await
    {
        state.claims.restore(vec![record]);
        return Err(error.into());
    }
    Ok(Json(ClaimResponse {
        claim: state.claims.body(&record),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_written_as_iso_8601_in_utc() {
        let at = |seconds: u64, micros: u32| UNIX_EPOCH + Duration::new(seconds, micros * 1000);
        assert_eq!(rfc3339(at(0, 0)), "1970-01-01T00:00:00.000000Z");
        assert_eq!(rfc3339(at(951_782_400, 5)), "2000-02-29T00:00:00.000005Z");
        assert_eq!(rfc3339(at(1_792_058_400, 0)), "2026-10-15T10:00:00.000000Z");
    }

    #[test]
    fn a_holder_is_named_by_run_and_role() {
        assert_eq!(
            holder(&json!({ "run": "run-2", "role": "reviewer", "model": null })),
            "run-2/reviewer"
        );
        assert_eq!(holder(&json!({ "run": null, "role": null })), "?/?");
    }
}
