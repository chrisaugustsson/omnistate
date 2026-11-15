//! US-059: tracks, the level below a project (Q51).
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
//! - `POST /tracks` (agent) takes `{"name", "description"}` and answers 201
//!   `{"track": Track}`. `name` is required: missing, blank or not a
//!   string is 400 `{"error": "invalid_request"}` and creates nothing.
//!   `description` is optional and defaults to `""`. A session (the human) is
//!   403 `{"error": "forbidden"}`.
//! - A Track (`TrackBody`) is `{"id" (UUIDv7), "project_id", "name",
//!   "description", "created_at"}` and nothing else: since US-110 a track
//!   has no status.
//! - Track names are unique in their project, ignoring case: a second
//!   track named "checkout" next to "Checkout" is 409 `{"error":
//!   "name_taken"}` and creates nothing. Another project may use the name.
//! - `GET /tracks` (both) answers `{"tracks": [Track, ...]}`, the
//!   project's tracks in the order they were created. `GET
//!   /tracks/{track_id}` (both) answers `{"track": Track}`.
//! - An unknown project, an unknown track, a track of another project and
//!   an ID that is not a UUID are 404 `{"error": "not_found"}`.
//! - Creating a track emits `track.created` on the project's live channel
//!   with `data` that has at least `{"track_id", "name"}`. A refused request
//!   emits nothing.
//! - Tracks live in the tenant table `tracks` (migrations/README.md):
//!   `id`, `workspace_id`, `project_id`, `name` (not blank), `description`
//!   (default `''`), `created_at` and `deleted_at` (US-110; null until an
//!   agent deletes it), with no `status` column, a composite foreign key to
//!   `projects`, `UNIQUE (workspace_id, project_id, id)` and a unique index on
//!   `(workspace_id, project_id, lower(name))` over the tracks that are not
//!   deleted, so a deleted track's name can be taken again.
//!   tests/workspace_delete.rs and the isolation suite seed it.
//! - US-305: a track sits on one part of its project. `POST /tracks` takes
//!   `part_id`: missing or null is 422 `{"error": "part_required"}`, and any
//!   value that names no undeleted part of the project (a deleted part, a
//!   part of another project, an unknown ID, a value that is not a UUID) is
//!   422 `{"error": "invalid_part"}`; both create nothing. A Track carries its
//!   `part_id`. `POST /tracks/{track_id}/move` (agent) takes `{"part_id"}`
//!   under the same rules and answers 200 `{"track": Track}` on the new part;
//!   moving to the part it is on changes nothing, and a refused move leaves
//!   the track on its part. `tracks.part_id` is NOT NULL, and the database
//!   refuses a part of another project.

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

const TEST_DB: &str = "omni_us059_tracks";
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
        // Nothing here reaches RustFS: tracks only use Postgres.
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

fn tracks_path(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}/tracks", f.workspace)
}

fn track_path(f: &Fixture, project: &str, track: &str) -> String {
    format!("{}/{track}", tracks_path(f, project))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    let created = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.workspace),
            &agent(f),
            Some(&json!({ "name": unique("track-project") })),
        ),
        201,
        "create a project",
    );
    created["project"]["id"]
        .as_str()
        .expect("the project has an id")
        .to_string()
}

/// Creates a part of the project as the agent; returns its ID.
fn new_part(f: &Fixture, client: &Client, project: &str, name: &str) -> String {
    must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects/{project}/parts", f.workspace),
            &agent(f),
            Some(&json!({ "name": name, "kind": "app" })),
        ),
        201,
        "create a part",
    )["part"]["id"]
        .as_str()
        .expect("the part has an id")
        .to_string()
}

fn delete_part(f: &Fixture, client: &Client, project: &str, part: &str) {
    let response = call(
        client,
        "DELETE",
        &format!("/api/v1/w/{}/projects/{project}/parts/{part}", f.workspace),
        &agent(f),
        None,
    );
    assert_eq!(response.status, 204, "delete a part: {}", response.text());
}

fn create_track_response(f: &Fixture, client: &Client, project: &str, body: &Value) -> Response {
    call(
        client,
        "POST",
        &tracks_path(f, project),
        &agent(f),
        Some(body),
    )
}

/// Creates a track as the agent, on a new part of the project; returns the
/// `track` of the answer.
fn create_track(f: &Fixture, client: &Client, project: &str, name: &str) -> Value {
    let part = new_part(f, client, project, &unique("part"));
    must(
        create_track_response(
            f,
            client,
            project,
            &json!({ "name": name, "part_id": part }),
        ),
        201,
        "create a track",
    )["track"]
        .clone()
}

fn move_track_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    track: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/move", track_path(f, project, track)),
        &agent(f),
        Some(body),
    )
}

/// The `part_id` of a track, as the API shows it.
fn part_of(f: &Fixture, client: &Client, project: &str, track: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &track_path(f, project, track),
            &agent(f),
            None,
        ),
        200,
        "show the track",
    )["track"]["part_id"]
        .clone()
}

fn list_tracks(f: &Fixture, client: &Client, auth: &Auth, project: &str) -> Vec<Value> {
    let listed = must(
        call(client, "GET", &tracks_path(f, project), auth, None),
        200,
        "list the tracks",
    );
    listed["tracks"]
        .as_array()
        .unwrap_or_else(|| panic!("`tracks` is a list: {listed}"))
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

/// How many rows the `tracks` table has for a project.
fn track_rows(project: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from tracks where project_id = $1::uuid")
            .bind(project)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the tracks: {e}"))
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
// Creating a track
// ---------------------------------------------------------------------------

#[test]
fn creating_a_track_records_its_name_and_description_and_no_status() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = new_part(f, client, &project, "Wision");
    let created = must(
        create_track_response(
            f,
            client,
            &project,
            &json!({
                "name": "Checkout",
                "description": "Pay for the basket in one step.",
                "part_id": part
            }),
        ),
        201,
        "create a track",
    );
    let track = &created["track"];
    assert_uuid_v7(&track["id"]);
    assert_eq!(track["project_id"], project.as_str(), "{track}");
    assert_eq!(track["name"], "Checkout", "{track}");
    assert_eq!(
        track["description"], "Pay for the basket in one step.",
        "{track}"
    );
    assert_eq!(track["part_id"], part.as_str(), "{track}");
    assert!(
        track.get("status").is_none(),
        "a track has no status (V7, US-110): {track}"
    );
    assert!(
        track["created_at"].as_str().is_some_and(|t| !t.is_empty()),
        "created_at is a timestamp: {track}"
    );

    // The agent and the human read the same track back.
    let id = track["id"].as_str().expect("an id");
    for auth in [agent(f), human(f)] {
        let shown = must(
            call(client, "GET", &track_path(f, &project, id), &auth, None),
            200,
            "show the track",
        );
        assert_eq!(&shown["track"], track, "{shown}");
    }
}

#[test]
fn a_track_needs_a_name() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = new_part(f, client, &project, "Wision");
    for body in [
        json!({ "part_id": part }),
        json!({ "description": "No name.", "part_id": part }),
        json!({ "name": "", "part_id": part }),
        json!({ "name": "   \n\t", "part_id": part }),
        json!({ "name": null, "part_id": part }),
        json!({ "name": 7, "part_id": part }),
    ] {
        let response = create_track_response(f, client, &project, &body);
        assert_error(&response, 400, "invalid_request");
    }
    let response = call(client, "POST", &tracks_path(f, &project), &agent(f), None);
    assert_eq!(response.status, 400, "no body: {}", response.text());
    assert!(
        list_tracks(f, client, &agent(f), &project).is_empty(),
        "a refused request creates nothing"
    );
    assert!(
        events_of(&project, "track.created").is_empty(),
        "a refused request emits nothing"
    );
}

#[test]
fn track_names_are_unique_in_a_project_ignoring_case() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let first = create_track(f, client, &project, "Checkout");
    let part = new_part(f, client, &project, "Wision");
    for name in ["checkout", "CHECKOUT", "Checkout"] {
        let response = create_track_response(
            f,
            client,
            &project,
            &json!({ "name": name, "part_id": part }),
        );
        assert_error(&response, 409, "name_taken");
    }
    let listed = list_tracks(f, client, &agent(f), &project);
    assert_eq!(listed, [first], "the refused names created nothing");
    assert_eq!(
        events_of(&project, "track.created").len(),
        1,
        "a refused request emits nothing"
    );

    // Another project of the workspace may use the name.
    let other = new_project(f, client);
    let reused = create_track(f, client, &other, "checkout");
    assert_eq!(reused["project_id"], other.as_str(), "{reused}");
}

#[test]
fn only_an_agent_creates_a_track() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = new_part(f, client, &project, "Wision");
    let response = call(
        client,
        "POST",
        &tracks_path(f, &project),
        &human(f),
        Some(&json!({ "name": "Checkout", "part_id": part })),
    );
    assert_error(&response, 403, "forbidden");
    assert_eq!(track_rows(&project), 0, "a refused request creates nothing");
}

// ---------------------------------------------------------------------------
// The part of a track (US-305)
// ---------------------------------------------------------------------------

#[test]
fn a_track_without_a_part_is_422_part_required_and_nothing_is_created() {
    let (f, client) = ready();
    let project = new_project(f, client);
    for body in [
        json!({ "name": "Long windows" }),
        json!({ "name": "Long windows", "part_id": null }),
    ] {
        let response = create_track_response(f, client, &project, &body);
        assert_error(&response, 422, "part_required");
    }
    assert_eq!(track_rows(&project), 0, "a refused request creates nothing");
}

#[test]
fn a_track_on_a_deleted_part_or_a_part_of_another_project_is_422_invalid_part() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let deleted = new_part(f, client, &project, "Old app");
    delete_part(f, client, &project, &deleted);
    let other_project = new_project(f, client);
    let theirs = new_part(f, client, &other_project, "Wision");
    for part in [deleted, theirs] {
        let response = create_track_response(
            f,
            client,
            &project,
            &json!({ "name": "Long windows", "part_id": part }),
        );
        assert_error(&response, 422, "invalid_part");
    }
    assert_eq!(track_rows(&project), 0, "a refused request creates nothing");
}

#[test]
fn a_part_id_that_names_no_part_is_422_invalid_part() {
    let (f, client) = ready();
    let project = new_project(f, client);
    for part in [json!(UNKNOWN_ID), json!("not-a-uuid"), json!(7)] {
        let response = create_track_response(
            f,
            client,
            &project,
            &json!({ "name": "Long windows", "part_id": part }),
        );
        assert_error(&response, 422, "invalid_part");
    }
    assert_eq!(track_rows(&project), 0, "a refused request creates nothing");
}

#[test]
fn moving_a_track_puts_it_on_the_other_part() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = new_part(f, client, &project, "Wision");
    let iot = new_part(f, client, &project, "iot-service");
    let track = must(
        create_track_response(
            f,
            client,
            &project,
            &json!({ "name": "Long windows", "part_id": wision }),
        ),
        201,
        "create a track",
    )["track"]["id"]
        .as_str()
        .expect("an id")
        .to_string();

    let moved = must(
        move_track_response(f, client, &project, &track, &json!({ "part_id": iot })),
        200,
        "move the track",
    );
    assert_eq!(moved["track"]["id"], track.as_str(), "{moved}");
    assert_eq!(moved["track"]["part_id"], iot.as_str(), "{moved}");
    assert_eq!(part_of(f, client, &project, &track), iot.as_str());
}

#[test]
fn moving_a_track_to_a_deleted_part_or_a_part_of_another_project_is_422_and_keeps_its_part() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = new_part(f, client, &project, "Wision");
    let deleted = new_part(f, client, &project, "Old app");
    delete_part(f, client, &project, &deleted);
    let other_project = new_project(f, client);
    let theirs = new_part(f, client, &other_project, "iot-service");
    let track = must(
        create_track_response(
            f,
            client,
            &project,
            &json!({ "name": "Long windows", "part_id": wision }),
        ),
        201,
        "create a track",
    )["track"]["id"]
        .as_str()
        .expect("an id")
        .to_string();

    for part in [deleted, theirs] {
        let response =
            move_track_response(f, client, &project, &track, &json!({ "part_id": part }));
        assert_error(&response, 422, "invalid_part");
        assert_eq!(
            part_of(f, client, &project, &track),
            wision.as_str(),
            "a refused move leaves the track on its part"
        );
    }
}

#[test]
fn moving_a_track_to_the_part_it_is_on_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = new_part(f, client, &project, "Wision");
    let created = must(
        create_track_response(
            f,
            client,
            &project,
            &json!({ "name": "Long windows", "part_id": wision }),
        ),
        201,
        "create a track",
    )["track"]
        .clone();
    let track = created["id"].as_str().expect("an id");

    let moved = must(
        move_track_response(f, client, &project, track, &json!({ "part_id": wision })),
        200,
        "move the track to its own part",
    );
    assert_eq!(moved["track"], created, "{moved}");
}

#[test]
fn a_move_without_a_part_is_422_part_required_and_keeps_its_part() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = new_part(f, client, &project, "Wision");
    let track = must(
        create_track_response(
            f,
            client,
            &project,
            &json!({ "name": "Long windows", "part_id": wision }),
        ),
        201,
        "create a track",
    )["track"]["id"]
        .as_str()
        .expect("an id")
        .to_string();

    for body in [json!({}), json!({ "part_id": null })] {
        let response = move_track_response(f, client, &project, &track, &body);
        assert_error(&response, 422, "part_required");
    }
    assert_eq!(part_of(f, client, &project, &track), wision.as_str());
}

// ---------------------------------------------------------------------------
// Reading tracks
// ---------------------------------------------------------------------------

#[test]
fn the_track_list_is_in_creation_order_for_the_agent_and_the_human() {
    let (f, client) = ready();
    let project = new_project(f, client);
    for auth in [agent(f), human(f)] {
        assert!(
            list_tracks(f, client, &auth, &project).is_empty(),
            "a new project has no tracks"
        );
    }
    let created: Vec<Value> = ["Zebra crossing", "Apple pay", "Mango theme"]
        .into_iter()
        .map(|name| create_track(f, client, &project, name))
        .collect();
    let other = new_project(f, client);
    create_track(f, client, &other, "Elsewhere");

    for auth in [agent(f), human(f)] {
        let listed = list_tracks(f, client, &auth, &project);
        assert_eq!(
            listed, created,
            "the project's tracks, in the order they were created"
        );
    }
}

#[test]
fn an_unknown_project_or_track_is_not_found() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let track = create_track(f, client, &project, "Checkout");
    let id = track["id"].as_str().expect("an id");
    let other = new_project(f, client);

    let response = create_track_response(f, client, UNKNOWN_ID, &json!({ "name": "Lost" }));
    assert_error(&response, 404, "not_found");
    for auth in [agent(f), human(f)] {
        for path in [
            tracks_path(f, UNKNOWN_ID),
            tracks_path(f, "not-a-uuid"),
            track_path(f, &project, UNKNOWN_ID),
            track_path(f, &project, "not-a-uuid"),
            track_path(f, UNKNOWN_ID, id),
            // A track is read through its own project only.
            track_path(f, &other, id),
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
fn the_tracks_table_keeps_its_rules_in_the_database() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = new_part(f, client, &project, "Wision");
    let workspace = f.workspace.as_str();

    let insert = "insert into tracks (workspace_id, project_id, name, part_id) \
                  values ($1::uuid, $2::uuid, $3, $4::uuid)";
    assert!(
        accepted(insert, &[workspace, &project, "Checkout", &part]),
        "a named track on a part of the row's own project is accepted"
    );
    for blank in ["", "   "] {
        assert!(
            !accepted(insert, &[workspace, &project, blank, &part]),
            "the blank name {blank:?} is refused"
        );
    }
    assert!(
        !accepted(insert, &[workspace, UNKNOWN_ID, "Nowhere", &part]),
        "a track needs a project"
    );
    assert!(
        !accepted(
            "insert into tracks (workspace_id, project_id, name) \
             values ($1::uuid, $2::uuid, $3)",
            &[workspace, &project, "Partless"],
        ),
        "a track needs a part"
    );
    let other_project = new_project(f, client);
    let theirs = new_part(f, client, &other_project, "Wision");
    assert!(
        !accepted(insert, &[workspace, &project, "Elsewhere", &theirs]),
        "a track's part is a part of its own project"
    );
}
