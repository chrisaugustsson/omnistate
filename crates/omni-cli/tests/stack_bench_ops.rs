//! US-022: the 50 ms target for single operations (Q31), against the real API.
//! Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432` and RustFS at `rustfs:9000`. It is a
//! test target of its own, so no other test runs next to it and skews the
//! times.
//!
//! The test builds a 1,000-block live doc and then applies 500 single
//! `set` operations one after the other, each with its own run of the built
//! `omni doc edit`, timed at the command line as an agent sees it. The 95th
//! percentile must be under 50 ms. `make bench-ops` measures the same against
//! the warm local stack.

mod common;

use common::*;
use omni_api::Config;
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::Duration;

const TEST_DB: &str = "omni_us022_bench";
const TEST_BUCKET: &str = "omni-us022-bench";
const KEY_PREFIX: &str = "omnibench022";
const BLOCKS: usize = 1_000;
const OPERATIONS: usize = 500;
const TARGET: Duration = Duration::from_millis(50);

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

fn run(home: &Scratch, args: &[&str], stdin: &[u8]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", "bench-1/writer"])
        .stdin(stdin)
        .run()
}

// The target holds for release builds, like the stack's own API. `make bench-ops`
// runs it there.
#[test]
#[cfg_attr(debug_assertions, ignore = "times only mean something in release")]
fn single_set_operations_take_under_50_ms_at_the_95th_percentile() {
    let f = fixture();
    let home = agent_home(f);
    let project = "bench-ops";
    let created = run(&home, &["project", "create", project], b"");
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let out = run(
        &home,
        &[
            "doc",
            "create",
            "--project",
            project,
            "--kind",
            "html",
            "--pillar",
            "spec",
            "--title",
            "A big PRD",
            "--json",
        ],
        b"",
    );
    assert_eq!(out.code, 0, "doc create: {}", out.stderr);
    let doc = out.json()["doc"]["id"]
        .as_str()
        .expect("a doc id")
        .to_string();
    let target = ["--project", project];

    // The 1,000-block doc, in batches of 100 inserts.
    let mut nodes = Vec::new();
    for batch in 0..BLOCKS / 100 {
        let ops: Vec<Value> = (0..100)
            .map(|i| {
                json!({
                    "op": "insert",
                    "parent": null,
                    "position": { "after": null },
                    "content": format!("<p>Block {} with some words in it.</p>", batch * 100 + i)
                })
            })
            .collect();
        let out = run(
            &home,
            &[&["doc", "edit", doc.as_str(), "--json"][..], &target].concat(),
            Value::from(ops).to_string().as_bytes(),
        );
        assert_eq!(out.code, 0, "build the doc: {}", out.stderr);
        for result in out.json()["results"].as_array().expect("results") {
            nodes.push(result["node"].as_str().expect("a node").to_string());
        }
    }
    assert_eq!(nodes.len(), BLOCKS);

    let mut times = Vec::with_capacity(OPERATIONS);
    for (i, node) in nodes.iter().take(OPERATIONS).enumerate() {
        let ops = json!([{
            "op": "set",
            "node": node,
            "rev": 1,
            "properties": { "html": format!("Block {i}, edited.") }
        }]);
        let out = run(
            &home,
            &[&["doc", "edit", doc.as_str()][..], &target].concat(),
            ops.to_string().as_bytes(),
        );
        assert_eq!(out.code, 0, "set #{i}: {}", out.stderr);
        times.push(out.elapsed);
    }
    times.sort();
    let p95 = times[(times.len() * 95).div_ceil(100) - 1];
    eprintln!(
        "{OPERATIONS} single set operations on a {BLOCKS}-block doc: p50 {:?}, p95 {p95:?}, max {:?}",
        times[times.len() / 2],
        times[times.len() - 1]
    );
    assert!(
        p95 < TARGET,
        "the 95th percentile of a single operation at the CLI is {p95:?}, over {TARGET:?}"
    );
}
