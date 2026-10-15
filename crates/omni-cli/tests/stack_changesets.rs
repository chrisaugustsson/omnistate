//! US-016: `omni project` and `omni changeset` against the real API. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key (the
//! agent) and a user with a live session (the human), and serve the API's
//! router on a local port against that database. The built `omni` runs against
//! that address with the key in its config file. The human's rejection is a
//! plain HTTP request with the session cookie, because the CLI has no command
//! for it. The contract of the CLI is in tests/changesets.rs.

mod common;

use common::*;
use omni_api::Config;
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us016_cli";
const KEY_PREFIX: &str = "omniustest16";
const HUMAN_TOKEN: &str = "us016-cli-human-session-0123456789abcdef0123456789abcdef0123456789";

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
        // Nothing here reaches RustFS: projects and change sets only use Postgres.
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
        .args(&["--as", "run-7/author"])
        .run()
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
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

/// Creates a project and a change set from the command line; returns (project
/// name, change set ID).
fn project_with_changeset(home: &Scratch) -> (String, String) {
    let name = unique("brand-site");
    let created = run(home, &["project", "create", &name]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let out = run(
        home,
        &[
            "changeset",
            "create",
            "--project",
            &name,
            "--title",
            "Hero section",
            "--description",
            "First pass of the landing hero.",
            "--run",
            "ui-research-7",
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "changeset create: {}", out.stderr);
    let changeset = out.json()["changeset"].clone();
    assert_eq!(changeset["status"], "open", "{changeset}");
    assert_eq!(changeset["run_tag"], "ui-research-7", "{changeset}");
    (name, changeset["id"].as_str().expect("an id").to_string())
}

#[test]
fn a_change_set_goes_through_the_lifecycle_from_the_command_line() {
    let f = fixture();
    let home = agent_home(f);
    let (project, id) = project_with_changeset(&home);

    let listed = run(&home, &["changeset", "list", "--project", &project]);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert!(
        listed.stdout.contains(&id) && listed.stdout.contains("Hero section"),
        "{:?}",
        listed.stdout
    );

    let submitted = run(&home, &["changeset", "submit", &id, "--project", &project]);
    assert_eq!(submitted.code, 0, "{}", submitted.stderr);
    assert!(
        submitted.stdout.contains("in_review"),
        "{:?}",
        submitted.stdout
    );

    // The human rejects in the web app; the CLI shows it, with the reason.
    let reason = "The hero contrast is too low; rework the headline.";
    let project_id = {
        let projects = run(&home, &["project", "list", "--json"]);
        assert_eq!(projects.code, 0, "{}", projects.stderr);
        projects.json()["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == project.as_str())
            .map(|p| p["id"].as_str().unwrap().to_string())
            .expect("the project is listed")
    };
    let (status, body) = as_human(
        f,
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project_id}/changesets/{id}/reject",
            f.workspace
        ),
        &json!({ "reason": reason }),
    );
    assert_eq!(status, 200, "the human rejects: {body}");

    let shown = run(&home, &["changeset", "show", &id, "--project", &project]);
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    assert!(
        shown.stdout.contains("rejected") && shown.stdout.contains(reason),
        "show prints the rejection and its reason: {:?}",
        shown.stdout
    );

    let reopened = run(&home, &["changeset", "reopen", &id, "--project", &project]);
    assert_eq!(reopened.code, 0, "{}", reopened.stderr);
    assert!(reopened.stdout.contains("open"), "{:?}", reopened.stdout);
    let again = run(&home, &["changeset", "submit", &id, "--project", &project]);
    assert_eq!(again.code, 0, "{}", again.stderr);

    let abandoned = run(&home, &["changeset", "abandon", &id, "--project", &project]);
    assert_eq!(abandoned.code, 0, "{}", abandoned.stderr);
    assert!(
        abandoned.stdout.contains("abandoned"),
        "{:?}",
        abandoned.stdout
    );

    // Abandoned is not deleted: it is shown with its whole history.
    let json = run(
        &home,
        &["changeset", "show", &id, "--project", &project, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    let changeset = json.json()["changeset"].clone();
    assert_eq!(changeset["status"], "abandoned", "{changeset}");
    let to_statuses: Vec<&str> = changeset["history"]
        .as_array()
        .expect("history")
        .iter()
        .map(|e| e["to_status"].as_str().unwrap_or("?"))
        .collect();
    assert_eq!(
        to_statuses,
        [
            "open",
            "in_review",
            "rejected",
            "open",
            "in_review",
            "abandoned"
        ]
    );
    let listed = run(&home, &["changeset", "list", "--project", &project]);
    assert!(
        listed.stdout.contains(&id) && listed.stdout.contains("abandoned"),
        "{:?}",
        listed.stdout
    );
}

#[test]
fn a_transition_outside_the_lifecycle_is_one_line_with_invalid_transition() {
    let f = fixture();
    let home = agent_home(f);
    let (project, id) = project_with_changeset(&home);
    let first = run(&home, &["changeset", "submit", &id, "--project", &project]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    for command in ["submit", "reopen"] {
        let out = run(&home, &["changeset", command, &id, "--project", &project]);
        assert_eq!(out.code, 1, "{command} from in_review: {}", out.stdout);
        assert_eq!(out.stdout, "", "an error prints nothing on standard output");
        let lines = out.stderr_lines();
        assert_eq!(lines.len(), 1, "one line: {:?}", out.stderr);
        assert!(lines[0].contains("invalid_transition"), "{:?}", lines[0]);
    }
    let shown = run(
        &home,
        &["changeset", "show", &id, "--project", &project, "--json"],
    );
    assert_eq!(shown.json()["changeset"]["status"], "in_review");
}

#[test]
fn creating_a_project_twice_is_one_line_with_the_error_code() {
    let f = fixture();
    let home = agent_home(f);
    let name = unique("twice");
    let first = run(&home, &["project", "create", &name]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let second = run(&home, &["project", "create", &name]);
    assert_eq!(second.code, 1, "{}", second.stdout);
    let lines = second.stderr_lines();
    assert_eq!(lines.len(), 1, "one line: {:?}", second.stderr);
    assert!(lines[0].contains("name_taken"), "{:?}", lines[0]);
}

#[test]
fn the_default_project_of_omni_toml_is_found_by_name() {
    let f = fixture();
    let home = agent_home(f);
    let name = unique("default-project");
    let created = run(&home, &["project", "create", &name]);
    assert_eq!(created.code, 0, "{}", created.stderr);

    let project_dir = Scratch::new("project");
    std::fs::write(
        project_dir.join(".omni.toml"),
        format!("project = \"{name}\"\n"),
    )
    .unwrap();
    let out = omni(&home.path)
        .cwd(&project_dir.path)
        .args(&["changeset", "create", "--title", "From the default"])
        .args(&["--as", "run-7/author"])
        .run();
    assert_eq!(out.code, 0, "{}", out.stderr);
    let listed = omni(&home.path)
        .cwd(&project_dir.path)
        .args(&["changeset", "list", "--as", "run-7/author"])
        .run();
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert!(
        listed.stdout.contains("From the default"),
        "{:?}",
        listed.stdout
    );
}
