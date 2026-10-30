//! US-103: an agent writes an HTML doc directly. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API (the agent),
//! create a scratch RustFS bucket and serve the API's router on a local port.
//! Docs are created with the live routes of US-102.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`):
//!
//! - `POST /docs/{doc_id}/ops` (agent, in the route registry) takes
//!   `{"ops": [...]}` with the batch format of
//!   docs/formats/operations.md, and applies it to the doc's head, all or
//!   nothing. It answers 200 `{"results": [...]}`, one result per operation
//!   (`node`, `revision`, `created`, `removed`), and the next read of the doc
//!   (`GET /docs/{doc_id}/html`, by the agent or the human) has the change.
//! - An operation whose `rev` is not the node's revision is 409 `{"error":
//!   "conflict", "conflict": {"index", "node", "expected", "current_revision",
//!   "current_html"}}`, where `current_html` is the node as it is now, with its
//!   `data-omni-id`. A refused operation is 400 `{"error": "invalid_operation",
//!   "index", "message"}` whose message names the unknown ID, and a body that
//!   is not a batch is 400 `invalid_request`. Every refusal leaves the head,
//!   `doc_changes` and the live events as they were.
//! - A doc that is not in the project is 404 `not_found`, and a session is 403
//!   `forbidden`: the human never edits.
//! - An acknowledged write is durable. An API started afresh on the same
//!   database and storage reads the doc as it was written and checks
//!   revisions against what was written, and an API that kept running sees
//!   the writes of another one: nothing about a head lives only in memory.
//! - Every applied batch adds exactly one row of `doc_changes`: the next
//!   `seq`, the `agent` the request declared (`run`, `role`, `model`,
//!   `harness`), the `operations` as the agent sent them (the JSON array),
//!   `head_before` (the head it applied to) and `head_after` (the new head,
//!   which is then `docs.head`, a chunk in RustFS named by the BLAKE3 hash of
//!   its bytes). It also emits `doc.changed` on the project's live channel
//!   (`live_events`) with `data` `{"doc_id", "head"}`, the head being the new
//!   one. Batches of agents that write one doc at the same time are applied
//!   one after the other: none is lost, and each row's `head_before` is the
//!   `head_after` of the row before it.
//! - US-109: an HTML doc holds any HTML. An insert of content with `<style>`,
//!   `<script>`, `<svg>`, tables, `<details>`, custom classes and inline
//!   handlers applies, and the head read gives it back as written, with a
//!   `data-omni-id` on every block-level element (the `created` IDs) and none
//!   on inline ones; an agent-supplied `data-omni-id` that is valid and unique
//!   is kept, also by an API started afresh. Operations address elements at
//!   any depth (inside a table cell in a `<details>`) with revision checks: a
//!   stale `rev` on a nested element is 409 with its `data-omni-id`.

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

const TEST_DB: &str = "omni_us103_live_edits";
const TEST_BUCKET: &str = "omni-us103-live-edits";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
/// A node ID in the right format that no node has.
const UNKNOWN_NODE: &str = "zzzzzzzz";
const OPS_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/ops";

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
        s3_endpoint: s3_endpoint(),
        s3_access_key: s3_access_key(),
        s3_secret_key: s3_secret_key(),
        s3_bucket: TEST_BUCKET.to_string(),
    }
}

fn s3_region() -> Region {
    Region::Custom {
        region: "us-east-1".to_string(),
        endpoint: s3_endpoint(),
    }
}

fn s3_credentials() -> Credentials {
    Credentials::new(
        Some(&s3_access_key()),
        Some(&s3_secret_key()),
        None,
        None,
        None,
    )
    .expect("storage credentials")
}

fn bucket() -> Box<Bucket> {
    Bucket::new(TEST_BUCKET, s3_region(), s3_credentials())
        .expect("bucket settings")
        .with_path_style()
}

async fn ensure_bucket() {
    let bucket = bucket();
    if !bucket.exists().await.unwrap_or(false) {
        Bucket::create_with_path_style(
            TEST_BUCKET,
            s3_region(),
            s3_credentials(),
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
    format!("us103-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

/// Who sends a request: an agent (an API key, with the run it declares) or
/// the human (a session cookie).
#[derive(Clone, Copy)]
enum Auth<'a> {
    Agent(&'a str, &'a str),
    Session(&'a str),
}

fn agent(f: &Fixture) -> Auth<'_> {
    Auth::Agent(&f.key, "run-103")
}

fn agent_of_run<'a>(f: &'a Fixture, run: &'a str) -> Auth<'a> {
    Auth::Agent(&f.key, run)
}

fn human(f: &Fixture) -> Auth<'_> {
    Auth::Session(&f.alice_token)
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let headers: Vec<(&str, String)> = match auth {
        Auth::Agent(key, run) => vec![
            ("Authorization", format!("Bearer {key}")),
            ("X-Omni-Run", run.to_string()),
            ("X-Omni-Role", "writer".to_string()),
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

/// 8 characters of lowercase base32: 0-9 and a-z without i, l, o and u.
fn is_node_id(id: &str) -> bool {
    id.len() == 8
        && id
            .chars()
            .all(|c| c.is_ascii_digit() || (c.is_ascii_lowercase() && !"ilou".contains(c)))
}

/// The name of a chunk: 64 lowercase hexadecimal characters.
fn is_chunk_name(name: &str) -> bool {
    name.len() == 64 && name.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
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

fn doc_path(f: &Fixture, project: &str, doc: &str) -> String {
    format!("{}/docs/{doc}", project_base(f, project))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &agent(f),
                Some(&json!({ "name": unique("live-edits-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

/// Creates an HTML doc at the app level (US-102);
/// returns its ID.
fn new_doc(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", project_base(f, project)),
                &agent(f),
                Some(&json!({ "kind": "html", "pillar": "spec", "title": unique("Plan") })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

fn edit_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    doc: &str,
    ops: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/ops", doc_path(f, project, doc)),
        auth,
        Some(&json!({ "ops": ops })),
    )
}

/// Applies a batch as the agent; returns the results.
fn edit(f: &Fixture, client: &Client, project: &str, doc: &str, ops: &Value) -> Vec<Value> {
    let answer = must(
        edit_response(f, client, &agent(f), project, doc, ops),
        200,
        "apply a batch to the doc",
    );
    answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer lists results: {answer}"))
        .clone()
}

fn insert_paragraph(text: &str) -> Value {
    json!({
        "op": "insert",
        "parent": null,
        "position": { "after": null },
        "content": format!("<p>{text}</p>")
    })
}

fn set_html(node: &str, rev: u64, text: &str) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": { "html": text } })
}

fn node_of(result: &Value) -> String {
    result["node"]
        .as_str()
        .unwrap_or_else(|| panic!("a result names its node: {result}"))
        .to_string()
}

/// A doc with one paragraph of `text`; returns (doc, the paragraph's node).
fn doc_with_paragraph(f: &Fixture, client: &Client, project: &str, text: &str) -> (String, String) {
    let doc = new_doc(f, client, project);
    let results = edit(f, client, project, &doc, &json!([insert_paragraph(text)]));
    (doc, node_of(&results[0]))
}

fn read_html_on(client: &Client, f: &Fixture, auth: &Auth, project: &str, doc: &str) -> String {
    let read = must(
        call(
            client,
            "GET",
            &format!("{}/html", doc_path(f, project, doc)),
            auth,
            None,
        ),
        200,
        "read the doc's HTML",
    );
    read["html"]
        .as_str()
        .unwrap_or_else(|| panic!("the read has the HTML: {read}"))
        .to_string()
}

fn read_html(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    read_html_on(client, f, &agent(f), project, doc)
}

fn scalar(sql: &str, binds: &[&str]) -> Option<String> {
    block(async {
        let mut conn = admin_connection().await;
        let mut query = sqlx::query_scalar::<_, Option<String>>(sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    })
}

fn count(sql: &str, binds: &[&str]) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        let mut query = sqlx::query_scalar::<_, i64>(sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    })
}

/// The doc's head as the database holds it.
fn head_of(doc: &str) -> String {
    scalar("SELECT head::text FROM docs WHERE id = $1::uuid", &[doc])
        .unwrap_or_else(|| panic!("the doc {doc} has a head"))
}

/// One row of `doc_changes`.
#[derive(Debug)]
struct Change {
    seq: i64,
    agent: Value,
    operations: Value,
    head_before: Option<String>,
    head_after: Option<String>,
}

type ChangeRow = (i64, String, String, Option<String>, Option<String>);

fn changes_of(doc: &str) -> Vec<Change> {
    block(async {
        let mut conn = admin_connection().await;
        let rows: Vec<ChangeRow> = sqlx::query_as(
            "SELECT seq::bigint, agent::text, operations::text, head_before::text, \
                 head_after::text \
                 FROM doc_changes WHERE doc_id = $1::uuid ORDER BY seq",
        )
        .bind(doc)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the doc changes of {doc}: {e}"));
        rows.into_iter()
            .map(|(seq, agent, operations, head_before, head_after)| Change {
                seq,
                agent: serde_json::from_str(&agent).expect("the agent is JSON"),
                operations: serde_json::from_str(&operations).expect("the operations are JSON"),
                head_before,
                head_after,
            })
            .collect()
    })
}

/// The `data` of every `doc.changed` event of the project that names `doc`,
/// in the order of the project's live channel.
fn doc_changed_events(project: &str, doc: &str) -> Vec<Value> {
    block(async {
        let mut conn = admin_connection().await;
        let rows: Vec<String> = sqlx::query_scalar(
            "select data::jsonb::text from live_events \
             where project_id = $1::uuid and event = 'doc.changed' order by seq",
        )
        .bind(project)
        .fetch_all(&mut conn)
        .await
        .expect("read the live events");
        rows.iter()
            .map(|text| serde_json::from_str::<Value>(text).expect("the data is JSON"))
            .filter(|data| data["doc_id"] == doc)
            .collect()
    })
}

/// The bytes stored at `key`, which must exist.
fn stored(key: &str) -> Vec<u8> {
    let object = block(bucket().get_object(key)).unwrap_or_else(|e| panic!("get {key}: {e}"));
    assert_eq!(
        object.status_code(),
        200,
        "{key} is stored in RustFS: {}",
        String::from_utf8_lossy(object.bytes())
    );
    object.to_vec()
}

/// The doc's head is a content-addressed chunk in RustFS; returns it.
fn assert_head_is_stored(f: &Fixture, doc: &str) -> String {
    let head = head_of(doc);
    assert!(
        is_chunk_name(&head),
        "the head of {doc} names a chunk (64 lowercase hex characters): {head:?}"
    );
    let bytes = stored(&format!("ws/{}/chunks/{head}", f.workspace));
    assert_eq!(
        blake3::hash(&bytes).to_hex().to_string(),
        head,
        "the head of {doc} is named by the BLAKE3 hash of its bytes"
    );
    head
}

/// What a refusal must leave as it was: the head, the doc's changes, its
/// `doc.changed` events and what a read of it shows.
struct Untouched {
    head: String,
    changes: usize,
    events: usize,
    html: String,
}

fn untouched(f: &Fixture, client: &Client, project: &str, doc: &str) -> Untouched {
    Untouched {
        head: head_of(doc),
        changes: changes_of(doc).len(),
        events: doc_changed_events(project, doc).len(),
        html: read_html(f, client, project, doc),
    }
}

fn assert_untouched(
    before: &Untouched,
    f: &Fixture,
    client: &Client,
    project: &str,
    doc: &str,
    what: &str,
) {
    assert_eq!(head_of(doc), before.head, "{what}: the head did not change");
    assert_eq!(
        changes_of(doc).len(),
        before.changes,
        "{what}: no row of doc_changes was added"
    );
    assert_eq!(
        doc_changed_events(project, doc).len(),
        before.events,
        "{what}: no doc.changed was sent"
    );
    assert_eq!(
        read_html(f, client, project, doc),
        before.html,
        "{what}: the doc reads as before"
    );
}

// ---------------------------------------------------------------------------
// The route
// ---------------------------------------------------------------------------

#[test]
fn the_live_ops_route_is_an_agent_route_of_the_registry() {
    let entry = registry()
        .into_iter()
        .find(|r| r.method == "POST" && r.path == OPS_ROUTE)
        .unwrap_or_else(|| panic!("the registry has no route POST {OPS_ROUTE}"));
    assert_eq!(entry.access, Access::Agent, "POST {OPS_ROUTE}");
}

// ---------------------------------------------------------------------------
// A batch applies to the head
// ---------------------------------------------------------------------------

#[test]
fn a_batch_applies_to_the_head_and_the_next_read_has_the_change() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = new_doc(f, client, &project);
    let (first, second) = (unique("Pay in one step"), unique("Cards and invoices"));

    let results = edit(
        f,
        client,
        &project,
        &doc,
        &json!([insert_paragraph(&first), insert_paragraph(&second)]),
    );
    assert_eq!(results.len(), 2, "one result per operation: {results:?}");
    for result in &results {
        let node = node_of(result);
        assert!(is_node_id(&node), "a new node has a node ID: {result}");
        assert_eq!(
            result["revision"], 1,
            "a new node is at revision 1: {result}"
        );
        assert_eq!(result["created"], json!([node]), "{result}");
        assert_eq!(result["removed"], json!([]), "{result}");
    }
    let (kept, gone) = (node_of(&results[1]), node_of(&results[0]));
    for auth in [agent(f), human(f)] {
        let html = read_html_on(client, f, &auth, &project, &doc);
        assert!(
            html.contains(&first) && html.contains(&second),
            "the next read has both paragraphs: {html:?}"
        );
        assert!(
            html.contains(&kept) && html.contains(&gone),
            "the blocks carry their node IDs: {html:?}"
        );
    }

    let changed = unique("Pay in two steps");
    let results = edit(
        f,
        client,
        &project,
        &doc,
        &json!([
            set_html(&kept, 1, &changed),
            { "op": "delete", "node": gone, "rev": 1 }
        ]),
    );
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0]["node"], kept.as_str(), "{results:?}");
    assert_eq!(
        results[0]["revision"], 2,
        "the set node's new revision: {results:?}"
    );
    assert_eq!(results[1]["node"], gone.as_str(), "{results:?}");
    assert_eq!(results[1]["removed"], json!([gone]), "{results:?}");

    let html = read_html(f, client, &project, &doc);
    assert!(html.contains(&changed), "the set applied: {html:?}");
    assert!(
        !html.contains(&second) && !html.contains(&gone),
        "the delete applied: {html:?}"
    );
    assert!(!html.contains(&first), "the old text is gone: {html:?}");

    // A later batch is checked against the revision the last one left.
    let again = unique("Pay later");
    let results = edit(
        f,
        client,
        &project,
        &doc,
        &json!([set_html(&kept, 2, &again)]),
    );
    assert_eq!(results[0]["revision"], 3, "{results:?}");
    assert!(read_html(f, client, &project, &doc).contains(&again));
}

#[test]
fn a_batch_with_a_refused_operation_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, node) = doc_with_paragraph(f, client, &project, &unique("Kept"));
    let before = untouched(f, client, &project, &doc);

    let inserted = unique("Never applied");
    let response = edit_response(
        f,
        client,
        &agent(f),
        &project,
        &doc,
        &json!([
            insert_paragraph(&inserted),
            set_html(&node, 1, "Never set"),
            { "op": "delete", "node": UNKNOWN_NODE, "rev": 1 }
        ]),
    );
    assert_eq!(response.status, 400, "{}", response.text());
    let body = response.json();
    assert_eq!(body["error"], "invalid_operation", "{body}");
    assert_eq!(body["index"], 2, "the third operation is refused: {body}");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains(UNKNOWN_NODE)),
        "the message names the unknown node: {body}"
    );
    assert_untouched(&before, f, client, &project, &doc, "a refused operation");

    // The node is still at revision 1: the set before the refusal did not
    // apply either.
    let results = edit(
        f,
        client,
        &project,
        &doc,
        &json!([set_html(&node, 1, "Set")]),
    );
    assert_eq!(results[0]["revision"], 2, "{results:?}");
}

#[test]
fn a_body_that_is_not_a_batch_is_refused() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, _) = doc_with_paragraph(f, client, &project, &unique("Kept"));
    let before = untouched(f, client, &project, &doc);
    for body in [json!({ "ops": "delete everything" }), json!([1, 2])] {
        let response = call(
            client,
            "POST",
            &format!("{}/ops", doc_path(f, &project, &doc)),
            &agent(f),
            Some(&body),
        );
        assert_eq!(response.status, 400, "{body}: {}", response.text());
        assert_eq!(response.json()["error"], "invalid_request", "{body}");
    }
    assert_untouched(
        &before,
        f,
        client,
        &project,
        &doc,
        "a body that is no batch",
    );
}

// ---------------------------------------------------------------------------
// Conflicts
// ---------------------------------------------------------------------------

#[test]
fn a_stale_rev_is_a_conflict_with_the_current_revision_and_content_and_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, node) = doc_with_paragraph(f, client, &project, &unique("First"));
    let current = unique("Changed by someone else");
    edit(
        f,
        client,
        &project,
        &doc,
        &json!([set_html(&node, 1, &current)]),
    );
    let before = untouched(f, client, &project, &doc);

    let inserted = unique("Never applied");
    let response = edit_response(
        f,
        client,
        &agent(f),
        &project,
        &doc,
        &json!([insert_paragraph(&inserted), set_html(&node, 1, "Stale")]),
    );
    assert_eq!(response.status, 409, "{}", response.text());
    let body = response.json();
    assert_eq!(body["error"], "conflict", "{body}");
    let conflict = &body["conflict"];
    assert_eq!(conflict["index"], 1, "the second operation: {body}");
    assert_eq!(conflict["node"], node.as_str(), "{body}");
    assert_eq!(conflict["expected"], 1, "the revision it listed: {body}");
    assert_eq!(
        conflict["current_revision"], 2,
        "the node's current revision: {body}"
    );
    let current_html = conflict["current_html"]
        .as_str()
        .unwrap_or_else(|| panic!("the conflict has the node's content: {body}"));
    assert!(
        current_html.contains(&current) && current_html.contains(&node),
        "the node as it is now, with its ID: {current_html:?}"
    );
    assert_untouched(&before, f, client, &project, &doc, "a conflict");

    // With the current revision the same change applies.
    let results = edit(
        f,
        client,
        &project,
        &doc,
        &json!([insert_paragraph(&inserted), set_html(&node, 2, "Merged")]),
    );
    assert_eq!(results[1]["revision"], 3, "{results:?}");
    let html = read_html(f, client, &project, &doc);
    assert!(
        html.contains(&inserted) && html.contains("Merged"),
        "{html:?}"
    );
}

// ---------------------------------------------------------------------------
// Who and where
// ---------------------------------------------------------------------------

#[test]
fn the_human_cannot_edit_a_doc() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, node) = doc_with_paragraph(f, client, &project, &unique("Kept"));
    let before = untouched(f, client, &project, &doc);
    assert_error(
        &edit_response(
            f,
            client,
            &human(f),
            &project,
            &doc,
            &json!([set_html(&node, 1, "By hand")]),
        ),
        403,
        "forbidden",
    );
    assert_untouched(&before, f, client, &project, &doc, "the human's edit");
}

#[test]
fn a_doc_that_is_not_in_the_project_is_not_found() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let other = new_project(f, client);
    let (doc, node) = doc_with_paragraph(f, client, &project, &unique("Kept"));
    let before = untouched(f, client, &project, &doc);
    let ops = json!([set_html(&node, 1, "Elsewhere")]);
    assert_error(
        &edit_response(f, client, &agent(f), &other, &doc, &ops),
        404,
        "not_found",
    );
    assert_error(
        &edit_response(f, client, &agent(f), &project, UNKNOWN_ID, &ops),
        404,
        "not_found",
    );
    assert_error(
        &edit_response(f, client, &agent(f), UNKNOWN_ID, &doc, &ops),
        404,
        "not_found",
    );
    assert_untouched(
        &before,
        f,
        client,
        &project,
        &doc,
        "an edit in another project",
    );
}

// ---------------------------------------------------------------------------
// Durability
// ---------------------------------------------------------------------------

#[test]
fn an_acknowledged_write_survives_an_api_restart() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, node) = doc_with_paragraph(f, client, &project, &unique("First"));
    let written = unique("Written before the restart");
    edit(
        f,
        client,
        &project,
        &doc,
        &json!([set_html(&node, 1, &written)]),
    );

    // An API started afresh has nothing in memory: it reads what was written
    // and checks revisions against it.
    let fresh = TestServer::start(&config());
    let html = read_html_on(&fresh.client, f, &agent(f), &project, &doc);
    assert!(
        html.contains(&written),
        "a fresh API reads the acknowledged write: {html:?}"
    );
    let stale = edit_response(
        f,
        &fresh.client,
        &agent(f),
        &project,
        &doc,
        &json!([set_html(&node, 1, "Stale")]),
    );
    assert_eq!(
        stale.status,
        409,
        "the node's revision survived the restart: {}",
        stale.text()
    );
    assert_eq!(
        stale.json()["conflict"]["current_revision"],
        2,
        "{}",
        stale.text()
    );
    let after = unique("Written after the restart");
    let results = must(
        edit_response(
            f,
            &fresh.client,
            &agent(f),
            &project,
            &doc,
            &json!([set_html(&node, 2, &after)]),
        ),
        200,
        "edit through the fresh API",
    );
    assert_eq!(results["results"][0]["revision"], 3, "{results}");

    // The API that kept running sees the fresh API's write too.
    let html = read_html(f, client, &project, &doc);
    assert!(
        html.contains(&after) && !html.contains(&written),
        "the running API reads the head, not a copy of its own: {html:?}"
    );
    let stale = edit_response(
        f,
        client,
        &agent(f),
        &project,
        &doc,
        &json!([set_html(&node, 2, "Stale")]),
    );
    assert_eq!(stale.status, 409, "{}", stale.text());
    assert_eq!(
        stale.json()["conflict"]["current_revision"],
        3,
        "{}",
        stale.text()
    );
    let last = unique("Written by the running API");
    let results = edit(
        f,
        client,
        &project,
        &doc,
        &json!([set_html(&node, 3, &last)]),
    );
    assert_eq!(results[0]["revision"], 4, "{results:?}");
    let html = read_html_on(&fresh.client, f, &agent(f), &project, &doc);
    assert!(html.contains(&last), "{html:?}");
}

// ---------------------------------------------------------------------------
// The log of writes and the live channel
// ---------------------------------------------------------------------------

#[test]
fn every_applied_batch_adds_one_doc_change_and_sends_doc_changed_with_the_new_head() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = new_doc(f, client, &project);
    let created_head = head_of(&doc);
    assert_eq!(
        changes_of(&doc).len(),
        1,
        "the creation is the first change"
    );
    let events_before = doc_changed_events(&project, &doc).len();

    let first = json!([
        insert_paragraph(&unique("One")),
        insert_paragraph(&unique("Two"))
    ]);
    let results = edit(f, client, &project, &doc, &first);
    let after_first = assert_head_is_stored(f, &doc);
    assert_ne!(after_first, created_head, "the batch made a new head");

    let node = node_of(&results[0]);
    let second = json!([set_html(&node, 1, &unique("Three"))]);
    edit(f, client, &project, &doc, &second);
    let after_second = assert_head_is_stored(f, &doc);
    assert_ne!(after_second, after_first, "the batch made a new head");

    let changes = changes_of(&doc);
    assert_eq!(
        changes.len(),
        3,
        "one row per batch, not per operation: {changes:?}"
    );
    for (change, (seq, ops, before, after)) in changes[1..].iter().zip([
        (2, &first, &created_head, &after_first),
        (3, &second, &after_first, &after_second),
    ]) {
        assert_eq!(change.seq, seq, "{change:?}");
        assert_eq!(change.agent["run"], "run-103", "{change:?}");
        assert_eq!(change.agent["role"], "writer", "{change:?}");
        assert_eq!(change.agent["model"], "test-model", "{change:?}");
        assert_eq!(change.agent["harness"], "test-harness", "{change:?}");
        assert_eq!(
            &change.operations, ops,
            "the row has the operations as the agent sent them"
        );
        assert_eq!(
            change.head_before.as_deref(),
            Some(before.as_str()),
            "the head the batch applied to: {change:?}"
        );
        assert_eq!(
            change.head_after.as_deref(),
            Some(after.as_str()),
            "the head the batch made: {change:?}"
        );
    }

    let events = doc_changed_events(&project, &doc);
    assert_eq!(
        events.len(),
        events_before + 2,
        "one doc.changed per applied batch: {events:?}"
    );
    for (event, head) in events[events_before..]
        .iter()
        .zip([&after_first, &after_second])
    {
        assert_eq!(event["doc_id"], doc.as_str(), "{event}");
        assert_eq!(event["head"], head.as_str(), "the new head: {event}");
    }
}

#[test]
fn batches_of_agents_writing_one_doc_at_once_are_all_applied_one_after_the_other() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = new_doc(f, client, &project);
    const ROUNDS: usize = 6;
    let runs = ["run-a", "run-b", "run-c"];
    let text = |run: &str, round: usize| format!("{run} {project} round {round}");

    std::thread::scope(|scope| {
        for run in runs {
            let (project, doc, text) = (&project, &doc, &text);
            scope.spawn(move || {
                for round in 0..ROUNDS {
                    let response = edit_response(
                        f,
                        client,
                        &agent_of_run(f, run),
                        project,
                        doc,
                        &json!([insert_paragraph(&text(run, round))]),
                    );
                    assert_eq!(
                        response.status,
                        200,
                        "{run} round {round}: {}",
                        response.text()
                    );
                }
            });
        }
    });

    let html = read_html(f, client, &project, &doc);
    for run in runs {
        for round in 0..ROUNDS {
            assert!(
                html.contains(&text(run, round)),
                "no batch is lost: {run} round {round} is missing from {html:?}"
            );
        }
    }
    let changes = changes_of(&doc);
    assert_eq!(
        changes.len(),
        1 + runs.len() * ROUNDS,
        "one row per batch: {changes:?}"
    );
    for (pair, expected_seq) in changes.windows(2).zip(2..) {
        assert_eq!(pair[1].seq, expected_seq, "{changes:?}");
        assert_eq!(
            pair[1].head_before, pair[0].head_after,
            "each batch applied to the head the one before it made: {changes:?}"
        );
    }
    assert_eq!(
        changes.last().and_then(|c| c.head_after.clone()),
        Some(head_of(&doc)),
        "the last row's head after is the doc's head"
    );
    for run in runs {
        assert_eq!(
            changes
                .iter()
                .filter(|change| change.agent["run"] == run)
                .count(),
            ROUNDS,
            "every batch of {run} is logged under its run"
        );
    }
}
