//! The OmniState API: the HTTP and WebSocket server that the web app and the
//! `omni` command line talk to. This is the skeleton behind the reverse proxy;
//! the routes of later stories hang off the same router.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{any, get};
use axum::{Json, Router};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::future::Future;
use std::time::Duration;

/// How long a health check waits for Postgres or RustFS before it reports an error.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Where the API listens and the services it talks to, read from the environment.
pub struct Config {
    pub bind: String,
    pub database_url: String,
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
}

impl AppState {
    /// Prepares the connections. Postgres connects on first use, so the API
    /// starts (and reports the problem on /api/health) while it is still down.
    pub fn new(config: &Config) -> Result<Self, String> {
        let pool = PgPoolOptions::new()
            .acquire_timeout(CHECK_TIMEOUT)
            .connect_lazy(&config.database_url)
            .map_err(|e| format!("invalid database url: {e}"))?;
        let bucket = Bucket::new(&config.s3_bucket, region(config), credentials(config)?)
            .map_err(|e| format!("invalid storage settings: {e}"))?
            .with_path_style();
        Ok(Self { pool, bucket })
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
                        region(config),
                        credentials(config)?,
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

fn region(config: &Config) -> Region {
    Region::Custom {
        region: "us-east-1".to_string(),
        endpoint: config.s3_endpoint.clone(),
    }
}

fn credentials(config: &Config) -> Result<Credentials, String> {
    Credentials::new(
        Some(&config.s3_access_key),
        Some(&config.s3_secret_key),
        None,
        None,
        None,
    )
    .map_err(|e| format!("invalid storage credentials: {e}"))
}

async fn check<T, E>(call: impl Future<Output = Result<T, E>>) -> bool {
    matches!(tokio::time::timeout(CHECK_TIMEOUT, call).await, Ok(Ok(_)))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/ws/w/{workspace_id}", any(unauthorized))
        .fallback(not_found)
        .with_state(state)
}

/// Reports whether the API can reach Postgres and the RustFS bucket.
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
