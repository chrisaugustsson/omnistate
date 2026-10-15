//! US-016: projects and change sets. Run with `--features stack-tests` in the
//! Compose test container, which reaches Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed two workspaces (A and B) with one
//! member each (alice in A, bob in B) and a live session for each, mint an API
//! key per workspace through the API, and serve the API's router on a local
//! port against that database.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}`; the workspace is only ever taken from the path):
//!
//! - `POST /projects` (agent) takes `{"name"}` and answers 201
//!   `{"project": {"id", "name", "created_at"}}`; `id` is a UUIDv7. A missing
//!   or blank name is 400 `{"error": "invalid_request"}`. A project name is
//!   unique in its workspace, so the omni CLI can address a project by name: a
//!   second project with the same name is 409 `{"error": "name_taken"}`.
//!   `GET /projects` (both) answers `{"projects": [...]}` with that workspace's
//!   projects. Projects are rows of the tenant table `projects`.
//! - `POST /projects/{project_id}/changesets` (agent) takes `{"title",
//!   "description", "run_tag"}` and answers 201 `{"changeset": {...}}`. Only
//!   `title` is required (missing or blank is 400 `invalid_request`);
//!   `description` defaults to `""` and `run_tag` to `null`. The change set
//!   object has `id` (UUIDv7), `project_id`, `title`, `description`, `status`,
//!   `run_tag`, `created_at` and `history`. A new change set is `open`.
//!   `GET /projects/{project_id}/changesets` (both) answers `{"changesets":
//!   [...]}`; `GET /projects/{project_id}/changesets/{changeset_id}` (both)
//!   answers `{"changeset": {...}}`. Create, show and every transition carry the
//!   status `history`, oldest entry first.
//! - A history entry is `{"from_status", "to_status", "actor", "reason", "at"}`.
//!   `from_status` is `null` for the entry that creates the change set;
//!   `actor` is `agent` or `human`; `reason` is `null` unless the human rejected
//!   the change set; `at` is a timestamp string. Every status change, and only
//!   a status change that happened, adds one entry.
//! - `POST .../changesets/{id}/submit` (agent), `.../reopen` (agent),
//!   `.../abandon` (agent) and `.../reject` (human, body `{"reason"}`) answer
//!   200 `{"changeset": {...}}` with the new status. The lifecycle of FR-20 that
//!   this story implements: open -> in_review (submit), in_review -> rejected
//!   (reject), rejected -> open (reopen), and open, in_review or rejected ->
//!   abandoned (abandon). Anything else is 409 `{"error": "invalid_transition"}`
//!   and changes nothing. A reject without a reason (missing or blank) is 400
//!   `{"error": "reason_required"}`. An API key calling `reject` is 403
//!   `forbidden`. Unknown or foreign projects and change sets are 404
//!   `not_found`, and a request without credentials is 401 `unauthorized`.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us016_changesets";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";

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
        // Nothing here reaches RustFS: the routes under test only use Postgres.
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

/// The first column of the first row of `sql` as text, or None for no row or NULL.
fn db_text(sql: &str, binds: &[&str]) -> Option<String> {
    block(async {
        let mut conn = admin_connection().await;
        let mut query = sqlx::query_scalar::<_, Option<String>>(sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_optional(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("query failed ({e}): {sql}"))
            .flatten()
    })
}

struct Fixture {
    a: String,
    b: String,
    alice_token: String,
    bob_token: String,
    key_a: String,
    key_b: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us016-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
        let (a, b) = block(async {
            let mut conn = admin_connection().await;
            let mut text = async |sql: &str, binds: &[&str]| -> String {
                let mut query = sqlx::query_scalar::<_, String>(sql);
                for bind in binds {
                    query = query.bind(*bind);
                }
                query.fetch_one(&mut conn).await.expect("seed")
            };
            let a = text(
                "insert into workspaces (name) values ('Alpha') returning id::text",
                &[],
            )
            .await;
            let b = text(
                "insert into workspaces (name) values ('Beta') returning id::text",
                &[],
            )
            .await;
            for (name, workspace) in [("alice", &a), ("bob", &b)] {
                let user = text(
                    "insert into users (email) values ($1) returning id::text",
                    &[&format!("{name}@example.test")],
                )
                .await;
                text(
                    "insert into sessions (user_id, token_hash, expires_at) \
                     values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
                     returning id::text",
                    &[&user, &session_token(name)],
                )
                .await;
                text(
                    "insert into memberships (workspace_id, user_id) \
                     values ($1::uuid, $2::uuid) returning id::text",
                    &[workspace, &user],
                )
                .await;
            }
            (a, b)
        });
        let server = TestServer::start(&config());
        let mint = |token: &str, workspace: &str| -> String {
            let response = call(
                &server.client,
                "POST",
                &format!("/api/v1/w/{workspace}/api-keys"),
                &Auth::Session(token),
                Some(&json!({})),
            );
            assert_eq!(response.status, 201, "mint a key: {}", response.text());
            response.json()["secret"]
                .as_str()
                .expect("the new key has a secret")
                .to_string()
        };
        let key_a = mint(&session_token("alice"), &a);
        let key_b = mint(&session_token("bob"), &b);
        Fixture {
            a,
            b,
            alice_token: session_token("alice"),
            bob_token: session_token("bob"),
            key_a,
            key_b,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

/// How a request authenticates: an API key (an agent), a session cookie (the
/// human) or nothing.
#[derive(Clone, Copy)]
enum Auth<'a> {
    Key(&'a str),
    Session(&'a str),
    Nobody,
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let header = match auth {
        Auth::Key(secret) => Some(("Authorization", format!("Bearer {secret}"))),
        Auth::Session(token) => Some(("Cookie", cookie(token))),
        Auth::Nobody => None,
    };
    let headers: Vec<(&str, &str)> = header.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let body = body.map(Value::to_string);
    client.request(method, path, &headers, body.as_deref())
}

fn assert_error(response: &Response, status: u16, code: &str) {
    assert_eq!(response.status, status, "body: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": code }),
        "the error body carries the code and nothing else"
    );
}

fn assert_uuid_v7(id: &Value) {
    let id = id
        .as_str()
        .unwrap_or_else(|| panic!("an ID is a string: {id}"));
    let bytes = id.as_bytes();
    assert!(
        bytes.len() == 36
            && bytes[14] == b'7'
            && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
            && id.chars().all(|c| c == '-' || c.is_ascii_hexdigit()),
        "{id:?} is not a UUIDv7"
    );
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn projects_path(workspace: &str) -> String {
    format!("/api/v1/w/{workspace}/projects")
}

fn changesets_path(workspace: &str, project: &str) -> String {
    format!("/api/v1/w/{workspace}/projects/{project}/changesets")
}

fn changeset_path(workspace: &str, project: &str, id: &str) -> String {
    format!("{}/{id}", changesets_path(workspace, project))
}

/// Creates a project in workspace A as its agent; returns its ID.
fn new_project(f: &Fixture, client: &Client, label: &str) -> String {
    let response = call(
        client,
        "POST",
        &projects_path(&f.a),
        &Auth::Key(&f.key_a),
        Some(&json!({ "name": unique(label) })),
    );
    assert_eq!(
        response.status,
        201,
        "create a project: {}",
        response.text()
    );
    response.json()["project"]["id"]
        .as_str()
        .expect("the project has an id")
        .to_string()
}

/// Creates a change set in a project of workspace A as its agent.
fn new_changeset(f: &Fixture, client: &Client, project: &str, title: &str) -> Value {
    let response = call(
        client,
        "POST",
        &changesets_path(&f.a, project),
        &Auth::Key(&f.key_a),
        Some(&json!({ "title": title, "description": "d", "run_tag": "run-1" })),
    );
    assert_eq!(
        response.status,
        201,
        "create a change set: {}",
        response.text()
    );
    response.json()["changeset"].clone()
}

fn id_of(value: &Value) -> String {
    value["id"]
        .as_str()
        .unwrap_or_else(|| panic!("there is an `id`: {value}"))
        .to_string()
}

/// Calls a lifecycle action of a change set of workspace A.
fn act(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    action: &str,
    auth: &Auth,
    body: Option<&Value>,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/{action}", changeset_path(&f.a, project, changeset)),
        auth,
        body,
    )
}

fn show(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    let response = call(
        client,
        "GET",
        &changeset_path(&f.a, project, changeset),
        &Auth::Key(&f.key_a),
        None,
    );
    assert_eq!(response.status, 200, "show: {}", response.text());
    response.json()["changeset"].clone()
}

fn status_of(f: &Fixture, client: &Client, project: &str, changeset: &str) -> String {
    show(f, client, project, changeset)["status"]
        .as_str()
        .expect("a status")
        .to_string()
}

fn history_of(changeset: &Value) -> Vec<Value> {
    changeset["history"]
        .as_array()
        .unwrap_or_else(|| panic!("the change set has a `history` array: {changeset}"))
        .clone()
}

/// (from_status, to_status, actor) of each history entry, oldest first.
fn steps(changeset: &Value) -> Vec<(Option<String>, String, String)> {
    history_of(changeset)
        .iter()
        .map(|entry| {
            (
                entry["from_status"].as_str().map(str::to_string),
                entry["to_status"].as_str().unwrap_or("?").to_string(),
                entry["actor"].as_str().unwrap_or("?").to_string(),
            )
        })
        .collect()
}

fn step(from: Option<&str>, to: &str, actor: &str) -> (Option<String>, String, String) {
    (from.map(str::to_string), to.to_string(), actor.to_string())
}

/// Brings a new change set to `status` through the lifecycle's own actions.
fn new_changeset_in(f: &Fixture, client: &Client, project: &str, status: &str) -> String {
    let id = id_of(&new_changeset(f, client, project, &unique(status)));
    let key = Auth::Key(&f.key_a);
    let human = Auth::Session(&f.alice_token);
    let path: &[&str] = match status {
        "open" => &[],
        "in_review" => &["submit"],
        "rejected" => &["submit", "reject"],
        "abandoned" => &["abandon"],
        other => panic!("no path to {other}"),
    };
    for action in path {
        let (auth, body) = if *action == "reject" {
            (&human, json!({ "reason": "not good enough yet" }))
        } else {
            (&key, json!({}))
        };
        let response = act(f, client, project, &id, action, auth, Some(&body));
        assert_eq!(response.status, 200, "{action}: {}", response.text());
    }
    assert_eq!(status_of(f, client, project, &id), status);
    id
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[test]
fn the_routes_are_registered_with_who_may_call_them_and_described_in_openapi() {
    let (_f, client) = ready();
    let expected = [
        ("GET", "/api/v1/w/{workspace_id}/projects", Access::Both),
        ("POST", "/api/v1/w/{workspace_id}/projects", Access::Agent),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/submit",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reject",
            Access::Human,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reopen",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/abandon",
            Access::Agent,
        ),
    ];
    let routes = registry();
    let document = client.get("/api/openapi.json").json();
    for (method, path, access) in expected {
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no entry for {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
        let word = match access {
            Access::Agent => "agent",
            Access::Human => "human",
            Access::Both => "both",
        };
        let operation = &document["paths"][path][method.to_lowercase()];
        assert_eq!(
            operation["x-omni-access"], word,
            "{method} {path} in the OpenAPI document: {operation}"
        );
    }
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

#[test]
fn an_agent_creates_a_project_and_the_agent_and_the_human_list_it() {
    let (f, client) = ready();
    let name = unique("brand-site");
    let created = call(
        client,
        "POST",
        &projects_path(&f.a),
        &Auth::Key(&f.key_a),
        Some(&json!({ "name": name })),
    );
    assert_eq!(created.status, 201, "{}", created.text());
    let project = created.json()["project"].clone();
    assert_uuid_v7(&project["id"]);
    assert_eq!(project["name"], name.as_str(), "{project}");
    assert!(
        project["created_at"]
            .as_str()
            .is_some_and(|t| !t.is_empty()),
        "created_at: {project}"
    );

    for (who, auth) in [
        ("the agent", Auth::Key(&f.key_a)),
        ("the human", Auth::Session(&f.alice_token)),
    ] {
        let list = call(client, "GET", &projects_path(&f.a), &auth, None);
        assert_eq!(list.status, 200, "{who} lists: {}", list.text());
        let projects = list.json()["projects"].clone();
        let found = projects
            .as_array()
            .unwrap_or_else(|| panic!("`projects` is an array: {projects}"))
            .iter()
            .find(|p| p["id"] == project["id"])
            .unwrap_or_else(|| panic!("{who} should see the new project in {projects}"));
        assert_eq!(found["name"], name.as_str(), "{found}");
    }
    assert_eq!(
        db_text(
            "select workspace_id::text from projects where id = $1::uuid",
            &[project["id"].as_str().unwrap()]
        )
        .as_deref(),
        Some(f.a.as_str()),
        "the project is a row of the tenant table `projects`, in workspace A"
    );
}

#[test]
fn a_workspace_lists_only_its_own_projects() {
    let (f, client) = ready();
    let name_a = unique("only-in-a");
    let name_b = unique("only-in-b");
    for (workspace, key, name) in [(&f.a, &f.key_a, &name_a), (&f.b, &f.key_b, &name_b)] {
        let response = call(
            client,
            "POST",
            &projects_path(workspace),
            &Auth::Key(key),
            Some(&json!({ "name": name })),
        );
        assert_eq!(response.status, 201, "{}", response.text());
    }
    let names = |workspace: &str, auth: Auth| -> Vec<String> {
        let list = call(client, "GET", &projects_path(workspace), &auth, None);
        assert_eq!(list.status, 200, "{}", list.text());
        list.json()["projects"]
            .as_array()
            .expect("projects")
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_string())
            .collect()
    };
    let in_a = names(&f.a, Auth::Key(&f.key_a));
    assert!(
        in_a.contains(&name_a) && !in_a.contains(&name_b),
        "{in_a:?}"
    );
    let in_b = names(&f.b, Auth::Session(&f.bob_token));
    assert!(
        in_b.contains(&name_b) && !in_b.contains(&name_a),
        "{in_b:?}"
    );

    // The other workspace's path is not reachable with these credentials.
    for auth in [Auth::Key(&f.key_b), Auth::Session(&f.bob_token)] {
        let response = call(client, "GET", &projects_path(&f.a), &auth, None);
        assert_error(&response, 404, "not_found");
        let response = call(
            client,
            "POST",
            &projects_path(&f.a),
            &auth,
            Some(&json!({ "name": unique("intruder") })),
        );
        assert_error(&response, 404, "not_found");
    }
}

#[test]
fn a_project_needs_a_name_and_a_name_is_unique_in_its_workspace() {
    let (f, client) = ready();
    for body in [json!({}), json!({ "name": "" }), json!({ "name": "   " })] {
        let response = call(
            client,
            "POST",
            &projects_path(&f.a),
            &Auth::Key(&f.key_a),
            Some(&body),
        );
        assert_error(&response, 400, "invalid_request");
    }

    let name = unique("twice");
    let first = call(
        client,
        "POST",
        &projects_path(&f.a),
        &Auth::Key(&f.key_a),
        Some(&json!({ "name": name })),
    );
    assert_eq!(first.status, 201, "{}", first.text());
    let again = call(
        client,
        "POST",
        &projects_path(&f.a),
        &Auth::Key(&f.key_a),
        Some(&json!({ "name": name })),
    );
    assert_error(&again, 409, "name_taken");
    let other_workspace = call(
        client,
        "POST",
        &projects_path(&f.b),
        &Auth::Key(&f.key_b),
        Some(&json!({ "name": name })),
    );
    assert_eq!(
        other_workspace.status,
        201,
        "another workspace may use the same name: {}",
        other_workspace.text()
    );
    let list = call(
        client,
        "GET",
        &projects_path(&f.a),
        &Auth::Key(&f.key_a),
        None,
    );
    let count = list.json()["projects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["name"] == name.as_str())
        .count();
    assert_eq!(count, 1, "the refused duplicate was not created");
}

#[test]
fn project_routes_need_credentials() {
    let (f, client) = ready();
    for (method, body) in [("GET", None), ("POST", Some(json!({ "name": "x" })))] {
        let response = call(
            client,
            method,
            &projects_path(&f.a),
            &Auth::Nobody,
            body.as_ref(),
        );
        assert_error(&response, 401, "unauthorized");
    }
}

// ---------------------------------------------------------------------------
// Change sets
// ---------------------------------------------------------------------------

#[test]
fn a_change_set_has_a_title_a_description_a_run_tag_and_starts_open() {
    let (f, client) = ready();
    let project = new_project(f, client, "fields");
    let created = call(
        client,
        "POST",
        &changesets_path(&f.a, &project),
        &Auth::Key(&f.key_a),
        Some(&json!({
            "title": "Hero section",
            "description": "First pass of the landing hero.",
            "run_tag": "ui-research-7"
        })),
    );
    assert_eq!(created.status, 201, "{}", created.text());
    let changeset = created.json()["changeset"].clone();
    assert_uuid_v7(&changeset["id"]);
    assert_eq!(changeset["project_id"], project.as_str(), "{changeset}");
    assert_eq!(changeset["title"], "Hero section", "{changeset}");
    assert_eq!(
        changeset["description"], "First pass of the landing hero.",
        "{changeset}"
    );
    assert_eq!(changeset["status"], "open", "{changeset}");
    assert_eq!(changeset["run_tag"], "ui-research-7", "{changeset}");
    assert!(
        changeset["created_at"]
            .as_str()
            .is_some_and(|t| !t.is_empty()),
        "created_at: {changeset}"
    );

    // The same object is what show and list answer, for the agent and the human.
    let id = id_of(&changeset);
    for auth in [Auth::Key(&f.key_a), Auth::Session(&f.alice_token)] {
        let shown = call(
            client,
            "GET",
            &changeset_path(&f.a, &project, &id),
            &auth,
            None,
        );
        assert_eq!(shown.status, 200, "{}", shown.text());
        let shown = shown.json()["changeset"].clone();
        for field in [
            "id",
            "project_id",
            "title",
            "description",
            "status",
            "run_tag",
        ] {
            assert_eq!(shown[field], changeset[field], "show: {field}");
        }
        let list = call(client, "GET", &changesets_path(&f.a, &project), &auth, None);
        assert_eq!(list.status, 200, "{}", list.text());
        let listed = list.json()["changesets"]
            .as_array()
            .expect("`changesets` is an array")
            .iter()
            .find(|c| c["id"] == changeset["id"])
            .cloned()
            .expect("the change set is listed");
        for field in ["project_id", "title", "description", "status", "run_tag"] {
            assert_eq!(listed[field], changeset[field], "list: {field}");
        }
    }

    // Creating it is the first entry of its history.
    assert_eq!(steps(&changeset), [step(None, "open", "agent")]);
    let first = &history_of(&changeset)[0];
    assert!(first["reason"].is_null(), "{first}");
    assert!(
        first["at"].as_str().is_some_and(|t| !t.is_empty()),
        "{first}"
    );
}

#[test]
fn a_change_set_needs_only_a_title_and_the_rest_has_defaults() {
    let (f, client) = ready();
    let project = new_project(f, client, "defaults");
    let created = call(
        client,
        "POST",
        &changesets_path(&f.a, &project),
        &Auth::Key(&f.key_a),
        Some(&json!({ "title": "Just a title" })),
    );
    assert_eq!(created.status, 201, "{}", created.text());
    let changeset = created.json()["changeset"].clone();
    assert_eq!(changeset["description"], "", "{changeset}");
    assert!(changeset["run_tag"].is_null(), "{changeset}");

    for body in [
        json!({}),
        json!({ "title": "" }),
        json!({ "title": "  \t " }),
        json!({ "description": "no title" }),
    ] {
        let response = call(
            client,
            "POST",
            &changesets_path(&f.a, &project),
            &Auth::Key(&f.key_a),
            Some(&body),
        );
        assert_error(&response, 400, "invalid_request");
    }
    let list = call(
        client,
        "GET",
        &changesets_path(&f.a, &project),
        &Auth::Key(&f.key_a),
        None,
    );
    assert_eq!(
        list.json()["changesets"].as_array().unwrap().len(),
        1,
        "the refused requests created nothing"
    );
}

#[test]
fn the_list_shows_every_change_set_of_the_project_and_only_that_project() {
    let (f, client) = ready();
    let project = new_project(f, client, "list");
    let other = new_project(f, client, "list-other");
    let open = id_of(&new_changeset(f, client, &project, "stays open"));
    let review = new_changeset_in(f, client, &project, "in_review");
    let gone = new_changeset_in(f, client, &project, "abandoned");
    let elsewhere = id_of(&new_changeset(f, client, &other, "in another project"));

    let list = call(
        client,
        "GET",
        &changesets_path(&f.a, &project),
        &Auth::Key(&f.key_a),
        None,
    );
    assert_eq!(list.status, 200, "{}", list.text());
    let listed: Vec<(String, String)> = list.json()["changesets"]
        .as_array()
        .expect("changesets")
        .iter()
        .map(|c| {
            (
                c["id"].as_str().unwrap().to_string(),
                c["status"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let mut expected = vec![
        (open, "open".to_string()),
        (review, "in_review".to_string()),
        (gone, "abandoned".to_string()),
    ];
    let mut listed_sorted = listed.clone();
    listed_sorted.sort();
    expected.sort();
    assert_eq!(
        listed_sorted, expected,
        "all three, abandoned included, and nothing of the other project"
    );
    assert!(!listed.iter().any(|(id, _)| *id == elsewhere));
}

#[test]
fn change_sets_are_reached_through_a_project_of_the_callers_workspace() {
    let (f, client) = ready();
    let project = new_project(f, client, "scope");
    let other = new_project(f, client, "scope-other");
    let id = id_of(&new_changeset(f, client, &project, "scoped"));
    let key = Auth::Key(&f.key_a);
    let body = json!({ "title": "x" });

    // Unknown, malformed and foreign projects and change sets are all 404.
    for project_id in [UNKNOWN_ID, "not-a-uuid"] {
        for (method, body) in [("GET", None), ("POST", Some(&body))] {
            let response = call(
                client,
                method,
                &changesets_path(&f.a, project_id),
                &key,
                body,
            );
            assert_error(&response, 404, "not_found");
        }
    }
    let wrong_project = changeset_path(&f.a, &other, &id);
    let unknown = changeset_path(&f.a, &project, UNKNOWN_ID);
    let malformed = changeset_path(&f.a, &project, "not-a-uuid");
    for path in [&wrong_project, &unknown, &malformed] {
        assert_error(&call(client, "GET", path, &key, None), 404, "not_found");
        for action in ["submit", "reopen", "abandon"] {
            let response = call(
                client,
                "POST",
                &format!("{path}/{action}"),
                &key,
                Some(&json!({})),
            );
            assert_error(&response, 404, "not_found");
        }
        let response = call(
            client,
            "POST",
            &format!("{path}/reject"),
            &Auth::Session(&f.alice_token),
            Some(&json!({ "reason": "r" })),
        );
        assert_error(&response, 404, "not_found");
    }
    assert_eq!(status_of(f, client, &project, &id), "open");

    // Another workspace's credentials reach neither the project nor its change sets.
    for auth in [Auth::Key(&f.key_b), Auth::Session(&f.bob_token)] {
        for path in [
            changesets_path(&f.a, &project),
            changeset_path(&f.a, &project, &id),
        ] {
            assert_error(&call(client, "GET", &path, &auth, None), 404, "not_found");
        }
    }
    let foreign = call(
        client,
        "POST",
        &format!("{}/abandon", changeset_path(&f.a, &project, &id)),
        &Auth::Key(&f.key_b),
        Some(&json!({})),
    );
    assert_error(&foreign, 404, "not_found");
    let foreign_reject = act(
        f,
        client,
        &project,
        &id,
        "reject",
        &Auth::Session(&f.bob_token),
        Some(&json!({ "reason": "from the wrong workspace" })),
    );
    assert_error(&foreign_reject, 404, "not_found");
    assert_eq!(status_of(f, client, &project, &id), "open");

    // A project of workspace B is not found under workspace A's path, either.
    let b_project = call(
        client,
        "POST",
        &projects_path(&f.b),
        &Auth::Key(&f.key_b),
        Some(&json!({ "name": unique("beta-project") })),
    )
    .json()["project"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_error(
        &call(
            client,
            "GET",
            &changesets_path(&f.a, &b_project),
            &key,
            None,
        ),
        404,
        "not_found",
    );
}

#[test]
fn change_set_routes_need_credentials() {
    let (f, client) = ready();
    let project = new_project(f, client, "auth");
    let id = id_of(&new_changeset(f, client, &project, "needs credentials"));
    let nobody = Auth::Nobody;
    let paths = [
        ("GET", changesets_path(&f.a, &project)),
        ("POST", changesets_path(&f.a, &project)),
        ("GET", changeset_path(&f.a, &project, &id)),
        (
            "POST",
            format!("{}/submit", changeset_path(&f.a, &project, &id)),
        ),
        (
            "POST",
            format!("{}/reject", changeset_path(&f.a, &project, &id)),
        ),
        (
            "POST",
            format!("{}/reopen", changeset_path(&f.a, &project, &id)),
        ),
        (
            "POST",
            format!("{}/abandon", changeset_path(&f.a, &project, &id)),
        ),
    ];
    for (method, path) in paths {
        let body = (method == "POST").then(|| json!({ "title": "x", "reason": "x" }));
        assert_error(
            &call(client, method, &path, &nobody, body.as_ref()),
            401,
            "unauthorized",
        );
    }
    assert_eq!(status_of(f, client, &project, &id), "open");
}

// ---------------------------------------------------------------------------
// The lifecycle
// ---------------------------------------------------------------------------

#[test]
fn an_agent_submits_an_open_change_set_for_review() {
    let (f, client) = ready();
    let project = new_project(f, client, "submit");
    let id = id_of(&new_changeset(f, client, &project, "to submit"));
    let response = act(
        f,
        client,
        &project,
        &id,
        "submit",
        &Auth::Key(&f.key_a),
        Some(&json!({})),
    );
    assert_eq!(response.status, 200, "{}", response.text());
    let changeset = response.json()["changeset"].clone();
    assert_eq!(changeset["status"], "in_review", "{changeset}");
    assert_eq!(
        steps(&changeset),
        [
            step(None, "open", "agent"),
            step(Some("open"), "in_review", "agent")
        ]
    );
    assert_eq!(status_of(f, client, &project, &id), "in_review");
}

#[test]
fn rejecting_is_for_the_human_alone_and_needs_a_reason() {
    let (f, client) = ready();
    let project = new_project(f, client, "reject");
    let id = new_changeset_in(f, client, &project, "in_review");
    let reason = "The hero contrast is too low; rework the headline.";

    // An agent cannot reject, with a reason or without one.
    for body in [json!({ "reason": reason }), json!({})] {
        let response = act(
            f,
            client,
            &project,
            &id,
            "reject",
            &Auth::Key(&f.key_a),
            Some(&body),
        );
        assert_error(&response, 403, "forbidden");
    }
    assert_eq!(status_of(f, client, &project, &id), "in_review");

    // The human must give a reason.
    for body in [
        json!({}),
        json!({ "reason": "" }),
        json!({ "reason": "  \n " }),
    ] {
        let response = act(
            f,
            client,
            &project,
            &id,
            "reject",
            &Auth::Session(&f.alice_token),
            Some(&body),
        );
        assert_error(&response, 400, "reason_required");
    }
    let unchanged = show(f, client, &project, &id);
    assert_eq!(unchanged["status"], "in_review", "{unchanged}");
    assert_eq!(history_of(&unchanged).len(), 2, "{unchanged}");

    let response = act(
        f,
        client,
        &project,
        &id,
        "reject",
        &Auth::Session(&f.alice_token),
        Some(&json!({ "reason": reason })),
    );
    assert_eq!(response.status, 200, "{}", response.text());
    let changeset = response.json()["changeset"].clone();
    assert_eq!(changeset["status"], "rejected", "{changeset}");
    let history = history_of(&changeset);
    let last = history.last().unwrap();
    assert_eq!(last["from_status"], "in_review", "{last}");
    assert_eq!(last["to_status"], "rejected", "{last}");
    assert_eq!(last["actor"], "human", "{last}");
    assert_eq!(last["reason"], reason, "the reason is stored: {last}");

    // The reason is still there when the change set is read later.
    let later = show(f, client, &project, &id);
    assert_eq!(later["status"], "rejected");
    assert_eq!(history_of(&later).last().unwrap()["reason"], reason);
}

#[test]
fn an_agent_reopens_a_rejected_change_set_and_the_cycle_can_repeat() {
    let (f, client) = ready();
    let project = new_project(f, client, "reopen");
    let id = new_changeset_in(f, client, &project, "rejected");
    let key = Auth::Key(&f.key_a);

    let reopened = act(f, client, &project, &id, "reopen", &key, Some(&json!({})));
    assert_eq!(reopened.status, 200, "{}", reopened.text());
    assert_eq!(reopened.json()["changeset"]["status"], "open");

    // The second round: submit again; reopening an open change set is refused.
    let submitted = act(f, client, &project, &id, "submit", &key, Some(&json!({})));
    assert_eq!(submitted.status, 200, "{}", submitted.text());
    assert_eq!(submitted.json()["changeset"]["status"], "in_review");
    let refused = act(f, client, &project, &id, "reopen", &key, Some(&json!({})));
    assert_error(&refused, 409, "invalid_transition");
    let human = Auth::Session(&f.alice_token);
    let rejected = act(
        f,
        client,
        &project,
        &id,
        "reject",
        &human,
        Some(&json!({ "reason": "second round, still no" })),
    );
    assert_eq!(rejected.status, 200, "{}", rejected.text());
    let reopened = act(f, client, &project, &id, "reopen", &key, Some(&json!({})));
    assert_eq!(reopened.status, 200, "{}", reopened.text());

    let changeset = show(f, client, &project, &id);
    assert_eq!(changeset["status"], "open");
    assert_eq!(
        steps(&changeset),
        [
            step(None, "open", "agent"),
            step(Some("open"), "in_review", "agent"),
            step(Some("in_review"), "rejected", "human"),
            step(Some("rejected"), "open", "agent"),
            step(Some("open"), "in_review", "agent"),
            step(Some("in_review"), "rejected", "human"),
            step(Some("rejected"), "open", "agent"),
        ],
        "every change is in the history, oldest first, with who made it"
    );
    let stored: Vec<String> = history_of(&changeset)
        .iter()
        .filter(|e| e["to_status"] == "rejected")
        .map(|e| e["reason"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(
        stored,
        [
            "not good enough yet".to_string(),
            "second round, still no".to_string()
        ],
        "each rejection keeps its own reason"
    );
}

#[test]
fn abandoning_marks_a_change_set_abandoned_and_it_is_never_deleted() {
    let (f, client) = ready();
    let project = new_project(f, client, "abandon");
    let key = Auth::Key(&f.key_a);
    for from in ["open", "in_review", "rejected"] {
        let id = new_changeset_in(f, client, &project, from);
        let before = history_of(&show(f, client, &project, &id)).len();
        let response = act(f, client, &project, &id, "abandon", &key, Some(&json!({})));
        assert_eq!(
            response.status,
            200,
            "abandon from {from}: {}",
            response.text()
        );
        let changeset = response.json()["changeset"].clone();
        assert_eq!(changeset["status"], "abandoned", "{changeset}");
        let history = history_of(&changeset);
        assert_eq!(history.len(), before + 1, "{changeset}");
        let last = history.last().unwrap();
        assert_eq!(last["from_status"], from, "{last}");
        assert_eq!(last["to_status"], "abandoned", "{last}");
        assert_eq!(last["actor"], "agent", "{last}");

        // It is still there: shown, listed and still in the database.
        assert_eq!(status_of(f, client, &project, &id), "abandoned");
        let list = call(client, "GET", &changesets_path(&f.a, &project), &key, None);
        assert!(
            list.json()["changesets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == id.as_str() && c["status"] == "abandoned"),
            "an abandoned change set is still listed: {}",
            list.text()
        );
    }
}

#[test]
fn every_transition_outside_the_lifecycle_is_a_409_that_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client, "matrix");
    let key = Auth::Key(&f.key_a);
    let human = Auth::Session(&f.alice_token);
    // (action, the statuses it may start from)
    let lifecycle = [
        ("submit", &["open"][..]),
        ("reject", &["in_review"][..]),
        ("reopen", &["rejected"][..]),
        ("abandon", &["open", "in_review", "rejected"][..]),
    ];
    for (action, allowed) in lifecycle {
        for status in ["open", "in_review", "rejected", "abandoned"] {
            if allowed.contains(&status) {
                continue;
            }
            let id = new_changeset_in(f, client, &project, status);
            let before = show(f, client, &project, &id);
            let (auth, body) = if action == "reject" {
                (&human, json!({ "reason": "valid reason" }))
            } else {
                (&key, json!({}))
            };
            let response = act(f, client, &project, &id, action, auth, Some(&body));
            assert_eq!(
                (response.status, response.json()),
                (409, json!({ "error": "invalid_transition" })),
                "{action} from {status}: {}",
                response.text()
            );
            let after = show(f, client, &project, &id);
            assert_eq!(after["status"], before["status"], "{action} from {status}");
            assert_eq!(
                history_of(&after),
                history_of(&before),
                "a refused {action} from {status} is not recorded"
            );
        }
    }
}

#[test]
fn two_agents_submitting_at_once_make_exactly_one_transition() {
    let (f, client) = ready();
    let project = new_project(f, client, "race");
    let id = id_of(&new_changeset(f, client, &project, "contended"));
    let path = format!("{}/submit", changeset_path(&f.a, &project, &id));
    let key = Auth::Key(&f.key_a);
    let statuses: Vec<u16> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| call(client, "POST", &path, &key, Some(&json!({}))).status))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(
        statuses.iter().filter(|s| **s == 200).count(),
        1,
        "exactly one submit wins: {statuses:?}"
    );
    assert_eq!(
        statuses.iter().filter(|s| **s == 409).count(),
        7,
        "the others are 409: {statuses:?}"
    );
    let changeset = show(f, client, &project, &id);
    assert_eq!(changeset["status"], "in_review");
    assert_eq!(
        steps(&changeset),
        [
            step(None, "open", "agent"),
            step(Some("open"), "in_review", "agent")
        ],
        "one transition, one history entry"
    );
}
