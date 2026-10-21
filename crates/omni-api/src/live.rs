//! Live updates over WebSockets. Every story that changes data records its
//! events with `emit`, inside the transaction of the change, and calls
//! `LiveBus::announce` once the transaction has committed. An event is a row of
//! `live_events` under the next sequence number of its project, so the table is
//! the one source of truth: a socket delivers new events and replays missed
//! ones with the same query, in order, once each.
//!
//! The protocol (JSON text frames) on `GET /ws/w/{workspace_id}`:
//!
//! - The client sends `{"type": "subscribe", "project_id": ..., "after": N}`
//!   (`after` is optional).
//! - The server answers `{"type": "subscribed", "project_id": ..., "last_seq":
//!   ...}`, then sends every event of the project above `after` (with no
//!   `after`, only the events from then on) and carries on with live ones.
//! - An event is `{"type": "event", "seq", "event", "project_id", "at",
//!   "data"}`; `event` is `<entity>.<action>`.
//! - Mistakes are answered with `{"type": "error", "error": ...}`
//!   (`not_found` or `invalid_request`) and leave the socket open.
//!
//! The socket is opened with the same caller check as the HTTP routes of the
//! workspace (`workspace::WorkspaceRequest`), before the upgrade.
//!
//! What a socket may see follows the caller (US-035). An event that names a
//! post (`data.post_id`) goes only to a caller that may read that post, the
//! human always and an agent identity (`X-Omni-Run`, `X-Omni-Role` of the
//! handshake) by the rule of sealed phases (`threads::visible`). The rule is
//! applied when events are read, for live ones and for those of a resume
//! alike, so a hidden post shows up in the replay of a later subscription once
//! the agent may read it.

use crate::AppState;
use crate::docs::AgentIdentity;
use crate::session::ApiError;
use crate::threads::{Viewer, bind_viewer, visible};
use crate::workspace::{Actor, WorkspaceRequest, parse_uuid};
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::header::{HOST, ORIGIN};
use axum::http::request::Parts;
use axum::response::Response;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

/// How many events a socket reads from the table at a time.
const BATCH: i64 = 500;
/// A socket looks for new events this often even when nothing announced them,
/// which covers a change whose announcement was missed.
const POLL_EVERY: Duration = Duration::from_secs(1);
/// How many announcements wait for a slow socket before it re-reads everything.
const ANNOUNCEMENTS: usize = 1024;

/// A timestamp column as ISO 8601 text in UTC (the format of the HTTP bodies).
const AT: &str = "to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')";

/// Tells the open sockets that a project has new events. The events themselves
/// are in `live_events`; an announcement only makes the sockets look.
#[derive(Clone)]
pub struct LiveBus {
    sender: broadcast::Sender<(String, String)>,
}

impl LiveBus {
    pub fn new() -> Self {
        Self {
            sender: broadcast::channel(ANNOUNCEMENTS).0,
        }
    }

    /// Call after the transaction that emitted events for `project_id` has
    /// committed, never before: a socket would find nothing yet.
    pub fn announce(&self, workspace_id: &str, project_id: &str) {
        // No receiver means no open socket, which is fine.
        let _ = self
            .sender
            .send((workspace_id.to_string(), project_id.to_string()));
    }
}

impl Default for LiveBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Records an event of a project in the transaction of the change and returns
/// its sequence number. Taking the number locks the project's row until the
/// transaction ends, so the numbers of a project are gapless and events become
/// visible in order. `event` is `<entity>.<action>`.
pub(crate) async fn emit(
    tx: &mut Transaction<'static, Postgres>,
    workspace_id: &str,
    project_id: &str,
    event: &str,
    data: &Value,
) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        "WITH next AS (\
             UPDATE projects SET last_event_seq = last_event_seq + 1 \
             WHERE id = $2::uuid RETURNING last_event_seq\
         ) \
         INSERT INTO live_events (workspace_id, project_id, seq, event, data) \
         SELECT $1::uuid, $2::uuid, last_event_seq, $3, $4::text::jsonb FROM next \
         RETURNING seq",
    )
    .bind(workspace_id)
    .bind(project_id)
    .bind(event)
    .bind(data.to_string())
    .fetch_one(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// Opens the socket of a workspace. Authentication and the workspace check
/// happen before the upgrade, so a refusal is a plain 401 or 404.
pub async fn socket(
    State(state): State<AppState>,
    origin: OriginCheck,
    request: WorkspaceRequest,
    identity: AgentIdentity,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    // A browser sends the session cookie with a socket from any site, so the
    // human's socket must come from the page of this host. An API key is not
    // sent by a browser on its own.
    if matches!(request.actor, Actor::Human { .. }) && !origin.same_host {
        return Err(ApiError::Forbidden);
    }
    let viewer = Viewer::of(&request.actor, &identity);
    let workspace_id = request.workspace_id;
    Ok(upgrade.on_upgrade(move |socket| run(socket, state, workspace_id, viewer)))
}

/// Whether the `Origin` header of the request, if any, names the host that the
/// request went to.
pub struct OriginCheck {
    same_host: bool,
}

impl axum::extract::FromRequestParts<AppState> for OriginCheck {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let same_host = match parts.headers.get(ORIGIN) {
            None => true,
            Some(origin) => {
                let origin = origin.to_str().ok();
                let host = parts.headers.get(HOST).and_then(|h| h.to_str().ok());
                match (origin, host) {
                    (Some(origin), Some(host)) => origin
                        .split_once("://")
                        .is_some_and(|(_, authority)| authority.eq_ignore_ascii_case(host)),
                    _ => false,
                }
            }
        };
        Ok(Self { same_host })
    }
}

/// Serves one socket until it closes. `subscriptions` maps each subscribed
/// project to the newest sequence number already sent for it.
async fn run(mut socket: WebSocket, state: AppState, workspace_id: String, viewer: Viewer) {
    let mut announcements = state.live.sender.subscribe();
    let mut poll = tokio::time::interval(POLL_EVERY);
    let mut subscriptions: HashMap<String, i64> = HashMap::new();
    loop {
        let sent = tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => {
                    handle_text(&mut socket, &state, &workspace_id, &viewer, &mut subscriptions, &text).await
                }
                Some(Ok(Message::Binary(_))) => send_error(&mut socket, "invalid_request").await,
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                // Pings are answered by the library.
                Some(Ok(_)) => Ok(()),
            },
            announced = announcements.recv() => match announced {
                Ok((announced_in, project_id)) => {
                    if announced_in == workspace_id && subscriptions.contains_key(&project_id) {
                        deliver(&mut socket, &state, &workspace_id, &viewer, &project_id, &mut subscriptions).await
                    } else {
                        Ok(())
                    }
                }
                // Announcements were missed: look at every project.
                Err(RecvError::Lagged(_)) => {
                    deliver_all(&mut socket, &state, &workspace_id, &viewer, &mut subscriptions).await
                }
                Err(RecvError::Closed) => return,
            },
            _ = poll.tick() => {
                deliver_all(&mut socket, &state, &workspace_id, &viewer, &mut subscriptions).await
            }
        };
        if sent.is_err() {
            return;
        }
    }
}

/// Answers one text frame. An error means the socket is gone.
async fn handle_text(
    socket: &mut WebSocket,
    state: &AppState,
    workspace_id: &str,
    viewer: &Viewer,
    subscriptions: &mut HashMap<String, i64>,
    text: &str,
) -> Result<(), ()> {
    let Some((project_id, after)) = parse_subscribe(text) else {
        return send_error(socket, "invalid_request").await;
    };
    let last_seq = match newest_seq(state, workspace_id, &project_id).await {
        Ok(Some(last_seq)) => last_seq,
        Ok(None) => return send_error(socket, "not_found").await,
        Err(_) => return send_error(socket, "unavailable").await,
    };
    send(
        socket,
        &json!({ "type": "subscribed", "project_id": project_id, "last_seq": last_seq }),
    )
    .await?;
    // A client that is ahead of the project (a number from another database)
    // is treated as caught up.
    let from = after.map_or(last_seq, |after| after.min(last_seq));
    subscriptions.insert(project_id.clone(), from);
    deliver(
        socket,
        state,
        workspace_id,
        viewer,
        &project_id,
        subscriptions,
    )
    .await
}

/// A subscribe message as (project ID, after); `None` for anything else.
fn parse_subscribe(text: &str) -> Option<(String, Option<i64>)> {
    let message: Value = serde_json::from_str(text).ok()?;
    if message["type"] != "subscribe" {
        return None;
    }
    // A project ID that is not a UUID is no project, which the caller answers
    // with not_found like any other unknown one.
    let project_id = message["project_id"].as_str()?;
    let project_id = parse_uuid(project_id).unwrap_or_else(|| UNKNOWN_PROJECT.to_string());
    let after = match &message["after"] {
        Value::Null => None,
        after => Some(after.as_i64().filter(|after| *after >= 0)?),
    };
    Some((project_id, after))
}

/// The ID that no project has: what a malformed project ID is looked up as.
const UNKNOWN_PROJECT: &str = "00000000-0000-0000-0000-000000000000";

/// The newest sequence number of the project, `None` if the workspace has no
/// such project.
async fn newest_seq(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
) -> Result<Option<i64>, ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    let seq = sqlx::query_scalar("SELECT last_event_seq FROM projects WHERE id = $1::uuid")
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(seq)
}

#[derive(sqlx::FromRow)]
struct StoredEvent {
    seq: i64,
    event: String,
    data: String,
    at: String,
}

/// Sends every event of the project that the socket has not got yet, oldest
/// first, and remembers the newest one sent.
async fn deliver(
    socket: &mut WebSocket,
    state: &AppState,
    workspace_id: &str,
    viewer: &Viewer,
    project_id: &str,
    subscriptions: &mut HashMap<String, i64>,
) -> Result<(), ()> {
    loop {
        let Some(&sent_up_to) = subscriptions.get(project_id) else {
            return Ok(());
        };
        let events = match read_events(state, workspace_id, viewer, project_id, sent_up_to).await {
            Ok(events) => events,
            // The next announcement or poll tries again.
            Err(_) => return Ok(()),
        };
        let count = events.len();
        for stored in events {
            let data = serde_json::from_str(&stored.data).unwrap_or(Value::Null);
            send(
                socket,
                &json!({
                    "type": "event",
                    "seq": stored.seq,
                    "event": stored.event,
                    "project_id": project_id,
                    "at": stored.at,
                    "data": data,
                }),
            )
            .await?;
            subscriptions.insert(project_id.to_string(), stored.seq);
        }
        if (count as i64) < BATCH {
            return Ok(());
        }
    }
}

async fn deliver_all(
    socket: &mut WebSocket,
    state: &AppState,
    workspace_id: &str,
    viewer: &Viewer,
    subscriptions: &mut HashMap<String, i64>,
) -> Result<(), ()> {
    let projects: Vec<String> = subscriptions.keys().cloned().collect();
    for project_id in projects {
        deliver(
            socket,
            state,
            workspace_id,
            viewer,
            &project_id,
            subscriptions,
        )
        .await?;
    }
    Ok(())
}

async fn read_events(
    state: &AppState,
    workspace_id: &str,
    viewer: &Viewer,
    project_id: &str,
    after: i64,
) -> Result<Vec<StoredEvent>, ApiError> {
    let mut tx = state.workspace_tx(workspace_id).await?;
    // An event that names a post goes to the callers that may read the post.
    let events = bind_viewer(
        sqlx::query_as(&format!(
            "SELECT e.seq, e.event, e.data::text AS data, {AT} AS at FROM live_events e \
             WHERE e.project_id = $1::uuid AND e.seq > $2 \
               AND (e.data->>'post_id' IS NULL OR EXISTS (\
                   SELECT 1 FROM posts p WHERE p.id::text = e.data->>'post_id' AND {})) \
             ORDER BY e.seq LIMIT $3",
            visible("p", 4)
        ))
        .bind(project_id)
        .bind(after)
        .bind(BATCH),
        viewer,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(events)
}

async fn send(socket: &mut WebSocket, message: &Value) -> Result<(), ()> {
    socket
        .send(Message::Text(message.to_string().into()))
        .await
        .map_err(|_| ())
}

async fn send_error(socket: &mut WebSocket, code: &str) -> Result<(), ()> {
    send(socket, &json!({ "type": "error", "error": code })).await
}
