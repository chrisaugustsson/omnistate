//! US-137: one smoke test of the `omni story` writes against the real API.
//! Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432`.
//!
//! The test migrates a scratch database, seeds a workspace with an API key
//! and serves the API's router on a local port against that database. The
//! CLI's contract is in tests/stories.rs, the API's rules in
//! crates/omni-api/tests/stories.rs. What this test adds is the round trip:
//! the bodies the CLI sends, with the version it read, are ones the API
//! takes, so a story added, claimed, ticked and moved to `done` reads back
//! as `done`.

mod common;

use common::*;
use omni_api::Config;
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us137_cli";
const KEY_PREFIX: &str = "omniustes137";

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
        // Nothing here reaches RustFS: stories only use Postgres.
        s3_endpoint: "http://127.0.0.1:1".to_string(),
        s3_access_key: "none".to_string(),
        s3_secret_key: "none".to_string(),
        s3_bucket: "omni".to_string(),
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
            let mut text = async |sql: &str, binds: &[&str]| -> String {
                let mut query = sqlx::query_scalar::<_, String>(sql);
                for bind in binds {
                    query = query.bind(*bind);
                }
                query.fetch_one(&mut conn).await.expect("seed")
            };
            let workspace = text(
                "insert into workspaces (name) values ('Alpha') returning id::text",
                &[],
            )
            .await;
            text(
                "insert into api_keys (workspace_id, prefix, key_hash, scopes) \
                 values ($1::uuid, $2, encode(sha256(convert_to($3, 'UTF8')), 'hex'), '{*}') \
                 returning id::text",
                &[&workspace, KEY_PREFIX, &secret],
            )
            .await;
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

/// A home directory whose config file holds the agents' key, as `omni login`
/// leaves it.
fn agent_home(f: &Fixture) -> Scratch {
    let home = Scratch::new("agent");
    write_config(&home.path, &f.url, &f.workspace, &f.secret);
    home
}

/// Runs `omni` as the agent `who` (`<run>/<role>`).
fn run_as(home: &Scratch, who: &str, args: &[&str]) -> Out {
    omni(&home.path).args(args).args(&["--as", who]).run()
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

const AGENT: &str = "run-1/implementer";

#[test]
fn a_story_added_claimed_ticked_and_moved_to_done_reads_back_as_done() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("board");
    let created = run_as(&home, AGENT, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let feature = run_as(
        &home,
        AGENT,
        &["feature", "create", "stories", "--project", &project],
    );
    assert_eq!(feature.code, 0, "feature create: {}", feature.stderr);
    let file = home.join("story.md");
    std::fs::write(
        &file,
        "### Smoke story\n\
         **Description:** As the workflow, I want to finish a story.\n\
         \n\
         **Acceptance Criteria:**\n\
         - [ ] It is done\n",
    )
    .unwrap();
    let scope = ["--project", project.as_str(), "--feature", "stories"];
    let story = |args: &[&str]| run_as(&home, AGENT, &[args, &scope[..]].concat());

    let added = story(&["story", "add", &file.display().to_string()]);
    assert_eq!(added.code, 0, "story add: {}", added.stderr);
    let key = added.stdout.trim().to_string();
    let claimed = story(&[
        "story",
        "claim",
        &key,
        "--run",
        "r1",
        "--stage",
        "implementing",
    ]);
    let ticked = story(&["story", "tick", &key, "--run", "r1", "--criterion", "0"]);
    let moved = story(&["story", "move", &key, "--run", "r1", "--to", "done"]);
    let read = story(&["story", "read", &key, "--json"]);

    assert_eq!(claimed.code, 0, "story claim: {}", claimed.stderr);
    assert_eq!(ticked.code, 0, "story tick: {}", ticked.stderr);
    assert_eq!(moved.code, 0, "story move: {}", moved.stderr);
    assert_eq!(read.code, 0, "story read: {}", read.stderr);
    let back = read.json();
    assert_eq!(back["key"], key.as_str());
    assert_eq!(back["status"], "done");
    assert_eq!(back["criteria"][0]["ticked"], true);
    assert_eq!(back["holder"], serde_json::Value::Null);
}
