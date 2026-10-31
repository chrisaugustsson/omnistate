//! US-131: stories as records, the work items of a feature.
//! Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API and serve the
//! API's router on a local port against that database.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}`):
//!
//! - `POST /stories` (agent and human) answers 201 `{"story": Story}`. The key
//!   is `US-<number>`, counted per project: the first story is `US-1`, or
//!   `US-<number_from + 1>` when the body carries `number_from`; every later
//!   story gets the next number whatever `number_from` says. A new story is in
//!   `backlog`, at `version` 1, not blocked, with priority 3 unless the body
//!   names one, and each criterion `{text, ticked: false, human_check, pending:
//!   false}`.
//! - 422 `{"error": "invalid_<field>"}`: a title that is blank after trimming,
//!   a priority outside 0 to 3, a link that is neither `{doc_id, node_id,
//!   label}` nor `{thread_id, label}`, a `blocked_by` id that is not a story
//!   of the same feature, a negative `number_from`. Duplicate `blocked_by` ids
//!   are kept once, in first-seen order.
//! - `GET /stories` answers `{"stories": [Story, ...]}`, every status when
//!   unfiltered, by priority and then number; `?status=<status>` filters and
//!   an unknown status is 422. `GET /stories/{story_id}` is 404 for a story
//!   of another feature or project.
//! - Creating a story emits `story.created` with `{story_id, key, feature_id}`.

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

const TEST_DB: &str = "omni_us131_stories";
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
    workspace: String,
    alice_token: String,
    key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us131-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    let created = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.workspace),
            &agent(f),
            Some(&json!({ "name": unique("story-project") })),
        ),
        201,
        "create a project",
    );
    created["project"]["id"]
        .as_str()
        .expect("the project has an id")
        .to_string()
}

fn new_feature(f: &Fixture, client: &Client, project: &str) -> String {
    let created = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects/{project}/features", f.workspace),
            &agent(f),
            Some(&json!({ "name": unique("story-feature") })),
        ),
        201,
        "create a feature",
    );
    created["feature"]["id"]
        .as_str()
        .expect("the feature has an id")
        .to_string()
}

fn stories_path(f: &Fixture, project: &str, feature: &str) -> String {
    format!(
        "/api/v1/w/{}/projects/{project}/features/{feature}/stories",
        f.workspace
    )
}

/// A body with every required field, the title given.
fn body(title: &str) -> Value {
    json!({ "title": title, "text": "", "topic": "", "criteria": [] })
}

/// `body(title)` with `extra`'s fields added.
fn body_with(title: &str, extra: Value) -> Value {
    let mut body = body(title);
    for (name, value) in extra.as_object().expect("an object") {
        body[name] = value.clone();
    }
    body
}

fn post_story(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &stories_path(f, project, feature),
        &agent(f),
        Some(body),
    )
}

/// Creates a story as the agent; returns the `story` of the answer.
fn create_story(f: &Fixture, client: &Client, project: &str, feature: &str, body: &Value) -> Value {
    must(
        post_story(f, client, project, feature, body),
        201,
        "create a story",
    )["story"]
        .clone()
}

fn id_of(story: &Value) -> String {
    story["id"]
        .as_str()
        .expect("the story has an id")
        .to_string()
}

fn keys_of(listed: &Value) -> Vec<Value> {
    listed["stories"]
        .as_array()
        .unwrap_or_else(|| panic!("`stories` is a list: {listed}"))
        .iter()
        .map(|story| story["key"].clone())
        .collect()
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

/// How many rows the `stories` table has for a project.
fn story_rows(project: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from stories where project_id = $1::uuid")
            .bind(project)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the stories: {e}"))
    })
}

/// Sets a story's status as the admin role: no route of this story moves one.
fn set_status(story: &str, status: &str) {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query("update stories set status = $2 where id = $1::uuid")
            .bind(story)
            .bind(status)
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("set the status: {e}"));
    })
}

// ---------------------------------------------------------------------------
// Creating a story
// ---------------------------------------------------------------------------

#[test]
fn the_first_story_of_a_project_is_us_1_in_backlog_at_version_1() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = create_story(
        f,
        client,
        &project,
        &feature,
        &json!({
            "title": "Stories as records",
            "text": "As an agent, I want to save a story.",
            "topic": "feature · api",
            "criteria": [{ "text": "a" }, { "text": "b", "human_check": true }],
            "priority": 2,
            "links": [{ "doc_id": UNKNOWN_ID, "node_id": "n1", "label": "design" }],
        }),
    );
    assert_eq!(story["key"], "US-1", "{story}");
    assert_eq!(story["status"], "backlog", "{story}");
    assert_eq!(story["version"], 1, "{story}");
    assert_eq!(story["blocked"], false, "{story}");
    assert_eq!(story["priority"], 2, "{story}");
    assert_eq!(
        story["criteria"],
        json!([
            { "text": "a", "ticked": false, "human_check": false, "pending": false },
            { "text": "b", "ticked": false, "human_check": true, "pending": false },
        ]),
        "{story}"
    );
    assert_eq!(
        story["links"],
        json!([{ "doc_id": UNKNOWN_ID, "node_id": "n1", "label": "design" }]),
        "a link is stored as given: {story}"
    );
}

#[test]
fn number_from_sets_only_the_first_number_of_the_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let first = create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("First", json!({ "number_from": 130 })),
    );
    let second = create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("Second", json!({ "number_from": 500 })),
    );
    assert_eq!(first["key"], "US-131", "{first}");
    assert_eq!(second["key"], "US-132", "{second}");
}

#[test]
fn story_numbers_are_counted_per_project_across_its_features() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let other_feature = new_feature(f, client, &project);
    let other_project = new_project(f, client);
    let other_project_feature = new_feature(f, client, &other_project);
    create_story(f, client, &project, &feature, &body("One"));
    let in_other_feature = create_story(f, client, &project, &other_feature, &body("Two"));
    let in_other_project = create_story(
        f,
        client,
        &other_project,
        &other_project_feature,
        &body("Elsewhere"),
    );
    assert_eq!(in_other_feature["key"], "US-2", "{in_other_feature}");
    assert_eq!(in_other_project["key"], "US-1", "{in_other_project}");
}

#[test]
fn a_negative_number_from_is_refused_and_zero_is_accepted() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    assert_error(
        &post_story(
            f,
            client,
            &project,
            &feature,
            &body_with("Below", json!({ "number_from": -1 })),
        ),
        422,
        "invalid_number_from",
    );
    let story = create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("Zero", json!({ "number_from": 0 })),
    );
    assert_eq!(story["key"], "US-1", "{story}");
}

#[test]
fn a_priority_outside_0_to_3_is_refused_and_creates_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    for priority in [-1, 4] {
        assert_error(
            &post_story(
                f,
                client,
                &project,
                &feature,
                &body_with("Out of range", json!({ "priority": priority })),
            ),
            422,
            "invalid_priority",
        );
    }
    assert_eq!(story_rows(&project), 0);
}

#[test]
fn priority_0_and_3_are_accepted_and_3_is_the_default() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let fix = create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("Fix", json!({ "priority": 0 })),
    );
    let lowest = create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("Low", json!({ "priority": 3 })),
    );
    let unset = create_story(f, client, &project, &feature, &body("Unset"));
    assert_eq!(fix["priority"], 0, "{fix}");
    assert_eq!(lowest["priority"], 3, "{lowest}");
    assert_eq!(unset["priority"], 3, "{unset}");
}

#[test]
fn a_title_blank_after_trimming_is_refused_and_creates_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    assert_error(
        &post_story(f, client, &project, &feature, &body(" \t ")),
        422,
        "invalid_title",
    );
    assert_eq!(story_rows(&project), 0);
    assert!(events_of(&project, "story.created").is_empty());
}

#[test]
fn empty_text_topic_and_criteria_are_accepted_but_a_missing_text_is_a_bad_request() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = create_story(f, client, &project, &feature, &body("Bare"));
    assert_eq!(story["text"], "", "{story}");
    assert_eq!(story["topic"], "", "{story}");
    assert_eq!(story["criteria"], json!([]), "{story}");
    assert_error(
        &post_story(
            f,
            client,
            &project,
            &feature,
            &json!({ "title": "No text", "topic": "", "criteria": [] }),
        ),
        400,
        "invalid_request",
    );
}

#[test]
fn a_link_of_neither_shape_is_refused_and_a_thread_link_is_accepted() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    for link in [
        json!({ "doc_id": UNKNOWN_ID, "label": "no node" }),
        json!({ "thread_id": UNKNOWN_ID }),
        json!("a text"),
    ] {
        assert_error(
            &post_story(
                f,
                client,
                &project,
                &feature,
                &body_with("Bad link", json!({ "links": [link] })),
            ),
            422,
            "invalid_links",
        );
    }
    let story = create_story(
        f,
        client,
        &project,
        &feature,
        &body_with(
            "Thread link",
            json!({ "links": [{ "thread_id": UNKNOWN_ID, "label": "Q1" }] }),
        ),
    );
    assert_eq!(
        story["links"],
        json!([{ "thread_id": UNKNOWN_ID, "label": "Q1" }]),
        "{story}"
    );
}

#[test]
fn a_blocker_must_be_a_story_of_the_same_feature() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let other_feature = new_feature(f, client, &project);
    let elsewhere = id_of(&create_story(
        f,
        client,
        &project,
        &other_feature,
        &body("Elsewhere"),
    ));
    for blocker in [elsewhere.as_str(), UNKNOWN_ID, "not-a-uuid"] {
        assert_error(
            &post_story(
                f,
                client,
                &project,
                &feature,
                &body_with("Blocked", json!({ "blocked_by": [blocker] })),
            ),
            422,
            "invalid_blocked_by",
        );
    }
    assert_eq!(story_rows(&project), 1, "a refused story is not stored");
}

#[test]
fn a_cancelled_story_of_the_feature_is_a_valid_blocker() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let cancelled = id_of(&create_story(
        f,
        client,
        &project,
        &feature,
        &body("Dropped"),
    ));
    set_status(&cancelled, "cancelled");
    let story = create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("Blocked", json!({ "blocked_by": [cancelled] })),
    );
    assert_eq!(story["blocked_by"], json!([cancelled]), "{story}");
}

#[test]
fn duplicate_blockers_are_kept_once_in_first_seen_order() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let a = id_of(&create_story(f, client, &project, &feature, &body("A")));
    let b = id_of(&create_story(f, client, &project, &feature, &body("B")));
    let story = create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("Blocked", json!({ "blocked_by": [b, a, b] })),
    );
    assert_eq!(story["blocked_by"], json!([b, a]), "{story}");
}

#[test]
fn creating_a_story_emits_story_created_with_its_id_key_and_feature() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = create_story(f, client, &project, &feature, &body("Announced"));
    assert_eq!(
        events_of(&project, "story.created"),
        vec![json!({ "story_id": story["id"], "key": "US-1", "feature_id": feature })]
    );
}

// ---------------------------------------------------------------------------
// Reading stories
// ---------------------------------------------------------------------------

#[test]
fn the_list_holds_every_status_ordered_by_priority_then_number() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("One", json!({ "priority": 3 })),
    );
    create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("Two", json!({ "priority": 1 })),
    );
    let three = create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("Three", json!({ "priority": 3 })),
    );
    create_story(
        f,
        client,
        &project,
        &feature,
        &body_with("Four", json!({ "priority": 1 })),
    );
    set_status(&id_of(&three), "cancelled");
    let listed = must(
        call(
            client,
            "GET",
            &stories_path(f, &project, &feature),
            &human(f),
            None,
        ),
        200,
        "list the stories",
    );
    assert_eq!(
        keys_of(&listed),
        vec![json!("US-2"), json!("US-4"), json!("US-1"), json!("US-3")]
    );
}

#[test]
fn the_status_filter_lists_only_that_status_and_refuses_an_unknown_one() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    create_story(f, client, &project, &feature, &body("Waiting"));
    let reviewed = create_story(f, client, &project, &feature, &body("Reviewed"));
    set_status(&id_of(&reviewed), "review");
    let path = stories_path(f, &project, &feature);
    let listed = must(
        call(
            client,
            "GET",
            &format!("{path}?status=review"),
            &agent(f),
            None,
        ),
        200,
        "list the stories in review",
    );
    assert_eq!(keys_of(&listed), vec![json!("US-2")]);
    assert_error(
        &call(
            client,
            "GET",
            &format!("{path}?status=bogus"),
            &agent(f),
            None,
        ),
        422,
        "invalid_status",
    );
}

#[test]
fn a_story_is_not_found_under_another_feature_or_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let other_feature = new_feature(f, client, &project);
    let other_project = new_project(f, client);
    let story = create_story(f, client, &project, &feature, &body("Here"));
    let id = id_of(&story);
    let shown = must(
        call(
            client,
            "GET",
            &format!("{}/{id}", stories_path(f, &project, &feature)),
            &human(f),
            None,
        ),
        200,
        "show the story",
    );
    assert_eq!(shown["story"], story);
    for path in [
        stories_path(f, &project, &other_feature),
        stories_path(f, &other_project, &feature),
    ] {
        assert_error(
            &call(client, "GET", &format!("{path}/{id}"), &agent(f), None),
            404,
            "not_found",
        );
    }
}
