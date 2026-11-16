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
//! - `POST /tracks` (both, since US-306) takes `{"name", "description"}` and
//!   answers 201 `{"track": Track}`. `name` is required: missing, blank or
//!   not a string is 400 `{"error": "invalid_request"}` and creates nothing.
//!   `description` is optional and defaults to `""`. A session (the human)
//!   creates a track as an API key does.
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
//! - US-311: a Track carries its `state`, `planning` when it is new.
//!   `POST /tracks/{track_id}/state` (agent) takes `{"state"}` and answers
//!   200 `{"track": Track}` for the five moves planning to building,
//!   building to planning, building to done, planning to dropped and building
//!   to dropped, and emits `track.state_changed`. Any other move, one to the
//!   state the track already has included, is 422 `{"error":
//!   "invalid_transition", "state": <current state>, ...}` and leaves the
//!   track where it is.
//! - US-313: a Track carries `owner_user_id`, the user of the member who
//!   created it: the session's member, or for an API key the member who
//!   created the key. `GET /tracks?owner=me` lists only the caller's tracks;
//!   any other `owner` is 422 `{"error": "invalid_owner"}`, checked before
//!   the project. No create or edit body sets the owner.
//! - US-314: the owner, with a session or any key the owner made, makes
//!   every state move. An admin who is not the owner may move planning and
//!   building both ways and drop; done, and any move by another member, is
//!   403 `{"error": "not_track_owner"}`, checked before the move itself.
//! - US-315: `POST /tracks/{track_id}/owner` takes `{"membership_id"}`. The
//!   owner or an admin hands the track to that member and gets 200 `{"track":
//!   Track}` with the new `owner_user_id`; anyone else is 403 `{"error":
//!   "not_track_owner"}`. From then on only the new owner's session and keys
//!   write. An ID that is no membership of the workspace is 422 `{"error":
//!   "not_a_member"}`; refused hand-overs keep the owner and emit nothing.
//! - US-320: `POST /tracks` with `split_from_track_id` splits a track off:
//!   the new track is planning, on the part it names, owned by the caller and
//!   carries `split_from_track_id`. Only the owner of that track may split
//!   from it; anyone else, an admin included, is 403 `{"error":
//!   "not_track_owner"}` and creates nothing. `POST
//!   /tracks/{track_id}/stories/{story_id}/track` takes `{expected_version,
//!   to_track_id}` and moves the story to another track of the project; it
//!   keeps its number, fields and blockers. A track of another project is 422
//!   `{"error": "invalid_track"}`; a caller who does not own both tracks is
//!   403 `{"error": "not_track_owner"}`; a refused move changes nothing.

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
    alice_user: String,
    /// A key that alice minted with her session.
    key: String,
    /// Another key that alice minted.
    alice_second_key: String,
    /// bob, a second member of the workspace, with a key he minted.
    bob_token: String,
    bob_user: String,
    bob_key: String,
    /// carol, an admin of the workspace, with a key she minted.
    carol_token: String,
    carol_key: String,
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
        let (workspace, users) = block(async {
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
            let mut users = Vec::new();
            for (name, role) in [("alice", "member"), ("bob", "member"), ("carol", "admin")] {
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
                    "insert into memberships (workspace_id, user_id, role) \
                     values ($1::uuid, $2::uuid, $3) returning id::text",
                    &[&workspace, &user, role],
                )
                .await;
                users.push(user);
            }
            (workspace, users)
        });
        let server = TestServer::start(&config());
        let mint = |name: &str| -> String {
            let response = call(
                &server.client,
                "POST",
                &format!("/api/v1/w/{workspace}/api-keys"),
                &Auth::Session(&session_token(name)),
                Some(&json!({})),
            );
            assert_eq!(response.status, 201, "mint a key: {}", response.text());
            response.json()["secret"]
                .as_str()
                .expect("the new key has a secret")
                .to_string()
        };
        let key = mint("alice");
        let bob_key = mint("bob");
        let alice_second_key = mint("alice");
        let carol_key = mint("carol");
        Fixture {
            workspace,
            alice_token: session_token("alice"),
            alice_user: users[0].clone(),
            key,
            alice_second_key,
            bob_token: session_token("bob"),
            bob_user: users[1].clone(),
            bob_key,
            carol_token: session_token("carol"),
            carol_key,
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
fn a_session_creates_a_track_on_the_part_it_names() {
    // US-306: the web creates a track with the signed-in member's session.
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = new_part(f, client, &project, "Wision");
    let created = must(
        call(
            client,
            "POST",
            &tracks_path(f, &project),
            &human(f),
            Some(&json!({ "name": "Checkout", "part_id": part })),
        ),
        201,
        "create a track with a session",
    )["track"]
        .clone();
    assert_eq!(created["name"], "Checkout");
    assert_eq!(created["part_id"], part.as_str());
    assert_eq!(created["state"], "planning");
    assert_eq!(track_rows(&project), 1);
    let listed = list_tracks(f, client, &human(f), &project);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], created["id"]);
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

// ---------------------------------------------------------------------------
// Track states (US-311)
// ---------------------------------------------------------------------------

fn set_state_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    track: &str,
    to: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/state", track_path(f, project, track)),
        &agent(f),
        Some(&json!({ "state": to })),
    )
}

/// Moves a track to `to`, which must be allowed, and checks the answer.
fn set_state(f: &Fixture, client: &Client, project: &str, track: &str, to: &str) {
    let moved = must(
        set_state_response(f, client, project, track, to),
        200,
        &format!("move the track to {to}"),
    );
    assert_eq!(moved["track"]["id"], track, "{moved}");
    assert_eq!(moved["track"]["state"], to, "{moved}");
}

/// The `state` of a track, as the API shows it.
fn state_of(f: &Fixture, client: &Client, project: &str, track: &str) -> Value {
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
    )["track"]["state"]
        .clone()
}

/// Creates a track and walks it through `moves`, each one allowed.
fn track_in(f: &Fixture, client: &Client, project: &str, moves: &[&str]) -> String {
    let track = create_track(f, client, project, &unique("track"));
    let id = track["id"].as_str().expect("an id").to_string();
    for to in moves {
        set_state(f, client, project, &id, to);
    }
    id
}

#[test]
fn a_new_track_is_planning_and_makes_the_five_allowed_moves() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let track = create_track(f, client, &project, "Long windows");
    assert_eq!(
        track["state"], "planning",
        "a new track is planning: {track}"
    );
    let id = track["id"].as_str().expect("an id");

    // planning to building, back to planning, to building again, to done.
    for to in ["building", "planning", "building", "done"] {
        set_state(f, client, &project, id, to);
        assert_eq!(state_of(f, client, &project, id), to);
    }
    // planning to dropped, and building to dropped.
    for moves in [&["dropped"][..], &["building", "dropped"][..]] {
        let other = track_in(f, client, &project, moves);
        assert_eq!(state_of(f, client, &project, &other), "dropped");
    }

    let changes: Vec<Value> = events_of(&project, "track.state_changed")
        .into_iter()
        .filter(|data| data["track_id"] == id)
        .collect();
    assert_eq!(
        changes
            .iter()
            .map(|data| data["state"].clone())
            .collect::<Vec<_>>(),
        ["building", "planning", "building", "done"],
        "each move emits track.state_changed with the new state: {changes:?}"
    );
}

#[test]
fn a_move_that_is_not_allowed_is_422_invalid_transition_with_the_current_state() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let cases: [(&[&str], &str, &[&str]); 4] = [
        (&[], "planning", &["done", "planning"]),
        (&["building"], "building", &["building"]),
        (
            &["building", "done"],
            "done",
            &["building", "planning", "dropped", "done"],
        ),
        (
            &["dropped"],
            "dropped",
            &["planning", "building", "done", "dropped"],
        ),
    ];
    for (moves, current, refused) in cases {
        let track = track_in(f, client, &project, moves);
        for to in refused {
            let response = set_state_response(f, client, &project, &track, to);
            assert_eq!(
                response.status,
                422,
                "{current} to {to}: {}",
                response.text()
            );
            let body = response.json();
            assert_eq!(
                body["error"], "invalid_transition",
                "{current} to {to}: {body}"
            );
            assert_eq!(body["state"], current, "{current} to {to}: {body}");
            assert_eq!(
                state_of(f, client, &project, &track),
                current,
                "a refused move to {to} leaves the track in {current}"
            );
        }
    }
}

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

// ---------------------------------------------------------------------------
// The owner of a track (US-313)
// ---------------------------------------------------------------------------

/// Creates a track with `auth`; returns the `track` of the answer.
fn create_track_as(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    body: &Value,
) -> Value {
    must(
        call(client, "POST", &tracks_path(f, project), auth, Some(body)),
        201,
        "create a track",
    )["track"]
        .clone()
}

/// The membership ID of a user in the fixture's workspace.
fn membership_of(f: &Fixture, user: &str) -> String {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar(
            "select id::text from memberships where workspace_id = $1::uuid and user_id = $2::uuid",
        )
        .bind(&f.workspace)
        .bind(user)
        .fetch_one(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the membership: {e}"))
    })
}

fn ids(tracks: &[Value]) -> Vec<String> {
    tracks
        .iter()
        .map(|t| t["id"].as_str().expect("an id").to_string())
        .collect()
}

#[test]
fn a_track_is_owned_by_the_sessions_member_or_by_the_member_who_created_the_key() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = new_part(f, client, &project, "Wision");
    for (auth, owner, who) in [
        (
            Auth::Session(&f.alice_token),
            &f.alice_user,
            "alice's session",
        ),
        (Auth::Agent(&f.bob_key), &f.bob_user, "bob's key"),
        (Auth::Session(&f.bob_token), &f.bob_user, "bob's session"),
        (Auth::Agent(&f.key), &f.alice_user, "alice's key"),
    ] {
        let track = create_track_as(
            f,
            client,
            &auth,
            &project,
            &json!({ "name": unique("owned"), "part_id": part }),
        );
        assert_eq!(
            track["owner_user_id"],
            owner.as_str(),
            "a track created with {who} is owned by that member: {track}"
        );
        let id = track["id"].as_str().expect("an id");
        let shown = must(
            call(client, "GET", &track_path(f, &project, id), &human(f), None),
            200,
            "show the track",
        );
        assert_eq!(shown["track"]["owner_user_id"], owner.as_str(), "{shown}");
    }
}

#[test]
fn owner_me_lists_the_callers_tracks_and_any_other_owner_is_422_invalid_owner() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = new_part(f, client, &project, "Wision");
    let create = |auth: Auth| {
        create_track_as(
            f,
            client,
            &auth,
            &project,
            &json!({ "name": unique("mine"), "part_id": part }),
        )
    };
    let alices = vec![
        create(Auth::Session(&f.alice_token)),
        create(Auth::Agent(&f.key)),
    ];
    let bobs = vec![
        create(Auth::Agent(&f.bob_key)),
        create(Auth::Session(&f.bob_token)),
    ];
    let mine = format!("{}?owner=me", tracks_path(f, &project));
    for (auth, expected, who) in [
        (Auth::Session(&f.alice_token), &alices, "alice's session"),
        (Auth::Agent(&f.key), &alices, "alice's key"),
        (Auth::Agent(&f.bob_key), &bobs, "bob's key"),
        (Auth::Session(&f.bob_token), &bobs, "bob's session"),
    ] {
        let listed = must(call(client, "GET", &mine, &auth, None), 200, "?owner=me");
        let listed = listed["tracks"].as_array().expect("`tracks` is a list");
        assert_eq!(
            ids(listed),
            ids(expected),
            "?owner=me with {who} lists every track its member owns and no other"
        );
    }

    for owner in ["bob", "", "ME", f.bob_user.as_str()] {
        let response = call(
            client,
            "GET",
            &format!("{}?owner={owner}", tracks_path(f, &project)),
            &agent(f),
            None,
        );
        assert_error(&response, 422, "invalid_owner");
    }
    // The owner is checked before the project.
    let response = call(
        client,
        "GET",
        &format!("{}?owner=bob", tracks_path(f, UNKNOWN_ID)),
        &agent(f),
        None,
    );
    assert_error(&response, 422, "invalid_owner");
}

#[test]
fn edits_to_a_track_never_change_its_owner() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = new_part(f, client, &project, "Wision");
    let other_part = new_part(f, client, &project, "iot-service");
    let takeover = json!({
        "owner_user_id": f.alice_user,
        "owner_membership_id": membership_of(f, &f.alice_user),
        "owner": f.alice_user,
    });
    let bob = Auth::Agent(&f.bob_key);

    // A create body that names another owner is owned by the caller anyway.
    let mut body = takeover.clone();
    body["name"] = json!("Long windows");
    body["part_id"] = json!(part);
    let track = create_track_as(f, client, &bob, &project, &body);
    assert_eq!(track["owner_user_id"], f.bob_user.as_str(), "{track}");
    let path = track_path(f, &project, track["id"].as_str().expect("an id"));

    for (method, route, fields) in [
        ("PATCH", path.clone(), json!({ "name": "Longer windows" })),
        (
            "POST",
            format!("{path}/move"),
            json!({ "part_id": other_part }),
        ),
        (
            "POST",
            format!("{path}/state"),
            json!({ "state": "building" }),
        ),
    ] {
        let mut body = takeover.clone();
        for (name, value) in fields.as_object().expect("an object") {
            body[name] = value.clone();
        }
        let edited = must(
            call(client, method, &route, &bob, Some(&body)),
            200,
            &format!("{method} {route}"),
        );
        assert_eq!(
            edited["track"]["owner_user_id"],
            f.bob_user.as_str(),
            "{method} {route} keeps the owner: {edited}"
        );
        let shown = must(
            call(client, "GET", &path, &human(f), None),
            200,
            "show the track",
        );
        assert_eq!(
            shown["track"]["owner_user_id"],
            f.bob_user.as_str(),
            "{shown}"
        );
    }
    let alices = must(
        call(
            client,
            "GET",
            &format!("{}?owner=me", tracks_path(f, &project)),
            &human(f),
            None,
        ),
        200,
        "?owner=me",
    );
    assert_eq!(alices["tracks"], json!([]), "alice owns none: {alices}");
}

// ---------------------------------------------------------------------------
// Only the owner changes a track (US-314)
// ---------------------------------------------------------------------------

fn state_as(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    track: &str,
    to: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/state", track_path(f, project, track)),
        auth,
        Some(&json!({ "state": to })),
    )
}

#[test]
fn the_owner_makes_every_state_move_with_a_session_or_any_key_they_created() {
    let (f, client) = ready();
    let project = new_project(f, client);
    for (auth, who) in [
        (human(f), "alice's session"),
        (agent(f), "alice's key"),
        (Auth::Agent(&f.alice_second_key), "alice's second key"),
    ] {
        for moves in [
            &["building", "planning", "building", "done"][..],
            &["dropped"][..],
            &["building", "dropped"][..],
        ] {
            // Made with alice's key, so alice owns the track.
            let track = track_in(f, client, &project, &[]);
            for to in moves {
                let moved = must(
                    state_as(f, client, &auth, &project, &track, to),
                    200,
                    &format!("the owner moves the track to {to} with {who}"),
                );
                assert_eq!(moved["track"]["state"], *to, "{moved}");
            }
        }
    }
}

#[test]
fn an_admin_who_is_not_the_owner_moves_and_drops_a_track_but_cannot_mark_it_done() {
    let (f, client) = ready();
    let project = new_project(f, client);
    for (auth, who) in [
        (Auth::Session(&f.carol_token), "carol's session"),
        (Auth::Agent(&f.carol_key), "carol's key"),
    ] {
        // Made with alice's key, so alice owns the track.
        let track = track_in(f, client, &project, &[]);
        for to in ["building", "planning", "building", "dropped"] {
            let moved = must(
                state_as(f, client, &auth, &project, &track, to),
                200,
                &format!("the admin moves the track to {to} with {who}"),
            );
            assert_eq!(moved["track"]["state"], to, "{moved}");
        }

        // Done is refused for the admin, also from a state done is no move from.
        for (moves, current) in [(&["building"][..], "building"), (&[][..], "planning")] {
            let track = track_in(f, client, &project, moves);
            let response = state_as(f, client, &auth, &project, &track, "done");
            assert_error(&response, 403, "not_track_owner");
            assert_eq!(state_of(f, client, &project, &track), current, "{who}");
        }
    }

    // A member who is neither the owner nor an admin makes no move at all.
    let track = track_in(f, client, &project, &[]);
    for auth in [Auth::Session(&f.bob_token), Auth::Agent(&f.bob_key)] {
        for to in ["building", "dropped"] {
            let response = state_as(f, client, &auth, &project, &track, to);
            assert_error(&response, 403, "not_track_owner");
        }
    }
    assert_eq!(state_of(f, client, &project, &track), "planning");
}

// ---------------------------------------------------------------------------
// Hand a track over (US-315)
// ---------------------------------------------------------------------------

fn hand_over(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    track: &str,
    membership: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/owner", track_path(f, project, track)),
        auth,
        Some(&json!({ "membership_id": membership })),
    )
}

/// The `owner_user_id` of a track, as the API shows it.
fn owner_of(f: &Fixture, client: &Client, project: &str, track: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &track_path(f, project, track),
            &human(f),
            None,
        ),
        200,
        "show the track",
    )["track"]["owner_user_id"]
        .clone()
}

#[test]
fn the_owner_or_an_admin_hands_a_track_over_and_anyone_else_is_403_not_track_owner() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let alice = membership_of(f, &f.alice_user);
    let bob = membership_of(f, &f.bob_user);

    // Made with alice's key, so alice owns the track.
    let track = track_in(f, client, &project, &[]);
    for auth in [Auth::Session(&f.bob_token), Auth::Agent(&f.bob_key)] {
        let response = hand_over(f, client, &auth, &project, &track, &bob);
        assert_error(&response, 403, "not_track_owner");
    }
    assert_eq!(owner_of(f, client, &project, &track), f.alice_user.as_str());
    assert!(events_of(&project, "track.owner_changed").is_empty());

    for (auth, to, user, who) in [
        (human(f), &bob, &f.bob_user, "the owner's session"),
        (
            Auth::Agent(&f.carol_key),
            &alice,
            &f.alice_user,
            "an admin's key",
        ),
        (agent(f), &bob, &f.bob_user, "the owner's key"),
        (
            Auth::Session(&f.carol_token),
            &alice,
            &f.alice_user,
            "an admin's session",
        ),
    ] {
        let handed = must(
            hand_over(f, client, &auth, &project, &track, to),
            200,
            &format!("hand the track over with {who}"),
        );
        assert_eq!(handed["track"]["id"], track.as_str(), "{handed}");
        assert_eq!(handed["track"]["owner_user_id"], user.as_str(), "{handed}");
        assert_eq!(
            owner_of(f, client, &project, &track),
            user.as_str(),
            "{who}"
        );
    }
    assert_eq!(events_of(&project, "track.owner_changed").len(), 4);
}

#[test]
fn after_a_hand_over_the_old_owners_keys_cannot_write_to_the_track_and_the_new_owners_can() {
    let (f, client) = ready();
    let project = new_project(f, client);
    // Made with alice's key, so alice owns the track.
    let track = track_in(f, client, &project, &[]);
    must(
        hand_over(
            f,
            client,
            &agent(f),
            &project,
            &track,
            &membership_of(f, &f.bob_user),
        ),
        200,
        "alice hands the track to bob",
    );

    for auth in [
        agent(f),
        Auth::Agent(&f.alice_second_key),
        Auth::Session(&f.alice_token),
    ] {
        let response = state_as(f, client, &auth, &project, &track, "building");
        assert_error(&response, 403, "not_track_owner");
        let response = hand_over(
            f,
            client,
            &auth,
            &project,
            &track,
            &membership_of(f, &f.alice_user),
        );
        assert_error(&response, 403, "not_track_owner");
    }
    assert_eq!(state_of(f, client, &project, &track), "planning");
    assert_eq!(owner_of(f, client, &project, &track), f.bob_user.as_str());

    for (auth, to) in [
        (Auth::Agent(&f.bob_key), "building"),
        (Auth::Session(&f.bob_token), "done"),
    ] {
        let moved = must(
            state_as(f, client, &auth, &project, &track, to),
            200,
            &format!("the new owner moves the track to {to}"),
        );
        assert_eq!(moved["track"]["state"], to, "{moved}");
    }
}

#[test]
fn handing_a_track_to_someone_who_is_not_a_member_of_the_workspace_is_422_not_a_member() {
    let (f, client) = ready();
    let project = new_project(f, client);
    // A member of another workspace, who has no membership in this one.
    let stranger = block(async {
        let mut conn = admin_connection().await;
        let workspace: String = sqlx::query_scalar(
            "insert into workspaces (name) values ('Elsewhere') returning id::text",
        )
        .fetch_one(&mut conn)
        .await
        .expect("seed a workspace");
        let user: String =
            sqlx::query_scalar("insert into users (email) values ($1) returning id::text")
                .bind(format!("{}@example.test", unique("stranger")))
                .fetch_one(&mut conn)
                .await
                .expect("seed a user");
        sqlx::query_scalar::<_, String>(
            "insert into memberships (workspace_id, user_id) \
             values ($1::uuid, $2::uuid) returning id::text",
        )
        .bind(&workspace)
        .bind(&user)
        .fetch_one(&mut conn)
        .await
        .expect("seed a membership")
    });

    // Made with alice's key, so alice owns the track.
    let track = track_in(f, client, &project, &[]);
    for (membership, what) in [
        (stranger.as_str(), "another workspace's membership"),
        (UNKNOWN_ID, "an unknown membership"),
        (
            f.bob_user.as_str(),
            "a member's user id, not a membership id",
        ),
    ] {
        let response = hand_over(f, client, &agent(f), &project, &track, membership);
        assert_error(&response, 422, "not_a_member");
        assert_eq!(
            owner_of(f, client, &project, &track),
            f.alice_user.as_str(),
            "{what}: the owner stays"
        );
    }
    assert!(events_of(&project, "track.owner_changed").is_empty());
}

// ---------------------------------------------------------------------------
// Split a track off and move stories to it (US-320)
// ---------------------------------------------------------------------------

/// The tracks `GET /tracks` lists for the project.
fn listed_tracks(f: &Fixture, client: &Client, project: &str) -> Vec<Value> {
    must(
        call(client, "GET", &tracks_path(f, project), &human(f), None),
        200,
        "list the tracks",
    )["tracks"]
        .as_array()
        .expect("`tracks` is a list")
        .clone()
}

#[test]
fn splitting_a_track_off_makes_a_planning_track_owned_by_the_caller_and_needs_the_from_track() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let wision = new_part(f, client, &project, "Wision");
    let iot = new_part(f, client, &project, "iot-service");
    // Made with bob's key, so bob owns the track that is split from.
    let long = create_track_as(
        f,
        client,
        &Auth::Agent(&f.bob_key),
        &project,
        &json!({ "name": "Long windows", "part_id": wision }),
    );
    let long = long["id"].as_str().expect("an id").to_string();
    let split = json!({
        "name": "Aggregated readings",
        "part_id": iot,
        "split_from_track_id": long,
    });

    for (auth, who) in [
        (agent(f), "alice's key"),
        (human(f), "alice's session"),
        (Auth::Agent(&f.carol_key), "an admin's key"),
        (Auth::Session(&f.carol_token), "an admin's session"),
    ] {
        let response = call(
            client,
            "POST",
            &tracks_path(f, &project),
            &auth,
            Some(&split),
        );
        assert_error(&response, 403, "not_track_owner");
        assert_eq!(
            ids(&listed_tracks(f, client, &project)),
            vec![long.clone()],
            "a split refused for {who} creates nothing"
        );
    }

    let made = create_track_as(f, client, &Auth::Session(&f.bob_token), &project, &split);
    assert_eq!(made["name"], "Aggregated readings", "{made}");
    assert_eq!(made["part_id"], iot.as_str(), "{made}");
    assert_eq!(made["state"], "planning", "{made}");
    assert_eq!(made["owner_user_id"], f.bob_user.as_str(), "{made}");
    assert_eq!(made["split_from_track_id"], long.as_str(), "{made}");
    let made = made["id"].as_str().expect("an id").to_string();
    let shown = must(
        call(
            client,
            "GET",
            &track_path(f, &project, &made),
            &human(f),
            None,
        ),
        200,
        "show the split-off track",
    );
    assert_eq!(
        shown["track"]["split_from_track_id"],
        long.as_str(),
        "{shown}"
    );
    let off_long: Vec<Value> = listed_tracks(f, client, &project)
        .into_iter()
        .filter(|track| track["split_from_track_id"] == long.as_str())
        .collect();
    assert_eq!(
        ids(&off_long),
        vec![made],
        "the list shows the one track split off from Long windows"
    );
}

fn stories_path(f: &Fixture, project: &str, track: &str) -> String {
    format!("{}/stories", track_path(f, project, track))
}

/// Creates a story on the track with alice's key; returns the `story`.
fn create_story(f: &Fixture, client: &Client, project: &str, track: &str, extra: Value) -> Value {
    let mut body = json!({ "title": unique("story"), "text": "", "topic": "", "criteria": [] });
    for (name, value) in extra.as_object().expect("an object") {
        body[name] = value.clone();
    }
    must(
        call(
            client,
            "POST",
            &stories_path(f, project, track),
            &agent(f),
            Some(&body),
        ),
        201,
        "create a story",
    )["story"]
        .clone()
}

fn get_story(f: &Fixture, client: &Client, project: &str, track: &str, story: &str) -> Response {
    call(
        client,
        "GET",
        &format!("{}/{story}", stories_path(f, project, track)),
        &agent(f),
        None,
    )
}

fn move_story_to(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    track: &str,
    story: &Value,
    to: &str,
) -> Response {
    let id = story["id"].as_str().expect("an id");
    call(
        client,
        "POST",
        &format!("{}/{id}/track", stories_path(f, project, track)),
        auth,
        Some(&json!({ "expected_version": story["version"], "to_track_id": to })),
    )
}

/// The keys `GET /stories?<query>` lists for the track.
fn story_keys(f: &Fixture, client: &Client, project: &str, track: &str, query: &str) -> Vec<Value> {
    must(
        call(
            client,
            "GET",
            &format!("{}?{query}", stories_path(f, project, track)),
            &agent(f),
            None,
        ),
        200,
        "list the stories",
    )["stories"]
        .as_array()
        .expect("`stories` is a list")
        .iter()
        .map(|story| story["key"].clone())
        .collect()
}

/// Marks a story done as the admin role.
fn finish_story(story: &Value) {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query("update stories set status = 'done' where id = $1::uuid")
            .bind(story["id"].as_str().expect("an id"))
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("finish the story: {e}"));
    })
}

/// A story without the fields a move to another track may change.
fn unmoved_fields(story: &Value) -> Value {
    let mut story = story.clone();
    for field in ["track_id", "version", "updated_at", "blockers"] {
        story.as_object_mut().expect("an object").remove(field);
    }
    story
}

#[test]
fn a_story_moved_to_a_split_off_track_keeps_its_number_fields_and_blockers() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let long = create_track(f, client, &project, "Long windows");
    let long = long["id"].as_str().expect("an id").to_string();
    let iot = new_part(f, client, &project, "iot-service");
    let aggregated = create_track_as(
        f,
        client,
        &agent(f),
        &project,
        &json!({ "name": "Aggregated readings", "part_id": iot, "split_from_track_id": long }),
    );
    let aggregated = aggregated["id"].as_str().expect("an id").to_string();

    let first = create_story(f, client, &project, &long, json!({}));
    let moved = create_story(
        f,
        client,
        &project,
        &long,
        json!({
            "blocked_by": [first["id"]],
            "priority": 1,
            "topic": "api",
            "text": "Aggregate the readings.",
            "criteria": [{ "text": "It aggregates" }],
        }),
    );
    let dependant = create_story(
        f,
        client,
        &project,
        &long,
        json!({ "blocked_by": [moved["id"]] }),
    );
    let id = moved["id"].as_str().expect("an id").to_string();
    let before = must(
        get_story(f, client, &project, &long, &id),
        200,
        "read the story",
    )["story"]
        .clone();

    let after = must(
        move_story_to(f, client, &agent(f), &project, &long, &before, &aggregated),
        200,
        "move the story to the split-off track",
    )["story"]
        .clone();
    assert_eq!(after["key"], "US-2", "{after}");
    assert_eq!(after["track_id"], aggregated.as_str(), "{after}");
    assert_eq!(after["blocked_by"], json!([first["id"]]), "{after}");
    assert_eq!(
        unmoved_fields(&after),
        unmoved_fields(&before),
        "only the track changes"
    );
    let read = must(
        get_story(f, client, &project, &aggregated, &id),
        200,
        "read the story on its new track",
    )["story"]
        .clone();
    assert_eq!(unmoved_fields(&read), unmoved_fields(&before), "{read}");
    assert_eq!(
        get_story(f, client, &project, &long, &id).status,
        404,
        "the story has left its old track"
    );
    assert_eq!(
        story_keys(f, client, &project, &long, "pickable=false"),
        vec![json!("US-1"), json!("US-3")]
    );
    assert_eq!(
        story_keys(f, client, &project, &aggregated, "pickable=false"),
        vec![json!("US-2")]
    );
    let changed: Vec<Value> = events_of(&project, "story.changed")
        .into_iter()
        .filter(|event| event["story_id"] == id.as_str())
        .collect();
    assert_eq!(changed.len(), 1, "the move is recorded once: {changed:?}");
    assert_eq!(changed[0]["key"], "US-2", "{changed:?}");

    // The blocker it has and the blocker it is still hold across the tracks.
    let dependant_now = must(
        get_story(
            f,
            client,
            &project,
            &long,
            dependant["id"].as_str().unwrap(),
        ),
        200,
        "read the dependant",
    );
    assert_eq!(
        dependant_now["story"]["blocked_by"],
        json!([id]),
        "{dependant_now}"
    );
    assert_eq!(
        story_keys(f, client, &project, &aggregated, "pickable=true"),
        Vec::<Value>::new(),
        "US-2 waits for US-1 on Long windows"
    );
    assert_eq!(
        story_keys(f, client, &project, &long, "pickable=true"),
        vec![json!("US-1")],
        "US-3 waits for US-2 on Aggregated readings"
    );
    finish_story(&first);
    assert_eq!(
        story_keys(f, client, &project, &aggregated, "pickable=true"),
        vec![json!("US-2")],
        "US-1 done releases US-2"
    );
    finish_story(&after);
    assert_eq!(
        story_keys(f, client, &project, &long, "pickable=true"),
        vec![json!("US-3")],
        "US-2 done releases US-3"
    );
}

#[test]
fn a_story_moves_only_within_its_project_and_only_for_the_owner_of_both_tracks() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = new_part(f, client, &project, "Wision");
    let make = |auth: Auth| {
        let track = create_track_as(
            f,
            client,
            &auth,
            &project,
            &json!({ "name": unique("track"), "part_id": part }),
        );
        track["id"].as_str().expect("an id").to_string()
    };
    // alice owns the source and `alices`; bob owns `bobs`.
    let source = make(agent(f));
    let alices = make(agent(f));
    let bobs = make(Auth::Agent(&f.bob_key));
    let elsewhere = track_in(f, client, &new_project(f, client), &[]);
    let story = create_story(f, client, &project, &source, json!({}));
    let id = story["id"].as_str().expect("an id").to_string();

    let refused = [
        (
            agent(f),
            &elsewhere,
            422,
            "invalid_track",
            "a track of another project",
        ),
        (
            agent(f),
            &bobs,
            403,
            "not_track_owner",
            "a target alice does not own",
        ),
        (
            human(f),
            &bobs,
            403,
            "not_track_owner",
            "a target alice does not own",
        ),
        (
            Auth::Agent(&f.bob_key),
            &bobs,
            403,
            "not_track_owner",
            "bob, not owning the source",
        ),
        (
            Auth::Session(&f.bob_token),
            &bobs,
            403,
            "not_track_owner",
            "bob, not owning the source",
        ),
        (
            Auth::Agent(&f.carol_key),
            &alices,
            403,
            "not_track_owner",
            "an admin owning neither",
        ),
        (
            Auth::Session(&f.carol_token),
            &alices,
            403,
            "not_track_owner",
            "an admin owning neither",
        ),
    ];
    for (auth, to, status, code, what) in refused {
        let response = move_story_to(f, client, &auth, &project, &source, &story, to);
        assert_eq!(response.status, status, "{what}: {}", response.text());
        assert_error(&response, status, code);
        let still = must(
            get_story(f, client, &project, &source, &id),
            200,
            &format!("{what}: the story stays on its track"),
        );
        assert_eq!(
            still["story"]["version"], story["version"],
            "{what}: {still}"
        );
    }
    assert!(
        events_of(&project, "story.changed").is_empty(),
        "a refused move emits nothing"
    );

    let moved = must(
        move_story_to(f, client, &human(f), &project, &source, &story, &alices),
        200,
        "alice's session moves the story between her tracks",
    );
    assert_eq!(moved["story"]["track_id"], alices.as_str(), "{moved}");
}
