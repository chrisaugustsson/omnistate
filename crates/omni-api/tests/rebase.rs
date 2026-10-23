//! US-026: rebasing a change set onto the current mainline and resolving its
//! conflicts through the API. Run with `--features stack-tests` in the Compose
//! test container, which reaches Postgres at `postgres:5432` and RustFS at
//! `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; change sets are those of
//! US-016 and US-025, tests/changesets.rs and tests/approve_merge.rs, docs and
//! drafts those of US-022, tests/docs.rs, and versions and their comparison
//! those of US-024, tests/versions.rs):
//!
//! - `POST /changesets/{changeset_id}/rebase` (agent) takes `{}` and rebases a
//!   change set that is `needs_rebase`. Any other status is 409 `{"error":
//!   "invalid_transition"}` and changes nothing; an unknown change set is 404
//!   `{"error": "not_found"}`; a session is 403 `{"error": "forbidden"}` and
//!   changes nothing. The answer is 200 `{"changeset": Changeset, "conflicts":
//!   [Conflict, ...]}`, the conflicts that this rebase recorded (`[]` for a
//!   clean one).
//! - A rebase replays the change set's changes onto the current mainline
//!   versions. The changes of a changed doc (one with a version saved in the
//!   change set, as in US-025) are what differs, node by node as in a US-024
//!   comparison (added, removed, changed or moved), from the base of the
//!   change set's draft to the doc's latest version in the change set; the
//!   mainline's changes are what differs from that base to the doc's mainline
//!   version now. Replaying keeps every node ID. The change set's draft of each
//!   doc is then based on the mainline version (`base_version_id`).
//! - Every node changed on both sides becomes a conflict. Without one, the
//!   rebase saves the replayed content of each doc whose base was outdated as
//!   a new version of the change set, keeps the approval (no entry clears it,
//!   and `approved_versions` lists one version per changed doc) and merges:
//!   each approved version is its doc's mainline version afterwards, the
//!   change set moves `needs_rebase` -> `merged` with the actor `agent`, and
//!   `changeset.merged` is emitted.
//! - With conflicts, nothing merges: the approval is cleared
//!   (`approved_versions` is `[]`), the change set moves `needs_rebase` ->
//!   `open` with the actor `agent` and `changeset.open` is emitted. The draft
//!   holds the replayed changes, and each conflicting node keeps its mainline
//!   state there (a node that the mainline removed stays removed). What the
//!   human approves next is that rebased state or the agent's later saves, so
//!   nothing that the mainline gained is lost even when the agent saves
//!   nothing more.
//! - Only nodes changed on both sides conflict, in nested structure too: a
//!   section that the change set removed after moving or wrapping its
//!   children elsewhere conflicts with nothing when the mainline changed
//!   something else. When both sides reorder the same siblings, the change
//!   set's reorder survives next to the mainline's or becomes a conflict; it
//!   is never silently lost. The same node reordered differently on both
//!   sides is a conflict and keeps its mainline place, unless both sides put
//!   it in the same place.
//! - Removing a doc (`POST /changesets/{id}/docs/{doc_id}/remove`) is a
//!   doc-level change. When one side removed the doc and the other changed
//!   anything in it since the base, the rebase records a conflict for the doc
//!   and keeps the mainline state: a doc the mainline removed stays removed
//!   in the draft (it has no nodes) and in what merges next, and a doc the
//!   change set removed is the mainline version, every node of it, in the
//!   draft. Otherwise the removal survives: a removed doc is never brought
//!   back, and a removal is never lost.
//! - A rebase never drops unsaved work: when a draft of the change set has
//!   operations after its latest saved version, the rebase is 409 `{"error":
//!   "unsaved_operations"}` and changes nothing.
//! - An edit or a read of a draft that waits while a rebase rewrites it works
//!   on the rebased draft, never on the one from before the rebase.
//! - A Conflict is `{"id", "changeset_id", "doc_id", "node", "mainline":
//!   Side, "changeset": Side, "resolution"}`, where a Side is `{"version_id",
//!   "html"}`: the version that side's state comes from (the mainline version
//!   that the rebase found, and the change set's latest version of the doc)
//!   and the node there as HTML with `data-node-id`, `null` when that side
//!   removed it. `resolution` is `null` until the agent resolves it, then
//!   `{"note", "resolved_at"}`.
//! - `GET /changesets/{changeset_id}/conflicts` (agent or human) answers
//!   `{"conflicts": [Conflict, ...]}`, every conflict that the change set's
//!   rebases recorded, resolved ones included, after a merge too: they are
//!   stored for the review screen (US-046). An unknown change set is 404.
//! - `POST /conflicts/{conflict_id}/resolve` (agent) takes `{"note": text}`
//!   and answers 200 `{"conflict": Conflict}` with the resolution. A missing
//!   or blank note is 400 `{"error": "note_required"}`, a conflict resolved
//!   already is 409 `{"error": "already_resolved"}`, and neither changes
//!   anything; an unknown conflict is 404. Resolving emits `conflict.resolved`
//!   with `data` holding `conflict_id` and `changeset_id`. The agent fixes the
//!   draft with normal operations before it resolves.
//! - Submitting a change set while any of its conflicts is unresolved is 409
//!   `{"error": "unresolved_conflicts"}` and changes nothing.
//! - A change set that needs a rebase may be abandoned (FR-20).
//! - Conflicts may live in the existing tables; if they live in a tenant table
//!   `conflicts`, its columns without a default are at most `workspace_id`,
//!   `changeset_id`, `doc_id`, `node`, `mainline_version_id` and
//!   `changeset_version_id`, and resolutions live in that table too
//!   (tests/workspace_delete.rs and the isolation suite seed it).

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
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
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const TEST_DB: &str = "omni_us026_rebase";
const TEST_BUCKET: &str = "omni-us026-rebase";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const REBASE_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/rebase";
const CONFLICTS_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/conflicts";
const RESOLVE_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/conflicts/{conflict_id}/resolve";

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
    format!("us026-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

fn diff_of(f: &Fixture, client: &Client, project: &str, from: &str, to: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/versions/{from}/diff/{to}", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "compare two versions",
    )["diff"]
        .clone()
}

/// The node IDs of a diff list.
fn ids(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap_or_else(|| panic!("a list of nodes: {list}"))
        .iter()
        .map(id_of)
        .collect()
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
    /// The base doc's paragraphs, by index.
    paragraphs: Vec<String>,
    /// The mainline version now: the one the other change set merged.
    ahead_version: String,
    behind: String,
    /// The latest version that `behind` saved of the doc.
    behind_version: String,
    /// The IDs of the paragraphs that `behind` inserted, in order.
    behind_new: Vec<String>,
    /// A doc that `behind` started itself, when asked.
    fresh: Option<String>,
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
    let behind_new = apply(
        f,
        client,
        &project,
        &behind,
        &doc,
        &paragraphs,
        behind_changes,
    );
    let behind_version = save(f, client, &project, &behind, &doc);
    let fresh = with_fresh_doc.then(|| {
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
        fresh
    });
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
        paragraphs,
        ahead_version,
        behind,
        behind_version,
        behind_new,
        fresh,
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
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_rebase_and_conflict_routes_are_in_the_registry() {
    let routes = registry();
    for (method, path, access) in [
        ("POST", REBASE_ROUTE, Access::Agent),
        ("GET", CONFLICTS_ROUTE, Access::Both),
        ("POST", RESOLVE_ROUTE, Access::Agent),
    ] {
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
    }
}

#[test]
fn rebasing_is_refused_outside_needs_rebase_and_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let doc = create_doc(f, client, &project, &changeset);
    edit(
        f,
        client,
        &project,
        &changeset,
        &doc,
        vec![insert_after(None, "Not behind anything")],
    );
    save(f, client, &project, &changeset, &doc);

    // Open, then in review: neither needs a rebase.
    for status in ["open", "in_review"] {
        if status == "in_review" {
            submit(f, client, &project, &changeset);
        }
        let before = changeset_of(f, client, &project, &changeset);
        let response = rebase_response(f, client, &agent(f), &project, &changeset);
        assert_error(&response, 409, "invalid_transition");
        let after = changeset_of(f, client, &project, &changeset);
        assert_eq!(after["status"], status, "{after}");
        assert_eq!(moves(&after), moves(&before), "the history is unchanged");
    }

    // Merged: nothing is left to rebase.
    assert_eq!(approve(f, client, &project, &changeset)["status"], "merged");
    let before = changeset_of(f, client, &project, &changeset);
    let response = rebase_response(f, client, &agent(f), &project, &changeset);
    assert_error(&response, 409, "invalid_transition");
    assert_eq!(
        moves(&changeset_of(f, client, &project, &changeset)),
        moves(&before),
        "the history is unchanged"
    );

    // An unknown change set.
    let response = rebase_response(f, client, &agent(f), &project, UNKNOWN_ID);
    assert_error(&response, 404, "not_found");

    // The control: a change set that needs a rebase is rebased.
    let s = clean_scene(f, client);
    let answer = rebase(f, client, &s.project, &s.behind);
    assert_eq!(answer["changeset"]["id"], s.behind.as_str(), "{answer}");
}

#[test]
fn rebasing_is_the_agents_and_a_session_changes_nothing() {
    let (f, client) = ready();
    let s = clean_scene(f, client);
    let before = changeset_of(f, client, &s.project, &s.behind);

    let response = rebase_response(f, client, &human(f), &s.project, &s.behind);
    assert_error(&response, 403, "forbidden");
    let after = changeset_of(f, client, &s.project, &s.behind);
    assert_eq!(after["status"], "needs_rebase", "{after}");
    assert_eq!(moves(&after), moves(&before), "the history is unchanged");
    assert_eq!(
        mainline_of(f, client, &s.project, &s.doc),
        s.ahead_version.as_str(),
        "nothing merged"
    );

    // The control: the agent's key rebases the same change set.
    let answer = rebase(f, client, &s.project, &s.behind);
    assert_eq!(answer["changeset"]["status"], "merged", "{answer}");
}

// ---------------------------------------------------------------------------
// A rebase without conflicts
// ---------------------------------------------------------------------------

#[test]
fn a_rebase_without_conflicts_keeps_the_approval_and_merges() {
    let (f, client) = ready();
    let s = clean_scene(f, client);
    let fresh = s.fresh.clone().expect("the scene has a fresh doc");

    let answer = rebase(f, client, &s.project, &s.behind);
    assert_eq!(
        answer["conflicts"],
        json!([]),
        "no node changed on both sides: {answer}"
    );
    let changeset = &answer["changeset"];
    assert_eq!(changeset["status"], "merged", "{answer}");
    assert_eq!(
        last_moves(changeset, 3),
        vec![
            step("in_review", "approved", "human"),
            step("approved", "needs_rebase", "system"),
            step("needs_rebase", "merged", "agent"),
        ],
        "the approval is kept, not cleared, and the agent's rebase merges: {changeset}"
    );

    // The approval still covers every changed doc, and what it approves is
    // what merged.
    let shown = changeset_of(f, client, &s.project, &s.behind);
    assert_eq!(shown["status"], "merged", "{shown}");
    let approved = approved_versions(&shown);
    assert_eq!(
        approved.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([s.doc.clone(), fresh.clone()]),
        "the approval is kept for each changed doc: {shown}"
    );
    for (doc, version) in &approved {
        assert_eq!(
            mainline_of(f, client, &s.project, doc),
            version.as_str(),
            "the approved version of {doc} is its mainline version"
        );
    }
    assert_eq!(
        approved_versions(changeset),
        approved,
        "the answer carries the approval"
    );

    assert_eq!(
        events_of(&s.project, &s.behind, "changeset.merged"),
        vec![json!({
            "changeset_id": s.behind,
            "from_status": "needs_rebase",
            "status": "merged",
            "actor": "agent",
            "feature_id": null
        })],
        "one changeset.merged event"
    );
    assert_eq!(
        events_of(&s.project, &s.behind, "changeset.in_review").len(),
        1,
        "the approval was never cleared: only the submission moved it to in_review"
    );
    assert_eq!(
        conflicts_of(f, client, &s.project, &s.behind),
        Vec::<Value>::new(),
        "no conflicts are recorded"
    );
}

#[test]
fn a_clean_rebase_replays_the_change_sets_changes_onto_the_current_mainline() {
    let (f, client) = ready();
    let s = clean_scene(f, client);
    let fresh = s.fresh.clone().expect("the scene has a fresh doc");
    let [p1, p2, p3] = [&s.paragraphs[0], &s.paragraphs[1], &s.paragraphs[2]];
    let added = &s.behind_new[0];

    rebase(f, client, &s.project, &s.behind);

    // The mainline holds both change sets' work, with every node's ID.
    let merged = mainline_of(f, client, &s.project, &s.doc);
    let merged = merged.as_str().expect("a mainline version").to_string();
    assert_ne!(merged, s.ahead_version, "a new mainline version");
    assert_ne!(
        merged, s.behind_version,
        "the version on the old base is not merged as it is"
    );
    let html = mainline_html(f, client, &s.project, &s.doc);
    for text in [
        "Alpha by ahead",
        "Beta by behind",
        "Delta by behind",
        "Gamma base",
    ] {
        assert!(html.contains(text), "the mainline has {text:?}: {html}");
    }
    for text in ["Alpha base", "Beta base"] {
        assert!(!html.contains(text), "the mainline lost {text:?}: {html}");
    }
    assert_eq!(
        node_ids(&html),
        vec![p1.clone(), p2.clone(), added.clone(), p3.clone()],
        "the replayed nodes keep their IDs and places: {html}"
    );

    // Compared with the mainline it was rebased onto, the new version holds
    // exactly the change set's changes.
    let diff = diff_of(f, client, &s.project, &s.ahead_version, &merged);
    assert_eq!(ids(&diff["added"]), vec![added.clone()], "{diff}");
    assert_eq!(ids(&diff["changed"]), vec![p2.clone()], "{diff}");
    assert_eq!(ids(&diff["removed"]), Vec::<String>::new(), "{diff}");
    assert_eq!(ids(&diff["moved"]), Vec::<String>::new(), "{diff}");

    // The new version belongs to the change set, whose draft is now based on
    // the mainline version that it was rebased onto.
    let version = version_of(f, client, &s.project, &merged);
    assert_eq!(version["changeset_id"], s.behind.as_str(), "{version}");
    let draft = read_draft(f, client, &s.project, &s.behind, &s.doc);
    assert_eq!(
        draft["base_version_id"],
        s.ahead_version.as_str(),
        "{draft}"
    );

    // The doc that the change set started was current all along and merges.
    let fresh_mainline = mainline_of(f, client, &s.project, &fresh);
    assert!(fresh_mainline.is_string(), "the new doc merged");
    let fresh_html = mainline_html(f, client, &s.project, &fresh);
    assert!(fresh_html.contains("Fresh doc text"), "{fresh_html}");
}

// ---------------------------------------------------------------------------
// A rebase with conflicts
// ---------------------------------------------------------------------------

#[test]
fn a_rebase_with_conflicts_clears_the_approval_and_reopens_the_change_set() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let before = changeset_of(f, client, &s.project, &s.behind);
    assert!(
        !approved_versions(&before).is_empty(),
        "set up an approval: {before}"
    );

    let answer = rebase(f, client, &s.project, &s.behind);
    let changeset = &answer["changeset"];
    assert_eq!(changeset["status"], "open", "{answer}");
    assert_eq!(
        last_moves(changeset, 2),
        vec![
            step("approved", "needs_rebase", "system"),
            step("needs_rebase", "open", "agent"),
        ],
        "{changeset}"
    );
    let shown = changeset_of(f, client, &s.project, &s.behind);
    assert_eq!(shown["status"], "open", "{shown}");
    assert_eq!(
        approved_versions(&shown),
        BTreeMap::new(),
        "the approval is cleared: {shown}"
    );
    assert_eq!(approved_versions(changeset), BTreeMap::new(), "{changeset}");

    assert_eq!(
        events_of(&s.project, &s.behind, "changeset.open")
            .into_iter()
            .filter(|e| e["from_status"] == "needs_rebase")
            .collect::<Vec<_>>(),
        vec![json!({
            "changeset_id": s.behind,
            "from_status": "needs_rebase",
            "status": "open",
            "actor": "agent",
            "feature_id": null
        })],
        "one changeset.open event for the rebase"
    );
    assert!(
        events_of(&s.project, &s.behind, "changeset.merged").is_empty(),
        "nothing merged"
    );
    assert_eq!(
        mainline_of(f, client, &s.project, &s.doc),
        s.ahead_version.as_str(),
        "the mainline is untouched"
    );
}

#[test]
fn every_node_changed_on_both_sides_becomes_a_conflict_with_both_versions_of_the_node() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let [p1, p2, p3] = [&s.paragraphs[0], &s.paragraphs[1], &s.paragraphs[2]];

    let answer = rebase(f, client, &s.project, &s.behind);
    let recorded = answer["conflicts"]
        .as_array()
        .unwrap_or_else(|| panic!("the rebase answers its conflicts: {answer}"))
        .clone();
    let found = by_node(&recorded);
    assert_eq!(
        found.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([p1.clone(), p2.clone()]),
        "the paragraphs changed on both sides conflict, the one only the change set \
         changed does not ({p3}): {answer}"
    );
    assert_eq!(
        conflict_ids(&recorded).len(),
        2,
        "each conflict has an ID of its own: {answer}"
    );

    for conflict in found.values() {
        assert!(conflict["id"].is_string(), "{conflict}");
        assert_eq!(conflict["changeset_id"], s.behind.as_str(), "{conflict}");
        assert_eq!(conflict["doc_id"], s.doc.as_str(), "{conflict}");
        assert_eq!(
            conflict["mainline"]["version_id"],
            s.ahead_version.as_str(),
            "the mainline side comes from the mainline version: {conflict}"
        );
        assert_eq!(
            conflict["changeset"]["version_id"],
            s.behind_version.as_str(),
            "the change set's side comes from its latest version: {conflict}"
        );
        assert_eq!(conflict["resolution"], Value::Null, "{conflict}");
    }

    // Both edited the first paragraph: both versions of it are recorded.
    let both = &found[p1];
    let mainline = both["mainline"]["html"]
        .as_str()
        .expect("the mainline side");
    let changeset = both["changeset"]["html"]
        .as_str()
        .expect("the change set's side");
    assert!(
        mainline.contains("Alpha by ahead") && mainline.contains(&format!("data-node-id=\"{p1}\"")),
        "{both}"
    );
    assert!(
        changeset.contains("Alpha by behind")
            && changeset.contains(&format!("data-node-id=\"{p1}\"")),
        "{both}"
    );

    // The mainline removed the second paragraph that the change set edited.
    let removed = &found[p2];
    assert_eq!(
        removed["mainline"]["html"],
        Value::Null,
        "the mainline removed it: {removed}"
    );
    let changeset = removed["changeset"]["html"]
        .as_str()
        .expect("the change set's side");
    assert!(changeset.contains("Beta by behind"), "{removed}");

    // The list shows the same conflicts.
    let listed = conflicts_of(f, client, &s.project, &s.behind);
    assert_eq!(
        by_node(&listed),
        found,
        "the stored conflicts are those the rebase answered"
    );
}

#[test]
fn conflicting_nodes_keep_their_mainline_state_in_the_draft() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let [p1, p2, p3] = [&s.paragraphs[0], &s.paragraphs[1], &s.paragraphs[2]];

    rebase(f, client, &s.project, &s.behind);

    let draft = read_draft(f, client, &s.project, &s.behind, &s.doc);
    assert_eq!(
        draft["base_version_id"],
        s.ahead_version.as_str(),
        "the draft is based on the current mainline version: {draft}"
    );
    let html = draft["html"].as_str().expect("the draft's html");
    assert!(
        html.contains("Alpha by ahead") && !html.contains("Alpha by behind"),
        "the first paragraph keeps its mainline state: {html}"
    );
    assert!(
        !node_ids(html).contains(p2) && !html.contains("Beta by behind"),
        "the second paragraph stays removed, as on the mainline: {html}"
    );
    assert!(
        html.contains("Gamma by behind") && !html.contains("Gamma base"),
        "the change set's change to the third paragraph is replayed: {html}"
    );
    assert_eq!(
        node_ids(html),
        vec![p1.clone(), p3.clone()],
        "the nodes keep their IDs: {html}"
    );
}

// ---------------------------------------------------------------------------
// Resolving
// ---------------------------------------------------------------------------

#[test]
fn the_agent_fixes_a_conflict_with_operations_and_resolves_it_with_a_note() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let p1 = &s.paragraphs[0];
    rebase(f, client, &s.project, &s.behind);
    let conflicts = by_node(&conflicts_of(f, client, &s.project, &s.behind));
    let conflict = id_of(&conflicts[p1]);

    // A normal operation on the draft fixes the paragraph.
    let html = draft_html(f, client, &s.project, &s.behind, &s.doc);
    edit(
        f,
        client,
        &s.project,
        &s.behind,
        &s.doc,
        vec![json!({
            "op": "set",
            "node": p1,
            "rev": rev_in(&html, p1),
            "properties": { "html": "Alpha by hand" }
        })],
    );
    save(f, client, &s.project, &s.behind, &s.doc);

    let note = "Kept the ahead wording and added the behind detail.";
    let resolved = resolve(f, client, &s.project, &conflict, note);
    assert_eq!(id_of(&resolved), conflict, "{resolved}");
    assert_eq!(resolved["node"], p1.as_str(), "{resolved}");
    assert_eq!(resolved["resolution"]["note"], note, "{resolved}");
    assert!(
        resolved["resolution"]["resolved_at"].is_string(),
        "{resolved}"
    );

    let listed = by_node(&conflicts_of(f, client, &s.project, &s.behind));
    assert_eq!(listed[p1]["resolution"]["note"], note, "{listed:?}");
    assert_eq!(
        listed[&s.paragraphs[1]]["resolution"],
        Value::Null,
        "the other conflict is still open"
    );

    let events = events_named(&s.project, "conflict.resolved");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["conflict_id"], conflict.as_str(), "{events:?}");
    assert_eq!(events[0]["changeset_id"], s.behind.as_str(), "{events:?}");
}

#[test]
fn resolving_needs_a_note_and_happens_once() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    rebase(f, client, &s.project, &s.behind);
    let conflicts = conflicts_of(f, client, &s.project, &s.behind);
    let conflict = id_of(&conflicts[0]);

    for body in [json!({}), json!({ "note": "" }), json!({ "note": "   " })] {
        let response = resolve_response(f, client, &s.project, &conflict, &body);
        assert_error(&response, 400, "note_required");
    }
    let listed = conflicts_of(f, client, &s.project, &s.behind);
    assert!(
        listed.iter().all(|c| c["resolution"].is_null()),
        "nothing was resolved: {listed:?}"
    );

    let response = resolve_response(
        f,
        client,
        &s.project,
        UNKNOWN_ID,
        &json!({ "note": "Nothing to resolve" }),
    );
    assert_error(&response, 404, "not_found");

    resolve(f, client, &s.project, &conflict, "First note");
    let response = resolve_response(
        f,
        client,
        &s.project,
        &conflict,
        &json!({ "note": "Second note" }),
    );
    assert_error(&response, 409, "already_resolved");
    let listed = conflicts_of(f, client, &s.project, &s.behind);
    let again = listed
        .iter()
        .find(|c| id_of(c) == conflict)
        .expect("the conflict is listed");
    assert_eq!(
        again["resolution"]["note"], "First note",
        "the first resolution stands: {again}"
    );

    // The conflicts of an unknown change set.
    let response = conflicts_response(f, client, &agent(f), &s.project, UNKNOWN_ID);
    assert_error(&response, 404, "not_found");
}

#[test]
fn a_change_set_cannot_be_submitted_while_a_conflict_is_unresolved() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    rebase(f, client, &s.project, &s.behind);
    let conflicts = conflicts_of(f, client, &s.project, &s.behind);
    assert_eq!(conflicts.len(), 2, "{conflicts:?}");

    let before = changeset_of(f, client, &s.project, &s.behind);
    let response = submit_response(f, client, &s.project, &s.behind);
    assert_error(&response, 409, "unresolved_conflicts");
    let after = changeset_of(f, client, &s.project, &s.behind);
    assert_eq!(after["status"], "open", "{after}");
    assert_eq!(moves(&after), moves(&before), "the history is unchanged");

    // One of two resolved is not enough.
    resolve(
        f,
        client,
        &s.project,
        &id_of(&conflicts[0]),
        "Fixed the first",
    );
    let response = submit_response(f, client, &s.project, &s.behind);
    assert_error(&response, 409, "unresolved_conflicts");

    resolve(
        f,
        client,
        &s.project,
        &id_of(&conflicts[1]),
        "Fixed the second",
    );
    submit(f, client, &s.project, &s.behind);
}

#[test]
fn once_resolved_the_change_set_merges_with_both_features_and_keeps_its_conflicts() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let [p1, p2] = [&s.paragraphs[0], &s.paragraphs[1]];
    rebase(f, client, &s.project, &s.behind);
    let conflicts = by_node(&conflicts_of(f, client, &s.project, &s.behind));

    // The agent rewrites the first paragraph, accepts that the second is gone,
    // saves and resolves both.
    let html = draft_html(f, client, &s.project, &s.behind, &s.doc);
    edit(
        f,
        client,
        &s.project,
        &s.behind,
        &s.doc,
        vec![json!({
            "op": "set",
            "node": p1,
            "rev": rev_in(&html, p1),
            "properties": { "html": "Alpha by hand" }
        })],
    );
    let fixed = save(f, client, &s.project, &s.behind, &s.doc);
    resolve(
        f,
        client,
        &s.project,
        &id_of(&conflicts[p1]),
        "Merged both wordings",
    );
    resolve(
        f,
        client,
        &s.project,
        &id_of(&conflicts[p2]),
        "The paragraph stays removed",
    );

    submit(f, client, &s.project, &s.behind);
    let merged = approve(f, client, &s.project, &s.behind);
    assert_eq!(merged["status"], "merged", "{merged}");
    assert_eq!(
        mainline_of(f, client, &s.project, &s.doc),
        fixed.as_str(),
        "the fixed version is the mainline"
    );
    let html = mainline_html(f, client, &s.project, &s.doc);
    assert!(html.contains("Alpha by hand"), "{html}");
    assert!(html.contains("Gamma by behind"), "{html}");
    assert!(!html.contains("Beta"), "{html}");

    // The conflicts and how each was resolved stay stored for the review
    // screen, which reads them with the human's session.
    let listed = must(
        conflicts_response(f, client, &human(f), &s.project, &s.behind),
        200,
        "the human lists the conflicts",
    )["conflicts"]
        .as_array()
        .expect("conflicts")
        .clone();
    let listed = by_node(&listed);
    assert_eq!(
        listed.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([p1.clone(), p2.clone()]),
        "{listed:?}"
    );
    assert_eq!(listed[p1]["resolution"]["note"], "Merged both wordings");
    assert_eq!(
        listed[p2]["resolution"]["note"],
        "The paragraph stays removed"
    );
    assert!(
        listed[p1]["mainline"]["html"]
            .as_str()
            .is_some_and(|h| h.contains("Alpha by ahead")),
        "both sides stay recorded: {:?}",
        listed[p1]
    );
    assert!(
        listed[p1]["changeset"]["html"]
            .as_str()
            .is_some_and(|h| h.contains("Alpha by behind")),
        "both sides stay recorded: {:?}",
        listed[p1]
    );
}

#[test]
fn a_rebase_with_conflicts_never_drops_what_the_mainline_gained() {
    // The agent resolves every conflict by keeping the mainline state and
    // saves nothing more: what merges is the rebased state, not the change
    // set's version on the old base.
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    rebase(f, client, &s.project, &s.behind);
    for conflict in conflicts_of(f, client, &s.project, &s.behind) {
        resolve(
            f,
            client,
            &s.project,
            &id_of(&conflict),
            "Kept the mainline",
        );
    }
    submit(f, client, &s.project, &s.behind);
    let merged = approve(f, client, &s.project, &s.behind);
    assert_eq!(merged["status"], "merged", "{merged}");
    assert_ne!(
        approved_versions(&merged).get(&s.doc),
        Some(&s.behind_version),
        "the version on the old base is not what is approved: {merged}"
    );

    let html = mainline_html(f, client, &s.project, &s.doc);
    assert!(
        html.contains("Alpha by ahead"),
        "the mainline's change survives: {html}"
    );
    assert!(
        !html.contains("Beta"),
        "the mainline's removal survives: {html}"
    );
    assert!(
        html.contains("Gamma by behind"),
        "the change set's own change merges: {html}"
    );
    assert!(!html.contains("Alpha by behind"), "{html}");
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

/// The behind change set makes its changes, saves and is approved on the
/// old base.
fn fall_behind(f: &Fixture, client: &Client, x: &Fork, ops: Vec<Value>) {
    edit_each(f, client, x, &x.behind, ops);
    save(f, client, &x.project, &x.behind, &x.doc);
    submit(f, client, &x.project, &x.behind);
    let approved = approve(f, client, &x.project, &x.behind);
    assert_eq!(
        approved["status"], "needs_rebase",
        "set up a change set behind the mainline: {approved}"
    );
}

fn move_to(node: &str, parent: Option<&str>, after: Option<&str>) -> Value {
    json!({
        "op": "move",
        "node": node,
        "parent": parent,
        "position": { "after": after }
    })
}

fn set_text(node: &str, text: &str) -> Value {
    json!({ "op": "set", "node": node, "properties": { "html": text } })
}

fn delete(node: &str) -> Value {
    json!({ "op": "delete", "node": node })
}

/// The base of the nested cases: an intro, then a section of two paragraphs.
const NESTED: [&str; 2] = ["<p>Intro</p>", "<section><p>One</p><p>Two</p></section>"];

/// The parents of the nodes in `html` as node ID -> parent ID, `None` at the
/// top level, read from the nesting of the tags that carry `data-node-id`.
fn parents(html: &str) -> BTreeMap<String, Option<String>> {
    let mut found = BTreeMap::new();
    let mut open: Vec<Option<String>> = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start + 1..start + end];
        rest = &rest[start + end + 1..];
        if tag.starts_with('/') {
            open.pop();
        } else if !tag.ends_with('/') && !VOID.iter().any(|v| tag.split(' ').next() == Some(v)) {
            let parent = open.iter().rev().flatten().next().cloned();
            let id = attribute(tag, "data-node-id");
            if let Some(id) = &id {
                found.insert(id.clone(), parent);
            }
            open.push(id);
        }
    }
    found
}

/// Elements without a closing tag.
const VOID: [&str; 4] = ["br", "img", "hr", "wbr"];

#[test]
fn a_section_removed_after_its_children_moved_out_rebases_cleanly() {
    // Nested structure: the change set moves both paragraphs out of the
    // section and then removes the empty section, while the mainline edits
    // the intro. Only the intro changed on the mainline, so nothing changed
    // on both sides.
    let (f, client) = ready();
    let x = fork(f, client, &NESTED);
    let [intro, section, one, two] = <[String; 4]>::try_from(x.nodes.clone())
        .unwrap_or_else(|ids| panic!("four nodes in document order: {ids:?}"));
    let ahead_version = merge_ahead(f, client, &x, vec![set_text(&intro, "Intro by ahead")]);
    fall_behind(
        f,
        client,
        &x,
        vec![
            move_to(&one, None, Some(&intro)),
            move_to(&two, None, Some(&one)),
            delete(&section),
        ],
    );

    let answer = rebase(f, client, &x.project, &x.behind);
    assert_eq!(
        answer["conflicts"],
        json!([]),
        "the section's children left before it was removed, so its removal \
         conflicts with nothing: {answer}"
    );
    assert_eq!(answer["changeset"]["status"], "merged", "{answer}");
    assert_eq!(
        conflicts_of(f, client, &x.project, &x.behind),
        Vec::<Value>::new(),
        "no conflicts are recorded"
    );

    let html = mainline_html(f, client, &x.project, &x.doc);
    assert!(html.contains("Intro by ahead"), "{html}");
    assert_eq!(
        node_ids(&html),
        vec![intro.clone(), one.clone(), two.clone()],
        "the paragraphs moved out and the section is gone: {html}"
    );
    assert_eq!(
        parents(&html),
        BTreeMap::from([(intro, None), (one.clone(), None), (two.clone(), None)]),
        "{html}"
    );

    let merged = mainline_of(f, client, &x.project, &x.doc);
    let merged = merged.as_str().expect("a mainline version");
    let diff = diff_of(f, client, &x.project, &ahead_version, merged);
    assert_eq!(ids(&diff["removed"]), vec![section], "{diff}");
    assert_eq!(ids(&diff["moved"]), vec![one, two], "{diff}");
    assert_eq!(ids(&diff["added"]), Vec::<String>::new(), "{diff}");
    assert_eq!(ids(&diff["changed"]), Vec::<String>::new(), "{diff}");
}

#[test]
fn a_section_removed_after_its_children_were_wrapped_in_a_new_one_rebases_cleanly() {
    // The change set wraps the paragraphs in a section of its own and removes
    // the old one; the mainline edits the intro.
    let (f, client) = ready();
    let x = fork(f, client, &NESTED);
    let [intro, section, one, two] = <[String; 4]>::try_from(x.nodes.clone())
        .unwrap_or_else(|ids| panic!("four nodes in document order: {ids:?}"));
    merge_ahead(f, client, &x, vec![set_text(&intro, "Intro by ahead")]);

    let results = edit(
        f,
        client,
        &x.project,
        &x.behind,
        &x.doc,
        vec![json!({
            "op": "insert",
            "parent": null,
            "position": { "after": intro },
            "content": "<section><h2>Wrapped</h2></section>"
        })],
    );
    let created: Vec<String> = results[0]["created"]
        .as_array()
        .expect("the created nodes")
        .iter()
        .map(|id| id.as_str().expect("an ID").to_string())
        .collect();
    let [wrap, heading] = <[String; 2]>::try_from(created)
        .unwrap_or_else(|ids| panic!("a section and its heading: {ids:?}"));
    fall_behind(
        f,
        client,
        &x,
        vec![
            move_to(&one, Some(&wrap), Some(&heading)),
            move_to(&two, Some(&wrap), Some(&one)),
            delete(&section),
        ],
    );

    let answer = rebase(f, client, &x.project, &x.behind);
    assert_eq!(
        answer["conflicts"],
        json!([]),
        "the old section's children left before it was removed: {answer}"
    );
    assert_eq!(answer["changeset"]["status"], "merged", "{answer}");

    let html = mainline_html(f, client, &x.project, &x.doc);
    assert!(
        html.contains("Intro by ahead") && html.contains("Wrapped"),
        "{html}"
    );
    assert_eq!(
        node_ids(&html),
        vec![
            intro.clone(),
            wrap.clone(),
            heading.clone(),
            one.clone(),
            two.clone()
        ],
        "the paragraphs sit in the new section and the old one is gone: {html}"
    );
    assert_eq!(
        parents(&html),
        BTreeMap::from([
            (intro, None),
            (wrap.clone(), None),
            (heading, Some(wrap.clone())),
            (one, Some(wrap.clone())),
            (two, Some(wrap)),
        ]),
        "{html}"
    );
}

#[test]
fn a_reorder_on_each_side_keeps_both_or_becomes_a_conflict() {
    // Base A B C D. The mainline moves D to the front (D A B C); the change
    // set moves A to the end (B C D A). Replaying "A after D" onto the
    // mainline would leave D A B C and silently drop the change set's move.
    let (f, client) = ready();
    let x = fork(f, client, &["<p>A</p>", "<p>B</p>", "<p>C</p>", "<p>D</p>"]);
    let [a, b, c, d] = <[String; 4]>::try_from(x.nodes.clone())
        .unwrap_or_else(|ids| panic!("four nodes in document order: {ids:?}"));
    let ahead_version = merge_ahead(f, client, &x, vec![move_to(&d, None, None)]);
    assert_eq!(
        node_ids(&mainline_html(f, client, &x.project, &x.doc)),
        vec![d.clone(), a.clone(), b.clone(), c.clone()],
        "set up the mainline's reorder"
    );
    fall_behind(f, client, &x, vec![move_to(&a, None, Some(&d))]);

    let answer = rebase(f, client, &x.project, &x.behind);
    let recorded = answer["conflicts"]
        .as_array()
        .unwrap_or_else(|| panic!("the rebase answers its conflicts: {answer}"))
        .clone();
    if recorded.is_empty() {
        // Both moves survive: D first as on the mainline, A last as in the
        // change set.
        assert_eq!(answer["changeset"]["status"], "merged", "{answer}");
        assert_eq!(
            node_ids(&mainline_html(f, client, &x.project, &x.doc)),
            vec![d, b, c, a],
            "a clean rebase keeps the mainline's move and the change set's move"
        );
    } else {
        // Or the two reorders conflict: nothing merges and the conflict names
        // a node that one of them moved.
        assert_eq!(answer["changeset"]["status"], "open", "{answer}");
        let nodes: BTreeSet<String> = by_node(&recorded).keys().cloned().collect();
        assert!(
            nodes.is_subset(&BTreeSet::from([a.clone(), d.clone()])),
            "only the moved nodes conflict: {answer}"
        );
        assert_eq!(
            mainline_of(f, client, &x.project, &x.doc),
            ahead_version.as_str(),
            "nothing merged"
        );
        let draft = node_ids(&draft_html(f, client, &x.project, &x.behind, &x.doc));
        assert_eq!(
            draft.first(),
            Some(&d),
            "the mainline's move stays in the draft: {draft:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The rest of the lifecycle
// ---------------------------------------------------------------------------

#[test]
fn a_change_set_that_needs_a_rebase_can_be_abandoned() {
    let (f, client) = ready();
    let s = clean_scene(f, client);
    let answer = must(
        call(
            client,
            "POST",
            &format!("{}/abandon", changeset_path(f, &s.project, &s.behind)),
            &agent(f),
            None,
        ),
        200,
        "abandon the change set that needs a rebase",
    );
    assert_eq!(answer["changeset"]["status"], "abandoned", "{answer}");
    assert_eq!(
        last_moves(&answer["changeset"], 1),
        vec![step("needs_rebase", "abandoned", "agent")],
        "{answer}"
    );
    assert_eq!(
        mainline_of(f, client, &s.project, &s.doc),
        s.ahead_version.as_str(),
        "nothing merged"
    );
}

#[test]
fn the_same_node_reordered_differently_on_both_sides_is_a_conflict() {
    // Base A B C D. The mainline moves A to the end (B C D A); the change set
    // moves A behind C (B C A D).
    let (f, client) = ready();
    let x = fork(f, client, &["<p>A</p>", "<p>B</p>", "<p>C</p>", "<p>D</p>"]);
    let [a, b, c, d] = <[String; 4]>::try_from(x.nodes.clone())
        .unwrap_or_else(|ids| panic!("four nodes in document order: {ids:?}"));
    let ahead_version = merge_ahead(f, client, &x, vec![move_to(&a, None, Some(&d))]);
    fall_behind(f, client, &x, vec![move_to(&a, None, Some(&c))]);

    let answer = rebase(f, client, &x.project, &x.behind);
    let recorded = answer["conflicts"]
        .as_array()
        .unwrap_or_else(|| panic!("the rebase answers its conflicts: {answer}"))
        .clone();
    assert_eq!(
        by_node(&recorded).keys().cloned().collect::<Vec<_>>(),
        vec![a.clone()],
        "A was reordered on both sides to different places: {answer}"
    );
    assert_eq!(answer["changeset"]["status"], "open", "{answer}");
    assert_eq!(
        mainline_of(f, client, &x.project, &x.doc),
        ahead_version.as_str(),
        "nothing merged"
    );
    assert_eq!(
        node_ids(&draft_html(f, client, &x.project, &x.behind, &x.doc)),
        vec![b, c, d, a],
        "the conflicting node keeps its mainline place in the draft"
    );
}

#[test]
fn the_same_node_reordered_to_the_same_place_on_both_sides_rebases_cleanly() {
    let (f, client) = ready();
    let x = fork(f, client, &["<p>A</p>", "<p>B</p>", "<p>C</p>", "<p>D</p>"]);
    let [a, b, c, d] = <[String; 4]>::try_from(x.nodes.clone())
        .unwrap_or_else(|ids| panic!("four nodes in document order: {ids:?}"));
    merge_ahead(f, client, &x, vec![move_to(&a, None, Some(&d))]);
    fall_behind(f, client, &x, vec![move_to(&a, None, Some(&d))]);

    let answer = rebase(f, client, &x.project, &x.behind);
    assert_eq!(answer["conflicts"], json!([]), "{answer}");
    assert_eq!(answer["changeset"]["status"], "merged", "{answer}");
    assert_eq!(
        node_ids(&mainline_html(f, client, &x.project, &x.doc)),
        vec![b, c, d, a]
    );
}

// ---------------------------------------------------------------------------
// Removed docs
// ---------------------------------------------------------------------------

/// Removes the doc in the change set; returns the version that marks it
/// removed.
fn remove_doc(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> String {
    let version = must(
        call(
            client,
            "POST",
            &format!("{}/remove", draft_path(f, project, changeset, doc)),
            &agent(f),
            Some(&json!({})),
        ),
        201,
        "remove the doc",
    )["version"]
        .clone();
    assert_eq!(version["removed"], true, "{version}");
    id_of(&version)
}

/// The ahead change set removes the doc and merges; returns the mainline
/// version, the removal.
fn merge_ahead_removal(f: &Fixture, client: &Client, x: &Fork) -> String {
    let removal = remove_doc(f, client, &x.project, &x.ahead, &x.doc);
    submit(f, client, &x.project, &x.ahead);
    let merged = approve(f, client, &x.project, &x.ahead);
    assert_eq!(merged["status"], "merged", "merge the removal: {merged}");
    assert_eq!(mainline_of(f, client, &x.project, &x.doc), removal.as_str());
    removal
}

/// The behind change set removes the doc and is approved on the old base.
fn fall_behind_with_removal(f: &Fixture, client: &Client, x: &Fork) -> String {
    let removal = remove_doc(f, client, &x.project, &x.behind, &x.doc);
    submit(f, client, &x.project, &x.behind);
    let approved = approve(f, client, &x.project, &x.behind);
    assert_eq!(
        approved["status"], "needs_rebase",
        "set up a change set behind the mainline: {approved}"
    );
    removal
}

/// Whether the doc's mainline version marks it removed.
fn mainline_removed(f: &Fixture, client: &Client, project: &str, doc: &str) -> bool {
    let mainline = mainline_of(f, client, project, doc);
    let mainline = mainline.as_str().expect("the doc has a mainline version");
    version_of(f, client, project, mainline)["removed"]
        .as_bool()
        .unwrap_or_else(|| panic!("a version says whether it is a removal"))
}

/// Every conflict of the answer is one of the doc, with the mainline side
/// from `mainline_version`; returns them.
fn doc_conflicts(answer: &Value, doc: &str, mainline_version: &str) -> Vec<Value> {
    let recorded = answer["conflicts"]
        .as_array()
        .unwrap_or_else(|| panic!("the rebase answers its conflicts: {answer}"))
        .clone();
    assert!(
        !recorded.is_empty(),
        "one side removed the doc and the other changed it: a conflict: {answer}"
    );
    for conflict in &recorded {
        assert_eq!(conflict["doc_id"], doc, "{conflict}");
        assert_eq!(
            conflict["mainline"]["version_id"], mainline_version,
            "{conflict}"
        );
    }
    recorded
}

#[test]
fn a_doc_the_mainline_removed_conflicts_with_a_change_set_that_added_to_it() {
    // The change set only adds a paragraph; the mainline removed the doc.
    let (f, client) = ready();
    let x = fork(f, client, &["<p>A</p>", "<p>B</p>"]);
    let b = x.nodes[1].clone();
    let removal = merge_ahead_removal(f, client, &x);
    fall_behind(
        f,
        client,
        &x,
        vec![json!({
            "op": "insert",
            "parent": null,
            "position": { "after": b },
            "content": "<p>Added by behind</p>"
        })],
    );

    let answer = rebase(f, client, &x.project, &x.behind);
    let recorded = doc_conflicts(&answer, &x.doc, &removal);
    assert_eq!(answer["changeset"]["status"], "open", "{answer}");
    assert_eq!(
        mainline_of(f, client, &x.project, &x.doc),
        removal.as_str(),
        "nothing merged"
    );
    let html = draft_html(f, client, &x.project, &x.behind, &x.doc);
    assert_eq!(
        node_ids(&html),
        Vec::<String>::new(),
        "the draft keeps the mainline state, a removed doc: {html}"
    );

    // Resolved without another save, what merges keeps the doc removed.
    for conflict in &recorded {
        resolve(f, client, &x.project, &id_of(conflict), "Kept the removal");
    }
    submit(f, client, &x.project, &x.behind);
    let merged = approve(f, client, &x.project, &x.behind);
    assert_eq!(merged["status"], "merged", "{merged}");
    assert!(
        mainline_removed(f, client, &x.project, &x.doc),
        "the removed doc is not brought back"
    );
}

#[test]
fn a_doc_the_mainline_removed_conflicts_with_a_change_set_that_edited_it() {
    let (f, client) = ready();
    let x = fork(f, client, &["<p>A</p>", "<p>B</p>"]);
    let a = x.nodes[0].clone();
    let removal = merge_ahead_removal(f, client, &x);
    fall_behind(f, client, &x, vec![set_text(&a, "Alpha by behind")]);

    let answer = rebase(f, client, &x.project, &x.behind);
    let recorded = doc_conflicts(&answer, &x.doc, &removal);
    assert_eq!(answer["changeset"]["status"], "open", "{answer}");
    let html = draft_html(f, client, &x.project, &x.behind, &x.doc);
    assert_eq!(
        node_ids(&html),
        Vec::<String>::new(),
        "the draft keeps the mainline state, a removed doc: {html}"
    );

    // Resolved without another save, what merges is the removal, not an
    // empty doc brought back.
    for conflict in &recorded {
        resolve(f, client, &x.project, &id_of(conflict), "Kept the removal");
    }
    submit(f, client, &x.project, &x.behind);
    let merged = approve(f, client, &x.project, &x.behind);
    assert_eq!(merged["status"], "merged", "{merged}");
    assert!(
        mainline_removed(f, client, &x.project, &x.doc),
        "the removed doc is not brought back"
    );
}

#[test]
fn a_doc_the_change_set_removed_conflicts_with_mainline_changes_and_keeps_the_mainline_state() {
    // Once the mainline edits a paragraph, once it only adds one.
    let (f, client) = ready();
    for edits_existing in [true, false] {
        let x = fork(f, client, &["<p>A</p>", "<p>B</p>", "<p>C</p>"]);
        let [a, b, c] = <[String; 3]>::try_from(x.nodes.clone())
            .unwrap_or_else(|ids| panic!("three nodes in document order: {ids:?}"));
        let ahead_version = if edits_existing {
            merge_ahead(f, client, &x, vec![set_text(&b, "Beta by ahead")])
        } else {
            merge_ahead(
                f,
                client,
                &x,
                vec![insert_after(Some(&c), "Delta by ahead")],
            )
        };
        let expected = node_ids(&mainline_html(f, client, &x.project, &x.doc));
        fall_behind_with_removal(f, client, &x);

        let answer = rebase(f, client, &x.project, &x.behind);
        doc_conflicts(&answer, &x.doc, &ahead_version);
        assert_eq!(answer["changeset"]["status"], "open", "{answer}");
        assert_eq!(
            mainline_of(f, client, &x.project, &x.doc),
            ahead_version.as_str(),
            "nothing merged"
        );
        let html = draft_html(f, client, &x.project, &x.behind, &x.doc);
        assert_eq!(
            node_ids(&html),
            expected,
            "the draft holds the whole mainline version, the paragraphs that \
             neither side edited ({a}, {c}) too: {html}"
        );
        let text = if edits_existing {
            "Beta by ahead"
        } else {
            "Delta by ahead"
        };
        assert!(html.contains(text), "the mainline's change stays: {html}");
    }
}

#[test]
fn a_removal_survives_a_rebase_when_the_other_side_changed_nothing_in_the_doc() {
    let (f, client) = ready();

    // The mainline removed the doc; the change set saved it without a change.
    let x = fork(f, client, &["<p>A</p>", "<p>B</p>"]);
    merge_ahead_removal(f, client, &x);
    fall_behind(f, client, &x, vec![]);
    let answer = rebase(f, client, &x.project, &x.behind);
    assert_eq!(answer["conflicts"], json!([]), "{answer}");
    assert_eq!(answer["changeset"]["status"], "merged", "{answer}");
    assert!(
        mainline_removed(f, client, &x.project, &x.doc),
        "the mainline's removal is not undone by a change set that changed nothing"
    );

    // Both sides removed the doc.
    let x = fork(f, client, &["<p>A</p>", "<p>B</p>"]);
    merge_ahead_removal(f, client, &x);
    fall_behind_with_removal(f, client, &x);
    let answer = rebase(f, client, &x.project, &x.behind);
    assert_eq!(answer["conflicts"], json!([]), "{answer}");
    assert_eq!(answer["changeset"]["status"], "merged", "{answer}");
    assert!(
        mainline_removed(f, client, &x.project, &x.doc),
        "the doc stays removed"
    );
}

// ---------------------------------------------------------------------------
// Unsaved work
// ---------------------------------------------------------------------------

#[test]
fn a_rebase_with_unsaved_operations_is_refused_and_changes_nothing() {
    let (f, client) = ready();
    let s = clean_scene(f, client);
    // The agent edits the draft after its latest saved version.
    edit(
        f,
        client,
        &s.project,
        &s.behind,
        &s.doc,
        vec![insert_after(Some(&s.paragraphs[2]), "Unsaved by behind")],
    );
    let before = changeset_of(f, client, &s.project, &s.behind);
    assert_eq!(before["status"], "needs_rebase", "{before}");

    let response = rebase_response(f, client, &agent(f), &s.project, &s.behind);
    assert_error(&response, 409, "unsaved_operations");

    let after = changeset_of(f, client, &s.project, &s.behind);
    assert_eq!(after["status"], "needs_rebase", "{after}");
    assert_eq!(moves(&after), moves(&before), "the history is unchanged");
    assert_eq!(
        approved_versions(&after),
        approved_versions(&before),
        "the approval is unchanged"
    );
    assert_eq!(
        mainline_of(f, client, &s.project, &s.doc),
        s.ahead_version.as_str(),
        "nothing merged"
    );
    assert_eq!(
        conflicts_of(f, client, &s.project, &s.behind),
        Vec::<Value>::new()
    );
    let draft = read_draft(f, client, &s.project, &s.behind, &s.doc);
    assert_ne!(
        draft["base_version_id"],
        s.ahead_version.as_str(),
        "the draft is not rebased: {draft}"
    );
    let html = draft["html"].as_str().expect("the draft's html");
    assert!(
        html.contains("Unsaved by behind") && html.contains("Beta by behind"),
        "the unsaved work is still in the draft: {html}"
    );
}

// ---------------------------------------------------------------------------
// Requests that wait on a rebase
// ---------------------------------------------------------------------------

/// A lock on a change set's row, held by another connection until it is
/// released: a rebase of the change set then waits in its transaction while
/// it holds its drafts.
struct RowLock {
    /// The backend that holds the lock.
    pid: i32,
    release: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

impl RowLock {
    fn on_changeset(changeset: &str) -> RowLock {
        let (locked, is_locked) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let changeset = changeset.to_string();
        let thread = std::thread::spawn(move || {
            block(async move {
                let mut conn = admin_connection().await;
                let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
                    .fetch_one(&mut conn)
                    .await
                    .expect("the backend's pid");
                let mut tx = conn.begin().await.expect("begin");
                sqlx::query("select 1 from changesets where id = $1::uuid for update")
                    .bind(&changeset)
                    .execute(&mut *tx)
                    .await
                    .expect("lock the change set's row");
                locked.send(pid).expect("report the lock");
                // Dropped or sent, the lock goes.
                let _ = released.recv();
                tx.rollback().await.expect("release the lock");
            })
        });
        let pid = is_locked.recv().expect("the change set's row is locked");
        RowLock {
            pid,
            release,
            thread,
        }
    }

    /// Waits until another backend waits on this lock.
    fn wait_for_a_waiter(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
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
            if waiting > 0 {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the rebase never waited on the change set's row"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn release(self) {
        let _ = self.release.send(());
        self.thread.join().expect("the lock's thread");
    }
}

/// Starts the rebase of the conflicted scene's change set and holds it in its
/// transaction, with its drafts locked; returns the lock and the rebase.
fn rebase_on_hold(
    f: &'static Fixture,
    client: &'static Client,
    s: &Scene,
) -> (RowLock, JoinHandle<Value>) {
    let lock = RowLock::on_changeset(&s.behind);
    let (project, behind) = (s.project.clone(), s.behind.clone());
    let rebasing = std::thread::spawn(move || rebase(f, client, &project, &behind));
    lock.wait_for_a_waiter();
    (lock, rebasing)
}

/// Gives a request time to reach the draft that the rebase holds, and checks
/// that it waits there.
fn waits_on_the_rebase<T>(request: &JoinHandle<T>, what: &str) {
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !request.is_finished(),
        "the {what} waits while the rebase holds the draft"
    );
}

#[test]
fn a_read_that_waits_on_a_rebase_reads_the_rebased_draft() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let [p1, p3] = [s.paragraphs[0].clone(), s.paragraphs[2].clone()];
    // The draft is in memory before the rebase.
    let before = draft_html(f, client, &s.project, &s.behind, &s.doc);
    assert!(before.contains("Alpha by behind"), "{before}");

    let (lock, rebasing) = rebase_on_hold(f, client, &s);
    let (project, behind, doc) = (s.project.clone(), s.behind.clone(), s.doc.clone());
    let reading = std::thread::spawn(move || draft_html(f, client, &project, &behind, &doc));
    waits_on_the_rebase(&reading, "read");
    lock.release();

    let answer = rebasing.join().expect("the rebase");
    assert_eq!(answer["changeset"]["status"], "open", "{answer}");
    let html = reading.join().expect("the read");
    assert!(
        html.contains("Alpha by ahead") && !html.contains("Alpha by behind"),
        "the read shows the rebased draft, not the one from before: {html}"
    );
    assert!(!html.contains("Beta by behind"), "{html}");
    assert_eq!(node_ids(&html), vec![p1.clone(), p3.clone()], "{html}");
    let again = draft_html(f, client, &s.project, &s.behind, &s.doc);
    assert_eq!(node_ids(&again), vec![p1, p3], "{again}");
    assert!(again.contains("Alpha by ahead"), "{again}");
}

#[test]
fn an_edit_that_waits_on_a_rebase_applies_to_the_rebased_draft() {
    let (f, client) = ready();
    let s = conflicted_scene(f, client);
    let [p1, p3] = [s.paragraphs[0].clone(), s.paragraphs[2].clone()];

    let (lock, rebasing) = rebase_on_hold(f, client, &s);
    let (project, behind, doc) = (s.project.clone(), s.behind.clone(), s.doc.clone());
    let editing = std::thread::spawn(move || {
        edit(
            f,
            client,
            &project,
            &behind,
            &doc,
            vec![insert_after(None, "Late by behind")],
        )
    });
    waits_on_the_rebase(&editing, "edit");
    lock.release();

    let answer = rebasing.join().expect("the rebase");
    assert_eq!(answer["changeset"]["status"], "open", "{answer}");
    let results = editing.join().expect("the edit");
    let late = results[0]["node"]
        .as_str()
        .expect("the inserted node")
        .to_string();

    let html = draft_html(f, client, &s.project, &s.behind, &s.doc);
    assert!(
        html.contains("Alpha by ahead") && !html.contains("Alpha by behind"),
        "the edit applied to the rebased draft, not the one from before: {html}"
    );
    assert!(!html.contains("Beta by behind"), "{html}");
    assert_eq!(node_ids(&html), vec![late, p1, p3], "{html}");

    // What the agent saves next holds the rebase and the edit.
    let saved = save(f, client, &s.project, &s.behind, &s.doc);
    let version = version_html(f, client, &s.project, &s.doc, &saved);
    assert!(
        version.contains("Alpha by ahead") && version.contains("Late by behind"),
        "{version}"
    );
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
