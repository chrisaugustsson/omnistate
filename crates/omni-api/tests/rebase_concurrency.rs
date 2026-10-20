//! US-052: rebases under concurrency, through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. The
//! routes are those of US-026 (tests/rebase.rs), whose contract this file
//! extends; all paths are under `/api/v1/w/{workspace_id}/projects/{project_id}`.
//!
//! To hold a request in flight, a test locks a row (the change set's or the
//! draft's) from an admin connection, so the request waits in its transaction
//! on that row, and tells that it waits from `pg_blocking_pids`.
//!
//! The contract these tests pin down:
//!
//! - Two rebases of the same change set sent at the same time give one result
//!   (200) and one 409 `{"error": "invalid_transition"}`: the change set moves
//!   out of `needs_rebase` once, one set of conflicts is recorded (the one the
//!   200 answers, and nothing else is listed), one version is saved per
//!   rebased doc (one `version.saved` and one `draft.rebased` event each) and
//!   the history holds one move out of `needs_rebase`.
//! - A rebase racing a change of the same change set's doc (a batch of
//!   operations, a saved version or a removal of the doc) leaves a consistent
//!   state, as if the two had run one after the other:
//!   - the change comes first: an operation the rebase did not replay stays in
//!     the draft and the rebase is 409 `unsaved_operations`; a saved version
//!     or a removal takes the change set out of `needs_rebase` (FR-20) and the
//!     rebase is 409 `invalid_transition`; nothing of the rebase is recorded;
//!   - the rebase comes first: the change applies to the rebased draft, so a
//!     version saved after it holds the rebased state (never the draft from
//!     before the rebase), and a removal is what merges next;
//!   - or the change is refused with a 409 of its own and changed nothing.
//! - A rebase racing the human's approval of the same change set leaves a
//!   consistent state: the approval of a change set that a rebase is
//!   rebasing waits for it and is 409 `invalid_transition` once the change set
//!   is `open`; a rebase sent while an approval is in flight is 409
//!   `invalid_transition` (the change set is not `needs_rebase` yet) or
//!   rebases what the approval left. Either way the history, the approval and
//!   the recorded conflicts agree with the status.
//! - Every Conflict carries a `kind`. A node changed on both sides (US-026)
//!   is `"both_changed"`. A node that the change set added and that the
//!   rebase cannot place in the mainline doc (for example a list item added
//!   to a list that the mainline removed, which cannot stand at the top
//!   level) is `"unplaced"`, never a `"both_changed"` conflict that looks like
//!   a node removed on the mainline. An unplaced node is not in the draft
//!   after the rebase; its conflict has the node in `changeset.html` and
//!   `mainline.html` `null`. `GET /changesets/{changeset_id}/conflicts` lists
//!   the same kinds.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const TEST_DB: &str = "omni_us052_rebase_concurrency";
const TEST_BUCKET: &str = "omni-us052-rebase-concurrency";

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
        // Versions keep their content in RustFS, so the API gets a real bucket.
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
    format!("us052-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

fn doc_path(f: &Fixture, project: &str, doc: &str) -> String {
    format!("{}/docs/{doc}", project_base(f, project))
}

fn draft_path(f: &Fixture, project: &str, changeset: &str, doc: &str) -> String {
    format!("{}/docs/{doc}", changeset_path(f, project, changeset))
}

fn resolve_path(f: &Fixture, project: &str, conflict: &str) -> String {
    format!("{}/conflicts/{conflict}/resolve", project_base(f, project))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &agent(f),
                Some(&json!({ "name": unique("rebase-project") })),
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

/// Starts an HTML PRD doc in the change set; returns the doc's ID.
fn create_doc(f: &Fixture, client: &Client, project: &str, changeset: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", changeset_path(f, project, changeset)),
                &agent(f),
                Some(&json!({ "kind": "html", "module": "prd", "title": unique("Checkout PRD") })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

/// Makes (or reuses) the change set's draft of a doc that has a mainline
/// version; returns the Draft.
fn open_doc(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> Value {
    let response = call(
        client,
        "POST",
        &format!("{}/open", draft_path(f, project, changeset, doc)),
        &agent(f),
        Some(&json!({})),
    );
    assert!(
        response.status == 200 || response.status == 201,
        "open the doc in the change set: {} {}",
        response.status,
        response.text()
    );
    response.json()["draft"].clone()
}

/// Applies a batch of operations to the change set's draft of the doc; returns
/// the results.
fn edit(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    ops: Vec<Value>,
) -> Vec<Value> {
    must(
        call(
            client,
            "POST",
            &format!("{}/ops", draft_path(f, project, changeset, doc)),
            &agent(f),
            Some(&json!({ "ops": ops })),
        ),
        200,
        "edit the draft",
    )["results"]
        .as_array()
        .expect("the results of the batch")
        .clone()
}

/// The change set's draft of the doc as the API reads it back.
fn read_draft(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &draft_path(f, project, changeset, doc),
            &agent(f),
            None,
        ),
        200,
        "read the draft",
    )["draft"]
        .clone()
}

fn draft_html(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> String {
    read_draft(f, client, project, changeset, doc)["html"]
        .as_str()
        .expect("the draft has its html")
        .to_string()
}

/// Saves the change set's draft of the doc as a version; returns its ID.
fn save(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/versions", draft_path(f, project, changeset, doc)),
                &agent(f),
                Some(&json!({})),
            ),
            201,
            "save a version",
        )["version"],
    )
}

fn submit_response(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Response {
    call(
        client,
        "POST",
        &format!("{}/submit", changeset_path(f, project, changeset)),
        &agent(f),
        None,
    )
}

fn submit(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let answer = must(
        submit_response(f, client, project, changeset),
        200,
        "submit the change set",
    );
    assert_eq!(answer["changeset"]["status"], "in_review", "{answer}");
}

/// The human approves; returns the Changeset of the answer.
fn approve(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    must(
        call(
            client,
            "POST",
            &format!("{}/approve", changeset_path(f, project, changeset)),
            &human(f),
            Some(&json!({})),
        ),
        200,
        "the human approves the change set",
    )["changeset"]
        .clone()
}

fn rebase_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/rebase", changeset_path(f, project, changeset)),
        auth,
        Some(&json!({})),
    )
}

/// The agent rebases; returns the whole answer.
fn rebase(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    must(
        rebase_response(f, client, &agent(f), project, changeset),
        200,
        "the agent rebases the change set",
    )
}

fn conflicts_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
) -> Response {
    call(
        client,
        "GET",
        &format!("{}/conflicts", changeset_path(f, project, changeset)),
        auth,
        None,
    )
}

/// The change set's conflicts as the agent lists them.
fn conflicts_of(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Vec<Value> {
    must(
        conflicts_response(f, client, &agent(f), project, changeset),
        200,
        "list the conflicts",
    )["conflicts"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer lists `conflicts`"))
        .clone()
}

fn resolve_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    conflict: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &resolve_path(f, project, conflict),
        &agent(f),
        Some(body),
    )
}

/// The agent resolves a conflict with a note; returns the Conflict.
fn resolve(f: &Fixture, client: &Client, project: &str, conflict: &str, note: &str) -> Value {
    must(
        resolve_response(f, client, project, conflict, &json!({ "note": note })),
        200,
        "resolve the conflict",
    )["conflict"]
        .clone()
}

fn changeset_of(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &changeset_path(f, project, changeset),
            &agent(f),
            None,
        ),
        200,
        "show the change set",
    )["changeset"]
        .clone()
}

fn doc_of(f: &Fixture, client: &Client, project: &str, doc: &str) -> Value {
    must(
        call(client, "GET", &doc_path(f, project, doc), &agent(f), None),
        200,
        "show the doc",
    )["doc"]
        .clone()
}

fn mainline_of(f: &Fixture, client: &Client, project: &str, doc: &str) -> Value {
    doc_of(f, client, project, doc)["mainline_version_id"].clone()
}

/// The doc's mainline version as `GET /docs/{doc_id}/html` answers it.
fn mainline_html(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    must(
        call(
            client,
            "GET",
            &format!("{}/html", doc_path(f, project, doc)),
            &agent(f),
            None,
        ),
        200,
        "read the doc's mainline version",
    )["html"]
        .as_str()
        .expect("the mainline html")
        .to_string()
}

fn version_of(f: &Fixture, client: &Client, project: &str, version: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/versions/{version}", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "show a version",
    )["version"]
        .clone()
}

/// The recorded approval of a Changeset as doc ID -> version ID.
fn approved_versions(changeset: &Value) -> BTreeMap<String, String> {
    changeset["approved_versions"]
        .as_array()
        .unwrap_or_else(|| panic!("a change set carries `approved_versions`: {changeset}"))
        .iter()
        .map(|entry| {
            let field = |name: &str| {
                entry[name]
                    .as_str()
                    .unwrap_or_else(|| panic!("an approved version has `{name}`: {entry}"))
                    .to_string()
            };
            (field("doc_id"), field("version_id"))
        })
        .collect()
}

/// The history of a Changeset as (from, to, actor).
fn moves(changeset: &Value) -> Vec<(Option<String>, String, String)> {
    changeset["history"]
        .as_array()
        .unwrap_or_else(|| panic!("a change set has its history: {changeset}"))
        .iter()
        .map(|entry| {
            (
                entry["from_status"].as_str().map(str::to_string),
                entry["to_status"].as_str().expect("to_status").to_string(),
                entry["actor"].as_str().expect("actor").to_string(),
            )
        })
        .collect()
}

fn step(from: &str, to: &str, actor: &str) -> (Option<String>, String, String) {
    (Some(from.to_string()), to.to_string(), actor.to_string())
}

/// The last `n` moves of a change set's history.
fn last_moves(changeset: &Value, n: usize) -> Vec<(Option<String>, String, String)> {
    let history = moves(changeset);
    assert!(history.len() >= n, "{changeset}");
    history[history.len() - n..].to_vec()
}

/// The live events of a project named `event`, oldest first, as their `data`.
fn events_named(project: &str, event: &str) -> Vec<Value> {
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

/// The live events of a project named `event` for one change set.
fn events_of(project: &str, changeset: &str, event: &str) -> Vec<Value> {
    events_named(project, event)
        .into_iter()
        .filter(|data| data["changeset_id"] == changeset)
        .collect()
}

/// The value of `name` in one start tag, if it has the attribute.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_string())
}

/// Every start tag of `html` that carries `data-node-id`, as (ID, the tag's
/// `data-rev` if it has one), in document order.
fn tagged(html: &str) -> Vec<(String, Option<String>)> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start..start + end];
        if let Some(id) = attribute(tag, "data-node-id") {
            found.push((id, attribute(tag, "data-rev")));
        }
        rest = &rest[start + end + 1..];
    }
    found
}

/// The node IDs of `html` in document order.
fn node_ids(html: &str) -> Vec<String> {
    tagged(html).into_iter().map(|(id, _)| id).collect()
}

/// The revision of `node` in the HTML of a draft.
fn rev_in(html: &str, node: &str) -> u64 {
    tagged(html)
        .into_iter()
        .find(|(id, _)| id == node)
        .and_then(|(_, rev)| rev)
        .and_then(|rev| rev.parse().ok())
        .unwrap_or_else(|| panic!("the draft has {node} with a data-rev: {html}"))
}

fn insert_after(after: Option<&str>, text: &str) -> Value {
    json!({
        "op": "insert",
        "parent": null,
        "position": { "after": after },
        "content": format!("<p>{text}</p>")
    })
}

// ---------------------------------------------------------------------------
// Two change sets on one base
// ---------------------------------------------------------------------------

/// The texts of the merged base doc's three paragraphs.
const BASE: [&str; 3] = ["Alpha base", "Beta base", "Gamma base"];

/// An edit of one side, on the base's paragraphs by their index.
#[derive(Clone, Copy)]
enum Change {
    /// Sets the paragraph's text.
    Set(usize, &'static str),
    /// Deletes the paragraph.
    Delete(usize),
    /// Inserts a new paragraph with the text right after the paragraph.
    InsertAfter(usize, &'static str),
}

/// A doc whose mainline moved on under a change set: both change sets started
/// from the same merged version, `ahead` merged its changes, and `behind`
/// saved its own, was approved and needs a rebase.
struct Scene {
    project: String,
    doc: String,
    behind: String,
}

/// Applies one side's changes to its draft of the doc in one batch, with the
/// revisions the draft shows; returns the IDs of the new paragraphs.
fn apply(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    paragraphs: &[String],
    changes: &[Change],
) -> Vec<String> {
    let html = draft_html(f, client, project, changeset, doc);
    let ops: Vec<Value> = changes
        .iter()
        .map(|change| match *change {
            Change::Set(i, text) => json!({
                "op": "set",
                "node": paragraphs[i],
                "rev": rev_in(&html, &paragraphs[i]),
                "properties": { "html": text }
            }),
            Change::Delete(i) => json!({
                "op": "delete",
                "node": paragraphs[i],
                "rev": rev_in(&html, &paragraphs[i])
            }),
            Change::InsertAfter(i, text) => insert_after(Some(&paragraphs[i]), text),
        })
        .collect();
    let results = edit(f, client, project, changeset, doc, ops);
    changes
        .iter()
        .zip(results)
        .filter(|(change, _)| matches!(change, Change::InsertAfter(..)))
        .map(|(_, result)| {
            result["node"]
                .as_str()
                .expect("the inserted node")
                .to_string()
        })
        .collect()
}

fn scene(
    f: &Fixture,
    client: &Client,
    ahead_changes: &[Change],
    behind_changes: &[Change],
    with_fresh_doc: bool,
) -> Scene {
    let project = new_project(f, client);

    // The base: a doc with three paragraphs, merged through a change set.
    let first = new_changeset(f, client, &project);
    let doc = create_doc(f, client, &project, &first);
    let mut paragraphs: Vec<String> = Vec::new();
    for text in BASE {
        let after = paragraphs.last().map(String::as_str);
        let results = edit(
            f,
            client,
            &project,
            &first,
            &doc,
            vec![insert_after(after, text)],
        );
        paragraphs.push(
            results[0]["node"]
                .as_str()
                .expect("the inserted node")
                .to_string(),
        );
    }
    let base = save(f, client, &project, &first, &doc);
    submit(f, client, &project, &first);
    let merged = approve(f, client, &project, &first);
    assert_eq!(merged["status"], "merged", "set up the base: {merged}");
    assert_eq!(mainline_of(f, client, &project, &doc), base.as_str());

    // Two change sets start from it.
    let ahead = new_changeset(f, client, &project);
    let behind = new_changeset(f, client, &project);
    for changeset in [&ahead, &behind] {
        let draft = open_doc(f, client, &project, changeset, &doc);
        assert_eq!(draft["base_version_id"], base.as_str(), "{draft}");
    }

    // One merges first.
    apply(
        f,
        client,
        &project,
        &ahead,
        &doc,
        &paragraphs,
        ahead_changes,
    );
    let ahead_version = save(f, client, &project, &ahead, &doc);
    submit(f, client, &project, &ahead);
    let merged = approve(f, client, &project, &ahead);
    assert_eq!(merged["status"], "merged", "merge the ahead side: {merged}");

    // The other one is approved on the old base and needs a rebase.
    apply(
        f,
        client,
        &project,
        &behind,
        &doc,
        &paragraphs,
        behind_changes,
    );
    save(f, client, &project, &behind, &doc);
    if with_fresh_doc {
        let fresh = create_doc(f, client, &project, &behind);
        edit(
            f,
            client,
            &project,
            &behind,
            &fresh,
            vec![insert_after(None, "Fresh doc text")],
        );
        save(f, client, &project, &behind, &fresh);
    }
    submit(f, client, &project, &behind);
    let approved = approve(f, client, &project, &behind);
    assert_eq!(
        approved["status"], "needs_rebase",
        "set up a change set behind the mainline: {approved}"
    );
    assert_eq!(
        mainline_of(f, client, &project, &doc),
        ahead_version.as_str(),
        "set up a mainline that moved on"
    );

    Scene {
        project,
        doc,
        behind,
    }
}

/// Changes on different paragraphs: a clean rebase.
fn clean_scene(f: &Fixture, client: &Client) -> Scene {
    scene(
        f,
        client,
        &[Change::Set(0, "Alpha by ahead")],
        &[
            Change::Set(1, "Beta by behind"),
            Change::InsertAfter(1, "Delta by behind"),
        ],
        true,
    )
}

/// Both sides change the first paragraph, the mainline removes the second
/// one that the change set edits, and only the change set touches the third.
fn conflicted_scene(f: &Fixture, client: &Client) -> Scene {
    scene(
        f,
        client,
        &[Change::Set(0, "Alpha by ahead"), Change::Delete(1)],
        &[
            Change::Set(0, "Alpha by behind"),
            Change::Set(1, "Beta by behind"),
            Change::Set(2, "Gamma by behind"),
        ],
        false,
    )
}

/// The conflicts of a list by node.
fn by_node(conflicts: &[Value]) -> BTreeMap<String, Value> {
    conflicts
        .iter()
        .map(|c| {
            (
                c["node"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a conflict names its node: {c}"))
                    .to_string(),
                c.clone(),
            )
        })
        .collect()
}

/// The conflict IDs of a list, sorted.
fn conflict_ids(conflicts: &[Value]) -> BTreeSet<String> {
    conflicts.iter().map(id_of).collect()
}

// ---------------------------------------------------------------------------
// Nested structure and concurrent reorders
// ---------------------------------------------------------------------------

/// A doc merged with some top-level blocks, and two change sets opened on
/// that version: `ahead` merges first and `behind` needs a rebase.
struct Fork {
    project: String,
    doc: String,
    ahead: String,
    behind: String,
    /// The base doc's node IDs in document order.
    nodes: Vec<String>,
}

fn fork(f: &Fixture, client: &Client, blocks: &[&str]) -> Fork {
    let project = new_project(f, client);
    let first = new_changeset(f, client, &project);
    let doc = create_doc(f, client, &project, &first);
    let mut nodes: Vec<String> = Vec::new();
    let mut last: Option<String> = None;
    for block in blocks {
        let results = edit(
            f,
            client,
            &project,
            &first,
            &doc,
            vec![json!({
                "op": "insert",
                "parent": null,
                "position": { "after": last },
                "content": block
            })],
        );
        last = Some(
            results[0]["node"]
                .as_str()
                .expect("the inserted node")
                .to_string(),
        );
        nodes.extend(
            results[0]["created"]
                .as_array()
                .expect("the created nodes")
                .iter()
                .map(|id| id.as_str().expect("an ID").to_string()),
        );
    }
    let base = save(f, client, &project, &first, &doc);
    submit(f, client, &project, &first);
    let merged = approve(f, client, &project, &first);
    assert_eq!(merged["status"], "merged", "set up the base: {merged}");

    let ahead = new_changeset(f, client, &project);
    let behind = new_changeset(f, client, &project);
    for changeset in [&ahead, &behind] {
        let draft = open_doc(f, client, &project, changeset, &doc);
        assert_eq!(draft["base_version_id"], base.as_str(), "{draft}");
    }
    Fork {
        project,
        doc,
        ahead,
        behind,
        nodes,
    }
}

/// Applies each operation in a batch of its own; an operation on a node gets
/// the revision that the draft shows for it at that point.
fn edit_each(f: &Fixture, client: &Client, x: &Fork, changeset: &str, ops: Vec<Value>) {
    for mut op in ops {
        if let Some(node) = op["node"].as_str().map(str::to_string) {
            let html = draft_html(f, client, &x.project, changeset, &x.doc);
            op["rev"] = json!(rev_in(&html, &node));
        }
        edit(f, client, &x.project, changeset, &x.doc, vec![op]);
    }
}

/// The ahead change set makes its changes and merges; returns the mainline
/// version.
fn merge_ahead(f: &Fixture, client: &Client, x: &Fork, ops: Vec<Value>) -> String {
    edit_each(f, client, x, &x.ahead, ops);
    let version = save(f, client, &x.project, &x.ahead, &x.doc);
    submit(f, client, &x.project, &x.ahead);
    let merged = approve(f, client, &x.project, &x.ahead);
    assert_eq!(merged["status"], "merged", "merge the ahead side: {merged}");
    version
}

fn set_text(node: &str, text: &str) -> Value {
    json!({ "op": "set", "node": node, "properties": { "html": text } })
}

fn delete(node: &str) -> Value {
    json!({ "op": "delete", "node": node })
}

/// A version of the doc as `GET /docs/{doc_id}/html?version=` answers it.
fn version_html(f: &Fixture, client: &Client, project: &str, doc: &str, version: &str) -> String {
    must(
        call(
            client,
            "GET",
            &format!("{}/html?version={version}", doc_path(f, project, doc)),
            &agent(f),
            None,
        ),
        200,
        "read a version of the doc",
    )["html"]
        .as_str()
        .expect("the version's html")
        .to_string()
}

// ---------------------------------------------------------------------------
// Requests in flight
// ---------------------------------------------------------------------------

/// A lock on one row, held by another connection until it is released: a
/// request that needs the row waits in its transaction.
struct RowLock {
    /// The backend that holds the lock.
    pid: i32,
    release: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

impl RowLock {
    /// Locks the row of `table` whose ID is `id` `FOR UPDATE`.
    fn on(table: &'static str, id: &str) -> RowLock {
        let (locked, is_locked) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let id = id.to_string();
        let thread = std::thread::spawn(move || {
            block(async move {
                let mut conn = admin_connection().await;
                let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
                    .fetch_one(&mut conn)
                    .await
                    .expect("the backend's pid");
                let mut tx = conn.begin().await.expect("begin");
                sqlx::query(&format!(
                    "select 1 from {table} where id = $1::uuid for update"
                ))
                .bind(&id)
                .execute(&mut *tx)
                .await
                .expect("lock the row");
                locked.send(pid).expect("report the lock");
                // Dropped or sent, the lock goes.
                let _ = released.recv();
                tx.rollback().await.expect("release the lock");
            })
        });
        let pid = is_locked.recv().expect("the row is locked");
        RowLock {
            pid,
            release,
            thread,
        }
    }

    /// Waits until `n` backends wait on this lock; `false` when they do not
    /// within `within`.
    fn wait_for_waiters(&self, n: i64, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        loop {
            let waiting: i64 = block(async {
                let mut conn = admin_connection().await;
                sqlx::query_scalar(
                    "select count(*) from pg_stat_activity where $1 = any(pg_blocking_pids(pid))",
                )
                .bind(self.pid)
                .fetch_one(&mut conn)
                .await
                .expect("read the waiting backends")
            });
            if waiting >= n {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn release(self) {
        let _ = self.release.send(());
        self.thread.join().expect("the lock's thread");
    }
}

/// How long a request gets to reach the row or the draft it waits on.
const REACH: Duration = Duration::from_secs(20);

/// The status and the JSON body of an answer (`null` when it has none).
fn answer(response: Response) -> (u16, Value) {
    let body = serde_json::from_str(&response.text()).unwrap_or(Value::Null);
    (response.status, body)
}

/// Sends a request on a thread of its own; joining gives its answer.
fn spawn(request: impl FnOnce() -> Response + Send + 'static) -> JoinHandle<(u16, Value)> {
    std::thread::spawn(move || answer(request()))
}

fn spawn_rebase(
    f: &'static Fixture,
    client: &'static Client,
    s: &Scene,
) -> JoinHandle<(u16, Value)> {
    let (project, behind) = (s.project.clone(), s.behind.clone());
    spawn(move || rebase_response(f, client, &agent(f), &project, &behind))
}

fn joined(request: JoinHandle<(u16, Value)>, what: &str) -> (u16, Value) {
    request
        .join()
        .unwrap_or_else(|_| panic!("the {what} request's thread"))
}

fn save_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/versions", draft_path(f, project, changeset, doc)),
        &agent(f),
        Some(&json!({})),
    )
}

fn remove_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/remove", draft_path(f, project, changeset, doc)),
        &agent(f),
        Some(&json!({})),
    )
}

fn approve_response(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Response {
    call(
        client,
        "POST",
        &format!("{}/approve", changeset_path(f, project, changeset)),
        &human(f),
        Some(&json!({})),
    )
}

/// Whether the doc's mainline version marks it removed.
fn mainline_removed(f: &Fixture, client: &Client, project: &str, doc: &str) -> bool {
    let mainline = mainline_of(f, client, project, doc);
    let mainline = mainline.as_str().expect("the doc has a mainline version");
    version_of(f, client, project, mainline)["removed"]
        .as_bool()
        .unwrap_or_else(|| panic!("a version says whether it is a removal"))
}

/// Takes an `open` change set with conflicts to `merged`: the agent resolves
/// every conflict and submits, and the human approves.
fn settle(f: &Fixture, client: &Client, s: &Scene) {
    for conflict in conflicts_of(f, client, &s.project, &s.behind) {
        if conflict["resolution"].is_null() {
            resolve(
                f,
                client,
                &s.project,
                &id_of(&conflict),
                "Kept the mainline",
            );
        }
    }
    submit(f, client, &s.project, &s.behind);
    let merged = approve(f, client, &s.project, &s.behind);
    assert_eq!(merged["status"], "merged", "{merged}");
}

/// The scene's change set as it is now, with what the rebases recorded of it.
struct Record {
    changeset: Value,
    moves: usize,
    conflicts: Vec<Value>,
    recorded: usize,
    rebased: usize,
    saved: usize,
}

fn record(f: &Fixture, client: &Client, s: &Scene) -> Record {
    let changeset = changeset_of(f, client, &s.project, &s.behind);
    Record {
        moves: moves(&changeset).len(),
        changeset,
        conflicts: conflicts_of(f, client, &s.project, &s.behind),
        recorded: events_of(&s.project, &s.behind, "conflict.recorded").len(),
        rebased: events_of(&s.project, &s.behind, "draft.rebased").len(),
        saved: events_of(&s.project, &s.behind, "version.saved").len(),
    }
}

/// Nothing of a rebase was recorded since `before`.
fn assert_not_rebased(after: &Record, before: &Record) {
    assert_eq!(after.conflicts, Vec::<Value>::new(), "no conflicts");
    assert_eq!(after.recorded, before.recorded, "no conflict.recorded");
    assert_eq!(after.rebased, before.rebased, "no draft.rebased");
}

/// Of two rebases of the scene's change set, one answered 200 and the other
/// 409 `invalid_transition`, and the change set was rebased exactly once
/// since `before`; returns the 200's answer.
fn assert_rebased_once(
    answers: [(u16, Value); 2],
    after: &Record,
    before: &Record,
    to: &str,
) -> Value {
    let mut statuses: Vec<u16> = answers.iter().map(|(status, _)| *status).collect();
    statuses.sort();
    assert_eq!(statuses, [200, 409], "one result and one 409: {answers:?}");
    let [(_, a), (_, b)] = answers;
    let (won, lost) = if a.get("changeset").is_some() {
        (a, b)
    } else {
        (b, a)
    };
    assert_eq!(lost, json!({ "error": "invalid_transition" }), "{lost}");
    assert_eq!(won["changeset"]["status"], to, "{won}");
    assert_eq!(after.changeset["status"], to, "{}", after.changeset);

    let answered = won["conflicts"]
        .as_array()
        .unwrap_or_else(|| panic!("the rebase answers its conflicts: {won}"))
        .clone();
    assert_eq!(
        conflict_ids(&after.conflicts),
        conflict_ids(&answered),
        "the change set lists exactly the conflicts of the one rebase"
    );
    assert_eq!(
        after.recorded - before.recorded,
        answered.len(),
        "one conflict.recorded per conflict of the one rebase"
    );
    assert_eq!(after.rebased - before.rebased, 1, "one draft.rebased");
    assert_eq!(after.saved - before.saved, 1, "one rebased version");
    assert_eq!(
        after.moves - before.moves,
        1,
        "one move out of needs_rebase: {}",
        after.changeset
    );
    assert_eq!(
        last_moves(&after.changeset, 1),
        vec![step("needs_rebase", to, "agent")]
    );
    won
}

// ---------------------------------------------------------------------------
// Two rebases at the same time
// ---------------------------------------------------------------------------

#[test]
fn two_rebases_of_the_same_change_set_give_one_result_and_one_409() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let before = record(f, client, &s);

    // The first rebase waits on the change set's row in its transaction; the
    // second one comes while it is in flight.
    let lock = RowLock::on("changesets", &s.behind);
    let first = spawn_rebase(f, client, &s);
    assert!(
        lock.wait_for_waiters(1, REACH),
        "the first rebase never reached the change set's row"
    );
    let second = spawn_rebase(f, client, &s);
    lock.wait_for_waiters(2, Duration::from_secs(2));
    lock.release();
    let answers = [
        joined(first, "first rebase"),
        joined(second, "second rebase"),
    ];

    let after = record(f, client, &s);
    let won = assert_rebased_once(answers, &after, &before, "open");
    assert!(!after.conflicts.is_empty(), "{won}");
    assert_eq!(
        approved_versions(&after.changeset),
        BTreeMap::new(),
        "the approval is cleared once"
    );
    let html = draft_html(f, client, &s.project, &s.behind, &s.doc);
    assert!(
        html.contains("Alpha by ahead") && html.contains("Gamma by behind"),
        "the draft is rebased once: {html}"
    );
}

#[test]
fn two_rebases_sent_together_merge_the_change_set_once() {
    let (f, client) = ready();
    let s = clean_scene(f, client);
    let before = record(f, client, &s);

    let together = Arc::new(Barrier::new(2));
    let send = |together: Arc<Barrier>| {
        let (project, behind) = (s.project.clone(), s.behind.clone());
        spawn(move || {
            together.wait();
            rebase_response(f, client, &agent(f), &project, &behind)
        })
    };
    let first = send(together.clone());
    let second = send(together);
    let answers = [
        joined(first, "first rebase"),
        joined(second, "second rebase"),
    ];

    let after = record(f, client, &s);
    let won = assert_rebased_once(answers, &after, &before, "merged");
    let approved = approved_versions(&after.changeset);
    assert_eq!(
        mainline_of(f, client, &s.project, &s.doc),
        approved[&s.doc].as_str(),
        "the doc's mainline is the one rebased version: {won}"
    );
    let html = mainline_html(f, client, &s.project, &s.doc);
    assert!(
        html.contains("Alpha by ahead") && html.contains("Beta by behind"),
        "{html}"
    );
}

// ---------------------------------------------------------------------------
// A rebase racing an edit
// ---------------------------------------------------------------------------

#[test]
fn an_operation_racing_a_rebase_stays_in_the_draft() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let draft = read_draft(f, client, &s.project, &s.behind, &s.doc);
    let before = record(f, client, &s);

    // The batch waits on the draft's row in its transaction, holding the
    // draft; the rebase comes while it is in flight.
    let lock = RowLock::on("drafts", &id_of(&draft));
    let (project, behind, doc) = (s.project.clone(), s.behind.clone(), s.doc.clone());
    let editing = spawn(move || {
        call(
            client,
            "POST",
            &format!("{}/ops", draft_path(f, &project, &behind, &doc)),
            &agent(f),
            Some(&json!({ "ops": [insert_after(None, "Racing edit by behind")] })),
        )
    });
    assert!(
        lock.wait_for_waiters(1, REACH),
        "the edit never reached the draft's row"
    );
    let rebasing = spawn_rebase(f, client, &s);
    lock.wait_for_waiters(2, Duration::from_secs(2));
    lock.release();
    let (edited, edit_body) = joined(editing, "edit");
    let (rebased, rebase_body) = joined(rebasing, "rebase");

    let after = record(f, client, &s);
    let html = draft_html(f, client, &s.project, &s.behind, &s.doc);
    match (edited, rebased) {
        (200, 409) => {
            assert_eq!(rebase_body, json!({ "error": "unsaved_operations" }));
            assert_eq!(after.changeset["status"], "needs_rebase");
            assert_eq!(after.moves, before.moves, "the history is unchanged");
            assert_not_rebased(&after, &before);
            assert_eq!(after.saved, before.saved, "no version");
            assert!(
                html.contains("Racing edit by behind") && html.contains("Alpha by behind"),
                "the edit stays in the draft that was not rebased: {html}"
            );
        }
        (200, 200) => {
            assert_eq!(after.changeset["status"], "open", "{rebase_body}");
            assert!(
                html.contains("Racing edit by behind") && html.contains("Alpha by ahead"),
                "the edit applied to the rebased draft: {html}"
            );
        }
        (409, 200) => {
            assert!(
                !html.contains("Racing edit by behind"),
                "{edit_body} {html}"
            );
            assert_eq!(after.changeset["status"], "open", "{rebase_body}");
        }
        other => panic!("an edit racing a rebase: {other:?} {edit_body} {rebase_body}"),
    }
}

#[test]
fn a_version_saved_while_a_rebase_runs_comes_first_or_holds_the_rebased_draft() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let before = record(f, client, &s);

    // The save waits on the change set's row in its transaction; the rebase
    // comes while it is in flight.
    let lock = RowLock::on("changesets", &s.behind);
    let (project, behind, doc) = (s.project.clone(), s.behind.clone(), s.doc.clone());
    let saving = spawn(move || save_response(f, client, &project, &behind, &doc));
    assert!(
        lock.wait_for_waiters(1, REACH),
        "the save never reached the change set's row"
    );
    let rebasing = spawn_rebase(f, client, &s);
    // The rebase reaches the row too, or waits for the save on the draft.
    lock.wait_for_waiters(2, Duration::from_secs(3));
    lock.release();
    let (saved, save_body) = joined(saving, "save");
    let (rebased, rebase_body) = joined(rebasing, "rebase");

    let after = record(f, client, &s);
    match rebased {
        200 => {
            assert_eq!(after.changeset["status"], "open", "{rebase_body}");
            if saved == 201 {
                let version = id_of(&save_body["version"]);
                let html = version_html(f, client, &s.project, &s.doc, &version);
                assert!(
                    html.contains("Alpha by ahead")
                        && !html.contains("Alpha by behind")
                        && !html.contains("Beta by behind"),
                    "a version saved once the rebase has run holds the rebased draft, \
                     never the one from before the rebase: {html}"
                );
            } else {
                assert_eq!(saved, 409, "{save_body}");
            }
            settle(f, client, &s);
            let html = mainline_html(f, client, &s.project, &s.doc);
            assert!(
                html.contains("Alpha by ahead") && html.contains("Gamma by behind"),
                "what merges next holds the rebase: {html}"
            );
        }
        409 => {
            assert_eq!(rebase_body, json!({ "error": "invalid_transition" }));
            assert_eq!(saved, 201, "{save_body}");
            assert_eq!(
                after.changeset["status"], "in_review",
                "{}",
                after.changeset
            );
            assert_eq!(
                last_moves(&after.changeset, 1),
                vec![step("needs_rebase", "in_review", "system")]
            );
            assert_eq!(approved_versions(&after.changeset), BTreeMap::new());
            assert_not_rebased(&after, &before);
        }
        other => panic!("a rebase racing a save: {other} {rebase_body} {saved} {save_body}"),
    }
}

#[test]
fn a_doc_removed_while_a_rebase_runs_is_removed_by_what_merges_next() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let before = record(f, client, &s);

    // The rebase waits on the change set's row in its transaction; the agent
    // removes the doc while it is in flight.
    let lock = RowLock::on("changesets", &s.behind);
    let rebasing = spawn_rebase(f, client, &s);
    assert!(
        lock.wait_for_waiters(1, REACH),
        "the rebase never reached the change set's row"
    );
    let (project, behind, doc) = (s.project.clone(), s.behind.clone(), s.doc.clone());
    let removing = spawn(move || remove_response(f, client, &project, &behind, &doc));
    // The removal reaches the row too, or waits for the rebase on the draft.
    lock.wait_for_waiters(2, Duration::from_secs(3));
    lock.release();
    let (rebased, rebase_body) = joined(rebasing, "rebase");
    let (removed, remove_body) = joined(removing, "removal");

    let after = record(f, client, &s);
    match (rebased, removed) {
        (200, 201) => {
            assert_eq!(after.changeset["status"], "open", "{rebase_body}");
            settle(f, client, &s);
            assert!(
                mainline_removed(f, client, &s.project, &s.doc),
                "the removal answered 201 after the rebase, so the doc is removed by \
                 what merges next: {remove_body}"
            );
        }
        (200, 409) => {
            assert_eq!(after.changeset["status"], "open", "{rebase_body}");
            settle(f, client, &s);
            assert!(
                !mainline_removed(f, client, &s.project, &s.doc),
                "{remove_body}"
            );
        }
        (409, 201) => {
            assert_eq!(rebase_body, json!({ "error": "invalid_transition" }));
            assert_eq!(
                after.changeset["status"], "in_review",
                "{}",
                after.changeset
            );
            assert_not_rebased(&after, &before);
        }
        other => panic!("a rebase racing a removal: {other:?} {rebase_body} {remove_body}"),
    }
}

// ---------------------------------------------------------------------------
// A rebase racing an approval
// ---------------------------------------------------------------------------

#[test]
fn an_approval_sent_while_a_rebase_runs_is_refused_once_the_change_set_is_open() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let before = record(f, client, &s);

    let lock = RowLock::on("changesets", &s.behind);
    let rebasing = spawn_rebase(f, client, &s);
    assert!(
        lock.wait_for_waiters(1, REACH),
        "the rebase never reached the change set's row"
    );
    let (project, behind) = (s.project.clone(), s.behind.clone());
    let approving = spawn(move || approve_response(f, client, &project, &behind));
    lock.wait_for_waiters(2, Duration::from_secs(3));
    lock.release();
    let (rebased, rebase_body) = joined(rebasing, "rebase");
    let (approved, approve_body) = joined(approving, "approval");

    assert_eq!(rebased, 200, "{rebase_body}");
    assert_eq!(rebase_body["changeset"]["status"], "open", "{rebase_body}");
    assert_eq!(approved, 409, "{approve_body}");
    assert_eq!(approve_body, json!({ "error": "invalid_transition" }));

    let after = record(f, client, &s);
    assert_eq!(after.changeset["status"], "open", "{}", after.changeset);
    assert_eq!(approved_versions(&after.changeset), BTreeMap::new());
    assert_eq!(after.moves - before.moves, 1, "{}", after.changeset);
    assert_eq!(
        last_moves(&after.changeset, 1),
        vec![step("needs_rebase", "open", "agent")]
    );
    assert_eq!(
        conflict_ids(&after.conflicts),
        conflict_ids(rebase_body["conflicts"].as_array().expect("the conflicts")),
    );
    assert_eq!(after.rebased - before.rebased, 1);
}

#[test]
fn a_rebase_sent_while_an_approval_runs_finds_the_state_the_approval_left() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    // The agent saves again: the approval is cleared and the change set is
    // in review on its old base, so approving it gives `needs_rebase`.
    let latest = save(f, client, &s.project, &s.behind, &s.doc);
    let review = changeset_of(f, client, &s.project, &s.behind);
    assert_eq!(review["status"], "in_review", "{review}");
    let before = record(f, client, &s);

    let lock = RowLock::on("changesets", &s.behind);
    let (project, behind) = (s.project.clone(), s.behind.clone());
    let approving = spawn(move || approve_response(f, client, &project, &behind));
    assert!(
        lock.wait_for_waiters(1, REACH),
        "the approval never reached the change set's row"
    );
    let rebasing = spawn_rebase(f, client, &s);
    lock.wait_for_waiters(2, Duration::from_secs(3));
    lock.release();
    let (approved, approve_body) = joined(approving, "approval");
    let (rebased, rebase_body) = joined(rebasing, "rebase");

    assert_eq!(approved, 200, "{approve_body}");
    assert_eq!(
        approve_body["changeset"]["status"], "needs_rebase",
        "{approve_body}"
    );
    let after = record(f, client, &s);
    match rebased {
        409 => {
            assert_eq!(rebase_body, json!({ "error": "invalid_transition" }));
            assert_eq!(after.changeset["status"], "needs_rebase");
            assert_eq!(
                approved_versions(&after.changeset),
                BTreeMap::from([(s.doc.clone(), latest.clone())]),
                "the approval of the latest version stands"
            );
            assert_eq!(
                last_moves(&after.changeset, 2),
                vec![
                    step("in_review", "approved", "human"),
                    step("approved", "needs_rebase", "system"),
                ]
            );
            assert_not_rebased(&after, &before);
            // The change set is rebased once the approval has gone through.
            let answer = rebase(f, client, &s.project, &s.behind);
            assert_eq!(answer["changeset"]["status"], "open", "{answer}");
        }
        200 => {
            assert_eq!(after.changeset["status"], "open", "{rebase_body}");
            assert_eq!(approved_versions(&after.changeset), BTreeMap::new());
            assert_eq!(
                last_moves(&after.changeset, 3),
                vec![
                    step("in_review", "approved", "human"),
                    step("approved", "needs_rebase", "system"),
                    step("needs_rebase", "open", "agent"),
                ]
            );
            assert_eq!(after.rebased - before.rebased, 1);
        }
        other => panic!("a rebase racing an approval: {other} {rebase_body}"),
    }
}

// ---------------------------------------------------------------------------
// Conflict kinds
// ---------------------------------------------------------------------------

#[test]
fn nodes_changed_on_both_sides_are_both_changed_conflicts() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let answer = rebase(f, client, &s.project, &s.behind);
    let answered = answer["conflicts"]
        .as_array()
        .expect("the conflicts")
        .clone();
    assert!(!answered.is_empty(), "{answer}");
    for conflict in answered
        .iter()
        .chain(conflicts_of(f, client, &s.project, &s.behind).iter())
    {
        assert_eq!(conflict["kind"], "both_changed", "{conflict}");
    }
}

#[test]
fn a_node_the_change_set_added_that_the_rebase_cannot_place_is_an_unplaced_conflict() {
    let (f, client) = ready();
    // An intro and a list with one item.
    let x = fork(f, client, &["<p>Intro</p>", "<ul><li>One</li></ul>"]);
    let [intro, list, one] = <[String; 3]>::try_from(x.nodes.clone())
        .unwrap_or_else(|ids| panic!("three nodes in document order: {ids:?}"));
    // The mainline edits the intro and removes the list.
    let ahead_version = merge_ahead(
        f,
        client,
        &x,
        vec![set_text(&intro, "Intro by ahead"), delete(&list)],
    );
    // The change set edits the intro too and adds an item to the list. A list
    // item cannot stand at the top level, so the rebase has no place for it.
    edit_each(
        f,
        client,
        &x,
        &x.behind,
        vec![set_text(&intro, "Intro by behind")],
    );
    let added = edit(
        f,
        client,
        &x.project,
        &x.behind,
        &x.doc,
        vec![json!({
            "op": "insert",
            "parent": list,
            "position": { "after": one },
            "content": "<li>Two by behind</li>"
        })],
    )[0]["node"]
        .as_str()
        .expect("the inserted node")
        .to_string();
    save(f, client, &x.project, &x.behind, &x.doc);
    submit(f, client, &x.project, &x.behind);
    let approved = approve(f, client, &x.project, &x.behind);
    assert_eq!(approved["status"], "needs_rebase", "{approved}");

    let answer = rebase(f, client, &x.project, &x.behind);
    assert_eq!(answer["changeset"]["status"], "open", "{answer}");
    let answered = answer["conflicts"]
        .as_array()
        .expect("the conflicts")
        .clone();
    let listed = conflicts_of(f, client, &x.project, &x.behind);
    for conflicts in [&answered, &listed] {
        let unplaced: Vec<&Value> = conflicts.iter().filter(|c| c["node"] == added).collect();
        assert_eq!(
            unplaced.len(),
            1,
            "one conflict for the added item: {conflicts:?}"
        );
        let unplaced = unplaced[0];
        assert_eq!(
            unplaced["kind"], "unplaced",
            "an added node without a place is its own kind of conflict, \
             not a node removed on the mainline: {unplaced}"
        );
        assert_eq!(unplaced["mainline"]["version_id"], ahead_version.as_str());
        assert_eq!(unplaced["mainline"]["html"], Value::Null, "{unplaced}");
        let mine = unplaced["changeset"]["html"]
            .as_str()
            .unwrap_or_else(|| panic!("the change set's side holds the node: {unplaced}"));
        assert!(mine.contains("Two by behind"), "{mine}");

        let by = by_node(conflicts);
        assert_eq!(by[&intro]["kind"], "both_changed", "{}", by[&intro]);
        for (node, conflict) in &by {
            if *node != added {
                assert_ne!(conflict["kind"], "unplaced", "{conflict}");
            }
        }
    }
    let html = draft_html(f, client, &x.project, &x.behind, &x.doc);
    assert!(!node_ids(&html).contains(&added), "{html}");
    assert!(!html.contains("Two by behind"), "{html}");
}
