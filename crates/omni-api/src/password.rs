//! The built-in `AuthProvider`: email and password login with Argon2id, and
//! sessions kept in the `sessions` table. A login method is a row of
//! `login_identities`; this provider reads the rows whose provider is
//! `password`, so identities of other providers can never log in here.

use crate::auth::{AuthError, AuthProvider, AuthUser, BoxFuture, Session};
use argon2::Argon2;
use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// How long a session lives after the login.
pub const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// The provider name in `login_identities` for email and password.
pub const PASSWORD_PROVIDER: &str = "password";

/// Hashes a password with Argon2id (the crate's default cost, 19 MiB of memory)
/// into a PHC string that carries its own salt and cost.
pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| format!("cannot hash the password: {e}"))
}

fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// Hashing takes tens of milliseconds, so it runs off the async workers.
async fn verify_blocking(hash: String, password: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&hash, &password))
        .await
        .unwrap_or(false)
}

/// A hash that no password matches, verified when the email is unknown so that
/// an unknown email takes as long as a wrong password.
fn dummy_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_password("no account has this password").expect("hash"))
}

/// A new opaque session token: 32 random bytes as 64 hex characters.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What `sessions.token_hash` holds: the SHA-256 of the cookie value.
fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn unavailable(e: sqlx::Error) -> AuthError {
    AuthError::Unavailable(e.to_string())
}

pub struct PasswordAuth {
    pool: PgPool,
}

impl PasswordAuth {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl AuthProvider for PasswordAuth {
    fn login<'a>(
        &'a self,
        email: &'a str,
        password: &'a str,
    ) -> BoxFuture<'a, Result<Session, AuthError>> {
        Box::pin(async move {
            let identity: Option<(String, String, Option<String>)> = sqlx::query_as(
                "SELECT u.id::text, u.email, i.secret_hash \
                 FROM login_identities i JOIN users u ON u.id = i.user_id \
                 WHERE i.provider = $1 AND i.subject = lower($2)",
            )
            .bind(PASSWORD_PROVIDER)
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(unavailable)?;

            let (user, hash) = match identity {
                Some((id, email, Some(hash))) => (AuthUser { id, email }, hash),
                _ => {
                    verify_blocking(dummy_hash().to_string(), password.to_string()).await;
                    return Err(AuthError::InvalidCredentials);
                }
            };
            if !verify_blocking(hash, password.to_string()).await {
                return Err(AuthError::InvalidCredentials);
            }

            let token = new_token();
            sqlx::query(
                "INSERT INTO sessions (user_id, token_hash, expires_at) \
                 VALUES ($1::uuid, $2, now() + make_interval(secs => $3))",
            )
            .bind(&user.id)
            .bind(token_hash(&token))
            .bind(SESSION_TTL.as_secs_f64())
            .execute(&self.pool)
            .await
            .map_err(unavailable)?;
            Ok(Session {
                token,
                expires_at: SystemTime::now() + SESSION_TTL,
                user,
            })
        })
    }

    fn session_user<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Option<AuthUser>, AuthError>> {
        Box::pin(async move {
            let user: Option<(String, String)> = sqlx::query_as(
                "WITH live AS (\
                     UPDATE sessions SET last_seen_at = now() \
                     WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > now() \
                     RETURNING user_id) \
                 SELECT u.id::text, u.email FROM live JOIN users u ON u.id = live.user_id",
            )
            .bind(token_hash(token))
            .fetch_optional(&self.pool)
            .await
            .map_err(unavailable)?;
            Ok(user.map(|(id, email)| AuthUser { id, email }))
        })
    }

    fn logout<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<(), AuthError>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE sessions SET revoked_at = now() \
                 WHERE token_hash = $1 AND revoked_at IS NULL",
            )
            .bind(token_hash(token))
            .execute(&self.pool)
            .await
            .map_err(unavailable)?;
            Ok(())
        })
    }
}
