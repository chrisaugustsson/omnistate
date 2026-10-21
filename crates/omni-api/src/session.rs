//! The login, logout and `me` routes, and the extractor that turns the session
//! cookie into the logged-in user. Every decision about who someone is goes
//! through the state's `AuthProvider`.

use crate::AppState;
use crate::auth::{AuthError, AuthUser};
use crate::workspace::{Caller, identify};
use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::SystemTime;
use utoipa::ToSchema;

/// The name of the session cookie.
pub const SESSION_COOKIE: &str = "omni_session";

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct UserBody {
    /// The user's UUID.
    pub id: String,
    pub email: String,
}

#[derive(Serialize, ToSchema)]
pub struct UserResponse {
    pub user: UserBody,
}

#[derive(Serialize, ToSchema)]
pub struct WorkspaceBody {
    /// The workspace's UUID.
    pub id: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct WorkspaceListResponse {
    pub workspaces: Vec<WorkspaceBody>,
}

/// The body of every error response: a stable code such as `unauthorized`.
#[derive(Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
}

impl From<AuthUser> for UserResponse {
    fn from(user: AuthUser) -> Self {
        Self {
            user: UserBody {
                id: user.id,
                email: user.email,
            },
        }
    }
}

/// Why a request was turned down; renders as the JSON error body.
pub enum ApiError {
    InvalidCredentials,
    Unauthorized,
    /// A human-only route was called with an API key.
    Forbidden,
    /// The workspace or record is not one the caller can reach. It is the same
    /// answer for one that does not exist, so nothing leaks.
    NotFound,
    /// The request is refused with a status of its own and a stable code, such
    /// as 409 `invalid_transition`.
    Refused(StatusCode, &'static str),
    /// As `Refused`, and the body names a record next to the code, such as 409
    /// `decision_not_approved` with its `decision_id`: the status, the code,
    /// the name of the field and the record's ID.
    Naming(StatusCode, &'static str, &'static str, String),
    /// As `Naming`, and the field holds any JSON, such as the comments that
    /// stop a save (409 `commented_nodes_removed`): the status, the code, the
    /// name of the field and its value.
    Listing(StatusCode, &'static str, &'static str, serde_json::Value),
    Unavailable,
}

impl ApiError {
    /// 400 with the stable `code` (`invalid_request`, `reason_required`, ...).
    pub fn bad_request(code: &'static str) -> Self {
        Self::Refused(StatusCode::BAD_REQUEST, code)
    }

    /// 409 with the stable `code` (`name_taken`, `invalid_transition`, ...).
    pub fn conflict(code: &'static str) -> Self {
        Self::Refused(StatusCode::CONFLICT, code)
    }

    /// A database failure: it is logged and the caller sees a 503.
    pub fn database(error: sqlx::Error) -> Self {
        eprintln!("omni-api: database error: {error}");
        Self::Unavailable
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            Self::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                json!({ "error": "invalid_credentials" }),
            ),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, json!({ "error": "unauthorized" })),
            Self::Forbidden => (StatusCode::FORBIDDEN, json!({ "error": "forbidden" })),
            Self::NotFound => (StatusCode::NOT_FOUND, json!({ "error": "not_found" })),
            Self::Refused(status, code) => (status, json!({ "error": code })),
            Self::Naming(status, code, field, id) => (status, json!({ "error": code, field: id })),
            Self::Listing(status, code, field, value) => {
                (status, json!({ "error": code, field: value }))
            }
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                json!({ "error": "unavailable" }),
            ),
        };
        (status, Json(body)).into_response()
    }
}

impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::InvalidCredentials => Self::InvalidCredentials,
            AuthError::Unavailable(reason) => {
                eprintln!("omni-api: the auth provider is unavailable: {reason}");
                Self::Unavailable
            }
        }
    }
}

/// The user of the request's session cookie. A request without a live session
/// is rejected with 401, and one with an `Authorization` header (an agent) with
/// 403, before the handler runs.
pub struct CurrentUser {
    pub token: String,
    pub user: AuthUser,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match identify(parts, state).await? {
            Caller::Session { token, user } => Ok(Self { token, user }),
            Caller::Key(_) => Err(ApiError::Forbidden),
        }
    }
}

/// The value of the session cookie, if the request has a non-empty one.
pub(crate) fn session_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, value)| *name == SESSION_COOKIE && !value.is_empty())
        .map(|(_, value)| value.to_string())
}

fn set_cookie(value: &str, max_age_secs: u64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={value}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={max_age_secs}"
    ))
    .expect("a session token is a valid header value")
}

/// Logs in with an email and password and starts a session.
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Logged in; the session cookie `omni_session` is set (HttpOnly, Secure, SameSite=Lax)", body = UserResponse),
        (status = 401, description = "Unknown email or wrong password (`invalid_credentials`)", body = ErrorBody),
        (status = 503, description = "The login provider cannot answer", body = ErrorBody),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let session = state.auth.login(&request.email, &request.password).await?;
    let max_age = session
        .expires_at
        .duration_since(SystemTime::now())
        .unwrap_or_default()
        .as_secs();
    let cookie = set_cookie(&session.token, max_age);
    let body = Json(UserResponse::from(session.user));
    Ok(([(SET_COOKIE, cookie)], body).into_response())
}

/// Ends the session of the cookie and clears it.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    responses(
        (status = 204, description = "The session is ended and the cookie cleared"),
        (status = 401, description = "No live session", body = ErrorBody),
        (status = 403, description = "Called with an API key (an Authorization header); human only", body = ErrorBody),
        (status = 503, description = "The login provider cannot answer", body = ErrorBody),
    )
)]
pub async fn logout(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Response, ApiError> {
    state.auth.logout(&current.token).await?;
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, set_cookie("", 0))]).into_response())
}

/// The logged-in user.
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, description = "The user of the session cookie", body = UserResponse),
        (status = 401, description = "No live session", body = ErrorBody),
        (status = 403, description = "Called with an API key (an Authorization header); human only", body = ErrorBody),
        (status = 503, description = "The login provider cannot answer", body = ErrorBody),
    )
)]
pub async fn me(current: CurrentUser) -> Json<UserResponse> {
    Json(UserResponse::from(current.user))
}

/// The workspaces the logged-in user is a member of, by name. It is how
/// `omni login` offers a choice before any workspace is picked.
#[utoipa::path(
    get,
    path = "/api/v1/auth/workspaces",
    responses(
        (status = 200, description = "The user's workspaces, ordered by name", body = WorkspaceListResponse),
        (status = 401, description = "No live session", body = ErrorBody),
        (status = 403, description = "Called with an API key (an Authorization header); human only", body = ErrorBody),
        (status = 503, description = "The database or the login provider cannot answer", body = ErrorBody),
    )
)]
pub async fn workspaces(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<WorkspaceListResponse>, ApiError> {
    let memberships: Vec<(String,)> =
        sqlx::query_as("SELECT workspace_id::text FROM user_memberships($1::uuid)")
            .bind(&current.user.id)
            .fetch_all(&state.pool)
            .await
            .map_err(ApiError::database)?;
    // Row-level security hides a workspace's name from every other workspace,
    // so each name is read in a transaction of its own workspace.
    let mut workspaces = Vec::with_capacity(memberships.len());
    for (id,) in memberships {
        let mut tx = state.workspace_tx(&id).await?;
        let name: Option<(String,)> =
            sqlx::query_as("SELECT name FROM workspaces WHERE id = $1::uuid")
                .bind(&id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(ApiError::database)?;
        tx.commit().await.map_err(ApiError::database)?;
        if let Some((name,)) = name {
            workspaces.push(WorkspaceBody { id, name });
        }
    }
    workspaces.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    Ok(Json(WorkspaceListResponse { workspaces }))
}
