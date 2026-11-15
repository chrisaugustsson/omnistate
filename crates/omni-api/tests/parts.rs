//! US-304: parts, the apps and services of a project (FR-2, D14).
//! Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with two members
//! (alice and bob), each with a live session, mint an API key for alice
//! through the API and serve the API's router on a local port against that
//! database.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`):
//!
//! - `POST /parts` takes `{"name", "kind", "description"}`. A kind that is
//!   missing or not `app` or `service` is 422 `{"error": "invalid_kind",
//!   "message": "kind must be app or service"}` and creates nothing.
//! - Part names follow the track name rule: unique in the project ignoring
//!   case, among the parts that are not deleted. A second part of the name,
//!   or a rename to the name of another part, is 409 `name_taken`.
//! - `DELETE /parts/{part_id}` hides the part: `GET /parts/{part_id}` is 404
//!   `not_found`, `GET /parts` leaves it out, and its name can be used again.
//! - Any member creates, renames and deletes any part of the project, with a
//!   session or an API key (D14).
//! - Creating, renaming and deleting a part emit `part.created`,
//!   `part.renamed` and `part.deleted` on the project's live channel.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us304_parts";

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

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
        // Nothing here reaches RustFS: parts only use Postgres.
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
    workspace: String,
    alice_token: String,
    bob_token: String,
    key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us304-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
            for name in ["alice", "bob"] {
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
                    &[&workspace, &user],
                )
                .await;
            }
            workspace
        });
        let server = TestServer::start(&config());
        let response = call(
            &server.client,
            "POST",
            &format!("/api/v1/w/{workspace}/api-keys"),
            &Auth::Session(&session_token("alice")),
            Some(&json!({})),
        );
        assert_eq!(response.status, 201, "mint a key: {}", response.text());
        let key = response.json()["secret"]
            .as_str()
            .expect("the new key has a secret")
            .to_string();
        Fixture {
            workspace,
            alice_token: session_token("alice"),
            bob_token: session_token("bob"),
            key,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

/// Who sends a request: an agent (alice's API key with its run and role) or
/// a member (a session cookie).
#[derive(Clone, Copy)]
enum Auth<'a> {
    Agent(&'a str),
    Session(&'a str),
}

fn agent(f: &Fixture) -> Auth<'_> {
    Auth::Agent(&f.key)
}

fn alice(f: &Fixture) -> Auth<'_> {
    Auth::Session(&f.alice_token)
}

fn bob(f: &Fixture) -> Auth<'_> {
    Auth::Session(&f.bob_token)
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let headers: Vec<(&str, String)> = match auth {
        Auth::Agent(key) => vec![
            ("Authorization", format!("Bearer {key}")),
            ("X-Omni-Run", "run-1".to_string()),
            ("X-Omni-Role", "planner".to_string()),
        ],
        Auth::Session(token) => vec![("Cookie", cookie(token))],
    };
    let headers: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let body = body.map(Value::to_string);
    client.request(method, path, &headers, body.as_deref())
}

fn must(response: Response, status: u16, what: &str) -> Value {
    assert_eq!(response.status, status, "{what}: {}", response.text());
    response.json()
}

fn assert_error(response: &Response, status: u16, code: &str) {
    assert_eq!(response.status, status, "body: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": code }),
        "the error body carries the code and nothing else"
    );
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn parts_path(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}/parts", f.workspace)
}

fn part_path(f: &Fixture, project: &str, part: &str) -> String {
    format!("{}/{part}", parts_path(f, project))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    let created = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.workspace),
            &agent(f),
            Some(&json!({ "name": unique("part-project") })),
        ),
        201,
        "create a project",
    );
    created["project"]["id"]
        .as_str()
        .expect("the project has an id")
        .to_string()
}

/// Creates a part with `auth`; returns its ID.
fn create_part(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    name: &str,
    kind: &str,
) -> String {
    let created = must(
        call(
            client,
            "POST",
            &parts_path(f, project),
            auth,
            Some(&json!({ "name": name, "kind": kind })),
        ),
        201,
        "create a part",
    );
    created["part"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the part has an id: {created}"))
        .to_string()
}

/// The `(id, name)` of each part that `GET /parts` lists, in its order.
fn listed(f: &Fixture, client: &Client, project: &str) -> Vec<(String, String)> {
    let listed = must(
        call(client, "GET", &parts_path(f, project), &agent(f), None),
        200,
        "list the parts",
    );
    listed["parts"]
        .as_array()
        .unwrap_or_else(|| panic!("`parts` is a list: {listed}"))
        .iter()
        .map(|part| {
            (
                part["id"].as_str().unwrap_or_default().to_string(),
                part["name"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// How many rows the `parts` table has for a project, deleted ones included.
fn part_rows(project: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from parts where project_id = $1::uuid")
            .bind(project)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the parts: {e}"))
    })
}

/// The names of the live events of a project, oldest first.
fn event_names(project: &str) -> Vec<String> {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select event from live_events where project_id = $1::uuid order by seq")
            .bind(project)
            .fetch_all(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("read the live events: {e}"))
    })
}

// ---------------------------------------------------------------------------
// Kind
// ---------------------------------------------------------------------------

#[test]
fn a_kind_other_than_app_or_service_is_422_invalid_kind_with_its_message() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let response = call(
        client,
        "POST",
        &parts_path(f, &project),
        &alice(f),
        Some(&json!({ "name": "X", "kind": "device" })),
    );
    assert_eq!(response.status, 422, "body: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "invalid_kind", "message": "kind must be app or service" })
    );
    assert_eq!(part_rows(&project), 0, "the refused part is not created");
}

#[test]
fn a_part_without_a_kind_is_422_invalid_kind_with_its_message() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let response = call(
        client,
        "POST",
        &parts_path(f, &project),
        &alice(f),
        Some(&json!({ "name": "X" })),
    );
    assert_eq!(response.status, 422, "body: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "invalid_kind", "message": "kind must be app or service" })
    );
    assert_eq!(part_rows(&project), 0, "the refused part is not created");
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

#[test]
fn a_second_part_whose_name_differs_only_in_case_is_409_name_taken() {
    let (f, client) = ready();
    let project = new_project(f, client);
    create_part(f, client, &agent(f), &project, "Wision", "app");
    let response = call(
        client,
        "POST",
        &parts_path(f, &project),
        &agent(f),
        Some(&json!({ "name": "wision", "kind": "service" })),
    );
    assert_error(&response, 409, "name_taken");
    assert_eq!(part_rows(&project), 1, "the second part is not created");
}

#[test]
fn renaming_a_part_to_the_name_of_another_part_is_409_name_taken_and_keeps_its_name() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = create_part(f, client, &agent(f), &project, "Wision", "app");
    let iot = create_part(f, client, &agent(f), &project, "iot-service", "service");
    let response = call(
        client,
        "PATCH",
        &part_path(f, &project, &iot),
        &agent(f),
        Some(&json!({ "name": "Wision" })),
    );
    assert_error(&response, 409, "name_taken");
    assert_eq!(
        listed(f, client, &project),
        vec![
            (wision, "Wision".to_string()),
            (iot, "iot-service".to_string())
        ]
    );
}

// ---------------------------------------------------------------------------
// Deleting
// ---------------------------------------------------------------------------

#[test]
fn a_deleted_part_is_404_not_found_by_its_id() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = create_part(f, client, &agent(f), &project, "Wision", "app");
    let deleted = call(
        client,
        "DELETE",
        &part_path(f, &project, &part),
        &agent(f),
        None,
    );
    assert_eq!(deleted.status, 204, "delete the part: {}", deleted.text());
    let response = call(
        client,
        "GET",
        &part_path(f, &project, &part),
        &agent(f),
        None,
    );
    assert_error(&response, 404, "not_found");
}

#[test]
fn a_deleted_part_leaves_the_list_and_its_name_can_be_used_again() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let old = create_part(f, client, &agent(f), &project, "Wision", "app");
    let deleted = call(
        client,
        "DELETE",
        &part_path(f, &project, &old),
        &agent(f),
        None,
    );
    assert_eq!(deleted.status, 204, "delete the part: {}", deleted.text());
    assert_eq!(listed(f, client, &project), vec![]);
    let new = create_part(f, client, &agent(f), &project, "Wision", "service");
    assert_eq!(
        listed(f, client, &project),
        vec![(new, "Wision".to_string())]
    );
}

// ---------------------------------------------------------------------------
// Who may change a part (D14)
// ---------------------------------------------------------------------------

#[test]
fn a_member_with_a_session_renames_and_deletes_a_part_that_another_member_created() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = create_part(f, client, &alice(f), &project, "Wision", "app");
    let renamed = must(
        call(
            client,
            "PATCH",
            &part_path(f, &project, &part),
            &bob(f),
            Some(&json!({ "name": "Wision web" })),
        ),
        200,
        "bob renames alice's part",
    );
    assert_eq!(renamed["part"]["name"], "Wision web", "{renamed}");
    let deleted = call(
        client,
        "DELETE",
        &part_path(f, &project, &part),
        &bob(f),
        None,
    );
    assert_eq!(
        deleted.status,
        204,
        "bob deletes alice's part: {}",
        deleted.text()
    );
    assert_eq!(listed(f, client, &project), vec![]);
}

// ---------------------------------------------------------------------------
// Live events
// ---------------------------------------------------------------------------

#[test]
fn creating_renaming_and_deleting_a_part_emit_its_live_events() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = create_part(f, client, &agent(f), &project, "Wision", "app");
    must(
        call(
            client,
            "PATCH",
            &part_path(f, &project, &part),
            &agent(f),
            Some(&json!({ "name": "Wision web" })),
        ),
        200,
        "rename the part",
    );
    let deleted = call(
        client,
        "DELETE",
        &part_path(f, &project, &part),
        &agent(f),
        None,
    );
    assert_eq!(deleted.status, 204, "delete the part: {}", deleted.text());
    assert_eq!(
        event_names(&project),
        vec!["part.created", "part.renamed", "part.deleted"]
    );
}
