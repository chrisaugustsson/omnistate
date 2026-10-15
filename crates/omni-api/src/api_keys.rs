//! API keys: the credential of an agent. A key is a public prefix, a `_` and 32
//! random bytes as 64 hex digits. The secret is shown once, when the key is
//! created; the database keeps only its SHA-256. A request finds its key by
//! the prefix and compares the hashes in constant time.
//!
//! Listing, creating and revoking keys are `human` routes: an agent cannot
//! mint or revoke keys.

use crate::AppState;
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{HumanRequest, parse_uuid, sha256_hex};
use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Serialize;
use utoipa::ToSchema;

/// The longest prefix a key may have.
const MAX_PREFIX_LEN: usize = 32;
/// How many random prefixes `create` tries before it gives up on a collision.
const PREFIX_ATTEMPTS: u32 = 5;

/// The columns of a key that the API shows, with timestamps as ISO 8601 text.
/// The hash is not among them.
const KEY_COLUMNS: &str = "id::text, prefix, scopes, \
    to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at, \
    to_char(last_used_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS last_used_at, \
    to_char(revoked_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS revoked_at";

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct ApiKeyBody {
    pub id: String,
    /// The public handle that finds the key; not secret.
    pub prefix: String,
    /// What the key may do. v1 issues full-access keys only: `["*"]`.
    pub scopes: Vec<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct KeyResponse {
    pub key: ApiKeyBody,
}

#[derive(Serialize, ToSchema)]
pub struct KeyListResponse {
    pub keys: Vec<ApiKeyBody>,
}

#[derive(Serialize, ToSchema)]
pub struct CreatedKeyResponse {
    pub key: ApiKeyBody,
    /// The whole key, to send as `Authorization: Bearer <secret>`. It is shown
    /// only in this response.
    pub secret: String,
}

/// A key that a request presented and that is valid: it exists, the secret
/// matches and it is not revoked.
pub struct ResolvedKey {
    pub workspace_id: String,
}

/// A new key: its prefix, the secret and the hash to store.
fn generate() -> (String, String, String) {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut prefix_bytes = [0u8; 8];
    OsRng.fill_bytes(&mut prefix_bytes);
    let prefix: String = std::iter::once("omni".to_string())
        .chain(
            prefix_bytes
                .iter()
                .map(|b| (ALPHABET[*b as usize % ALPHABET.len()] as char).to_string()),
        )
        .collect();
    let mut random = [0u8; 32];
    OsRng.fill_bytes(&mut random);
    let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let secret = format!("{prefix}_{hex}");
    let hash = sha256_hex(&secret);
    (prefix, secret, hash)
}

/// Compares in time that does not depend on where the inputs differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Looks a presented secret up by its prefix and checks it. `None` when the
/// secret is malformed, the prefix is unknown, the hash differs or the key is
/// revoked. A valid key's `last_used_at` is refreshed, at most once a minute.
pub async fn authenticate(state: &AppState, secret: &str) -> Result<Option<ResolvedKey>, ApiError> {
    let Some((prefix, _)) = secret.rsplit_once('_') else {
        return Ok(None);
    };
    if prefix.is_empty()
        || prefix.len() > MAX_PREFIX_LEN
        || !prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Ok(None);
    }
    let row: Option<(String, String, String, bool, bool)> = sqlx::query_as(
        "SELECT id::text, workspace_id::text, key_hash, revoked_at IS NOT NULL, \
                last_used_at IS NULL OR last_used_at < now() - interval '1 minute' \
         FROM resolve_api_key($1)",
    )
    .bind(prefix)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?;
    let Some((id, workspace_id, key_hash, revoked, stale)) = row else {
        return Ok(None);
    };
    let matches = constant_time_eq(key_hash.as_bytes(), sha256_hex(secret).as_bytes());
    if !matches || revoked {
        return Ok(None);
    }
    if stale {
        record_use(state.clone(), id, workspace_id.clone());
    }
    Ok(Some(ResolvedKey { workspace_id }))
}

/// Writes `last_used_at` in the background, in a transaction of the key's
/// workspace, so the request does not wait for it. The condition repeats the
/// one-minute rule, so two requests racing for the first write cause one.
fn record_use(state: AppState, key_id: String, workspace_id: String) {
    tokio::spawn(async move {
        let write = async {
            let mut tx = state.workspace_tx(&workspace_id).await?;
            sqlx::query(
                "UPDATE api_keys SET last_used_at = now() WHERE id = $1::uuid \
                 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')",
            )
            .bind(&key_id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::database)?;
            tx.commit().await.map_err(ApiError::database)
        };
        // A failed write is already logged and costs nothing but a stale timestamp.
        let _ = write.await;
    });
}

/// The keys of the workspace, revoked ones included.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/api-keys",
    params(("workspace_id" = String, Path, description = "The workspace's UUID")),
    responses(
        (status = 200, description = "The workspace's keys, without secrets", body = KeyListResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "The session is not a member of this workspace", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    request: HumanRequest,
) -> Result<Json<KeyListResponse>, ApiError> {
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let keys = sqlx::query_as(&format!(
        "SELECT {KEY_COLUMNS} FROM api_keys ORDER BY created_at, id"
    ))
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::database)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(KeyListResponse { keys }))
}

/// Creates a full-access key for the workspace and shows its secret, once.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/api-keys",
    params(("workspace_id" = String, Path, description = "The workspace's UUID")),
    responses(
        (status = 201, description = "The new key and its secret, shown only now", body = CreatedKeyResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "The session is not a member of this workspace", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    request: HumanRequest,
) -> Result<(StatusCode, Json<CreatedKeyResponse>), ApiError> {
    for _ in 0..PREFIX_ATTEMPTS {
        let (prefix, secret, hash) = generate();
        let mut tx = state.workspace_tx(&request.workspace_id).await?;
        let inserted = sqlx::query_as(&format!(
            "INSERT INTO api_keys (workspace_id, prefix, key_hash, scopes, created_by_membership_id) \
             VALUES ($1::uuid, $2, $3, ARRAY['*'], $4::uuid) RETURNING {KEY_COLUMNS}"
        ))
        .bind(&request.workspace_id)
        .bind(&prefix)
        .bind(&hash)
        .bind(&request.membership_id)
        .fetch_one(&mut *tx)
        .await;
        match inserted {
            Ok(key) => {
                tx.commit().await.map_err(ApiError::database)?;
                return Ok((
                    StatusCode::CREATED,
                    Json(CreatedKeyResponse { key, secret }),
                ));
            }
            // The prefix is taken, in this or another workspace: try another.
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => continue,
            Err(e) => return Err(ApiError::database(e)),
        }
    }
    eprintln!("omni-api: no free API key prefix after {PREFIX_ATTEMPTS} attempts");
    Err(ApiError::Unavailable)
}

/// Revokes a key. The row is kept, with `revoked_at` set, and the key stops
/// working at once.
#[utoipa::path(
    post,
    path = "/api/v1/w/{workspace_id}/api-keys/{key_id}/revoke",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("key_id" = String, Path, description = "The key's UUID"),
    ),
    responses(
        (status = 200, description = "The revoked key", body = KeyResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "No such key in this workspace", body = ErrorBody),
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    request: HumanRequest,
    Path((_, key_id)): Path<(String, String)>,
) -> Result<Json<KeyResponse>, ApiError> {
    let key_id = parse_uuid(&key_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    // Row-level security hides the keys of other workspaces, so they are 404 too.
    let key = sqlx::query_as(&format!(
        "UPDATE api_keys SET revoked_at = COALESCE(revoked_at, now()) \
         WHERE id = $1::uuid RETURNING {KEY_COLUMNS}"
    ))
    .bind(&key_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::database)?;
    Ok(Json(KeyResponse { key }))
}
