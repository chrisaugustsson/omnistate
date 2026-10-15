//! The OmniState API: the HTTP and WebSocket server that the web app and the
//! `omni` command line talk to. This is the skeleton behind the reverse proxy;
//! the routes of later stories hang off the same router.
//!
//! The API talks to Postgres as `omni_api`, a role that owns no table and cannot
//! bypass row-level security (migrations/README.md). Only start-up uses the
//! owner's login: it applies the migrations and sets the API role's password.

mod api_keys;
pub mod auth;
mod changesets;
pub mod delete_workspace;
mod openapi;
pub mod password;
pub mod routes;
mod session;
pub mod setup;
mod workspace;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{any, get, post};
use axum::{Json, Router};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, PgConnection, PgPool};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

/// How long a health check waits for Postgres or RustFS before it reports an error.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Where the API listens and the services it talks to, read from the environment.
pub struct Config {
    pub bind: String,
    /// The login of the API role `omni_api`, which every request runs as.
    pub database_url: String,
    /// The login of the database owner, used at start-up to apply migrations.
    pub admin_database_url: String,
    /// The password to give the API role, the one in `database_url`.
    pub api_db_password: String,
    pub s3_endpoint: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_bucket: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let required = |name: &str| std::env::var(name).map_err(|_| format!("{name} is not set"));
        Ok(Self {
            bind: std::env::var("OMNI_API_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            database_url: required("OMNI_DATABASE_URL")?,
            admin_database_url: required("OMNI_ADMIN_DATABASE_URL")?,
            api_db_password: required("OMNI_API_DB_PASSWORD")?,
            s3_endpoint: required("OMNI_S3_ENDPOINT")?,
            s3_access_key: required("OMNI_S3_ACCESS_KEY")?,
            s3_secret_key: required("OMNI_S3_SECRET_KEY")?,
            s3_bucket: required("OMNI_S3_BUCKET")?,
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    pool: PgPool,
    bucket: Box<Bucket>,
    /// Decides who a login or a session cookie belongs to.
    auth: Arc<dyn auth::AuthProvider>,
}

impl AppState {
    /// Prepares the connections. Postgres connects on first use, so the API
    /// starts (and reports the problem on /api/health) while it is still down.
    pub fn new(config: &Config) -> Result<Self, String> {
        Self::build(config, None)
    }

    /// Like `new`, but every login and session check goes to `provider`
    /// instead of the built-in password login.
    pub fn with_auth_provider(
        config: &Config,
        provider: Arc<dyn auth::AuthProvider>,
    ) -> Result<Self, String> {
        Self::build(config, Some(provider))
    }

    fn build(
        config: &Config,
        provider: Option<Arc<dyn auth::AuthProvider>>,
    ) -> Result<Self, String> {
        let pool = PgPoolOptions::new()
            .acquire_timeout(CHECK_TIMEOUT)
            .connect_lazy(&config.database_url)
            .map_err(|e| format!("invalid database url: {e}"))?;
        let bucket = Bucket::new(
            &config.s3_bucket,
            region(&config.s3_endpoint),
            credentials(&config.s3_access_key, &config.s3_secret_key)?,
        )
        .map_err(|e| format!("invalid storage settings: {e}"))?
        .with_path_style();
        let auth = provider.unwrap_or_else(|| Arc::new(password::PasswordAuth::new(pool.clone())));
        Ok(Self { pool, bucket, auth })
    }

    /// Creates the RustFS bucket if it does not exist yet, waiting up to
    /// `attempts` seconds for RustFS to come up.
    pub async fn ensure_bucket(&self, config: &Config, attempts: u32) -> Result<(), String> {
        let mut last_error = String::new();
        for _ in 0..attempts {
            match self.bucket.exists().await {
                Ok(true) => return Ok(()),
                Ok(false) => {
                    let created = Bucket::create_with_path_style(
                        &config.s3_bucket,
                        region(&config.s3_endpoint),
                        credentials(&config.s3_access_key, &config.s3_secret_key)?,
                        BucketConfiguration::default(),
                    )
                    .await;
                    match created {
                        Ok(_) => return Ok(()),
                        Err(e) => last_error = e.to_string(),
                    }
                }
                Err(e) => last_error = e.to_string(),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err(format!(
            "could not create the bucket {}: {last_error}",
            config.s3_bucket
        ))
    }

    async fn bucket_exists(&self) -> bool {
        matches!(
            tokio::time::timeout(CHECK_TIMEOUT, self.bucket.exists()).await,
            Ok(Ok(true))
        )
    }

    async fn database_ok(&self) -> bool {
        check(sqlx::query("SELECT 1").execute(&self.pool)).await
    }
}

/// Applies the migrations of `migrations/` as the database owner and sets
/// the API role's password, waiting up to `attempts` seconds for Postgres to
/// come up. Migrations that are applied already are skipped.
pub async fn migrate(config: &Config, attempts: u32) -> Result<(), String> {
    let mut last_error = String::new();
    for _ in 0..attempts {
        match PgConnection::connect(&config.admin_database_url).await {
            Ok(mut conn) => return apply_migrations(&mut conn, &config.api_db_password).await,
            Err(e) => last_error = e.to_string(),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(format!("cannot connect to Postgres: {last_error}"))
}

async fn apply_migrations(conn: &mut PgConnection, api_password: &str) -> Result<(), String> {
    sqlx::migrate!("../../migrations")
        .run(&mut *conn)
        .await
        .map_err(|e| format!("applying migrations failed: {e}"))?;
    // ALTER ROLE takes no bind parameters, so Postgres quotes the literal.
    let alter: String =
        sqlx::query_scalar("SELECT format('ALTER ROLE omni_api PASSWORD %L', $1::text)")
            .bind(api_password)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| e.to_string())?;
    sqlx::query(&alter)
        .execute(&mut *conn)
        .await
        .map_err(|e| format!("cannot set the password of omni_api: {e}"))?;
    Ok(())
}

fn region(endpoint: &str) -> Region {
    Region::Custom {
        region: "us-east-1".to_string(),
        endpoint: endpoint.to_string(),
    }
}

fn credentials(access_key: &str, secret_key: &str) -> Result<Credentials, String> {
    Credentials::new(Some(access_key), Some(secret_key), None, None, None)
        .map_err(|e| format!("invalid storage credentials: {e}"))
}

async fn check<T, E>(call: impl Future<Output = Result<T, E>>) -> bool {
    matches!(tokio::time::timeout(CHECK_TIMEOUT, call).await, Ok(Ok(_)))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/openapi.json", get(openapi::document))
        .route("/api/v1/auth/login", post(session::login))
        .route("/api/v1/auth/logout", post(session::logout))
        .route("/api/v1/auth/me", get(session::me))
        .route("/api/v1/auth/workspaces", get(session::workspaces))
        .route("/api/v1/w/{workspace_id}/whoami", get(workspace::whoami))
        .route(
            "/api/v1/w/{workspace_id}/api-keys",
            get(api_keys::list).post(api_keys::create),
        )
        .route(
            "/api/v1/w/{workspace_id}/api-keys/{key_id}/revoke",
            post(api_keys::revoke),
        )
        .route(
            "/api/v1/w/{workspace_id}/projects",
            get(changesets::list_projects).post(changesets::create_project),
        )
        .route(
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
            get(changesets::list_changesets).post(changesets::create_changeset),
        )
        .route(
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}",
            get(changesets::show_changeset),
        )
        .route(
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/submit",
            post(changesets::submit),
        )
        .route(
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reject",
            post(changesets::reject),
        )
        .route(
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reopen",
            post(changesets::reopen),
        )
        .route(
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/abandon",
            post(changesets::abandon),
        )
        .route("/ws/w/{workspace_id}", any(unauthorized))
        .fallback(not_found)
        .with_state(state)
}

/// Reports whether the API can reach Postgres and the RustFS bucket.
#[utoipa::path(
    get,
    path = "/api/health",
    responses(
        (status = 200, description = "Postgres and the RustFS bucket are reachable"),
        (status = 503, description = "One of them is not; the body says which"),
    )
)]
async fn health(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let (database, storage) = tokio::join!(state.database_ok(), state.bucket_exists());
    let word = |ok: bool| if ok { "ok" } else { "error" };
    let all_ok = database && storage;
    let status = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "status": if all_ok { "ok" } else { "degraded" },
            "database": word(database),
            "storage": word(storage),
        })),
    )
}

/// A workspace socket needs credentials, and no login exists yet.
async fn unauthorized() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "unauthorized" })),
    )
}

async fn not_found() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" })))
}
