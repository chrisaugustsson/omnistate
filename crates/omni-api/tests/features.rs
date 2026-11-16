//! US-307: features, what a part can do (D5, D14).
//! Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with two members
//! (alice and bob), each with a live session, mint an API key for alice
//! through the API and serve the API's router on a local port against that
//! database.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}`):
//!
//! - `POST /features` takes `{"name", "description"}`. A feature name is
//!   unique in its part ignoring case, among the features that are not
//!   deleted: a second feature of the name on the part, or a rename to the
//!   name of another feature of the part, is 409 `name_taken`. Another part
//!   may have a feature of the same name.
//! - `DELETE /features/{feature_id}` hides the feature: `GET
//!   /features/{feature_id}` is 404 `not_found`, `GET /features` leaves it
//!   out, and its name can be used again on the part.
//! - The features of a deleted part are hidden with it: its feature routes
//!   are 404 `not_found`.
//! - Any member creates, renames and deletes any feature, with a session or
//!   an API key (D14).

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

const TEST_DB: &str = "omni_us307_features";

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
        // Nothing here reaches RustFS: features only use Postgres.
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
    format!("us307-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

fn part_path(f: &Fixture, project: &str, part: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}/parts/{part}", f.workspace)
}

fn features_path(f: &Fixture, project: &str, part: &str) -> String {
    format!("{}/features", part_path(f, project, part))
}

fn feature_path(f: &Fixture, project: &str, part: &str, feature: &str) -> String {
    format!("{}/{feature}", features_path(f, project, part))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    let created = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.workspace),
            &agent(f),
            Some(&json!({ "name": unique("feature-project") })),
        ),
        201,
        "create a project",
    );
    created["project"]["id"]
        .as_str()
        .expect("the project has an id")
        .to_string()
}

/// Creates a part of the project; returns its ID.
fn create_part(f: &Fixture, client: &Client, project: &str, name: &str, kind: &str) -> String {
    let created = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects/{project}/parts", f.workspace),
            &agent(f),
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

/// Creates a feature on a part with `auth`; returns its ID.
fn create_feature(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    part: &str,
    name: &str,
) -> String {
    let created = must(
        call(
            client,
            "POST",
            &features_path(f, project, part),
            auth,
            Some(&json!({ "name": name, "description": "Time-series chart per channel" })),
        ),
        201,
        "create a feature",
    );
    created["feature"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the feature has an id: {created}"))
        .to_string()
}

/// The `(id, name)` of each feature that `GET /features` lists, in its order.
fn listed(f: &Fixture, client: &Client, project: &str, part: &str) -> Vec<(String, String)> {
    let listed = must(
        call(
            client,
            "GET",
            &features_path(f, project, part),
            &agent(f),
            None,
        ),
        200,
        "list the features",
    );
    listed["features"]
        .as_array()
        .unwrap_or_else(|| panic!("`features` is a list: {listed}"))
        .iter()
        .map(|feature| {
            (
                feature["id"].as_str().unwrap_or_default().to_string(),
                feature["name"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

#[test]
fn a_second_feature_of_the_name_on_the_part_is_409_name_taken_ignoring_case_and_another_part_may_have_it()
 {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = create_part(f, client, &project, "Wision", "app");
    let iot = create_part(f, client, &project, "iot-service", "service");
    let analyze = create_feature(f, client, &agent(f), &project, &wision, "Analyze");
    for name in ["Analyze", "analyze", "ANALYZE"] {
        let response = call(
            client,
            "POST",
            &features_path(f, &project, &wision),
            &agent(f),
            Some(&json!({ "name": name })),
        );
        assert_error(&response, 409, "name_taken");
    }
    assert_eq!(
        listed(f, client, &project, &wision),
        vec![(analyze, "Analyze".to_string())],
        "no second Analyze is created on Wision"
    );
    let on_iot = create_feature(f, client, &agent(f), &project, &iot, "Analyze");
    assert_eq!(
        listed(f, client, &project, &iot),
        vec![(on_iot, "Analyze".to_string())]
    );
}

#[test]
fn renaming_a_feature_to_the_name_of_another_feature_of_its_part_is_409_name_taken_and_keeps_its_name()
 {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = create_part(f, client, &project, "Wision", "app");
    let analyze = create_feature(f, client, &agent(f), &project, &wision, "Analyze");
    let alarms = create_feature(f, client, &agent(f), &project, &wision, "Alarms");
    let response = call(
        client,
        "PATCH",
        &feature_path(f, &project, &wision, &alarms),
        &agent(f),
        Some(&json!({ "name": "analyze" })),
    );
    assert_error(&response, 409, "name_taken");
    assert_eq!(
        listed(f, client, &project, &wision),
        vec![
            (analyze, "Analyze".to_string()),
            (alarms, "Alarms".to_string())
        ]
    );
}

// ---------------------------------------------------------------------------
// Deleting hides (D14)
// ---------------------------------------------------------------------------

#[test]
fn a_deleted_feature_is_404_by_its_id_leaves_the_list_and_its_name_can_be_used_again() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = create_part(f, client, &project, "Wision", "app");
    let old = create_feature(f, client, &agent(f), &project, &wision, "Analyze");
    let deleted = call(
        client,
        "DELETE",
        &feature_path(f, &project, &wision, &old),
        &agent(f),
        None,
    );
    assert_eq!(
        deleted.status,
        204,
        "delete the feature: {}",
        deleted.text()
    );
    for method in ["GET", "PATCH", "DELETE"] {
        let body = (method == "PATCH").then(|| json!({ "name": "Analyze again" }));
        let response = call(
            client,
            method,
            &feature_path(f, &project, &wision, &old),
            &agent(f),
            body.as_ref(),
        );
        assert_error(&response, 404, "not_found");
    }
    assert_eq!(listed(f, client, &project, &wision), vec![]);
    let new = create_feature(f, client, &agent(f), &project, &wision, "Analyze");
    assert_eq!(
        listed(f, client, &project, &wision),
        vec![(new, "Analyze".to_string())]
    );
}

#[test]
fn the_features_of_a_deleted_part_are_404_not_found() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = create_part(f, client, &project, "Wision", "app");
    let analyze = create_feature(f, client, &agent(f), &project, &wision, "Analyze");
    let deleted = call(
        client,
        "DELETE",
        &part_path(f, &project, &wision),
        &agent(f),
        None,
    );
    assert_eq!(deleted.status, 204, "delete the part: {}", deleted.text());
    for (method, path) in [
        ("GET", features_path(f, &project, &wision)),
        ("GET", feature_path(f, &project, &wision, &analyze)),
        ("DELETE", feature_path(f, &project, &wision, &analyze)),
    ] {
        let response = call(client, method, &path, &agent(f), None);
        assert_error(&response, 404, "not_found");
    }
}

// ---------------------------------------------------------------------------
// Who may change a feature (D14)
// ---------------------------------------------------------------------------

#[test]
fn a_member_with_a_session_renames_and_deletes_a_feature_that_another_member_created() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = create_part(f, client, &project, "Wision", "app");
    let feature = create_feature(f, client, &alice(f), &project, &wision, "Analyze");
    let renamed = must(
        call(
            client,
            "PATCH",
            &feature_path(f, &project, &wision, &feature),
            &bob(f),
            Some(&json!({ "name": "Analyze channels" })),
        ),
        200,
        "bob renames alice's feature",
    );
    assert_eq!(renamed["feature"]["name"], "Analyze channels", "{renamed}");
    let deleted = call(
        client,
        "DELETE",
        &feature_path(f, &project, &wision, &feature),
        &bob(f),
        None,
    );
    assert_eq!(
        deleted.status,
        204,
        "bob deletes alice's feature: {}",
        deleted.text()
    );
    assert_eq!(listed(f, client, &project, &wision), vec![]);
}
