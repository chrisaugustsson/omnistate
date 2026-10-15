//! Who is calling and which workspace they reach. Every route under
//! `/api/v1/w/{workspace_id}` takes one of the extractors here, which
//! authenticates the request, decides whether it acts as an agent (an API key)
//! or as the human (a session), and checks that the workspace in the path is
//! one the caller can reach. The workspace comes from the path and nowhere
//! else; a body, query parameter or header never names it.

use crate::AppState;
use crate::session::{ApiError, session_token};
use axum::Json;
use axum::extract::{FromRequestParts, RawPathParams};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use utoipa::ToSchema;

/// Who made a request to a workspace route.
pub enum Actor {
    /// An API key. Agents never act as the human.
    Agent,
    /// A logged-in browser session, as a member of the workspace.
    Human {
        user_id: String,
        membership_id: String,
    },
}

/// A request whose caller is authenticated and may reach the path's workspace.
pub struct WorkspaceRequest {
    /// The workspace of the path, as a lowercase UUID.
    pub workspace_id: String,
    pub actor: Actor,
}

impl FromRequestParts<AppState> for WorkspaceRequest {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        resolve(parts, state, false).await
    }
}

/// A request to a `human` route: only a session passes, and an API key is
/// turned down with 403 before the handler or any lookup of the path's IDs.
pub struct HumanRequest {
    pub workspace_id: String,
    pub membership_id: String,
}

impl FromRequestParts<AppState> for HumanRequest {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let request = resolve(parts, state, true).await?;
        match request.actor {
            Actor::Human { membership_id, .. } => Ok(Self {
                workspace_id: request.workspace_id,
                membership_id,
            }),
            Actor::Agent => Err(ApiError::Forbidden),
        }
    }
}

/// A request to an `agent` route: only an API key passes. The human never
/// writes to a plan, so a session is turned down with 403, after the
/// workspace of the path was checked (a session of another workspace is 404).
pub struct AgentRequest {
    pub workspace_id: String,
}

impl FromRequestParts<AppState> for AgentRequest {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let request = resolve(parts, state, false).await?;
        match request.actor {
            Actor::Agent => Ok(Self {
                workspace_id: request.workspace_id,
            }),
            Actor::Human { .. } => Err(ApiError::Forbidden),
        }
    }
}

impl AppState {
    /// Starts a transaction that runs as `workspace_id`: `app.workspace_id` is
    /// set for the transaction only (`SET LOCAL`), so row-level security shows
    /// that workspace's rows and no others.
    pub(crate) async fn workspace_tx(
        &self,
        workspace_id: &str,
    ) -> Result<Transaction<'static, Postgres>, ApiError> {
        let mut tx = self.pool.begin().await.map_err(ApiError::database)?;
        sqlx::query("SELECT set_config('app.workspace_id', $1, true)")
            .bind(workspace_id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::database)?;
        Ok(tx)
    }
}

/// Who a request is: the one place that decides between agent and human, for
/// every route that needs a caller. A key wins over a cookie, and any
/// `Authorization` header means an agent, so a bad or revoked key is a 401 and
/// never falls back to the session cookie next to it. `/auth/login`, which
/// checks a password instead, does not call this.
pub(crate) async fn identify(parts: &Parts, state: &AppState) -> Result<Caller, ApiError> {
    if let Some(header) = parts.headers.get(AUTHORIZATION) {
        let secret = header.to_str().ok().and_then(bearer_secret);
        let key = match secret {
            Some(secret) => crate::api_keys::authenticate(state, secret).await?,
            None => None,
        };
        return key.map(Caller::Key).ok_or(ApiError::Unauthorized);
    }
    let token = session_token(parts).ok_or(ApiError::Unauthorized)?;
    match state.auth.session_user(&token).await? {
        Some(user) => Ok(Caller::Session { token, user }),
        None => Err(ApiError::Unauthorized),
    }
}

/// The authenticated caller of a request.
pub(crate) enum Caller {
    Key(crate::api_keys::ResolvedKey),
    Session {
        token: String,
        user: crate::auth::AuthUser,
    },
}

/// Authenticates the request, then checks the workspace. Credentials come
/// first, so a bad credential is a 401 whatever the workspace. With
/// `human_only`, an API key is a 403 before the workspace is looked at.
async fn resolve(
    parts: &mut Parts,
    state: &AppState,
    human_only: bool,
) -> Result<WorkspaceRequest, ApiError> {
    let caller = identify(parts, state).await?;
    if human_only && matches!(caller, Caller::Key(_)) {
        return Err(ApiError::Forbidden);
    }

    let raw = RawPathParams::from_request_parts(parts, state)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let workspace_id = raw
        .iter()
        .find(|(name, _)| *name == "workspace_id")
        .and_then(|(_, value)| parse_uuid(value))
        .ok_or(ApiError::NotFound)?;

    let actor = match caller {
        Caller::Key(key) => {
            if key.workspace_id != workspace_id {
                return Err(ApiError::NotFound);
            }
            Actor::Agent
        }
        Caller::Session { user, .. } => {
            let membership: Option<(String,)> = sqlx::query_as(
                "SELECT id::text FROM user_memberships($1::uuid) WHERE workspace_id = $2::uuid",
            )
            .bind(&user.id)
            .bind(&workspace_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(ApiError::database)?;
            let (membership_id,) = membership.ok_or(ApiError::NotFound)?;
            Actor::Human {
                user_id: user.id,
                membership_id,
            }
        }
    };
    Ok(WorkspaceRequest {
        workspace_id,
        actor,
    })
}

/// The secret of an `Authorization: Bearer <secret>` header value.
fn bearer_secret(header: &str) -> Option<&str> {
    let (scheme, secret) = header.split_once(' ')?;
    let secret = secret.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !secret.is_empty()).then_some(secret)
}

/// A UUID in its 36-character form, as lowercase; `None` for anything else.
pub(crate) fn parse_uuid(text: &str) -> Option<String> {
    let well_formed = text.len() == 36
        && text.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        });
    well_formed.then(|| text.to_ascii_lowercase())
}

/// The lowercase hex SHA-256 of `text`: what `api_keys.key_hash` holds.
pub(crate) fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Serialize, ToSchema)]
pub struct ActorBody {
    /// `agent` for an API key, `human` for a browser session.
    pub kind: String,
    /// The user's UUID; only for `human`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct WhoamiResponse {
    pub workspace_id: String,
    pub actor: ActorBody,
}

/// Who the caller is in this workspace: an agent (API key) or the human (session).
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/whoami",
    params(("workspace_id" = String, Path, description = "The workspace's UUID")),
    responses(
        (status = 200, description = "The caller and the workspace", body = WhoamiResponse),
        (status = 401, description = "No valid API key or session", body = crate::session::ErrorBody),
        (status = 404, description = "The caller cannot reach this workspace", body = crate::session::ErrorBody),
    )
)]
pub async fn whoami(request: WorkspaceRequest) -> Json<WhoamiResponse> {
    let actor = match request.actor {
        Actor::Agent => ActorBody {
            kind: "agent".to_string(),
            user_id: None,
        },
        Actor::Human { user_id, .. } => ActorBody {
            kind: "human".to_string(),
            user_id: Some(user_id),
        },
    };
    Json(WhoamiResponse {
        workspace_id: request.workspace_id,
        actor,
    })
}
