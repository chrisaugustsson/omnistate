//! US-034: posts through the API. Run with `--features stack-tests` in the
//! Compose test container, which reaches Postgres at `postgres:5432` and RustFS
//! at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; threads are those of
//! tests/live_threads.rs and attachments those of tests/attachments.rs):
//!
//! - `POST /threads/{thread_id}/posts` (both) records a post in the thread: 201
//!   `{"post": Post}`. It takes `{"summary", "body"}` and, optionally, `links`,
//!   `attachments`, `confidence` and `replaces`.
//!   - `summary` is one line (no line break) of at most 200 characters and not
//!     blank; `body` is a string of Markdown. Anything else is 400.
//!   - `links` is a list (default `[]`) of `{"kind": "post", "post_id"}` (a
//!     post of the project), `{"kind": "url", "url"}` (an http or https web
//!     source), `{"kind": "node", "doc_id", "address"}` (a doc of the project
//!     and a node address: a node ID of 8 lowercase base32 characters, or
//!     `<instanceId>/<componentNodeId>`; the address is checked for its form
//!     only) and `{"kind": "attachment", "attachment_id"}` (an attachment of
//!     the workspace).
//!   - `attachments` is a list (default `[]`) of IDs of the workspace's
//!     attachments, kept in order.
//!   - `confidence` is a number from 0 to 1, both included (default `null`).
//!   - `replaces` is the ID of an earlier post in the same thread by the same
//!     agent identity, its run and role (default `null`). The earlier post
//!     stays as it was.
//!   - A link, attachment, confidence or replaced post that breaks these rules
//!     is 400 `invalid_request`, and a refused post records nothing and emits
//!     nothing.
//! - A Post is `{"id" (UUIDv7), "thread_id", "author", "summary", "body",
//!   "body_html", "links", "attachments", "confidence", "replaces",
//!   "created_at"}`. `author` is `{"kind": "agent", "run", "role", "model",
//!   "harness"}` (the identity headers of the request, `null` when missing) or
//!   `{"kind": "human", "user_id"}`. `body` is the Markdown as sent; `body_html`
//!   is it rendered by `omni_core::markdown::render`, with raw HTML switched
//!   off, links only to http, https and mailto, and images only from
//!   `attachment:<id>` (tests/markdown.rs of omni-core). `links` and
//!   `attachments` are the lists as given.
//! - Posts are never edited or deleted: no route changes or removes one, and
//!   the API role has no UPDATE, DELETE or TRUNCATE privilege on `posts`.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::registry;
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us034_posts";
const TEST_BUCKET: &str = "omni-us034-posts";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
/// The post types an agent may post.
const AGENT_TYPES: [&str; 7] = [
    "note",
    "finding",
    "position",
    "objection",
    "support",
    "question",
    "verdict",
];
/// The smallest PNG: one transparent pixel.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

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
        // Posts carry attachments, which keep their bytes in RustFS.
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
    alice: String,
    alice_token: String,
    key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us034-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
        let (workspace, alice) = block(async {
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
            (workspace, user)
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
            alice,
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
    Agent {
        key: &'a str,
        run: &'a str,
        role: &'a str,
    },
    Session(&'a str),
}

/// The agent `run-1/moderator`.
fn agent(f: &Fixture) -> Auth<'_> {
    agent_as(f, "run-1", "moderator")
}

fn agent_as<'a>(f: &'a Fixture, run: &'a str, role: &'a str) -> Auth<'a> {
    Auth::Agent {
        key: &f.key,
        run,
        role,
    }
}

fn human(f: &Fixture) -> Auth<'_> {
    Auth::Session(&f.alice_token)
}

fn auth_headers(auth: &Auth) -> Vec<(&'static str, String)> {
    match auth {
        Auth::Agent { key, run, role } => vec![
            ("Authorization", format!("Bearer {key}")),
            ("X-Omni-Run", run.to_string()),
            ("X-Omni-Role", role.to_string()),
            ("X-Omni-Model", "test-model".to_string()),
            ("X-Omni-Harness", "test-harness".to_string()),
        ],
        Auth::Session(token) => vec![("Cookie", cookie(token))],
    }
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let headers = auth_headers(auth);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let body = body.map(Value::to_string);
    client.request(method, path, &headers, body.as_deref())
}

fn must(response: Response, status: u16, what: &str) -> Value {
    assert_eq!(response.status, status, "{what}: {}", response.text());
    response.json()
}

fn assert_error(response: &Response, status: u16, code: &str, what: &str) {
    assert_eq!(response.status, status, "{what}: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": code }),
        "{what}: the error body carries the code and nothing else"
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

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
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
                Some(&json!({ "name": unique("post-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

fn new_thread(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/threads", project_base(f, project)),
                &agent(f),
                Some(&json!({ "question": unique("Which hero layout?") })),
            ),
            201,
            "start a thread",
        )["thread"],
    )
}

/// A project with a thread in it: (project, thread).
fn setting(f: &Fixture, client: &Client) -> (String, String) {
    let project = new_project(f, client);
    let thread = new_thread(f, client, &project);
    (project, thread)
}

fn post_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/posts", thread_path(f, project, thread)),
        auth,
        Some(body),
    )
}

/// Records a post; returns the whole answer.
fn post_answer(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    body: &Value,
) -> Value {
    must(
        post_response(f, client, auth, project, thread, body),
        201,
        &format!("post {body}"),
    )
}

/// Records a post; returns the Post.
fn post(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    body: &Value,
) -> Value {
    post_answer(f, client, auth, project, thread, body)["post"].clone()
}

fn note(summary: &str) -> Value {
    json!({
        "summary": summary,
        "body": format!("Some *reasoning* about {summary}.")
    })
}

/// Uploads a file to the workspace's attachments as the agent; returns its ID.
fn upload(f: &Fixture, client: &Client, bytes: &[u8], filename: &str) -> String {
    let owned = auth_headers(&agent(f));
    let mut headers: Vec<(&str, &str)> = owned.iter().map(|(n, v)| (*n, v.as_str())).collect();
    headers.push(("Content-Type", "application/octet-stream"));
    headers.push(("X-Omni-Filename", filename));
    let response = client.request_bytes(
        "POST",
        &format!("/api/v1/w/{}/attachments", f.workspace),
        &headers,
        Some(bytes),
    );
    id_of(&must(response, 201, "upload an attachment")["attachment"])
}

/// A doc of the project with one block: (doc, the block's node ID).
fn doc_with_node(f: &Fixture, client: &Client, project: &str) -> (String, String) {
    let path = project_base(f, project);
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
    let answer = must(
        call(
            client,
            "POST",
            &format!("{path}/docs/{doc}/ops"),
            &agent(f),
            Some(&json!({ "ops": [{
                "op": "insert",
                "parent": null,
                "position": { "after": null },
                "content": "<p>The hero</p>"
            }] })),
        ),
        200,
        "edit the doc",
    );
    let node = answer["results"][0]["created"][0]
        .as_str()
        .unwrap_or_else(|| panic!("the insert created a node: {answer}"))
        .to_string();
    (doc, node)
}

fn ids(posts: &[Value]) -> Vec<String> {
    posts.iter().map(id_of).collect()
}

/// How many posts the thread has, as the admin role sees them.
fn posts_in(thread: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from posts where thread_id = $1::uuid")
            .bind(thread)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the thread's posts: {e}"))
    })
}

/// The row of a post as text, to tell that it did not change.
fn post_row(post: &str) -> String {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select p::text from posts p where p.id = $1::uuid")
            .bind(post)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("read the post's row: {e}"))
    })
}

/// How many live events of the project there are: a refused request must not
/// add one.
fn event_count(project: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar::<_, i64>("select count(*) from live_events where project_id = $1::uuid")
            .bind(project)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the live events: {e}"))
    })
}

/// Sends every body to the posts route as `auth` and expects each to be
/// refused with `status` and `code`, recording nothing and emitting nothing.
fn assert_refused(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    (project, thread): (&str, &str),
    bodies: &[Value],
    status: u16,
    code: &str,
) {
    let posts_before = posts_in(thread);
    let events_before = event_count(project);
    for body in bodies {
        let response = post_response(f, client, auth, project, thread, body);
        assert_error(&response, status, code, &body.to_string());
    }
    assert_eq!(posts_in(thread), posts_before, "nothing was recorded");
    assert_eq!(
        event_count(project),
        events_before,
        "a refused post emits nothing"
    );
}

// ---------------------------------------------------------------------------
// What every post has
// ---------------------------------------------------------------------------

#[test]
fn a_posts_author_is_the_agent_identity_of_the_request_or_the_human_user() {
    let (f, client) = ready();
    let (project, thread) = setting(f, client);
    let body = "## Why\n\nThe *left* layout reads better at 320 px.\n";
    let posted = post(
        f,
        client,
        &agent(f),
        &project,
        &thread,
        &json!({ "summary": "Pick the left layout", "body": body }),
    );
    assert_eq!(posted["thread_id"], thread.as_str(), "{posted}");
    assert_eq!(posted["summary"], "Pick the left layout", "{posted}");
    assert_eq!(
        posted["body"], body,
        "the Markdown is kept as sent: {posted}"
    );
    assert_eq!(
        posted["author"],
        json!({
            "kind": "agent",
            "run": "run-1",
            "role": "moderator",
            "model": "test-model",
            "harness": "test-harness"
        }),
        "the author is the agent identity of the request: {posted}"
    );

    let by_human = post(
        f,
        client,
        &human(f),
        &project,
        &thread,
        &json!({ "summary": "Why not the right one?", "body": "Curious." }),
    );
    assert_eq!(
        by_human["author"],
        json!({ "kind": "human", "user_id": f.alice }),
        "the author is the human user: {by_human}"
    );
}

#[test]
fn the_summary_is_one_line_of_at_most_200_characters_and_the_body_is_required() {
    let (f, client) = ready();
    let (project, thread) = setting(f, client);
    let longest = "é".repeat(200);
    let posted = post(
        f,
        client,
        &agent(f),
        &project,
        &thread,
        &json!({ "summary": longest, "body": "Long." }),
    );
    assert_eq!(
        posted["summary"],
        longest.as_str(),
        "200 characters fit: {posted}"
    );
    assert_refused(
        f,
        client,
        &agent(f),
        (&project, &thread),
        &[
            json!({ "summary": "é".repeat(201), "body": "Long." }),
            json!({ "summary": "Two\nlines", "body": "Long." }),
            json!({ "summary": "", "body": "Long." }),
            json!({ "summary": "   ", "body": "Long." }),
            json!({ "body": "Long." }),
            json!({ "summary": 7, "body": "Long." }),
            json!({ "summary": "No body" }),
            json!({ "summary": "No body", "body": null }),
            json!({ "summary": "No body", "body": 7 }),
        ],
        400,
        "invalid_request",
    );
}

// ---------------------------------------------------------------------------
// Links, attachments, confidence and the post it replaces
// ---------------------------------------------------------------------------

#[test]
fn a_post_may_link_to_posts_web_sources_nodes_and_attachments() {
    let (f, client) = ready();
    let (project, thread) = setting(f, client);
    let earlier = id_of(&post(
        f,
        client,
        &agent(f),
        &project,
        &thread,
        &note("Earlier"),
    ));
    // A post of another thread of the project may be linked too.
    let other_thread = new_thread(f, client, &project);
    let elsewhere = id_of(&post(
        f,
        client,
        &agent(f),
        &project,
        &other_thread,
        &note("Elsewhere"),
    ));
    let (doc, node) = doc_with_node(f, client, &project);
    let attachment = upload(f, client, PNG, "hero.png");
    let links = json!([
        { "kind": "post", "post_id": earlier },
        { "kind": "post", "post_id": elsewhere },
        { "kind": "url", "url": "https://example.com/research?q=hero" },
        { "kind": "url", "url": "http://example.com/old" },
        { "kind": "node", "doc_id": doc, "address": node },
        { "kind": "node", "doc_id": doc, "address": "a0000024/b0000031" },
        { "kind": "attachment", "attachment_id": attachment }
    ]);
    let posted = post(
        f,
        client,
        &agent(f),
        &project,
        &thread,
        &json!({ "summary": "Sources", "body": "See the links.", "links": links }),
    );
    assert_eq!(posted["links"], links, "the links, as given: {posted}");
}

#[test]
fn a_link_that_points_nowhere_or_outside_the_project_is_refused() {
    let (f, client) = ready();
    let (project, thread) = setting(f, client);
    let (doc, _) = doc_with_node(f, client, &project);
    // Records of another project of the workspace.
    let (other_project, other_thread) = setting(f, client);
    let foreign_post = id_of(&post(
        f,
        client,
        &agent(f),
        &other_project,
        &other_thread,
        &note("Foreign"),
    ));
    let (foreign_doc, _) = doc_with_node(f, client, &other_project);
    let bodies: Vec<Value> = [
        json!({ "kind": "post", "post_id": UNKNOWN_ID }),
        json!({ "kind": "post", "post_id": foreign_post }),
        json!({ "kind": "post", "post_id": "not-a-uuid" }),
        json!({ "kind": "post" }),
        json!({ "kind": "url", "url": "javascript:alert(1)" }),
        json!({ "kind": "url", "url": "ftp://example.com/file" }),
        json!({ "kind": "url", "url": "data:text/html,hi" }),
        json!({ "kind": "url", "url": "/api/v1/w/x" }),
        json!({ "kind": "url", "url": "" }),
        json!({ "kind": "node", "doc_id": UNKNOWN_ID, "address": "a0000024" }),
        json!({ "kind": "node", "doc_id": foreign_doc, "address": "a0000024" }),
        json!({ "kind": "node", "doc_id": doc, "address": "A0000024" }),
        json!({ "kind": "node", "doc_id": doc, "address": "a000002" }),
        json!({ "kind": "node", "doc_id": doc, "address": "a000002i" }),
        json!({ "kind": "node", "doc_id": doc, "address": "a0000024/" }),
        json!({ "kind": "node", "doc_id": doc, "address": "" }),
        json!({ "kind": "node", "doc_id": doc }),
        json!({ "kind": "attachment", "attachment_id": UNKNOWN_ID }),
        json!({ "kind": "file", "url": "https://example.com" }),
        json!("https://example.com"),
    ]
    .into_iter()
    .map(|link| json!({ "summary": "Bad link", "body": "x", "links": [link] }))
    .chain([json!({
        "summary": "Links that are no list",
        "body": "x",
        "links": { "kind": "url", "url": "https://example.com" }
    })])
    .collect();
    assert_refused(
        f,
        client,
        &agent(f),
        (&project, &thread),
        &bodies,
        400,
        "invalid_request",
    );
}

#[test]
fn a_post_may_carry_attachments_and_a_confidence_between_0_and_1() {
    let (f, client) = ready();
    let (project, thread) = setting(f, client);
    let sketch = upload(f, client, PNG, "sketch.png");
    let notes = upload(f, client, b"plain notes", "notes.txt");
    let posted = post(
        f,
        client,
        &agent(f),
        &project,
        &thread,
        &json!({
            "summary": "With evidence",
            "body": "See the files.",
            "attachments": [notes, sketch],
            "confidence": 0.75
        }),
    );
    assert_eq!(
        posted["attachments"],
        json!([notes, sketch]),
        "the attachments, in order: {posted}"
    );
    assert_eq!(posted["confidence"], 0.75, "{posted}");
    for confidence in [json!(0), json!(1), json!(0.0), json!(1.0)] {
        let posted = post(
            f,
            client,
            &agent(f),
            &project,
            &thread,
            &json!({ "summary": "Sure", "body": "x", "confidence": confidence }),
        );
        assert_eq!(
            posted["confidence"].as_f64(),
            confidence.as_f64(),
            "{posted}"
        );
    }

    assert_refused(
        f,
        client,
        &agent(f),
        (&project, &thread),
        &[
            json!({ "summary": "x", "body": "x", "confidence": -0.01 }),
            json!({ "summary": "x", "body": "x", "confidence": 1.01 }),
            json!({ "summary": "x", "body": "x", "confidence": 75 }),
            json!({ "summary": "x", "body": "x", "confidence": "0.5" }),
            json!({ "summary": "x", "body": "x", "attachments": [UNKNOWN_ID] }),
            json!({ "summary": "x", "body": "x", "attachments": [sketch, "nope"] }),
            json!({ "summary": "x", "body": "x", "attachments": sketch }),
        ],
        400,
        "invalid_request",
    );
}

#[test]
fn a_post_may_replace_an_earlier_post_by_the_same_agent_identity() {
    let (f, client) = ready();
    let (project, thread) = setting(f, client);
    let first = id_of(&post(
        f,
        client,
        &agent(f),
        &project,
        &thread,
        &json!({ "summary": "Left layout", "body": "First take." }),
    ));
    let row_before = post_row(&first);
    let second = post(
        f,
        client,
        &agent(f),
        &project,
        &thread,
        &json!({
            "summary": "Left layout, with a smaller hero",
            "body": "Second take.",
            "replaces": first
        }),
    );
    assert_eq!(second["replaces"], first.as_str(), "{second}");
    assert_ne!(
        id_of(&second),
        first,
        "the replacement is a post of its own"
    );
    assert_eq!(
        post_row(&first),
        row_before,
        "the replaced post stays as it was"
    );

    let by_critic = id_of(&post(
        f,
        client,
        &agent_as(f, "run-1", "critic"),
        &project,
        &thread,
        &note("The critic's take"),
    ));
    let by_human = id_of(&post(
        f,
        client,
        &human(f),
        &project,
        &thread,
        &note("The human's take"),
    ));
    let other_thread = new_thread(f, client, &project);
    let in_other_thread = id_of(&post(
        f,
        client,
        &agent(f),
        &project,
        &other_thread,
        &note("Another thread"),
    ));
    let bodies: Vec<Value> = [
        json!(by_critic),
        json!(by_human),
        json!(in_other_thread),
        json!(UNKNOWN_ID),
        json!("not-a-uuid"),
        json!(7),
    ]
    .into_iter()
    .map(|replaces| {
        json!({ "summary": "Replacing", "body": "x", "replaces": replaces })
    })
    .collect();
    assert_refused(
        f,
        client,
        &agent(f),
        (&project, &thread),
        &bodies,
        400,
        "invalid_request",
    );
    // Another run with the same role is another identity too.
    assert_refused(
        f,
        client,
        &agent_as(f, "run-2", "moderator"),
        (&project, &thread),
        &[json!({ "summary": "Replacing", "body": "x", "replaces": first })],
        400,
        "invalid_request",
    );
}

// ---------------------------------------------------------------------------
// Never edited, never deleted
// ---------------------------------------------------------------------------

#[test]
fn posts_can_never_be_edited_or_deleted() {
    for entry in registry() {
        let names_a_post = entry.path.contains("/posts");
        assert!(
            !names_a_post || entry.method == "GET" || entry.method == "POST",
            "{} {} could change or remove a post",
            entry.method,
            entry.path
        );
        assert!(
            !(names_a_post && entry.path.contains("{post_id}") && entry.method == "POST"),
            "{} {} could change a post",
            entry.method,
            entry.path
        );
    }

    let (f, client) = ready();
    let (project, thread) = setting(f, client);
    let posted = id_of(&post(
        f,
        client,
        &agent(f),
        &project,
        &thread,
        &note("Final"),
    ));
    let row = post_row(&posted);
    let path = format!("{}/posts/{posted}", thread_path(f, &project, &thread));
    for method in ["PUT", "PATCH", "DELETE"] {
        for auth in [agent(f), human(f)] {
            let response = call(
                client,
                method,
                &path,
                &auth,
                Some(&json!({ "summary": "Edited", "body": "Edited" })),
            );
            assert!(
                matches!(response.status, 404 | 405),
                "{method} {path}: {} {}",
                response.status,
                response.text()
            );
        }
    }
    assert_eq!(post_row(&posted), row, "the post did not change");

    block(async {
        let mut conn = admin_connection().await;
        for privilege in ["UPDATE", "DELETE", "TRUNCATE"] {
            let granted: bool =
                sqlx::query_scalar("select has_table_privilege('omni_api', 'public.posts', $1)")
                    .bind(privilege)
                    .fetch_one(&mut conn)
                    .await
                    .expect("read the privilege");
            assert!(!granted, "omni_api has the {privilege} privilege on posts");
        }
    });
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

#[test]
fn the_body_is_rendered_without_raw_html_and_with_safe_links_and_images() {
    let (f, client) = ready();
    let (project, thread) = setting(f, client);
    let sketch = upload(f, client, PNG, "sketch.png");
    let body = format!(
        "Some **bold** text. <script>alert(1)</script> <b onclick=\"x()\">raw</b>\n\n\
         [a source](https://example.com/a) and [a trap](javascript:alert(1))\n\n\
         ![the sketch](attachment:{sketch}) ![a tracker](https://example.com/pixel.png)\n"
    );
    let posted = post(
        f,
        client,
        &agent(f),
        &project,
        &thread,
        &json!({ "summary": "Rendered", "body": body }),
    );
    assert_eq!(
        posted["body"],
        body.as_str(),
        "the Markdown is kept: {posted}"
    );
    let html = posted["body_html"]
        .as_str()
        .unwrap_or_else(|| panic!("a post has `body_html`: {posted}"));
    assert!(html.contains("<strong>bold</strong>"), "{html}");
    let lower = html.to_ascii_lowercase();
    assert!(
        !lower.contains("<script") && !lower.contains("<b "),
        "raw HTML is switched off: {html}"
    );
    assert!(
        html.contains("href=\"https://example.com/a\""),
        "an https link keeps its target: {html}"
    );
    assert!(
        !lower.contains("javascript:alert"),
        "a javascript: link loses its target: {html}"
    );
    assert!(
        html.contains(&format!("src=\"attachment:{sketch}\"")),
        "an image from an attachment is drawn: {html}"
    );
    assert!(
        !html.contains("src=\"https://example.com/pixel.png\""),
        "an image from the web is not drawn: {html}"
    );
}
