//! US-031: the time targets of design edits (Q31), against the real API. Run
//! with `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432` and RustFS at `rustfs:9000`. It is a test target
//! of its own with a single test, so no other test runs next to it and skews
//! the times.
//!
//! The test creates a design, which starts with one page, then inserts three
//! boards of 300 nodes each under the page, each with its own run of the built
//! `omni design edit`, timed at the command line as an agent sees it: every
//! insert must take under 500 ms. Then it applies 500 single `set`
//! operations to nodes of those boards one after the other, each with its own
//! `omni design edit`, and the 95th percentile must be under 50 ms, the
//! target of single operations from US-022 (tests/stack_bench_ops.rs).

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

const TEST_DB: &str = "omni_us031_bench";
const TEST_BUCKET: &str = "omni-us031-bench";
const KEY_PREFIX: &str = "omnibench031";
const AGENT: &str = "bench-31/designer";
/// The boards of 300 nodes that the test inserts.
const BOARDS: usize = 3;
/// A board, 13 frames and 22 texts in each frame: 300 nodes.
const FRAMES: usize = 13;
const TEXTS: usize = 22;
const BOARD_NODES: usize = 1 + FRAMES + FRAMES * TEXTS;
const BOARD_TARGET: Duration = Duration::from_millis(500);
const OPERATIONS: usize = 500;
const OPERATION_TARGET: Duration = Duration::from_millis(50);

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

fn run(home: &Scratch, args: &[&str], stdin: &[u8]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", AGENT])
        .stdin(stdin)
        .run()
}

fn json_of(out: &Out, what: &str) -> Value {
    assert_eq!(out.code, 0, "{what}: {}", out.stderr);
    out.json()
}

/// The tags of a board of 300 nodes, with no IDs: all of its nodes are new.
fn big_board(index: usize) -> String {
    let mut tags = format!(
        "<Board name=\"Screen {index}\" x={{{}}} y={{0}} width={{1440}} height={{2400}} dir=\"col\" gap={{8}}>\n",
        index * 1600
    );
    for frame in 0..FRAMES {
        tags.push_str(&format!(
            "  <Frame name=\"Row {index}.{frame}\" dir=\"row\" wrap=\"wrap\" gap={{4}} pad={{8}}>\n"
        ));
        for text in 0..TEXTS {
            tags.push_str(&format!(
                "    <Text name=\"Cell {index}.{frame}.{text}\" content=\"Cell {text} of row {frame}\" fontFamily=\"Inter\" fontSize={{14}}/>\n"
            ));
        }
        tags.push_str("  </Frame>\n");
    }
    tags.push_str("</Board>");
    tags
}

// The target holds for release builds, like the stack's own API. `make bench-ops`
// runs it there.
#[test]
#[cfg_attr(debug_assertions, ignore = "times only mean something in release")]
fn a_300_node_board_inserts_in_under_500_ms_and_single_operations_take_under_50_ms() {
    let f = fixture();
    let home = Scratch::new("agent");
    write_config(&home.path, &f.url, &f.workspace, &f.secret);
    let project = "bench-design";
    json_of(
        &run(&home, &["project", "create", project, "--json"], b""),
        "project create",
    );
    let out = run(
        &home,
        &[
            "doc",
            "create",
            "--project",
            project,
            "--kind",
            "design",
            "--pillar",
            "ui",
            "--title",
            "Big screens",
            "--json",
        ],
        b"",
    );
    let doc = json_of(&out, "doc create")["doc"]["id"]
        .as_str()
        .expect("a doc id")
        .to_string();
    let head = json_of(
        &run(
            &home,
            &["doc", "read", &doc, "--project", project, "--json"],
            b"",
        ),
        "read the new design",
    );
    let page = head["pages"][0]["id"]
        .as_str()
        .expect("the page has an ID")
        .to_string();
    let edit: Vec<&str> = vec!["design", "edit", "--project", project, "--doc", &doc];

    let mut nodes = Vec::new();
    let mut board_times = Vec::new();
    let mut after: Option<String> = None;
    for index in 0..BOARDS {
        let ops = json!([{
            "op": "insert",
            "parent": page,
            "position": { "after": after },
            "content": big_board(index)
        }]);
        let out = run(
            &home,
            &[&edit[..], &["--json"]].concat(),
            ops.to_string().as_bytes(),
        );
        let answer = json_of(&out, "insert a 300-node board");
        let ids: Vec<String> = answer["results"][0]["created"]
            .as_array()
            .unwrap_or_else(|| panic!("the insert lists its new nodes: {answer}"))
            .iter()
            .map(|id| id.as_str().expect("an ID").to_string())
            .collect();
        assert_eq!(ids.len(), BOARD_NODES, "every node of the board is new");
        after = Some(ids[0].clone());
        nodes.extend(ids);
        board_times.push(out.elapsed);
    }
    eprintln!("{BOARDS} inserts of a {BOARD_NODES}-node board: {board_times:?}");
    for (index, time) in board_times.iter().enumerate() {
        assert!(
            *time < BOARD_TARGET,
            "inserting board {index} of {BOARD_NODES} nodes took {time:?} at the CLI, over {BOARD_TARGET:?}"
        );
    }

    let mut times = Vec::with_capacity(OPERATIONS);
    for (i, node) in nodes.iter().take(OPERATIONS).enumerate() {
        let ops = json!([{
            "op": "set",
            "node": node,
            "rev": 1,
            "properties": { "name": format!("Edited {i}") }
        }]);
        let out = run(&home, &edit, ops.to_string().as_bytes());
        assert_eq!(out.code, 0, "set #{i}: {}", out.stderr);
        times.push(out.elapsed);
    }
    times.sort();
    let p95 = times[(times.len() * 95).div_ceil(100) - 1];
    eprintln!(
        "{OPERATIONS} single set operations on a {}-node design: p50 {:?}, p95 {p95:?}, max {:?}",
        nodes.len() + 1,
        times[times.len() / 2],
        times[times.len() - 1]
    );
    assert!(
        p95 < OPERATION_TARGET,
        "the 95th percentile of a single design operation at the CLI is {p95:?}, over {OPERATION_TARGET:?}"
    );
}
