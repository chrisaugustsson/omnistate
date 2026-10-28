//! US-019: `omni attach` against the real API and RustFS. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key (the
//! agent), create a scratch RustFS bucket and serve the API's router on a local
//! port against both. The built `omni` runs against that address with the key
//! in its config file. The contract of the CLI is in tests/attach.rs, and the
//! API's in crates/omni-api/tests/attachments.rs: the file's bytes are the
//! body of `POST /api/v1/w/{workspace}/attachments`, `GET` on the same path
//! plus `/{id}` answers them.

mod common;

use common::*;
use omni_api::Config;
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

const TEST_DB: &str = "omni_us019_cli";
const TEST_BUCKET: &str = "omni-us019-cli";
const KEY_PREFIX: &str = "omniustest19";

fn block<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(future)
}

fn admin_url() -> String {
    std::env::var("OMNI_ADMIN_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://omni:omni-dev@postgres:5432/omni".to_string())
}

fn admin_options() -> PgConnectOptions {
    PgConnectOptions::from_str(&admin_url()).expect("OMNI_ADMIN_DATABASE_URL")
}

fn api_password() -> String {
    // The password that the stack's API gives omni_api (docker-compose.yml). The
    // role is cluster-wide, so the scratch database must use the same one.
    std::env::var("OMNI_API_DB_PASSWORD").unwrap_or_else(|_| "omni-api-dev".to_string())
}

fn config() -> Config {
    let admin = admin_options();
    let url = admin_url();
    let (server, _database) = url.rsplit_once('/').expect("OMNI_ADMIN_DATABASE_URL");
    Config {
        bind: "127.0.0.1:0".to_string(),
        database_url: format!(
            "postgres://omni_api:{}@{}:{}/{TEST_DB}",
            api_password(),
            admin.get_host(),
            admin.get_port()
        ),
        admin_database_url: format!("{server}/{TEST_DB}"),
        api_db_password: api_password(),
        s3_endpoint: std::env::var("OMNI_S3_ENDPOINT")
            .unwrap_or_else(|_| "http://rustfs:9000".to_string()),
        s3_access_key: std::env::var("OMNI_S3_ACCESS_KEY")
            .unwrap_or_else(|_| "omni-dev".to_string()),
        s3_secret_key: std::env::var("OMNI_S3_SECRET_KEY")
            .unwrap_or_else(|_| "omni-dev-secret".to_string()),
        s3_bucket: TEST_BUCKET.to_string(),
    }
}

async fn admin_connection() -> PgConnection {
    PgConnection::connect_with(&admin_options().database(TEST_DB))
        .await
        .expect("connect to the scratch database as the admin role")
}

struct Fixture {
    url: String,
    port: u16,
    workspace: String,
    secret: String,
    /// Keeps the API's runtime, and so the server, alive.
    _runtime: tokio::runtime::Runtime,
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        block(async {
            let mut admin = PgConnection::connect_with(&admin_options())
                .await
                .unwrap_or_else(|e| panic!("cannot connect to Postgres as the admin role: {e}"));
            sqlx::query(&format!("DROP DATABASE IF EXISTS {TEST_DB} WITH (FORCE)"))
                .execute(&mut admin)
                .await
                .expect("drop the scratch database");
            sqlx::query(&format!("CREATE DATABASE {TEST_DB}"))
                .execute(&mut admin)
                .await
                .expect("create the scratch database");
            omni_api::migrate(&config(), 30)
                .await
                .expect("apply the migrations to the scratch database");
        });
        let secret = format!("{KEY_PREFIX}_{}", "0123456789abcdef".repeat(4));
        let workspace = block(async {
            let mut conn = admin_connection().await;
            let workspace: String = sqlx::query_scalar(
                "insert into workspaces (name) values ('Alpha') returning id::text",
            )
            .fetch_one(&mut conn)
            .await
            .expect("seed a workspace");
            sqlx::query(
                "insert into api_keys (workspace_id, prefix, key_hash, scopes) \
                 values ($1::uuid, $2, encode(sha256(convert_to($3, 'UTF8')), 'hex'), '{*}')",
            )
            .bind(&workspace)
            .bind(KEY_PREFIX)
            .bind(&secret)
            .execute(&mut conn)
            .await
            .expect("seed an API key");
            workspace
        });

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("bind a local port");
        let port = listener.local_addr().unwrap().port();
        let router = {
            // The connection pool needs a runtime to be created in.
            let _guard = runtime.enter();
            let state = omni_api::AppState::new(&config()).expect("build the API state");
            runtime
                .block_on(state.ensure_bucket(&config(), 30))
                .expect("create the scratch bucket");
            omni_api::router(state)
        };
        runtime.spawn(async move {
            axum::serve(listener, router).await.expect("serve the API");
        });
        Fixture {
            url: format!("http://127.0.0.1:{port}"),
            port,
            workspace,
            secret,
            _runtime: runtime,
        }
    })
}

/// A home directory whose config file holds the agent's key, as `omni login`
/// leaves it.
fn agent_home(f: &Fixture) -> Scratch {
    let home = Scratch::new("agent");
    write_config(&home.path, &f.url, &f.workspace, &f.secret);
    home
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", "run-7/author"])
        .run()
}

/// The tests that count the workspace's attachments take this lock, and so do
/// the ones that add attachments, so a count is not changed under a test.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

struct Fetched {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Fetched {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// `GET path` with the agent's key; the body comes back as bytes.
fn fetch(f: &Fixture, path: &str) -> Fetched {
    let mut stream = TcpStream::connect(("127.0.0.1", f.port)).expect("connect to the API");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
         Authorization: Bearer {}\r\n\r\n",
        f.secret
    );
    stream.write_all(request.as_bytes()).expect("send request");
    let mut raw = Vec::new();
    // A reset after the response is complete is not an error for these tests.
    let _ = stream.read_to_end(&mut raw);
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no HTTP response: {:?}", String::from_utf8_lossy(&raw)));
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .expect("status line");
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
        .collect();
    Fetched {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

fn attachment_path(f: &Fixture, id: &str) -> String {
    format!("/api/v1/w/{}/attachments/{id}", f.workspace)
}

#[test]
fn a_text_file_attached_from_the_command_line_is_a_download_with_its_name() {
    let _guard = serial();
    let f = fixture();
    let home = agent_home(f);
    let name = unique("notes") + ".txt";
    let file = home.join(&name);
    std::fs::write(&file, b"remember the milk\n").unwrap();

    let out = run(&home, &["attach", file.to_str().unwrap(), "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let attachment = out.json()["attachment"].clone();
    let id = attachment["id"].as_str().expect("an id").to_string();
    assert_eq!(attachment["ref"], format!("attachment:{id}"));
    assert_eq!(attachment["filename"], name.as_str());
    assert_eq!(attachment["kind"], "file");

    let served = fetch(f, &attachment_path(f, &id));
    assert_eq!(served.status, 200);
    assert_eq!(served.body, b"remember the milk\n");
    let disposition = served
        .header("content-disposition")
        .unwrap_or("")
        .to_string();
    assert!(
        disposition.to_lowercase().starts_with("attachment") && disposition.contains(&name),
        "{disposition:?}"
    );
    assert_eq!(served.header("x-content-type-options"), Some("nosniff"));
}
