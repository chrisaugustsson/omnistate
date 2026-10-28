//! US-033: threads and phases through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; change sets are those of
//! US-016, tests/changesets.rs, and approving and merging is US-025,
//! tests/approve_merge.rs):
//!
//! - `POST /changesets/{changeset_id}/threads` (agent) takes `{"question",
//!   "criteria", "sealed"}` and starts a thread in the change set: 201
//!   `{"thread": Thread}`. `question` is a non-blank string, `criteria` a list
//!   of strings (one decision criterion each, in order) and `sealed` an
//!   optional boolean, `false` when missing. A missing or blank question, or
//!   criteria that are missing or not a list of strings, is 400 `{"error":
//!   "invalid_request"}` and creates nothing. A change set that the project
//!   does not have is 404 `{"error": "not_found"}`.
//! - A Thread is `{"id" (UUIDv7), "project_id", "changeset_id", "question",
//!   "criteria", "run_tag", "created_at", "phases"}`. `run_tag` is the run of
//!   the agent that started the thread (its `X-Omni-Run`). `phases` lists the
//!   thread's phases in order, each `{"number", "sealed", "started_at"}`,
//!   numbered from 1; the last one is the current phase. A new thread has one
//!   phase, which is sealed if and only if `sealed` was `true`.
//! - `GET /threads/{thread_id}` (both) answers 200 `{"thread": Thread}`.
//! - `POST /threads/{thread_id}/phases` (agent) takes `{"sealed"}`, a
//!   required boolean, and starts the thread's next phase, which becomes the
//!   current one: 201 `{"thread": Thread}`. Anything else in `sealed` is 400
//!   `invalid_request` and starts nothing.
//! - A thread that the project does not have is 404 `not_found` on every
//!   thread route.
//! - `POST /threads/{thread_id}/posts` (both) takes at least `{"type",
//!   "summary", "body"}` and records a post in the thread's current phase: 201
//!   `{"post": Post}`, where a Post has at least `{"id", "thread_id", "phase",
//!   "type", "summary", "body"}` and `phase` is the number of the phase it was
//!   posted in. US-034 adds the rest of the post's rules; this story pins that
//!   a thread keeps accepting posts, and new phases, after its change set
//!   merged, so builders can raise problems with the plan.
//! - Starting a thread emits `thread.created` with `data` that has at least
//!   `{"thread_id", "changeset_id"}`. Starting a phase with the phases route
//!   emits `phase.started` with `data` that has at least `{"thread_id",
//!   "changeset_id", "phase", "sealed"}`, where `phase` is the new phase's
//!   number. A refused request emits nothing.
//! - Threads live in the tenant table `threads`, whose columns without a
//!   default are `workspace_id`, `project_id`, `changeset_id`, `question` and
//!   `criteria` (jsonb, the list of strings). Phases may live in `threads`; if
//!   they live in a tenant table `thread_phases`, its columns without a default
//!   are `workspace_id`, `thread_id`, `number` and `sealed`. Posts live in the
//!   append-only tenant table `posts` (no UPDATE privilege), whose columns
//!   without a default are `workspace_id`, `thread_id`, `phase`, `type`,
//!   `summary`, `body` and `author` (jsonb). tests/workspace_delete.rs and the
//!   isolation suite seed them.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us033_threads";
const TEST_BUCKET: &str = "omni-us033-threads";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const THREADS_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/threads";
const THREAD_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}";
const PHASES_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/phases";
const POSTS_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/posts";

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

fn s3_endpoint() -> String {
    std::env::var("OMNI_S3_ENDPOINT").unwrap_or_else(|_| "http://rustfs:9000".to_string())
}

fn s3_access_key() -> String {
    std::env::var("OMNI_S3_ACCESS_KEY").unwrap_or_else(|_| "omni-dev".to_string())
}

fn s3_secret_key() -> String {
    std::env::var("OMNI_S3_SECRET_KEY").unwrap_or_else(|_| "omni-dev-secret".to_string())
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
        // Merging a change set saves versions, which keep their content in
        // RustFS, so the API gets a real bucket.
        s3_endpoint: s3_endpoint(),
        s3_access_key: s3_access_key(),
        s3_secret_key: s3_secret_key(),
        s3_bucket: TEST_BUCKET.to_string(),
    }
}

async fn ensure_bucket() {
    let region = Region::Custom {
        region: "us-east-1".to_string(),
        endpoint: s3_endpoint(),
    };
    let credentials = Credentials::new(
        Some(&s3_access_key()),
        Some(&s3_secret_key()),
        None,
        None,
        None,
    )
    .expect("storage credentials");
    let bucket = Bucket::new(TEST_BUCKET, region.clone(), credentials.clone())
        .expect("bucket settings")
        .with_path_style();
    if !bucket.exists().await.unwrap_or(false) {
        Bucket::create_with_path_style(
            TEST_BUCKET,
            region,
            credentials,
            BucketConfiguration::default(),
        )
        .await
        .unwrap_or_else(|e| panic!("cannot create the scratch bucket: {e}"));
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
    format!("us033-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
            ensure_bucket().await;
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
            ("X-Omni-Role", "moderator".to_string()),
            ("X-Omni-Model", "test-model".to_string()),
            ("X-Omni-Harness", "test-harness".to_string()),
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

fn id_of(value: &Value) -> String {
    value["id"]
        .as_str()
        .unwrap_or_else(|| panic!("there is an `id`: {value}"))
        .to_string()
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
}

fn changeset_path(f: &Fixture, project: &str, changeset: &str) -> String {
    format!("{}/changesets/{changeset}", project_base(f, project))
}

fn thread_path(f: &Fixture, project: &str, thread: &str) -> String {
    format!("{}/threads/{thread}", project_base(f, project))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &agent(f),
                Some(&json!({ "name": unique("thread-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

fn new_changeset(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/changesets", project_base(f, project)),
                &agent(f),
                Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
            ),
            201,
            "create a change set",
        )["changeset"],
    )
}

fn create_thread_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/threads", changeset_path(f, project, changeset)),
        &agent(f),
        Some(body),
    )
}

/// Starts a thread in the change set; returns the Thread of the answer.
fn create_thread(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    body: &Value,
) -> Value {
    must(
        create_thread_response(f, client, project, changeset, body),
        201,
        "start a thread",
    )["thread"]
        .clone()
}

fn thread_body(question: &str, sealed: Option<bool>) -> Value {
    let mut body = json!({
        "question": question,
        "criteria": [
            "Fits the brand's tone",
            "Readable at 320 px",
            "Costs no more than one sprint"
        ]
    });
    if let Some(sealed) = sealed {
        body["sealed"] = json!(sealed);
    }
    body
}

fn start_phase_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    thread: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/phases", thread_path(f, project, thread)),
        &agent(f),
        Some(body),
    )
}

/// Starts the thread's next phase; returns the Thread of the answer.
fn start_phase(f: &Fixture, client: &Client, project: &str, thread: &str, sealed: bool) -> Value {
    must(
        start_phase_response(f, client, project, thread, &json!({ "sealed": sealed })),
        201,
        "start the next phase",
    )["thread"]
        .clone()
}

fn thread_of(f: &Fixture, client: &Client, auth: &Auth, project: &str, thread: &str) -> Value {
    must(
        call(client, "GET", &thread_path(f, project, thread), auth, None),
        200,
        "show the thread",
    )["thread"]
        .clone()
}

fn post_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    summary: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/posts", thread_path(f, project, thread)),
        auth,
        Some(&json!({
            "type": "question",
            "summary": summary,
            "body": format!("Raised while building: {summary}")
        })),
    )
}

/// The phases of a Thread as (number, sealed), in the order listed.
fn phases(thread: &Value) -> Vec<(u64, bool)> {
    thread["phases"]
        .as_array()
        .unwrap_or_else(|| panic!("a thread lists its `phases`: {thread}"))
        .iter()
        .map(|phase| {
            assert!(
                phase["started_at"].is_string(),
                "a phase has `started_at`: {phase}"
            );
            (
                phase["number"]
                    .as_u64()
                    .unwrap_or_else(|| panic!("a phase has a `number`: {phase}")),
                phase["sealed"]
                    .as_bool()
                    .unwrap_or_else(|| panic!("a phase has `sealed`: {phase}")),
            )
        })
        .collect()
}

/// The live events of a project named `event` for one thread, oldest first,
/// as (seq, data).
fn events_of(project: &str, thread: &str, event: &str) -> Vec<(i64, Value)> {
    block(async {
        let mut conn = admin_connection().await;
        let rows = sqlx::query_as::<_, (i64, String)>(
            "select seq, data::jsonb::text from live_events \
             where project_id = $1::uuid and event = $2 order by seq",
        )
        .bind(project)
        .bind(event)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the live events: {e}"));
        rows.iter()
            .map(|(seq, text)| (*seq, parse(text)))
            .filter(|(_, data)| data["thread_id"] == thread)
            .collect()
    })
}

/// How many live events of the project carry a `thread_id`: a refused
/// request must not add one.
fn thread_event_count(project: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar::<_, i64>(
            "select count(*) from live_events \
             where project_id = $1::uuid and data ? 'thread_id'",
        )
        .bind(project)
        .fetch_one(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("count the live events: {e}"))
    })
}

/// How many threads the change set has, as the admin role sees them.
fn threads_in(changeset: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar::<_, i64>("select count(*) from threads where changeset_id = $1::uuid")
            .bind(changeset)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the threads: {e}"))
    })
}

/// The change set merged through a doc of its own: started, saved, submitted
/// and approved by the human.
fn merge(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let path = changeset_path(f, project, changeset);
    let doc = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{path}/docs"),
                &agent(f),
                Some(&json!({ "kind": "html", "pillar": "spec", "title": unique("Plan") })),
            ),
            201,
            "create a doc",
        )["doc"],
    );
    let draft = format!("{path}/docs/{doc}");
    must(
        call(
            client,
            "POST",
            &format!("{draft}/ops"),
            &agent(f),
            Some(&json!({ "ops": [{
                "op": "insert",
                "parent": null,
                "position": { "after": null },
                "content": format!("<p>{}</p>", unique("The plan"))
            }] })),
        ),
        200,
        "edit the draft",
    );
    must(
        call(
            client,
            "POST",
            &format!("{draft}/versions"),
            &agent(f),
            Some(&json!({})),
        ),
        201,
        "save a version",
    );
    must(
        call(client, "POST", &format!("{path}/submit"), &agent(f), None),
        200,
        "submit the change set",
    );
    let approved = must(
        call(
            client,
            "POST",
            &format!("{path}/approve"),
            &human(f),
            Some(&json!({})),
        ),
        200,
        "the human approves the change set",
    );
    assert_eq!(
        approved["changeset"]["status"], "merged",
        "set up a merged change set: {approved}"
    );
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_thread_routes_are_in_the_registry_with_who_may_call_them() {
    for (method, path, access) in [
        ("POST", THREADS_ROUTE, Access::Agent),
        ("GET", THREAD_ROUTE, Access::Both),
        ("POST", PHASES_ROUTE, Access::Agent),
        ("POST", POSTS_ROUTE, Access::Both),
    ] {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
    }
}

// ---------------------------------------------------------------------------
// Starting a thread
// ---------------------------------------------------------------------------

#[test]
fn a_thread_records_its_question_and_criteria_and_starts_with_an_open_phase() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let question = unique("Which hero layout should the landing page use?");
    let thread = create_thread(
        f,
        client,
        &project,
        &changeset,
        &thread_body(&question, None),
    );

    assert_uuid_v7(&thread["id"]);
    assert_eq!(thread["project_id"], project.as_str(), "{thread}");
    assert_eq!(thread["changeset_id"], changeset.as_str(), "{thread}");
    assert_eq!(thread["question"], question.as_str(), "{thread}");
    assert_eq!(
        thread["criteria"],
        json!([
            "Fits the brand's tone",
            "Readable at 320 px",
            "Costs no more than one sprint"
        ]),
        "the criteria are kept in order: {thread}"
    );
    assert_eq!(
        thread["run_tag"], "run-1",
        "the run tag is the run of the agent that started it: {thread}"
    );
    assert!(thread["created_at"].is_string(), "{thread}");
    assert_eq!(
        phases(&thread),
        vec![(1, false)],
        "a thread starts with one open phase"
    );

    // The same thread, read back by the agent and by the human.
    let id = id_of(&thread);
    for auth in [agent(f), human(f)] {
        let shown = thread_of(f, client, &auth, &project, &id);
        assert_eq!(shown["id"], id.as_str(), "{shown}");
        assert_eq!(shown["question"], thread["question"], "{shown}");
        assert_eq!(shown["criteria"], thread["criteria"], "{shown}");
        assert_eq!(shown["changeset_id"], changeset.as_str(), "{shown}");
        assert_eq!(phases(&shown), vec![(1, false)], "{shown}");
    }
}

#[test]
fn a_thread_started_sealed_starts_with_a_sealed_phase() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let thread = create_thread(
        f,
        client,
        &project,
        &changeset,
        &thread_body(&unique("Which checkout flow wins?"), Some(true)),
    );
    assert_eq!(
        phases(&thread),
        vec![(1, true)],
        "`sealed: true` seals the first phase"
    );
    let shown = thread_of(f, client, &agent(f), &project, &id_of(&thread));
    assert_eq!(phases(&shown), vec![(1, true)], "{shown}");
}

#[test]
fn a_thread_needs_a_question_and_a_list_of_criteria() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let events_before = thread_event_count(&project);
    for body in [
        json!({ "criteria": ["Readable"] }),
        json!({ "question": "", "criteria": ["Readable"] }),
        json!({ "question": "   ", "criteria": ["Readable"] }),
        json!({ "question": 42, "criteria": ["Readable"] }),
        json!({ "question": "Which layout?" }),
        json!({ "question": "Which layout?", "criteria": "Readable" }),
        json!({ "question": "Which layout?", "criteria": ["Readable", 7] }),
        json!({ "question": "Which layout?", "criteria": ["Readable"], "sealed": "yes" }),
    ] {
        let response = create_thread_response(f, client, &project, &changeset, &body);
        assert_eq!(response.status, 400, "{body}: {}", response.text());
        assert_eq!(
            response.json()["error"],
            "invalid_request",
            "{body}: {}",
            response.text()
        );
    }
    assert_eq!(threads_in(&changeset), 0, "no thread was started");
    assert_eq!(
        thread_event_count(&project),
        events_before,
        "a refused request emits nothing"
    );
}

#[test]
fn a_thread_needs_a_change_set_of_its_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let other_project = new_project(f, client);
    let foreign_changeset = new_changeset(f, client, &other_project);
    let body = thread_body(&unique("Lost question"), None);

    let response = create_thread_response(f, client, &project, UNKNOWN_ID, &body);
    assert_error(&response, 404, "not_found");
    let response = create_thread_response(f, client, &project, &foreign_changeset, &body);
    assert_error(&response, 404, "not_found");
    assert_eq!(
        threads_in(&foreign_changeset),
        0,
        "nothing was started in the other project's change set"
    );
}

#[test]
fn a_thread_that_the_project_does_not_have_is_not_found_on_every_thread_route() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let other_project = new_project(f, client);
    let changeset = new_changeset(f, client, &other_project);
    let elsewhere = id_of(&create_thread(
        f,
        client,
        &other_project,
        &changeset,
        &thread_body(&unique("Elsewhere"), None),
    ));

    for thread in [UNKNOWN_ID, elsewhere.as_str()] {
        let response = call(
            client,
            "GET",
            &thread_path(f, &project, thread),
            &agent(f),
            None,
        );
        assert_error(&response, 404, "not_found");
        let response =
            start_phase_response(f, client, &project, thread, &json!({ "sealed": true }));
        assert_error(&response, 404, "not_found");
        let response = post_response(f, client, &agent(f), &project, thread, "Anyone there?");
        assert_error(&response, 404, "not_found");
    }
    let untouched = thread_of(f, client, &agent(f), &other_project, &elsewhere);
    assert_eq!(
        phases(&untouched),
        vec![(1, false)],
        "the thread under the other project got no phase: {untouched}"
    );
}

// ---------------------------------------------------------------------------
// Phases
// ---------------------------------------------------------------------------

#[test]
fn starting_a_phase_appends_the_next_phase_as_the_current_one() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let thread = id_of(&create_thread(
        f,
        client,
        &project,
        &changeset,
        &thread_body(&unique("Which nav pattern?"), None),
    ));

    let sealed = start_phase(f, client, &project, &thread, true);
    assert_eq!(sealed["id"], thread.as_str(), "{sealed}");
    assert_eq!(
        phases(&sealed),
        vec![(1, false), (2, true)],
        "the sealed phase follows the open one"
    );
    let open = start_phase(f, client, &project, &thread, false);
    assert_eq!(
        phases(&open),
        vec![(1, false), (2, true), (3, false)],
        "an open phase follows the sealed one"
    );
    let shown = thread_of(f, client, &human(f), &project, &thread);
    assert_eq!(
        phases(&shown),
        vec![(1, false), (2, true), (3, false)],
        "{shown}"
    );
}

#[test]
fn a_phase_needs_to_say_whether_it_is_sealed() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let thread = id_of(&create_thread(
        f,
        client,
        &project,
        &changeset,
        &thread_body(&unique("Which palette?"), None),
    ));
    let events_before = thread_event_count(&project);
    for body in [
        json!({}),
        json!({ "sealed": null }),
        json!({ "sealed": "true" }),
        json!({ "sealed": 1 }),
    ] {
        let response = start_phase_response(f, client, &project, &thread, &body);
        assert_eq!(response.status, 400, "{body}: {}", response.text());
        assert_eq!(
            response.json()["error"],
            "invalid_request",
            "{body}: {}",
            response.text()
        );
    }
    let shown = thread_of(f, client, &agent(f), &project, &thread);
    assert_eq!(phases(&shown), vec![(1, false)], "no phase was started");
    assert_eq!(
        thread_event_count(&project),
        events_before,
        "a refused request emits nothing"
    );
}

// ---------------------------------------------------------------------------
// After the change set merges
// ---------------------------------------------------------------------------

#[test]
fn a_thread_keeps_accepting_posts_and_phases_after_its_change_set_merges() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let thread = id_of(&create_thread(
        f,
        client,
        &project,
        &changeset,
        &thread_body(&unique("Which hero layout?"), None),
    ));
    // The control: the thread takes a post while its change set is open.
    let before = must(
        post_response(f, client, &agent(f), &project, &thread, "Before the merge"),
        201,
        "post while the change set is open",
    )["post"]
        .clone();
    assert_eq!(before["thread_id"], thread.as_str(), "{before}");
    assert_eq!(before["phase"], 1, "{before}");

    merge(f, client, &project, &changeset);

    let raised = must(
        post_response(
            f,
            client,
            &agent(f),
            &project,
            &thread,
            "The hero image has no alt text in the plan",
        ),
        201,
        "a builder raises a problem after the merge",
    )["post"]
        .clone();
    assert_eq!(raised["thread_id"], thread.as_str(), "{raised}");
    assert_eq!(raised["phase"], 1, "{raised}");
    assert_eq!(raised["type"], "question", "{raised}");
    assert_eq!(
        raised["summary"], "The hero image has no alt text in the plan",
        "{raised}"
    );
    assert_ne!(raised["id"], before["id"], "a post of its own");

    let human_post = post_response(f, client, &human(f), &project, &thread, "Noted, thanks");
    assert_eq!(
        human_post.status,
        201,
        "the human posts after the merge too: {}",
        human_post.text()
    );

    let next = start_phase(f, client, &project, &thread, false);
    assert_eq!(phases(&next), vec![(1, false), (2, false)], "{next}");
    let later = must(
        post_response(f, client, &agent(f), &project, &thread, "One more problem"),
        201,
        "post in the phase started after the merge",
    )["post"]
        .clone();
    assert_eq!(
        later["phase"], 2,
        "the post lands in the current phase: {later}"
    );

    let stored: i64 = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from posts where thread_id = $1::uuid")
            .bind(&thread)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the thread's posts: {e}"))
    });
    assert_eq!(stored, 4, "every post was recorded");
}

// ---------------------------------------------------------------------------
// Live events
// ---------------------------------------------------------------------------

#[test]
fn starting_a_thread_emits_thread_created() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let thread = id_of(&create_thread(
        f,
        client,
        &project,
        &changeset,
        &thread_body(&unique("Which onboarding flow?"), Some(true)),
    ));
    let created = events_of(&project, &thread, "thread.created");
    assert_eq!(
        created.len(),
        1,
        "one thread.created for the thread: {created:?}"
    );
    let (_, data) = &created[0];
    assert_eq!(data["thread_id"], thread.as_str(), "{data}");
    assert_eq!(data["changeset_id"], changeset.as_str(), "{data}");
}

#[test]
fn starting_a_phase_emits_phase_started_with_its_number_and_whether_it_is_sealed() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let thread = id_of(&create_thread(
        f,
        client,
        &project,
        &changeset,
        &thread_body(&unique("Which pricing page?"), None),
    ));
    start_phase(f, client, &project, &thread, true);
    start_phase(f, client, &project, &thread, false);

    let created = events_of(&project, &thread, "thread.created");
    assert_eq!(created.len(), 1, "{created:?}");
    let started: Vec<(i64, Value)> = events_of(&project, &thread, "phase.started")
        .into_iter()
        .filter(|(_, data)| data["phase"].as_u64().is_some_and(|n| n >= 2))
        .collect();
    assert_eq!(
        started.len(),
        2,
        "one phase.started for each phase that the phases route started: {started:?}"
    );
    for ((seq, data), (number, sealed)) in started.iter().zip([(2, true), (3, false)]) {
        assert_eq!(data["thread_id"], thread.as_str(), "{data}");
        assert_eq!(data["changeset_id"], changeset.as_str(), "{data}");
        assert_eq!(data["phase"], number, "{data}");
        assert_eq!(data["sealed"], sealed, "{data}");
        assert!(
            *seq > created[0].0,
            "phase {number} started after the thread was created"
        );
    }
    assert!(started[0].0 < started[1].0, "the events are in order");
}
