//! US-059: `omni feature` against the real API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key (the
//! agent) and a user with a live session (the human), and serve the API's
//! router on a local port against that database. The built `omni` runs against
//! that address with the key in its config file. The contract of the CLI is in
//! tests/features.rs, that of the API in crates/omni-api/tests/features.rs.

mod common;

use common::*;
use omni_api::Config;
use serde_json::Value;
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us059_cli";
const KEY_PREFIX: &str = "omniustest59";
const HUMAN_TOKEN: &str = "us059-cli-human-session-0123456789abcdef0123456789abcdef0123456789";

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
        // Nothing here reaches RustFS: projects and features only use Postgres.
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
            let user = text(
                "insert into users (email) values ('hanna@example.test') returning id::text",
                &[],
            )
            .await;
            text(
                "insert into sessions (user_id, token_hash, expires_at) \
                 values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
                 returning id::text",
                &[&user, HUMAN_TOKEN],
            )
            .await;
            text(
                "insert into memberships (workspace_id, user_id) \
                 values ($1::uuid, $2::uuid) returning id::text",
                &[&workspace, &user],
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
        .args(&["--as", "run-7/planner"])
        .run()
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[test]
fn a_feature_is_created_listed_and_shown_from_the_command_line() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("shop");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);

    let out = run(
        &home,
        &[
            "feature",
            "create",
            "Checkout",
            "--description",
            "Pay for the basket in one step.",
            "--project",
            &project,
        ],
    );
    assert_eq!(out.code, 0, "feature create: {}", out.stderr);
    let lines = non_empty_lines(&out.stdout);
    assert_eq!(lines.len(), 1, "{:?}", out.stdout);
    let (checkout, name) = lines[0]
        .split_once("  ")
        .unwrap_or_else(|| panic!("`<id>  <name>`: {:?}", lines[0]));
    assert_eq!(name, "Checkout");
    let checkout = checkout.to_string();

    let out = run(
        &home,
        &[
            "feature",
            "create",
            "Wish list",
            "--project",
            &project,
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "feature create --json: {}", out.stderr);
    let wish_list = out.json()["feature"].clone();
    assert_eq!(wish_list["status"], "active", "{wish_list}");
    assert_eq!(wish_list["description"], "", "{wish_list}");
    let wish_list = wish_list["id"].as_str().expect("an id").to_string();

    // Names are unique in the project, ignoring case.
    let taken = run(
        &home,
        &["feature", "create", "checkout", "--project", &project],
    );
    assert_ne!(taken.code, 0, "{:?}", taken.stdout);
    assert!(
        taken.stderr.contains("name_taken"),
        "the error names the code: {:?}",
        taken.stderr
    );

    let listed = run(&home, &["feature", "list", "--project", &project]);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert_eq!(
        non_empty_lines(&listed.stdout),
        [
            format!("{checkout}  active  Checkout"),
            format!("{wish_list}  active  Wish list"),
        ],
        "the features in creation order"
    );

    for which in [checkout.as_str(), "Checkout"] {
        let shown = run(&home, &["feature", "show", which, "--project", &project]);
        assert_eq!(shown.code, 0, "show {which}: {}", shown.stderr);
        for needle in [
            checkout.as_str(),
            "Checkout",
            "active",
            "Pay for the basket in one step.",
        ] {
            assert!(
                shown.stdout.contains(needle),
                "show {which} prints {needle:?}: {:?}",
                shown.stdout
            );
        }
    }

    // The default project of .omni.toml works the same way.
    let project_dir = Scratch::new("project");
    std::fs::write(
        project_dir.join(".omni.toml"),
        format!("project = \"{project}\"\n"),
    )
    .unwrap();
    let shown = omni(&home.path)
        .cwd(&project_dir.path)
        .args(&["feature", "show", "Wish list", "--json"])
        .args(&["--as", "run-7/planner"])
        .run();
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    assert_eq!(shown.json()["feature"]["id"], wish_list.as_str());
}

// US-061: `omni doc list` against the real API: a doc takes the scope of the
// change set that creates it, and the list prints it.
#[test]
fn doc_list_prints_the_scope_of_each_doc_from_the_command_line() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("shop");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let created = run(
        &home,
        &["feature", "create", "Checkout", "--project", &project],
    );
    assert_eq!(created.code, 0, "feature create: {}", created.stderr);

    let changeset = |extra: &[&str]| {
        let out = run(
            &home,
            &[
                &[
                    "changeset",
                    "create",
                    "--title",
                    "Docs",
                    "--project",
                    &project,
                    "--json",
                ][..],
                extra,
            ]
            .concat(),
        );
        assert_eq!(out.code, 0, "changeset create {extra:?}: {}", out.stderr);
        out.json()["changeset"]["id"]
            .as_str()
            .expect("an id")
            .to_string()
    };
    let global = changeset(&["--global"]);
    let checkout = changeset(&["--feature", "Checkout"]);
    let doc = |changeset: &str, kind: &str, pillar: &str, title: &str| {
        let out = run(
            &home,
            &[
                "doc",
                "create",
                "--changeset",
                changeset,
                "--kind",
                kind,
                "--pillar",
                pillar,
                "--title",
                title,
                "--project",
                &project,
                "--json",
            ],
        );
        assert_eq!(out.code, 0, "doc create {title}: {}", out.stderr);
        out.json()["doc"]["id"].as_str().expect("an id").to_string()
    };
    let prd = doc(&global, "html", "spec", "Brand PRD");
    let architecture = doc(&checkout, "html", "architecture", "Checkout architecture");

    let listed = run(&home, &["doc", "list", "--project", &project]);
    assert_eq!(listed.code, 0, "doc list: {}", listed.stderr);
    assert_eq!(
        non_empty_lines(&listed.stdout),
        [
            format!("{prd}  project  html  spec  Brand PRD"),
            format!("{architecture}  Checkout  html  architecture  Checkout architecture"),
        ],
        "the docs in creation order with their scope"
    );
    for (filter, expected) in [
        (&["--feature", "Checkout"][..], &architecture),
        (&["--global"][..], &prd),
    ] {
        let out = run(
            &home,
            &[&["doc", "list", "--project", &project][..], filter].concat(),
        );
        assert_eq!(out.code, 0, "doc list {filter:?}: {}", out.stderr);
        let lines = non_empty_lines(&out.stdout);
        assert_eq!(lines.len(), 1, "doc list {filter:?}: {:?}", out.stdout);
        assert!(
            lines[0].starts_with(expected.as_str()),
            "doc list {filter:?} lists {expected}: {:?}",
            lines[0]
        );
    }
}

/// One request with the human's session cookie; returns (status, body).
fn as_human(f: &Fixture, method: &str, path: &str, body: &Value) -> (u16, Value) {
    let body = body.to_string();
    let mut stream = TcpStream::connect(("127.0.0.1", f.port)).expect("connect to the API");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
         Cookie: omni_session={HUMAN_TOKEN}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("send request");
    let mut raw = Vec::new();
    // A reset after the response is complete is not an error for these tests.
    let _ = stream.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("no HTTP response: {text:?}"));
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    // The API may send the body chunked; the JSON object is what lies between
    // the first `{` and the last `}`.
    let json = match (body.find('{'), body.rfind('}')) {
        (Some(start), Some(end)) => serde_json::from_str(&body[start..=end]).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    (status, json)
}

// US-062: the human marks a feature done in the web app; the command line
// then lists and shows it as done, refuses to mark it itself and opens no new
// change set in it.
#[test]
fn a_done_feature_is_listed_as_done_from_the_command_line() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("shop");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let out = run(
        &home,
        &[
            "feature",
            "create",
            "Checkout",
            "--project",
            &project,
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "feature create: {}", out.stderr);
    let checkout = out.json()["feature"].clone();
    let id = checkout["id"].as_str().expect("an id").to_string();
    let project_id = checkout["project_id"]
        .as_str()
        .expect("a project id")
        .to_string();
    let out = run(
        &home,
        &["feature", "create", "Search", "--project", &project],
    );
    assert_eq!(out.code, 0, "feature create: {}", out.stderr);
    let search = non_empty_lines(&out.stdout)[0]
        .split_once("  ")
        .map(|(id, _)| id.to_string())
        .unwrap_or_else(|| panic!("`<id>  <name>`: {:?}", out.stdout));

    let refused = run(
        &home,
        &["feature", "done", "Checkout", "--project", &project],
    );
    assert_ne!(refused.code, 0, "{:?}", refused.stdout);
    assert!(
        refused
            .stderr
            .contains("marking a feature done is the reviewer's call, in the web app"),
        "{:?}",
        refused.stderr
    );

    let (status, answer) = as_human(
        f,
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project_id}/features/{id}/done",
            f.workspace
        ),
        &serde_json::json!({}),
    );
    assert_eq!(status, 200, "the human marks the feature done: {answer}");

    let listed = run(&home, &["feature", "list", "--project", &project]);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert_eq!(
        non_empty_lines(&listed.stdout),
        [
            format!("{id}  done  Checkout"),
            format!("{search}  active  Search"),
        ],
        "the done feature is listed as done"
    );
    let shown = run(
        &home,
        &["feature", "show", "Checkout", "--project", &project],
    );
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    assert!(shown.stdout.contains("done"), "{:?}", shown.stdout);

    let out = run(
        &home,
        &[
            "changeset",
            "create",
            "--title",
            "More checkout",
            "--feature",
            "Checkout",
            "--project",
            &project,
        ],
    );
    assert_ne!(out.code, 0, "{:?}", out.stdout);
    assert!(
        out.stderr.contains("feature_done"),
        "the error names the code: {:?}",
        out.stderr
    );
}
