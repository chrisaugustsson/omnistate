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
//! - US-132: `GET /stories?pickable=true` lists the `backlog` stories that are
//!   not `blocked`, whose `blocked_by` stories are all `done` or `cancelled`,
//!   and whose files share no path with a story of the feature in
//!   `implementing` or `review` (a story's files are `files_changed` when not
//!   empty, else `files_planned`). They come by priority, then by open (not
//!   done, not cancelled) dependants, most first, then by number.
//!   `pickable=false` gives the normal list, any other value is 422; with
//!   `?status=` the list is empty unless the status is `backlog`.
//! - US-133: `PATCH /stories/{story_id}` (agent and human) takes
//!   `expected_version` and any of `title, text, topic, criteria, blocked_by,
//!   files_planned, priority, links`, answers 200 `{"story": Story}` with
//!   `version` one higher and emits `story.changed` with `{story_id, key,
//!   fields}`, every field of the body in that order. Checked in this order: a
//!   body with no field but `expected_version`, or a `blocked_by` naming a
//!   story of another feature or the story itself, is 422; a stale
//!   `expected_version` is 409 `{"error": "conflict", "current_version": n}`;
//!   a `done` or `cancelled` story is 409 `not_open`; `title, text, topic,
//!   criteria, files_planned` outside `backlog` are 409 `not_in_backlog`. A
//!   refused request changes nothing. `POST /stories/{story_id}/cancel` with
//!   `{expected_version}` answers 200 with the story `cancelled`, its holder
//!   cleared, emits `story.cancelled` with `{story_id, key}`, and is 409
//!   `not_open` from `done` or `cancelled`.
//! - US-134: `POST /stories/{story_id}/claim` `{expected_version, run,
//!   stage}`, `/release` `{expected_version, run}` and `/move`
//!   `{expected_version, run, to}` are agent only (a session is 403). A claim
//!   sets `holder: {run, stage, at}` and the status to the stage; the same run
//!   may claim again, another is 409 `{"error": "held", "holder": ...}`. Any
//!   run may release; the status stays. Only the holder's run moves (else 409
//!   `not_holder`); a move to a stage sets `holder.stage`, to `done` or
//!   `backlog` clears the holder, and `done` with an unticked criterion is 409
//!   `{"error": "criteria_open", "open": [0-based indexes]}`. Each accepted
//!   write bumps `version` and emits `story.changed`; a refused one changes
//!   nothing. Checked in this order: stale version (409 `conflict`), `not_open`
//!   (done or cancelled), `held` or `not_holder`, `criteria_open`.
//! - US-135: `POST /stories/{story_id}/tick` `{expected_version, run,
//!   criterion, ticked, pending?}`, `/findings` `{.., stage, text, files}`,
//!   `/findings/{index}/resolve`, `/decisions` `{.., question, choice, by}`
//!   and `/files` `{.., files_changed}` are agent only and, on a held story,
//!   only from the holder's run (else 409 `not_holder`); a criterion index
//!   out of range is 422 `invalid_criterion`. `/block` and `/unblock` and
//!   `/decisions` on an unheld story take any run. `POST /check`
//!   `{expected_version, criterion}` is human only and clears `pending` in
//!   any status, `done` included.

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

/// Sets a story's `blocked` flag as the admin role: no route of this story
/// blocks one.
fn set_blocked(story: &str) {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query("update stories set blocked = true where id = $1::uuid")
            .bind(story)
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("block the story: {e}"));
    })
}

/// Sets a story's `files_changed` as the admin role: no route of this story
/// writes them.
fn set_files_changed(story: &str, files: Value) {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query("update stories set files_changed = $2::jsonb where id = $1::uuid")
            .bind(story)
            .bind(files.to_string())
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("set the changed files: {e}"));
    })
}

/// Sets a story's holder as the admin role: no route of this story claims one.
fn set_holder(story: &str, holder: Value) {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query("update stories set holder = $2::jsonb where id = $1::uuid")
            .bind(story)
            .bind(holder.to_string())
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("set the holder: {e}"));
    })
}

fn story_path(f: &Fixture, project: &str, feature: &str, story: &str) -> String {
    format!("{}/{story}", stories_path(f, project, feature))
}

/// `PATCH`es a story as the agent.
fn patch(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: &str,
    story: &str,
    body: Value,
) -> Response {
    call(
        client,
        "PATCH",
        &story_path(f, project, feature, story),
        &agent(f),
        Some(&body),
    )
}

/// Cancels a story as the agent.
fn cancel(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: &str,
    story: &str,
    expected_version: i64,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/cancel", story_path(f, project, feature, story)),
        &agent(f),
        Some(&json!({ "expected_version": expected_version })),
    )
}

/// The story as `GET /stories/{story_id}` answers it.
fn get_story(f: &Fixture, client: &Client, project: &str, feature: &str, story: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &story_path(f, project, feature, story),
            &agent(f),
            None,
        ),
        200,
        "read the story",
    )["story"]
        .clone()
}

/// Creates a story with `extra`'s fields as the agent; returns its id.
fn new_story(f: &Fixture, client: &Client, project: &str, feature: &str, extra: Value) -> String {
    id_of(&create_story(
        f,
        client,
        project,
        feature,
        &body_with("Story", extra),
    ))
}

/// The keys `GET /stories?<query>` lists.
fn listed_keys(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: &str,
    query: &str,
) -> Vec<Value> {
    keys_of(&must(
        call(
            client,
            "GET",
            &format!("{}?{query}", stories_path(f, project, feature)),
            &agent(f),
            None,
        ),
        200,
        "list the stories",
    ))
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

// ---------------------------------------------------------------------------
// Pickable stories (US-132)
// ---------------------------------------------------------------------------

#[test]
fn pickable_lists_backlog_stories_whose_blockers_are_closed_and_whose_files_are_free() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let d = new_story(f, client, &project, &feature, json!({}));
    set_status(&d, "cancelled");
    let a = new_story(f, client, &project, &feature, json!({}));
    new_story(f, client, &project, &feature, json!({ "blocked_by": [a] }));
    new_story(f, client, &project, &feature, json!({ "blocked_by": [d] }));
    let busy = new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/a.rs"] }),
    );
    set_status(&busy, "implementing");
    new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/b.rs", "src/a.rs"] }),
    );
    assert_eq!(
        listed_keys(f, client, &project, &feature, "pickable=true"),
        vec![json!("US-2"), json!("US-4")],
        "A (US-2) and C (US-4) only; not D (cancelled), B (blocked by backlog A), the implementing story or E (shares src/a.rs)"
    );
}

#[test]
fn a_done_blocker_releases_its_story() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let blocker = new_story(f, client, &project, &feature, json!({}));
    set_status(&blocker, "done");
    new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "blocked_by": [blocker] }),
    );
    assert_eq!(
        listed_keys(f, client, &project, &feature, "pickable=true"),
        vec![json!("US-2")]
    );
}

#[test]
fn a_story_in_review_holds_its_files_and_one_in_testing_does_not() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let reviewed = new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/a.rs"] }),
    );
    set_status(&reviewed, "review");
    let tested = new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/b.rs"] }),
    );
    set_status(&tested, "testing");
    new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/a.rs"] }),
    );
    new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/b.rs"] }),
    );
    assert_eq!(
        listed_keys(f, client, &project, &feature, "pickable=true"),
        vec![json!("US-4")]
    );
}

#[test]
fn pickable_stories_come_by_priority_then_open_dependants_then_number() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    new_story(f, client, &project, &feature, json!({ "priority": 1 }));
    let q = new_story(f, client, &project, &feature, json!({ "priority": 1 }));
    new_story(f, client, &project, &feature, json!({ "priority": 0 }));
    let r = new_story(f, client, &project, &feature, json!({ "priority": 1 }));
    new_story(f, client, &project, &feature, json!({ "blocked_by": [q] }));
    let done = new_story(f, client, &project, &feature, json!({ "blocked_by": [r] }));
    set_status(&done, "done");
    let cancelled = new_story(f, client, &project, &feature, json!({ "blocked_by": [r] }));
    set_status(&cancelled, "cancelled");
    assert_eq!(
        listed_keys(f, client, &project, &feature, "pickable=true"),
        vec![json!("US-3"), json!("US-2"), json!("US-1"), json!("US-4")],
        "priority 0 first; then US-2 with one open dependant; then US-1 and US-4, \
         whose only dependants are done or cancelled, by number"
    );
}

#[test]
fn a_blocked_story_is_never_pickable() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let flagged = new_story(f, client, &project, &feature, json!({}));
    set_blocked(&flagged);
    new_story(f, client, &project, &feature, json!({}));
    assert_eq!(
        listed_keys(f, client, &project, &feature, "pickable=true"),
        vec![json!("US-2")]
    );
}

#[test]
fn changed_files_replace_planned_files_on_both_sides_of_the_comparison() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let busy = new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/planned.rs"] }),
    );
    set_files_changed(&busy, json!(["src/changed.rs"]));
    set_status(&busy, "implementing");
    new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/planned.rs"] }),
    );
    new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/changed.rs"] }),
    );
    let moved = new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/changed.rs"] }),
    );
    set_files_changed(&moved, json!(["src/other.rs"]));
    assert_eq!(
        listed_keys(f, client, &project, &feature, "pickable=true"),
        vec![json!("US-2"), json!("US-4")],
        "the busy story holds src/changed.rs only, and US-4 occupies src/other.rs only"
    );
}

#[test]
fn paths_are_compared_as_exact_strings() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let busy = new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/a.rs"] }),
    );
    set_status(&busy, "implementing");
    new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["./src/a.rs"] }),
    );
    assert_eq!(
        listed_keys(f, client, &project, &feature, "pickable=true"),
        vec![json!("US-2")]
    );
}

#[test]
fn pickable_with_a_status_filter_is_empty_unless_the_status_is_backlog() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let busy = new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/a.rs"] }),
    );
    set_status(&busy, "implementing");
    new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "files_planned": ["src/a.rs"] }),
    );
    new_story(f, client, &project, &feature, json!({}));
    assert_eq!(
        listed_keys(
            f,
            client,
            &project,
            &feature,
            "pickable=true&status=backlog"
        ),
        vec![json!("US-3")],
        "the implementing story still holds src/a.rs when only backlog is listed"
    );
    assert_eq!(
        listed_keys(
            f,
            client,
            &project,
            &feature,
            "pickable=true&status=implementing"
        ),
        Vec::<Value>::new()
    );
}

#[test]
fn pickable_false_gives_the_normal_list_and_another_value_is_refused() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let busy = new_story(f, client, &project, &feature, json!({}));
    set_status(&busy, "implementing");
    new_story(f, client, &project, &feature, json!({}));
    assert_eq!(
        listed_keys(f, client, &project, &feature, "pickable=false"),
        vec![json!("US-1"), json!("US-2")]
    );
    assert_error(
        &call(
            client,
            "GET",
            &format!("{}?pickable=yes", stories_path(f, &project, &feature)),
            &agent(f),
            None,
        ),
        422,
        "invalid_pickable",
    );
}

// ---------------------------------------------------------------------------
// Editing and cancelling a story
// ---------------------------------------------------------------------------

#[test]
fn a_patch_at_the_current_version_bumps_it_and_the_old_one_conflicts_before_the_status_rules() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = new_story(f, client, &project, &feature, json!({}));
    let path = story_path(f, &project, &feature, &story);
    let edit = json!({ "expected_version": 1, "priority": 1 });
    let edited = must(
        call(client, "PATCH", &path, &human(f), Some(&edit)),
        200,
        "patch at the current version",
    )["story"]
        .clone();
    assert_eq!(edited["version"], 2, "{edited}");
    assert_eq!(edited["priority"], 1, "{edited}");
    assert_eq!(
        events_of(&project, "story.changed"),
        vec![json!({ "story_id": story, "key": "US-1", "fields": ["priority"] })]
    );
    // Done, and `text` is backlog only: the stale version is still the answer.
    set_status(&story, "done");
    let stale = call(
        client,
        "PATCH",
        &path,
        &human(f),
        Some(&json!({ "expected_version": 1, "text": "New" })),
    );
    assert_eq!(stale.status, 409, "body: {}", stale.text());
    assert_eq!(
        stale.json(),
        json!({ "error": "conflict", "current_version": 2 })
    );
    assert_eq!(
        get_story(f, client, &project, &feature, &story)["version"],
        2
    );
    assert_eq!(events_of(&project, "story.changed").len(), 1);
}

#[test]
fn story_changed_names_every_field_of_the_body_in_schema_order_changed_or_not() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = new_story(f, client, &project, &feature, json!({}));
    // `links` sorts before `title` by name, but comes after it in the schema;
    // the priority is the default 3, so it does not change.
    must(
        patch(
            f,
            client,
            &project,
            &feature,
            &story,
            json!({ "expected_version": 1, "links": [], "priority": 3, "title": "Renamed" }),
        ),
        200,
        "patch three fields",
    );
    assert_eq!(
        events_of(&project, "story.changed"),
        vec![json!({ "story_id": story, "key": "US-1", "fields": ["title", "priority", "links"] })]
    );
}

#[test]
fn outside_backlog_only_priority_blockers_and_links_change() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let blocker = new_story(f, client, &project, &feature, json!({}));
    let story = new_story(f, client, &project, &feature, json!({}));
    set_status(&story, "implementing");
    let refuse = |body: Value| {
        assert_error(
            &patch(f, client, &project, &feature, &story, body),
            409,
            "not_in_backlog",
        )
    };
    refuse(json!({ "expected_version": 1, "title": "New" }));
    refuse(json!({ "expected_version": 1, "text": "New" }));
    refuse(json!({ "expected_version": 1, "topic": "New" }));
    refuse(json!({ "expected_version": 1, "criteria": [{ "text": "New" }] }));
    refuse(json!({ "expected_version": 1, "files_planned": ["src/new.rs"] }));
    refuse(json!({ "expected_version": 1, "text": "New", "priority": 1 }));
    let unchanged = get_story(f, client, &project, &feature, &story);
    assert_eq!(unchanged["version"], 1, "{unchanged}");
    assert_eq!(unchanged["priority"], 3, "{unchanged}");
    assert_eq!(unchanged["text"], "", "{unchanged}");
    let edited = must(
        patch(
            f,
            client,
            &project,
            &feature,
            &story,
            json!({
                "expected_version": 1,
                "priority": 1,
                "blocked_by": [blocker],
                "links": [{ "thread_id": UNKNOWN_ID, "label": "question" }],
            }),
        ),
        200,
        "patch priority, blockers and links in implementing",
    )["story"]
        .clone();
    assert_eq!(edited["version"], 2, "{edited}");
    assert_eq!(edited["priority"], 1, "{edited}");
    assert_eq!(edited["blocked_by"], json!([blocker]), "{edited}");
    assert_eq!(
        edited["links"],
        json!([{ "thread_id": UNKNOWN_ID, "label": "question" }]),
        "{edited}"
    );
}

#[test]
fn a_done_or_cancelled_story_refuses_every_field_as_not_open() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let done = new_story(f, client, &project, &feature, json!({}));
    set_status(&done, "done");
    let cancelled = new_story(f, client, &project, &feature, json!({}));
    set_status(&cancelled, "cancelled");
    assert_error(
        &patch(
            f,
            client,
            &project,
            &feature,
            &done,
            json!({ "expected_version": 1, "priority": 1 }),
        ),
        409,
        "not_open",
    );
    // A text field breaks both rules; a closed story answers `not_open`.
    assert_error(
        &patch(
            f,
            client,
            &project,
            &feature,
            &done,
            json!({ "expected_version": 1, "text": "New" }),
        ),
        409,
        "not_open",
    );
    assert_error(
        &patch(
            f,
            client,
            &project,
            &feature,
            &cancelled,
            json!({ "expected_version": 1, "links": [] }),
        ),
        409,
        "not_open",
    );
    assert_eq!(
        get_story(f, client, &project, &feature, &done)["version"],
        1
    );
    assert!(events_of(&project, "story.changed").is_empty());
}

#[test]
fn an_invalid_body_is_refused_before_a_stale_version_is_reported() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let other_feature = new_feature(f, client, &project);
    let elsewhere = new_story(f, client, &project, &other_feature, json!({}));
    let story = new_story(f, client, &project, &feature, json!({}));
    let only_version = patch(
        f,
        client,
        &project,
        &feature,
        &story,
        json!({ "expected_version": 7 }),
    );
    assert_eq!(only_version.status, 422, "body: {}", only_version.text());
    assert_error(
        &patch(
            f,
            client,
            &project,
            &feature,
            &story,
            json!({ "expected_version": 7, "blocked_by": [elsewhere] }),
        ),
        422,
        "invalid_blocked_by",
    );
}

#[test]
fn a_patched_blocker_must_be_another_story_of_the_same_feature() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let other_feature = new_feature(f, client, &project);
    let elsewhere = new_story(f, client, &project, &other_feature, json!({}));
    let story = new_story(f, client, &project, &feature, json!({}));
    assert_error(
        &patch(
            f,
            client,
            &project,
            &feature,
            &story,
            json!({ "expected_version": 1, "blocked_by": [elsewhere] }),
        ),
        422,
        "invalid_blocked_by",
    );
    assert_error(
        &patch(
            f,
            client,
            &project,
            &feature,
            &story,
            json!({ "expected_version": 1, "blocked_by": [story] }),
        ),
        422,
        "invalid_blocked_by",
    );
    let unchanged = get_story(f, client, &project, &feature, &story);
    assert_eq!(unchanged["version"], 1, "{unchanged}");
    assert_eq!(unchanged["blocked_by"], json!([]), "{unchanged}");
}

#[test]
fn two_stories_may_block_each_other() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let a = new_story(f, client, &project, &feature, json!({}));
    let b = new_story(f, client, &project, &feature, json!({ "blocked_by": [a] }));
    let edited = must(
        patch(
            f,
            client,
            &project,
            &feature,
            &a,
            json!({ "expected_version": 1, "blocked_by": [b] }),
        ),
        200,
        "block A by B while B is blocked by A",
    )["story"]
        .clone();
    assert_eq!(edited["blocked_by"], json!([b]), "{edited}");
}

#[test]
fn cancelling_answers_the_cancelled_story_without_its_holder() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = new_story(f, client, &project, &feature, json!({}));
    set_status(&story, "implementing");
    set_holder(
        &story,
        json!({ "run": "run-1", "stage": "implementing", "at": "2026-10-31T00:00:00Z" }),
    );
    let stale = cancel(f, client, &project, &feature, &story, 7);
    assert_eq!(stale.status, 409, "body: {}", stale.text());
    assert_eq!(
        stale.json(),
        json!({ "error": "conflict", "current_version": 1 })
    );
    let cancelled = must(
        cancel(f, client, &project, &feature, &story, 1),
        200,
        "cancel the story",
    )["story"]
        .clone();
    assert_eq!(cancelled["id"], story, "{cancelled}");
    assert_eq!(cancelled["status"], "cancelled", "{cancelled}");
    assert_eq!(cancelled["version"], 2, "{cancelled}");
    assert_eq!(cancelled["holder"], Value::Null, "{cancelled}");
    assert_eq!(
        events_of(&project, "story.cancelled"),
        vec![json!({ "story_id": story, "key": "US-1" })]
    );
}

#[test]
fn a_done_or_cancelled_story_cannot_be_cancelled() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let done = new_story(f, client, &project, &feature, json!({}));
    set_status(&done, "done");
    let story = new_story(f, client, &project, &feature, json!({}));
    must(
        cancel(f, client, &project, &feature, &story, 1),
        200,
        "cancel once",
    );
    assert_error(
        &cancel(f, client, &project, &feature, &story, 2),
        409,
        "not_open",
    );
    assert_error(
        &cancel(f, client, &project, &feature, &done, 1),
        409,
        "not_open",
    );
    assert_eq!(
        get_story(f, client, &project, &feature, &done)["status"],
        "done"
    );
    assert_eq!(events_of(&project, "story.cancelled").len(), 1);
}

// ---------------------------------------------------------------------------
// Claims and column moves
// ---------------------------------------------------------------------------

/// Sets each criterion's `ticked` as the admin role: no route of this story
/// ticks one.
fn set_ticked(story: &str, ticked: Value) {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query(
            "update stories set criteria = (select jsonb_agg(jsonb_set(c, '{ticked}', t) \
             order by i) from jsonb_array_elements(criteria) with ordinality as a(c, i) \
             join jsonb_array_elements($2::jsonb) with ordinality as b(t, j) on i = j) \
             where id = $1::uuid",
        )
        .bind(story)
        .bind(ticked.to_string())
        .execute(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("tick the criteria: {e}"));
    })
}

/// `POST`s `body` to `/stories/{story_id}/<action>` as `auth`.
fn act(client: &Client, auth: &Auth, path: &str, action: &str, body: Value) -> Response {
    call(
        client,
        "POST",
        &format!("{path}/{action}"),
        auth,
        Some(&body),
    )
}

#[test]
fn a_claim_holds_the_story_for_its_run_until_any_run_releases_it() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = new_story(f, client, &project, &feature, json!({}));
    let path = story_path(f, &project, &feature, &story);
    let a = agent(f);
    let first = must(
        act(
            client,
            &a,
            &path,
            "claim",
            json!({ "expected_version": 1, "run": "r1", "stage": "implementing" }),
        ),
        200,
        "r1 claims",
    )["story"]
        .clone();
    assert_eq!(first["status"], "implementing", "{first}");
    assert_eq!(first["holder"]["run"], "r1", "{first}");
    assert_eq!(first["holder"]["stage"], "implementing", "{first}");
    assert_eq!(first["version"], 2, "{first}");
    must(
        act(
            client,
            &a,
            &path,
            "move",
            json!({ "expected_version": 2, "run": "r1", "to": "implemented" }),
        ),
        200,
        "r1 moves to implemented",
    );
    let again = must(
        act(
            client,
            &a,
            &path,
            "claim",
            json!({ "expected_version": 3, "run": "r1", "stage": "review" }),
        ),
        200,
        "r1 claims again for review",
    )["story"]
        .clone();
    assert_eq!(again["status"], "review", "{again}");
    assert_eq!(again["holder"]["run"], "r1", "{again}");
    assert_eq!(again["holder"]["stage"], "review", "{again}");
    assert_eq!(again["version"], 4, "{again}");
    // Held comes before not_waiting: a story in review is not waiting for
    // implementing either.
    let held = act(
        client,
        &a,
        &path,
        "claim",
        json!({ "expected_version": 4, "run": "r2", "stage": "implementing" }),
    );
    assert_eq!(held.status, 409, "body: {}", held.text());
    assert_eq!(held.json()["error"], "held");
    assert_eq!(held.json()["holder"], again["holder"]);
    assert_eq!(
        get_story(f, client, &project, &feature, &story)["version"],
        4,
        "a refused claim changes nothing"
    );
    let released = must(
        act(
            client,
            &a,
            &path,
            "release",
            json!({ "expected_version": 4, "run": "r2" }),
        ),
        200,
        "r2 releases r1's claim",
    )["story"]
        .clone();
    assert_eq!(released["holder"], Value::Null, "{released}");
    assert_eq!(
        released["status"], "review",
        "a release leaves the status: {released}"
    );
    assert_eq!(released["version"], 5, "{released}");
    let changed: Vec<Value> = events_of(&project, "story.changed")
        .iter()
        .map(|event| json!([event["story_id"], event["key"]]))
        .collect();
    assert_eq!(
        changed,
        vec![json!([story, "US-1"]); 4],
        "one event per accepted write"
    );
}

#[test]
fn only_the_holding_run_moves_the_story_and_its_stage_follows_the_column() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let unheld = new_story(f, client, &project, &feature, json!({}));
    let story = new_story(f, client, &project, &feature, json!({}));
    let path = story_path(f, &project, &feature, &story);
    let a = agent(f);
    assert_error(
        &act(
            client,
            &a,
            &story_path(f, &project, &feature, &unheld),
            "move",
            json!({ "expected_version": 1, "run": "r1", "to": "implementing" }),
        ),
        409,
        "not_holder",
    );
    must(
        act(
            client,
            &a,
            &path,
            "claim",
            json!({ "expected_version": 1, "run": "r2", "stage": "implementing" }),
        ),
        200,
        "r2 claims",
    );
    assert_error(
        &act(
            client,
            &a,
            &path,
            "move",
            json!({ "expected_version": 2, "run": "r1", "to": "testing" }),
        ),
        409,
        "not_holder",
    );
    let moved = must(
        act(
            client,
            &a,
            &path,
            "move",
            json!({ "expected_version": 2, "run": "r2", "to": "testing" }),
        ),
        200,
        "the holder moves to testing",
    )["story"]
        .clone();
    assert_eq!(moved["status"], "testing", "{moved}");
    assert_eq!(moved["holder"]["run"], "r2", "{moved}");
    assert_eq!(moved["holder"]["stage"], "testing", "{moved}");
    assert_eq!(moved["version"], 3, "{moved}");
    let back = must(
        act(
            client,
            &a,
            &path,
            "move",
            json!({ "expected_version": 3, "run": "r2", "to": "backlog" }),
        ),
        200,
        "the holder moves back to backlog",
    )["story"]
        .clone();
    assert_eq!(back["status"], "backlog", "{back}");
    assert_eq!(back["holder"], Value::Null, "{back}");
    assert_eq!(
        get_story(f, client, &project, &feature, &unheld)["version"],
        1,
        "a refused move changes nothing"
    );
}

#[test]
fn the_holder_moves_an_implementing_story_to_implemented_which_clears_the_holder() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    new_story(f, client, &project, &feature, json!({}));
    let story = new_story(f, client, &project, &feature, json!({}));
    let path = story_path(f, &project, &feature, &story);
    let a = agent(f);
    must(
        act(
            client,
            &a,
            &path,
            "claim",
            json!({ "expected_version": 1, "run": "r1", "stage": "implementing" }),
        ),
        200,
        "r1 claims for implementing",
    );
    let moved = must(
        act(
            client,
            &a,
            &path,
            "move",
            json!({ "expected_version": 2, "run": "r1", "to": "implemented" }),
        ),
        200,
        "the holder moves to implemented",
    )["story"]
        .clone();
    assert_eq!(moved["status"], "implemented", "{moved}");
    assert_eq!(moved["holder"], Value::Null, "{moved}");
    assert_eq!(
        listed_keys(f, client, &project, &feature, "status=implemented"),
        vec![json!("US-2")]
    );
}

#[test]
fn the_holder_moves_a_story_in_review_to_reviewed_which_clears_the_holder() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    new_story(f, client, &project, &feature, json!({}));
    let story = new_story(f, client, &project, &feature, json!({}));
    set_status(&story, "implemented");
    let path = story_path(f, &project, &feature, &story);
    let a = agent(f);
    must(
        act(
            client,
            &a,
            &path,
            "claim",
            json!({ "expected_version": 1, "run": "r1", "stage": "review" }),
        ),
        200,
        "r1 claims for review",
    );
    let moved = must(
        act(
            client,
            &a,
            &path,
            "move",
            json!({ "expected_version": 2, "run": "r1", "to": "reviewed" }),
        ),
        200,
        "the holder moves to reviewed",
    )["story"]
        .clone();
    assert_eq!(moved["status"], "reviewed", "{moved}");
    assert_eq!(moved["holder"], Value::Null, "{moved}");
    assert_eq!(
        listed_keys(f, client, &project, &feature, "status=reviewed"),
        vec![json!("US-2")]
    );
}

#[test]
fn a_claim_for_review_on_a_backlog_story_is_not_waiting_and_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = new_story(f, client, &project, &feature, json!({}));
    let path = story_path(f, &project, &feature, &story);
    let refused = act(
        client,
        &agent(f),
        &path,
        "claim",
        json!({ "expected_version": 1, "run": "r1", "stage": "review" }),
    );
    assert_eq!(refused.status, 409, "body: {}", refused.text());
    let refused = refused.json();
    assert_eq!(refused["error"], "not_waiting", "{refused}");
    assert_eq!(refused["status"], "backlog", "{refused}");
    assert_eq!(refused["stage"], "review", "{refused}");
    let unchanged = get_story(f, client, &project, &feature, &story);
    assert_eq!(unchanged["status"], "backlog", "{unchanged}");
    assert_eq!(unchanged["holder"], Value::Null, "{unchanged}");
    assert_eq!(unchanged["version"], 1, "{unchanged}");
    assert!(events_of(&project, "story.changed").is_empty());
}

#[test]
fn the_holder_of_a_story_in_review_claims_it_for_testing_only_after_moving_it_to_reviewed() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = new_story(f, client, &project, &feature, json!({}));
    set_status(&story, "implemented");
    let path = story_path(f, &project, &feature, &story);
    let a = agent(f);
    must(
        act(
            client,
            &a,
            &path,
            "claim",
            json!({ "expected_version": 1, "run": "r1", "stage": "review" }),
        ),
        200,
        "r1 claims for review",
    );
    let skipped = act(
        client,
        &a,
        &path,
        "claim",
        json!({ "expected_version": 2, "run": "r1", "stage": "testing" }),
    );
    assert_eq!(skipped.status, 409, "body: {}", skipped.text());
    let skipped = skipped.json();
    assert_eq!(skipped["error"], "not_waiting", "{skipped}");
    assert_eq!(skipped["status"], "review", "{skipped}");
    assert_eq!(skipped["stage"], "testing", "{skipped}");
    must(
        act(
            client,
            &a,
            &path,
            "move",
            json!({ "expected_version": 2, "run": "r1", "to": "reviewed" }),
        ),
        200,
        "r1 moves to reviewed",
    );
    let testing = must(
        act(
            client,
            &a,
            &path,
            "claim",
            json!({ "expected_version": 3, "run": "r1", "stage": "testing" }),
        ),
        200,
        "r1 claims for testing",
    )["story"]
        .clone();
    assert_eq!(testing["status"], "testing", "{testing}");
    assert_eq!(testing["holder"]["run"], "r1", "{testing}");
    assert_eq!(testing["holder"]["stage"], "testing", "{testing}");
}

#[test]
fn a_move_to_done_needs_every_criterion_ticked_and_clears_the_holder() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "criteria": [{ "text": "a" }, { "text": "b" }] }),
    );
    set_status(&story, "reviewed");
    let path = story_path(f, &project, &feature, &story);
    let a = agent(f);
    must(
        act(
            client,
            &a,
            &path,
            "claim",
            json!({ "expected_version": 1, "run": "r1", "stage": "testing" }),
        ),
        200,
        "r1 claims for testing",
    );
    set_ticked(&story, json!([true, false]));
    let open = act(
        client,
        &a,
        &path,
        "move",
        json!({ "expected_version": 2, "run": "r1", "to": "done" }),
    );
    assert_eq!(open.status, 409, "body: {}", open.text());
    assert_eq!(
        open.json(),
        json!({ "error": "criteria_open", "open": [1] })
    );
    assert_eq!(
        get_story(f, client, &project, &feature, &story)["status"],
        "testing"
    );
    set_ticked(&story, json!([true, true]));
    let done = must(
        act(
            client,
            &a,
            &path,
            "move",
            json!({ "expected_version": 2, "run": "r1", "to": "done" }),
        ),
        200,
        "move to done with every criterion ticked",
    )["story"]
        .clone();
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(done["holder"], Value::Null, "{done}");
    assert_eq!(done["version"], 3, "{done}");
}

#[test]
fn a_done_or_cancelled_story_cannot_be_claimed_or_moved() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let a = agent(f);
    let cancelled = new_story(f, client, &project, &feature, json!({}));
    must(
        cancel(f, client, &project, &feature, &cancelled, 1),
        200,
        "cancel",
    );
    let cancelled = story_path(f, &project, &feature, &cancelled);
    assert_error(
        &act(
            client,
            &a,
            &cancelled,
            "claim",
            json!({ "expected_version": 2, "run": "r1", "stage": "implementing" }),
        ),
        409,
        "not_open",
    );
    let done = new_story(f, client, &project, &feature, json!({}));
    set_status(&done, "done");
    let done = story_path(f, &project, &feature, &done);
    assert_error(
        &act(
            client,
            &a,
            &done,
            "claim",
            json!({ "expected_version": 1, "run": "r1", "stage": "implementing" }),
        ),
        409,
        "not_open",
    );
    // No holder, so not_open must come before not_holder.
    assert_error(
        &act(
            client,
            &a,
            &done,
            "move",
            json!({ "expected_version": 1, "run": "r1", "to": "backlog" }),
        ),
        409,
        "not_open",
    );
    assert!(events_of(&project, "story.changed").is_empty());
}

#[test]
fn the_version_is_checked_before_the_claim_and_the_holder_before_the_criteria() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let a = agent(f);
    let story = new_story(
        f,
        client,
        &project,
        &feature,
        json!({ "criteria": [{ "text": "a" }] }),
    );
    set_status(&story, "reviewed");
    let path = story_path(f, &project, &feature, &story);
    must(
        act(
            client,
            &a,
            &path,
            "claim",
            json!({ "expected_version": 1, "run": "r1", "stage": "testing" }),
        ),
        200,
        "r1 claims",
    );
    let stale = act(
        client,
        &a,
        &path,
        "claim",
        json!({ "expected_version": 1, "run": "r2", "stage": "testing" }),
    );
    assert_eq!(stale.status, 409, "body: {}", stale.text());
    assert_eq!(
        stale.json(),
        json!({ "error": "conflict", "current_version": 2 })
    );
    let stale = act(
        client,
        &a,
        &path,
        "move",
        json!({ "expected_version": 1, "run": "r2", "to": "done" }),
    );
    assert_eq!(stale.status, 409, "body: {}", stale.text());
    assert_eq!(
        stale.json(),
        json!({ "error": "conflict", "current_version": 2 })
    );
    let stale = act(
        client,
        &a,
        &path,
        "release",
        json!({ "expected_version": 1, "run": "r1" }),
    );
    assert_eq!(stale.status, 409, "body: {}", stale.text());
    assert_eq!(
        stale.json(),
        json!({ "error": "conflict", "current_version": 2 })
    );
    assert_error(
        &act(
            client,
            &a,
            &path,
            "move",
            json!({ "expected_version": 2, "run": "r2", "to": "done" }),
        ),
        409,
        "not_holder",
    );
    let closed = new_story(f, client, &project, &feature, json!({}));
    set_status(&closed, "done");
    let stale = act(
        client,
        &a,
        &story_path(f, &project, &feature, &closed),
        "claim",
        json!({ "expected_version": 7, "run": "r1", "stage": "testing" }),
    );
    assert_eq!(stale.status, 409, "body: {}", stale.text());
    assert_eq!(
        stale.json(),
        json!({ "error": "conflict", "current_version": 1 })
    );
    let held = get_story(f, client, &project, &feature, &story);
    assert_eq!(held["version"], 2, "{held}");
    assert_eq!(held["holder"]["run"], "r1", "{held}");
}

#[test]
fn a_session_is_refused_on_claim_release_and_move() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = new_story(f, client, &project, &feature, json!({}));
    let path = story_path(f, &project, &feature, &story);
    let h = human(f);
    assert_error(
        &act(
            client,
            &h,
            &path,
            "claim",
            json!({ "expected_version": 1, "run": "r1", "stage": "implementing" }),
        ),
        403,
        "forbidden",
    );
    assert_error(
        &act(
            client,
            &h,
            &path,
            "release",
            json!({ "expected_version": 1, "run": "r1" }),
        ),
        403,
        "forbidden",
    );
    assert_error(
        &act(
            client,
            &h,
            &path,
            "move",
            json!({ "expected_version": 1, "run": "r1", "to": "review" }),
        ),
        403,
        "forbidden",
    );
    let unchanged = get_story(f, client, &project, &feature, &story);
    assert_eq!(unchanged["version"], 1, "{unchanged}");
    assert_eq!(unchanged["status"], "backlog", "{unchanged}");
}

// ---------------------------------------------------------------------------
// Ticks, findings, decisions, files, blocking and the human's check
// ---------------------------------------------------------------------------

/// Creates a story with `criteria` waiting in `implemented` and has run `r1`
/// claim it for review; returns its path. The story is at version 2.
fn held_story(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: &str,
    criteria: Value,
) -> String {
    let story = new_story(f, client, project, feature, json!({ "criteria": criteria }));
    set_status(&story, "implemented");
    let path = story_path(f, project, feature, &story);
    must(
        act(
            client,
            &agent(f),
            &path,
            "claim",
            json!({ "expected_version": 1, "run": "r1", "stage": "review" }),
        ),
        200,
        "r1 claims",
    );
    path
}

#[test]
fn the_holder_ticks_records_findings_decisions_and_changed_files() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let path = held_story(
        f,
        client,
        &project,
        &feature,
        json!([{ "text": "a", "human_check": true }, { "text": "b" }]),
    );
    let a = agent(f);
    let ticked = must(
        act(
            client,
            &a,
            &path,
            "tick",
            json!({ "expected_version": 2, "run": "r1", "criterion": 0, "ticked": true, "pending": true }),
        ),
        200,
        "tick criterion 0",
    )["story"]
        .clone();
    assert_eq!(
        ticked["criteria"],
        json!([
            { "text": "a", "ticked": true, "human_check": true, "pending": true },
            { "text": "b", "ticked": false, "human_check": false, "pending": false },
        ])
    );
    assert_eq!(ticked["version"], 3, "{ticked}");
    let stale = act(
        client,
        &a,
        &path,
        "tick",
        json!({ "expected_version": 2, "run": "r1", "criterion": 1, "ticked": true }),
    );
    assert_eq!(stale.status, 409, "body: {}", stale.text());
    assert_eq!(
        stale.json(),
        json!({ "error": "conflict", "current_version": 3 })
    );
    assert_error(
        &act(
            client,
            &a,
            &path,
            "tick",
            json!({ "expected_version": 3, "run": "r1", "criterion": 2, "ticked": true }),
        ),
        422,
        "invalid_criterion",
    );
    assert_error(
        &act(
            client,
            &a,
            &path,
            "tick",
            json!({ "expected_version": 3, "run": "r1", "criterion": 9, "ticked": true }),
        ),
        422,
        "invalid_criterion",
    );
    must(
        act(
            client,
            &a,
            &path,
            "findings",
            json!({ "expected_version": 3, "run": "r1", "stage": "review", "text": "first", "files": ["a.rs"] }),
        ),
        200,
        "record a finding",
    );
    let found = must(
        act(
            client,
            &a,
            &path,
            "findings",
            json!({ "expected_version": 4, "run": "r1", "stage": "testing", "text": "second", "files": [] }),
        ),
        200,
        "record a second finding",
    )["story"]
        .clone();
    assert_eq!(
        found["findings"],
        json!([
            { "stage": "review", "text": "first", "files": ["a.rs"], "resolved": false },
            { "stage": "testing", "text": "second", "files": [], "resolved": false },
        ])
    );
    let resolved = must(
        act(
            client,
            &a,
            &path,
            "findings/0/resolve",
            json!({ "expected_version": 5, "run": "r1" }),
        ),
        200,
        "resolve finding 0",
    )["story"]
        .clone();
    assert_eq!(
        resolved["findings"],
        json!([
            { "stage": "review", "text": "first", "files": ["a.rs"], "resolved": true },
            { "stage": "testing", "text": "second", "files": [], "resolved": false },
        ])
    );
    must(
        act(
            client,
            &a,
            &path,
            "files",
            json!({ "expected_version": 6, "run": "r1", "files_changed": ["x.rs", "y.rs"] }),
        ),
        200,
        "set the changed files",
    );
    let files = must(
        act(
            client,
            &a,
            &path,
            "files",
            json!({ "expected_version": 7, "run": "r1", "files_changed": ["a.rs"] }),
        ),
        200,
        "replace the changed files",
    )["story"]
        .clone();
    assert_eq!(files["files_changed"], json!(["a.rs"]), "{files}");
    let decided = must(
        act(
            client,
            &a,
            &path,
            "decisions",
            json!({ "expected_version": 8, "run": "r1", "question": "Which?", "choice": "This", "by": "agent" }),
        ),
        200,
        "record a decision",
    )["story"]
        .clone();
    assert_eq!(
        decided["decisions"],
        json!([{ "question": "Which?", "choice": "This", "by": "agent" }])
    );
    assert_eq!(decided["version"], 9, "{decided}");
    assert_eq!(
        events_of(&project, "story.changed").len(),
        8,
        "one event for the claim and one per accepted write"
    );
}

#[test]
fn another_run_cannot_tick_find_resolve_decide_or_set_files_on_a_held_story() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let path = held_story(f, client, &project, &feature, json!([{ "text": "a" }]));
    let a = agent(f);
    must(
        act(
            client,
            &a,
            &path,
            "findings",
            json!({ "expected_version": 2, "run": "r1", "stage": "review", "text": "t", "files": [] }),
        ),
        200,
        "the holder records a finding",
    );
    assert_error(
        &act(
            client,
            &a,
            &path,
            "tick",
            json!({ "expected_version": 3, "run": "r2", "criterion": 0, "ticked": true }),
        ),
        409,
        "not_holder",
    );
    assert_error(
        &act(
            client,
            &a,
            &path,
            "findings",
            json!({ "expected_version": 3, "run": "r2", "stage": "review", "text": "t", "files": [] }),
        ),
        409,
        "not_holder",
    );
    assert_error(
        &act(
            client,
            &a,
            &path,
            "findings/0/resolve",
            json!({ "expected_version": 3, "run": "r2" }),
        ),
        409,
        "not_holder",
    );
    assert_error(
        &act(
            client,
            &a,
            &path,
            "decisions",
            json!({ "expected_version": 3, "run": "r2", "question": "q", "choice": "c", "by": "agent" }),
        ),
        409,
        "not_holder",
    );
    assert_error(
        &act(
            client,
            &a,
            &path,
            "files",
            json!({ "expected_version": 3, "run": "r2", "files_changed": ["a.rs"] }),
        ),
        409,
        "not_holder",
    );
    let unchanged =
        must(call(client, "GET", &path, &a, None), 200, "read the story")["story"].clone();
    assert_eq!(unchanged["version"], 3, "{unchanged}");
    assert_eq!(unchanged["criteria"][0]["ticked"], false, "{unchanged}");
    assert_eq!(
        unchanged["findings"],
        json!([{ "stage": "review", "text": "t", "files": [], "resolved": false }])
    );
    assert_eq!(unchanged["decisions"], json!([]), "{unchanged}");
    assert_eq!(unchanged["files_changed"], json!([]), "{unchanged}");
}

#[test]
fn any_run_blocks_decides_and_unblocks_a_story_nothing_holds() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let story = new_story(f, client, &project, &feature, json!({}));
    let path = story_path(f, &project, &feature, &story);
    let a = agent(f);
    let blocked = must(
        act(
            client,
            &a,
            &path,
            "block",
            json!({ "expected_version": 1, "run": "r9" }),
        ),
        200,
        "block an unheld story",
    )["story"]
        .clone();
    assert_eq!(blocked["blocked"], true, "{blocked}");
    assert_eq!(blocked["status"], "backlog", "{blocked}");
    assert_eq!(blocked["holder"], Value::Null, "{blocked}");
    assert_error(
        &act(
            client,
            &a,
            &path,
            "decisions",
            json!({ "expected_version": 2, "run": "r9", "question": "q", "choice": "c", "by": "robot" }),
        ),
        422,
        "invalid_by",
    );
    let decided = must(
        act(
            client,
            &a,
            &path,
            "decisions",
            json!({ "expected_version": 2, "run": "r9", "question": "Split it?", "choice": "Yes", "by": "human" }),
        ),
        200,
        "decide on an unheld story",
    )["story"]
        .clone();
    assert_eq!(
        decided["decisions"],
        json!([{ "question": "Split it?", "choice": "Yes", "by": "human" }])
    );
    let unblocked = must(
        act(
            client,
            &a,
            &path,
            "unblock",
            json!({ "expected_version": 3, "run": "r8" }),
        ),
        200,
        "unblock",
    )["story"]
        .clone();
    assert_eq!(unblocked["blocked"], false, "{unblocked}");
    assert_eq!(unblocked["version"], 4, "{unblocked}");
}

#[test]
fn the_human_clears_a_pending_check_on_a_done_story_and_only_the_human() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let path = held_story(
        f,
        client,
        &project,
        &feature,
        json!([{ "text": "a", "human_check": true }]),
    );
    let a = agent(f);
    let h = human(f);
    assert_error(
        &act(
            client,
            &h,
            &path,
            "tick",
            json!({ "expected_version": 2, "run": "r1", "criterion": 0, "ticked": true }),
        ),
        403,
        "forbidden",
    );
    must(
        act(
            client,
            &a,
            &path,
            "tick",
            json!({ "expected_version": 2, "run": "r1", "criterion": 0, "ticked": true, "pending": true }),
        ),
        200,
        "tick with the human check pending",
    );
    must(
        act(
            client,
            &a,
            &path,
            "move",
            json!({ "expected_version": 3, "run": "r1", "to": "done" }),
        ),
        200,
        "move to done",
    );
    assert_error(
        &act(
            client,
            &a,
            &path,
            "check",
            json!({ "expected_version": 4, "criterion": 0 }),
        ),
        403,
        "forbidden",
    );
    let checked = must(
        act(
            client,
            &h,
            &path,
            "check",
            json!({ "expected_version": 4, "criterion": 0 }),
        ),
        200,
        "the human clears the check on a done story",
    )["story"]
        .clone();
    assert_eq!(
        checked["criteria"],
        json!([{ "text": "a", "ticked": true, "human_check": true, "pending": false }])
    );
    assert_eq!(checked["status"], "done", "{checked}");
    assert_eq!(checked["version"], 5, "{checked}");
}
