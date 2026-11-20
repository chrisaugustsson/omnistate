//! API keys: the credential of an agent. A key is a public prefix, a `_` and 32
//! random bytes as 64 hex digits. The secret is shown once, when the key is
//! created; the database keeps only its SHA-256. A request finds its key by
//! the prefix and compares the hashes in constant time.
//!
//! A key belongs to the user who created it and reaches every workspace that
//! user is a member of; `workspace.rs` checks the membership on each request.
//! Listing, creating and revoking keys are `human` routes of the account, not
//! of a workspace: an agent cannot mint or revoke keys.

use crate::AppState;
use crate::session::{ApiError, CurrentUser, ErrorBody};
use crate::workspace::{parse_uuid, sha256_hex};
use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Serialize;
use subtle::ConstantTimeEq;
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
    /// The user the key belongs to.
    pub user_id: String,
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
    a.ct_eq(b).into()
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
        "SELECT id::text, user_id::text, key_hash, revoked_at IS NOT NULL, \
                last_used_at IS NULL OR last_used_at < now() - interval '1 minute' \
         FROM resolve_api_key($1)",
    )
    .bind(prefix)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?;
    let Some((id, user_id, key_hash, revoked, stale)) = row else {
        return Ok(None);
    };
    let matches = constant_time_eq(key_hash.as_bytes(), sha256_hex(secret).as_bytes());
    if !matches || revoked {
        return Ok(None);
    }
    if stale {
        record_use(state.clone(), id);
    }
    Ok(Some(ResolvedKey { user_id }))
}

/// Writes `last_used_at` in the background, so the request does not wait for
/// it. The condition repeats the one-minute rule, so two requests racing for
/// the first write cause one.
fn record_use(state: AppState, key_id: String) {
    tokio::spawn(async move {
        let written = sqlx::query(
            "UPDATE api_keys SET last_used_at = now() WHERE id = $1::uuid \
             AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')",
        )
        .bind(&key_id)
        .execute(&state.pool)
        .await;
        // A failed write costs nothing but a stale timestamp; it is logged.
        if let Err(e) = written {
            let _ = ApiError::database(e);
        }
    });
}

/// The logged-in user's keys, revoked ones included.
#[utoipa::path(
    get,
    path = "/api/v1/api-keys",
    responses(
        (status = 200, description = "The user's keys, without secrets", body = KeyListResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<KeyListResponse>, ApiError> {
    let keys = sqlx::query_as(&format!(
        "SELECT {KEY_COLUMNS} FROM api_keys WHERE user_id = $1::uuid ORDER BY created_at, id"
    ))
    .bind(&current.user.id)
    .fetch_all(&state.pool)
    .await
    .map_err(ApiError::database)?;
    Ok(Json(KeyListResponse { keys }))
}

/// Creates a full-access key for the logged-in user and shows its secret,
/// once. The key reaches every workspace the user is a member of.
#[utoipa::path(
    post,
    path = "/api/v1/api-keys",
    responses(
        (status = 201, description = "The new key and its secret, shown only now", body = CreatedKeyResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<(StatusCode, Json<CreatedKeyResponse>), ApiError> {
    for _ in 0..PREFIX_ATTEMPTS {
        let (prefix, secret, hash) = generate();
        let inserted = sqlx::query_as(&format!(
            "INSERT INTO api_keys (user_id, prefix, key_hash, scopes) \
             VALUES ($1::uuid, $2, $3, ARRAY['*']) RETURNING {KEY_COLUMNS}"
        ))
        .bind(&current.user.id)
        .bind(&prefix)
        .bind(&hash)
        .fetch_one(&state.pool)
        .await;
        match inserted {
            Ok(key) => {
                return Ok((
                    StatusCode::CREATED,
                    Json(CreatedKeyResponse { key, secret }),
                ));
            }
            // The prefix is taken, by this or another user's key: try another.
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
    path = "/api/v1/api-keys/{key_id}/revoke",
    params(("key_id" = String, Path, description = "The key's UUID")),
    responses(
        (status = 200, description = "The revoked key", body = KeyResponse),
        (status = 401, description = "No valid session", body = ErrorBody),
        (status = 403, description = "Called with an API key; human only", body = ErrorBody),
        (status = 404, description = "The user has no such key", body = ErrorBody),
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(key_id): Path<String>,
) -> Result<Json<KeyResponse>, ApiError> {
    let key_id = parse_uuid(&key_id).ok_or(ApiError::NotFound)?;
    // Another user's key is a 404 too, the same answer as one that does not exist.
    let key = sqlx::query_as(&format!(
        "UPDATE api_keys SET revoked_at = COALESCE(revoked_at, now()) \
         WHERE id = $1::uuid AND user_id = $2::uuid RETURNING {KEY_COLUMNS}"
    ))
    .bind(&key_id)
    .bind(&current.user.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::database)?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(KeyResponse { key }))
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    /// A SHA-256 as 64 hex digits, what the comparison sees in `authenticate`.
    const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    #[test]
    fn equal_inputs_match() {
        assert!(constant_time_eq(HASH.as_bytes(), HASH.as_bytes()));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn inputs_of_different_lengths_never_match() {
        let shorter = &HASH.as_bytes()[..63];
        assert!(!constant_time_eq(HASH.as_bytes(), shorter));
        assert!(!constant_time_eq(shorter, HASH.as_bytes()));
        // One side is a prefix of the other, so equal as far as the shorter goes.
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(!constant_time_eq(b"", HASH.as_bytes()));
        assert!(!constant_time_eq(HASH.as_bytes(), b""));
        // A stored hash twice as long, whose first half is the presented one.
        let doubled = format!("{HASH}{HASH}");
        assert!(!constant_time_eq(HASH.as_bytes(), doubled.as_bytes()));
    }

    #[test]
    fn a_difference_in_any_single_byte_does_not_match() {
        for i in 0..HASH.len() {
            let mut other = HASH.as_bytes().to_vec();
            other[i] ^= 0x01;
            assert!(
                !constant_time_eq(HASH.as_bytes(), &other),
                "a difference at byte {i} must not match"
            );
        }
    }
}
