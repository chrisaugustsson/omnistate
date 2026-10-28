//! US-059: features, the level between a project and its change sets (Q51).
//! Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API and serve the
//! API's router on a local port against that database.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`):
//!
//! - `POST /features` (agent) takes `{"name", "description"}` and answers 201
//!   `{"feature": Feature}`. `name` is required: missing, blank or not a
//!   string is 400 `{"error": "invalid_request"}` and creates nothing.
//!   `description` is optional and defaults to `""`. A session (the human) is
//!   403 `{"error": "forbidden"}`.
//! - A Feature (`FeatureBody`) is `{"id" (UUIDv7), "project_id", "name",
//!   "description", "created_at"}` and nothing else: since US-110 a feature
//!   has no status.
//! - Feature names are unique in their project, ignoring case: a second
//!   feature named "checkout" next to "Checkout" is 409 `{"error":
//!   "name_taken"}` and creates nothing. Another project may use the name.
//! - `GET /features` (both) answers `{"features": [Feature, ...]}`, the
//!   project's features in the order they were created. `GET
//!   /features/{feature_id}` (both) answers `{"feature": Feature}`.
//! - An unknown project, an unknown feature, a feature of another project and
//!   an ID that is not a UUID are 404 `{"error": "not_found"}`.
//! - Creating a feature emits `feature.created` on the project's live channel
//!   with `data` that has at least `{"feature_id", "name"}`. A refused request
//!   emits nothing.
//! - Features live in the tenant table `features` (migrations/README.md):
//!   `id`, `workspace_id`, `project_id`, `name` (not blank), `description`
//!   (default `''`), `created_at` and `deleted_at` (US-110; null until an
//!   agent deletes it), with no `status` column, a composite foreign key to
//!   `projects`, `UNIQUE (workspace_id, project_id, id)` and a unique index on
//!   `(workspace_id, project_id, lower(name))` over the features that are not
//!   deleted, so a deleted feature's name can be taken again.
//!   tests/workspace_delete.rs and the isolation suite seed it.
//!
//! US-060 adds change sets in a feature:
//!
//! - `changesets.feature_id` is nullable (null is a project-level change set)
//!   with the foreign key `(workspace_id, project_id, feature_id)` to
//!   `features (workspace_id, project_id, id)`.
//! - `POST /changesets` takes an optional `feature_id`. Without it (or with
//!   `null`) the change set is project level. A feature that is not one of the
//!   project's (another project's, or no feature at all) is 422 `{"error":
//!   "feature_not_in_project"}` and creates nothing.
//! - A ChangesetBody and a ThreadBody carry `feature_id` (`null` at project
//!   level); a thread takes its change set's. So does the `data` of the
//!   `changeset.*` and `thread.created` live events.
//! - `GET /changesets` lists every change set of the project, `?feature=<id>`
//!   only that feature's and `?feature=global` only the project-level ones.

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

const TEST_DB: &str = "omni_us059_features";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";

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
    key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us059-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
            let user = text(
                "insert into users (email) values ('alice@example.test') returning id::text",
                &[],
            )
            .await;
            text(
                "insert into sessions (user_id, token_hash, expires_at) \
                 values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
                 returning id::text",
                &[&user, &session_token("alice")],
            )
            .await;
            text(
                "insert into memberships (workspace_id, user_id) \
                 values ($1::uuid, $2::uuid) returning id::text",
                &[&workspace, &user],
            )
            .await;
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
            key,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

/// Who sends a request: an agent (an API key with its run and role) or the
/// human (a session cookie).
#[derive(Clone, Copy)]
enum Auth<'a> {
    Agent(&'a str),
    Session(&'a str),
}

fn agent(f: &Fixture) -> Auth<'_> {
    Auth::Agent(&f.key)
}

fn human(f: &Fixture) -> Auth<'_> {
    Auth::Session(&f.alice_token)
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

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

fn features_path(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}/features", f.workspace)
}

fn feature_path(f: &Fixture, project: &str, feature: &str) -> String {
    format!("{}/{feature}", features_path(f, project))
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

fn create_feature_response(f: &Fixture, client: &Client, project: &str, body: &Value) -> Response {
    call(
        client,
        "POST",
        &features_path(f, project),
        &agent(f),
        Some(body),
    )
}

/// Creates a feature as the agent; returns the `feature` of the answer.
fn create_feature(f: &Fixture, client: &Client, project: &str, name: &str) -> Value {
    must(
        create_feature_response(f, client, project, &json!({ "name": name })),
        201,
        "create a feature",
    )["feature"]
        .clone()
}

fn list_features(f: &Fixture, client: &Client, auth: &Auth, project: &str) -> Vec<Value> {
    let listed = must(
        call(client, "GET", &features_path(f, project), auth, None),
        200,
        "list the features",
    );
    listed["features"]
        .as_array()
        .unwrap_or_else(|| panic!("`features` is a list: {listed}"))
        .clone()
}

/// The `data` of the live events of a project named `event`, oldest first.
fn events_of(project: &str, event: &str) -> Vec<Value> {
    block(async {
        let mut conn = admin_connection().await;
        let rows = sqlx::query_scalar::<_, String>(
            "select data::jsonb::text from live_events \
             where project_id = $1::uuid and event = $2 order by seq",
        )
        .bind(project)
        .bind(event)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the live events: {e}"));
        rows.iter().map(|text| parse(text)).collect()
    })
}

/// How many rows the `features` table has for a project.
fn feature_rows(project: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from features where project_id = $1::uuid")
            .bind(project)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the features: {e}"))
    })
}

/// Runs `sql` as the admin role and says whether the database accepted it;
/// whatever it did is rolled back.
fn accepted(sql: &str, binds: &[&str]) -> bool {
    block(async {
        let mut conn = admin_connection().await;
        let mut tx = conn.begin().await.expect("begin");
        let mut query = sqlx::query(sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        let result = query.execute(&mut *tx).await;
        tx.rollback().await.expect("roll back");
        result.is_ok()
    })
}

// ---------------------------------------------------------------------------
// Creating a feature
// ---------------------------------------------------------------------------

#[test]
fn creating_a_feature_records_its_name_and_description_and_no_status() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let created = must(
        create_feature_response(
            f,
            client,
            &project,
            &json!({ "name": "Checkout", "description": "Pay for the basket in one step." }),
        ),
        201,
        "create a feature",
    );
    let feature = &created["feature"];
    assert_uuid_v7(&feature["id"]);
    assert_eq!(feature["project_id"], project.as_str(), "{feature}");
    assert_eq!(feature["name"], "Checkout", "{feature}");
    assert_eq!(
        feature["description"], "Pay for the basket in one step.",
        "{feature}"
    );
    assert!(
        feature.get("status").is_none(),
        "a feature has no status (V7, US-110): {feature}"
    );
    assert!(
        feature["created_at"]
            .as_str()
            .is_some_and(|t| !t.is_empty()),
        "created_at is a timestamp: {feature}"
    );

    // The agent and the human read the same feature back.
    let id = feature["id"].as_str().expect("an id");
    for auth in [agent(f), human(f)] {
        let shown = must(
            call(client, "GET", &feature_path(f, &project, id), &auth, None),
            200,
            "show the feature",
        );
        assert_eq!(&shown["feature"], feature, "{shown}");
    }
}

#[test]
fn a_feature_needs_a_name() {
    let (f, client) = ready();
    let project = new_project(f, client);
    for body in [
        json!({}),
        json!({ "description": "No name." }),
        json!({ "name": "" }),
        json!({ "name": "   \n\t" }),
        json!({ "name": null }),
        json!({ "name": 7 }),
    ] {
        let response = create_feature_response(f, client, &project, &body);
        assert_error(&response, 400, "invalid_request");
    }
    let response = call(client, "POST", &features_path(f, &project), &agent(f), None);
    assert_eq!(response.status, 400, "no body: {}", response.text());
    assert!(
        list_features(f, client, &agent(f), &project).is_empty(),
        "a refused request creates nothing"
    );
    assert!(
        events_of(&project, "feature.created").is_empty(),
        "a refused request emits nothing"
    );
}

#[test]
fn feature_names_are_unique_in_a_project_ignoring_case() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let first = create_feature(f, client, &project, "Checkout");
    for name in ["checkout", "CHECKOUT", "Checkout"] {
        let response = create_feature_response(f, client, &project, &json!({ "name": name }));
        assert_error(&response, 409, "name_taken");
    }
    let listed = list_features(f, client, &agent(f), &project);
    assert_eq!(listed, [first], "the refused names created nothing");
    assert_eq!(
        events_of(&project, "feature.created").len(),
        1,
        "a refused request emits nothing"
    );

    // Another project of the workspace may use the name.
    let other = new_project(f, client);
    let reused = create_feature(f, client, &other, "checkout");
    assert_eq!(reused["project_id"], other.as_str(), "{reused}");
}

#[test]
fn only_an_agent_creates_a_feature() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let response = call(
        client,
        "POST",
        &features_path(f, &project),
        &human(f),
        Some(&json!({ "name": "Checkout" })),
    );
    assert_error(&response, 403, "forbidden");
    assert_eq!(
        feature_rows(&project),
        0,
        "a refused request creates nothing"
    );
}

// ---------------------------------------------------------------------------
// Reading features
// ---------------------------------------------------------------------------

#[test]
fn the_feature_list_is_in_creation_order_for_the_agent_and_the_human() {
    let (f, client) = ready();
    let project = new_project(f, client);
    for auth in [agent(f), human(f)] {
        assert!(
            list_features(f, client, &auth, &project).is_empty(),
            "a new project has no features"
        );
    }
    let created: Vec<Value> = ["Zebra crossing", "Apple pay", "Mango theme"]
        .into_iter()
        .map(|name| create_feature(f, client, &project, name))
        .collect();
    let other = new_project(f, client);
    create_feature(f, client, &other, "Elsewhere");

    for auth in [agent(f), human(f)] {
        let listed = list_features(f, client, &auth, &project);
        assert_eq!(
            listed, created,
            "the project's features, in the order they were created"
        );
    }
}

#[test]
fn an_unknown_project_or_feature_is_not_found() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = create_feature(f, client, &project, "Checkout");
    let id = feature["id"].as_str().expect("an id");
    let other = new_project(f, client);

    let response = create_feature_response(f, client, UNKNOWN_ID, &json!({ "name": "Lost" }));
    assert_error(&response, 404, "not_found");
    for auth in [agent(f), human(f)] {
        for path in [
            features_path(f, UNKNOWN_ID),
            features_path(f, "not-a-uuid"),
            feature_path(f, &project, UNKNOWN_ID),
            feature_path(f, &project, "not-a-uuid"),
            feature_path(f, UNKNOWN_ID, id),
            // A feature is read through its own project only.
            feature_path(f, &other, id),
        ] {
            let response = call(client, "GET", &path, &auth, None);
            assert_error(&response, 404, "not_found");
        }
    }
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

#[test]
fn the_features_table_keeps_its_rules_in_the_database() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let workspace = f.workspace.as_str();

    let insert = "insert into features (workspace_id, project_id, name) \
                  values ($1::uuid, $2::uuid, $3)";
    assert!(
        accepted(insert, &[workspace, &project, "Checkout"]),
        "a named feature of the row's own project is accepted"
    );
    for blank in ["", "   "] {
        assert!(
            !accepted(insert, &[workspace, &project, blank]),
            "the blank name {blank:?} is refused"
        );
    }
    assert!(
        !accepted(insert, &[workspace, UNKNOWN_ID, "Nowhere"]),
        "a feature needs a project"
    );
}

// ---------------------------------------------------------------------------
// US-060: change sets in a feature
// ---------------------------------------------------------------------------

fn changesets_path(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}/changesets", f.workspace)
}

fn create_changeset_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &changesets_path(f, project),
        &agent(f),
        Some(body),
    )
}

/// Opens a change set as the agent; returns the `changeset` of the answer.
fn create_changeset(f: &Fixture, client: &Client, project: &str, body: &Value) -> Value {
    must(
        create_changeset_response(f, client, project, body),
        201,
        "create a change set",
    )["changeset"]
        .clone()
}

fn list_changesets(client: &Client, auth: &Auth, path: &str) -> Vec<Value> {
    let listed = must(
        call(client, "GET", path, auth, None),
        200,
        "list the change sets",
    );
    listed["changesets"]
        .as_array()
        .unwrap_or_else(|| panic!("`changesets` is a list: {listed}"))
        .clone()
}

fn ids(items: &[Value]) -> Vec<Value> {
    items.iter().map(|item| item["id"].clone()).collect()
}

/// Asserts that `value` has the key `feature_id` and that it is `expected`
/// (`Value::Null` at project level).
fn assert_feature_id(value: &Value, expected: &Value, what: &str) {
    assert!(
        value
            .as_object()
            .is_some_and(|o| o.contains_key("feature_id")),
        "{what} carries `feature_id`: {value}"
    );
    assert_eq!(&value["feature_id"], expected, "{what}: {value}");
}

#[test]
fn a_change_set_opens_in_a_feature_or_at_project_level() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = create_feature(f, client, &project, "Checkout");

    let in_feature = create_changeset(
        f,
        client,
        &project,
        &json!({ "title": "Checkout flow", "feature_id": feature["id"] }),
    );
    assert_feature_id(&in_feature, &feature["id"], "a change set in a feature");
    assert_eq!(in_feature["project_id"], project.as_str(), "{in_feature}");

    let global = create_changeset(f, client, &project, &json!({ "title": "Brand refresh" }));
    assert_feature_id(&global, &Value::Null, "a change set without a feature");
    let explicit = create_changeset(
        f,
        client,
        &project,
        &json!({ "title": "Typography", "feature_id": null }),
    );
    assert_feature_id(&explicit, &Value::Null, "a change set with feature_id null");

    // The agent and the human read the scope back.
    for auth in [agent(f), human(f)] {
        for (changeset, expected) in [(&in_feature, &feature["id"]), (&global, &Value::Null)] {
            let id = changeset["id"].as_str().expect("an id");
            let shown = must(
                call(
                    client,
                    "GET",
                    &format!("{}/{id}", changesets_path(f, &project)),
                    &auth,
                    None,
                ),
                200,
                "show the change set",
            );
            assert_feature_id(&shown["changeset"], expected, "the shown change set");
        }
    }
}

#[test]
fn a_change_set_cannot_open_in_a_feature_of_another_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let other = new_project(f, client);
    let foreign = create_feature(f, client, &other, "Checkout");
    let opened_before = events_of(&project, "changeset.open").len();

    for feature_id in [foreign["id"].clone(), json!(UNKNOWN_ID)] {
        let response = create_changeset_response(
            f,
            client,
            &project,
            &json!({ "title": "Borrowed", "feature_id": feature_id }),
        );
        assert_error(&response, 422, "feature_not_in_project");
    }
    assert!(
        list_changesets(client, &agent(f), &changesets_path(f, &project)).is_empty(),
        "a refused request creates nothing"
    );
    assert_eq!(
        events_of(&project, "changeset.open").len(),
        opened_before,
        "a refused request emits nothing"
    );
}

#[test]
fn the_changesets_table_ties_a_change_set_to_a_feature_of_its_own_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let other = new_project(f, client);
    let own = create_feature(f, client, &project, "Checkout");
    let foreign = create_feature(f, client, &other, "Checkout");
    let workspace = f.workspace.as_str();
    let insert = "insert into changesets (workspace_id, project_id, title, feature_id) \
                  values ($1::uuid, $2::uuid, 'Direct', $3::uuid)";
    assert!(
        accepted(insert, &[workspace, &project, own["id"].as_str().unwrap()]),
        "a change set may belong to a feature of its project"
    );
    assert!(
        !accepted(
            insert,
            &[workspace, &project, foreign["id"].as_str().unwrap()]
        ),
        "a change set cannot belong to a feature of another project"
    );
    assert!(
        accepted(
            "insert into changesets (workspace_id, project_id, title) \
             values ($1::uuid, $2::uuid, 'Project level')",
            &[workspace, &project]
        ),
        "a change set without a feature is project level"
    );

    // The foreign key is (workspace_id, project_id, feature_id) to features.
    let keys: Vec<String> = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar(
            "select (select string_agg(a.attname::text, ',' order by a.attname::text) \
                     from pg_attribute a \
                     where a.attrelid = c.conrelid and a.attnum = any(c.conkey)) \
             from pg_constraint c \
             where c.contype = 'f' and c.conrelid = 'changesets'::regclass \
               and c.confrelid = 'features'::regclass",
        )
        .fetch_all(&mut conn)
        .await
        .expect("look for the foreign key")
    });
    assert_eq!(
        keys,
        ["feature_id,project_id,workspace_id"],
        "changesets has the foreign key (workspace_id, project_id, feature_id) to features"
    );
    let nullable: String = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar(
            "select coalesce((select is_nullable from information_schema.columns \
             where table_name = 'changesets' and column_name = 'feature_id'), 'missing')",
        )
        .fetch_one(&mut conn)
        .await
        .expect("look for the column")
    });
    assert_eq!(nullable, "YES", "changesets.feature_id is nullable");
}

#[test]
fn a_thread_takes_the_feature_of_its_change_set() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = create_feature(f, client, &project, "Checkout");
    let in_feature = create_changeset(
        f,
        client,
        &project,
        &json!({ "title": "Checkout flow", "feature_id": feature["id"] }),
    );
    let global = create_changeset(f, client, &project, &json!({ "title": "Brand refresh" }));

    for (changeset, expected) in [(&in_feature, &feature["id"]), (&global, &Value::Null)] {
        let changeset_id = changeset["id"].as_str().expect("an id");
        let created = must(
            call(
                client,
                "POST",
                &format!("{}/{changeset_id}/threads", changesets_path(f, &project)),
                &agent(f),
                Some(&json!({
                    "question": "Which payment step comes first?",
                    "criteria": ["Fewest taps"]
                })),
            ),
            201,
            "start a thread",
        );
        let thread = &created["thread"];
        assert_feature_id(thread, expected, "a new thread");
        let thread_id = thread["id"].as_str().expect("an id");
        for auth in [agent(f), human(f)] {
            let shown = must(
                call(
                    client,
                    "GET",
                    &format!(
                        "/api/v1/w/{}/projects/{project}/threads/{thread_id}",
                        f.workspace
                    ),
                    &auth,
                    None,
                ),
                200,
                "read the thread",
            );
            assert_feature_id(&shown["thread"], expected, "the read thread");
        }
        let event = events_of(&project, "thread.created")
            .into_iter()
            .find(|data| data["thread_id"] == thread["id"])
            .unwrap_or_else(|| panic!("thread.created for {thread_id}"));
        assert_feature_id(&event, expected, "the data of thread.created");
    }
}

#[test]
fn the_changeset_events_carry_the_feature() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = create_feature(f, client, &project, "Checkout");
    let in_feature = create_changeset(
        f,
        client,
        &project,
        &json!({ "title": "Checkout flow", "feature_id": feature["id"] }),
    );
    let global = create_changeset(f, client, &project, &json!({ "title": "Brand refresh" }));

    for (changeset, expected) in [(&in_feature, &feature["id"]), (&global, &Value::Null)] {
        let id = changeset["id"].as_str().expect("an id");
        must(
            call(
                client,
                "POST",
                &format!("{}/{id}/submit", changesets_path(f, &project)),
                &agent(f),
                Some(&json!({})),
            ),
            200,
            "submit the change set",
        );
        for event in ["changeset.open", "changeset.in_review"] {
            let data = events_of(&project, event)
                .into_iter()
                .find(|data| data["changeset_id"] == changeset["id"])
                .unwrap_or_else(|| panic!("{event} for {id}"));
            assert_feature_id(&data, expected, &format!("the data of {event}"));
        }
    }
}

#[test]
fn the_change_set_list_filters_by_feature_or_global() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = create_feature(f, client, &project, "Checkout");
    let feature_id = feature["id"].as_str().expect("an id");
    let global = create_changeset(f, client, &project, &json!({ "title": "Brand refresh" }));
    let in_feature = create_changeset(
        f,
        client,
        &project,
        &json!({ "title": "Checkout flow", "feature_id": feature_id }),
    );

    let path = changesets_path(f, &project);
    for auth in [agent(f), human(f)] {
        let all = list_changesets(client, &auth, &path);
        assert_eq!(
            ids(&all),
            [global["id"].clone(), in_feature["id"].clone()],
            "without a filter, every change set of the project"
        );
        assert_feature_id(&all[0], &Value::Null, "the listed project-level change set");
        assert_feature_id(
            &all[1],
            &feature["id"],
            "the listed change set of the feature",
        );

        let of_feature = list_changesets(client, &auth, &format!("{path}?feature={feature_id}"));
        assert_eq!(
            ids(&of_feature),
            [in_feature["id"].clone()],
            "?feature=<id> lists that feature's change sets only"
        );

        let project_level = list_changesets(client, &auth, &format!("{path}?feature=global"));
        assert_eq!(
            ids(&project_level),
            [global["id"].clone()],
            "?feature=global lists the project-level change sets only"
        );
    }
}
