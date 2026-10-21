//! Directives given with agent requests (US-034). Only the human posts
//! directives. Every successful agent request inside a change set, which is
//! one whose path names a change set (`/changesets/{changeset_id}` and below)
//! or a thread (`/threads/{thread_id}` and below), is answered with a
//! `directives` field next to its usual JSON: the directive posts of the
//! change set's threads that the agent identity of the request (its run and
//! role, as for claims) has not been given yet, oldest first. A directive is
//! given to each identity once; when there is none the field is left out.
//!
//! This is a layer over the whole router, so no handler has to remember it.
//! The human's requests and the requests that fail get nothing and use nothing
//! up, and a directive is only used up when it is in the answer.

use crate::AppState;
use crate::docs::AgentIdentity;
use crate::session::ApiError;
use crate::threads::{POST_COLUMNS, PostRow, Viewer, hide_unseen_links};
use crate::workspace::parse_uuid;
use axum::body::{Body, to_bytes};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::HeaderValue;
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

/// Where in the workspace a request is: a change set, or a thread of one.
enum Scope {
    Changeset(String),
    Thread(String),
}

/// The workspace, the project and the scope that a path names, `None` for a
/// path that is not inside a change set.
fn scope_of(path: &str) -> Option<(String, String, Scope)> {
    let mut parts = path.strip_prefix("/api/v1/w/")?.split('/');
    let workspace_id = parse_uuid(parts.next()?)?;
    if parts.next()? != "projects" {
        return None;
    }
    let project_id = parse_uuid(parts.next()?)?;
    let kind = parts.next()?;
    let id = parse_uuid(parts.next()?)?;
    let scope = match kind {
        "changesets" => Scope::Changeset(id),
        "threads" => Scope::Thread(id),
        _ => return None,
    };
    Some((workspace_id, project_id, scope))
}

/// The layer: adds the directives to the answer of an agent request.
pub(crate) async fn attach(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let scope = scope_of(request.uri().path());
    let (mut parts, body) = request.into_parts();
    // Any `Authorization` header means an agent (workspace::identify); a
    // request that succeeded had a key of the path's workspace.
    let mut agent = None;
    if let Some(scope) = scope.filter(|_| parts.headers.contains_key(AUTHORIZATION)) {
        let identity = AgentIdentity::from_request_parts(&mut parts, &())
            .await
            .unwrap_or_else(|never| match never {});
        agent = Some((scope, identity));
    }
    let response = next.run(Request::from_parts(parts, body)).await;
    let Some(((workspace_id, project_id, scope), identity)) = agent else {
        return response;
    };
    let is_json = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    if !response.status().is_success() || !is_json {
        return response;
    }
    match with_directives(
        &state,
        &workspace_id,
        &project_id,
        &scope,
        &identity,
        response,
    )
    .await
    {
        Ok(response) => response,
        Err((response, error)) => {
            eprintln!("omni-api: could not add the directives to an answer: {error:?}");
            response
        }
    }
}

/// What `with_directives` could not do.
#[derive(Debug)]
enum Failure {
    Database,
    NoObject,
}

/// The answer with the directives the identity has not been given yet. The
/// receipts are written in a transaction that commits once the answer has them,
/// so a directive is never used up without being given. On failure the answer
/// comes back as it was.
async fn with_directives(
    state: &AppState,
    workspace_id: &str,
    project_id: &str,
    scope: &Scope,
    identity: &AgentIdentity,
    response: Response,
) -> Result<Response, (Response, Failure)> {
    let mut tx = match state.workspace_tx(workspace_id).await {
        Ok(tx) => tx,
        Err(_) => return Err((response, Failure::Database)),
    };
    let (changeset, thread) = match scope {
        Scope::Changeset(id) => (Some(id.as_str()), None),
        Scope::Thread(id) => (None, Some(id.as_str())),
    };
    let rows: Result<Vec<PostRow>, sqlx::Error> = sqlx::query_as(&format!(
        "WITH given AS (\
             INSERT INTO directive_receipts (workspace_id, post_id, agent) \
             SELECT p.workspace_id, p.id, $4::text::jsonb \
             FROM posts p \
             JOIN threads t ON t.workspace_id = p.workspace_id AND t.id = p.thread_id \
             WHERE p.type = 'directive' AND t.project_id = $1::uuid AND t.changeset_id = COALESCE(\
                 $2::uuid, (SELECT changeset_id FROM threads WHERE id = $3::uuid AND project_id = $1::uuid)) \
             ON CONFLICT (workspace_id, post_id, agent) DO NOTHING \
             RETURNING post_id\
         ) \
         SELECT {POST_COLUMNS} FROM posts WHERE id IN (SELECT post_id FROM given) \
         ORDER BY created_at, id"
    ))
    .bind(project_id)
    .bind(changeset)
    .bind(thread)
    .bind(json!({ "run": identity.run(), "role": identity.role() }).to_string())
    .fetch_all(&mut *tx)
    .await;
    let rows = match rows {
        Ok(rows) if rows.is_empty() => return Ok(response),
        Ok(rows) => rows,
        Err(error) => {
            eprintln!("omni-api: database error: {error}");
            return Err((response, Failure::Database));
        }
    };
    let mut directives = match rows
        .into_iter()
        .map(PostRow::into_body)
        .collect::<Result<Vec<_>, ApiError>>()
    {
        Ok(directives) => directives,
        Err(_) => return Err((response, Failure::Database)),
    };
    // A link of a directive to a post that a sealed phase hides from the agent
    // is left out, as in every other read.
    if hide_unseen_links(&mut tx, &Viewer::agent(identity), &mut directives)
        .await
        .is_err()
    {
        return Err((response, Failure::Database));
    }

    let (mut parts, body) = response.into_parts();
    let bytes = match to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(_) => return Err((ApiError::Unavailable.into_response(), Failure::Database)),
    };
    let answer = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|answer| match answer {
            Value::Object(mut object) => {
                object.insert("directives".to_string(), json!(directives));
                Some(Value::Object(object))
            }
            _ => None,
        });
    let Some(answer) = answer else {
        return Err((
            Response::from_parts(parts, Body::from(bytes)),
            Failure::NoObject,
        ));
    };
    if tx.commit().await.is_err() {
        return Err((
            Response::from_parts(parts, Body::from(bytes)),
            Failure::Database,
        ));
    }
    let answer = answer.to_string().into_bytes();
    parts
        .headers
        .insert(CONTENT_LENGTH, HeaderValue::from(answer.len()));
    Ok(Response::from_parts(parts, Body::from(answer)))
}
