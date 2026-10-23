//! US-025: approving a change set and merging it through the API. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; change sets are those of
//! US-016, tests/changesets.rs, docs and drafts those of US-022, tests/docs.rs,
//! and versions those of US-024, tests/versions.rs):
//!
//! - `POST /changesets/{changeset_id}/approve` (human) takes `{}` and approves
//!   a change set that is `in_review`. Any other status is 409 `{"error":
//!   "invalid_transition"}` and changes nothing; an unknown change set is 404
//!   `{"error": "not_found"}`. An API key is 403 `{"error": "forbidden"}` and
//!   changes nothing.
//! - Approving records the exact version ID of each changed doc (FR-21): a
//!   changed doc is one that has at least one version saved in the change set
//!   (a removal included), and its approved version is the latest one saved
//!   there. A doc whose draft was only opened is no changed doc, and draft
//!   edits made after the last save are not part of the approval: what is
//!   approved and merged is the saved version that the reviewer saw. Every
//!   Changeset body (show, list and the answers of every transition) carries
//!   `"approved_versions": [{"doc_id", "version_id"}, ...]`, the recorded
//!   approval in any order, and `[]` while the change set has none. Saving a
//!   version while `approved` or `needs_rebase` clears the approval (US-024),
//!   so the list is `[]` again.
//! - In the same request the server checks every changed doc's base, the
//!   `base_version_id` of the change set's draft of it (`null` for a doc that
//!   the change set started). When every base is still the doc's mainline
//!   version (`null` for a doc without one), the change set merges at once:
//!   each approved version becomes its doc's `mainline_version_id`, which is
//!   what `GET /docs/{doc_id}/html` then reads and what a new draft is based
//!   on, and the change set moves to `merged`. When any base is outdated, no
//!   doc changes and the change set moves to `needs_rebase`. A merge happens
//!   only onto the current mainline: of two change sets on the same base that
//!   are approved at the same time, one merges and the other needs a rebase.
//! - The answer is 200 `{"changeset": Changeset}`, with the status that the
//!   request ended in (`merged` or `needs_rebase`). The history gains
//!   `{"from_status": "in_review", "to_status": "approved", "actor": "human"}`
//!   and then `{"from_status": "approved", "to_status": "merged" or
//!   "needs_rebase", "actor": "system"}`, and each move emits its live event
//!   (US-017): `changeset.approved`, then `changeset.merged` or
//!   `changeset.needs_rebase`, with `data` `{"changeset_id", "from_status",
//!   "status", "actor", "feature_id"}` (`feature_id` null at project level,
//!   US-060).
//! - The approval may live in the existing tables; if it lives in a tenant
//!   table `approvals`, the columns without a default are `workspace_id`,
//!   `changeset_id`, `doc_id` and `version_id` (tests/workspace_delete.rs and
//!   the isolation suite seed it).

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::BTreeSet;
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us025_approve";
const TEST_BUCKET: &str = "omni-us025-approve";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const APPROVE_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/approve";

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
    format!("us025-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &agent(f),
                Some(&json!({ "name": unique("approve-project") })),
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
                Some(&json!({ "kind": "html", "pillar": "spec", "title": unique("Checkout PRD") })),
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

/// Adds a top-level paragraph with `text` to the change set's draft of the doc.
fn add_paragraph(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    text: &str,
) {
    must(
        call(
            client,
            "POST",
            &format!("{}/ops", draft_path(f, project, changeset, doc)),
            &agent(f),
            Some(&json!({ "ops": [{
                "op": "insert",
                "parent": null,
                "position": { "after": null },
                "content": format!("<p>{text}</p>")
            }] })),
        ),
        200,
        "edit the draft",
    );
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

/// Removes the doc in the change set; returns the ID of the removal version.
fn remove(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/remove", draft_path(f, project, changeset, doc)),
                &agent(f),
                Some(&json!({})),
            ),
            201,
            "remove the doc",
        )["version"],
    )
}

fn submit(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let answer = must(
        call(
            client,
            "POST",
            &format!("{}/submit", changeset_path(f, project, changeset)),
            &agent(f),
            None,
        ),
        200,
        "submit the change set",
    );
    assert_eq!(answer["changeset"]["status"], "in_review", "{answer}");
}

fn approve_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/approve", changeset_path(f, project, changeset)),
        auth,
        Some(&json!({})),
    )
}

/// The human approves; returns the Changeset of the answer.
fn approve(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    must(
        approve_response(f, client, &human(f), project, changeset),
        200,
        "the human approves the change set",
    )["changeset"]
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

/// The doc's mainline version as `GET /docs/{doc_id}/html` answers it.
fn mainline_html(f: &Fixture, client: &Client, project: &str, doc: &str) -> Value {
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
    )
}

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
        "read a version's HTML",
    )["html"]
        .as_str()
        .expect("the version's html")
        .to_string()
}

/// The recorded approval of a Changeset as (doc ID, version ID) pairs.
fn approved_versions(changeset: &Value) -> BTreeSet<(String, String)> {
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

fn pairs(list: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    list.iter()
        .map(|(doc, version)| (doc.to_string(), version.to_string()))
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

/// The live events of a project named `event` for one change set, oldest
/// first, as their `data`.
fn events_of(project: &str, changeset: &str, event: &str) -> Vec<Value> {
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
        rows.iter()
            .map(|text| parse(text))
            .filter(|data| data["changeset_id"] == changeset)
            .collect()
    })
}

/// The sequence number of the first event named `event` for the change set.
fn first_seq(project: &str, changeset: &str, event: &str) -> Option<i64> {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar::<_, i64>(
            "select seq from live_events \
             where project_id = $1::uuid and event = $2 and data->>'changeset_id' = $3 \
             order by seq limit 1",
        )
        .bind(project)
        .bind(event)
        .bind(changeset)
        .fetch_optional(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the live events: {e}"))
    })
}

/// A doc with a mainline version: started, saved and merged through a change
/// set of its own. Returns (doc ID, its mainline version ID).
fn merged_doc(f: &Fixture, client: &Client, project: &str) -> (String, String) {
    let changeset = new_changeset(f, client, project);
    let doc = create_doc(f, client, project, &changeset);
    add_paragraph(
        f,
        client,
        project,
        &changeset,
        &doc,
        &unique("Mainline text"),
    );
    let version = save(f, client, project, &changeset, &doc);
    submit(f, client, project, &changeset);
    let approved = approve(f, client, project, &changeset);
    assert_eq!(
        approved["status"], "merged",
        "set up a mainline: {approved}"
    );
    let mainline = doc_of(f, client, project, &doc)["mainline_version_id"].clone();
    assert_eq!(mainline, version.as_str(), "set up a mainline");
    (doc, version)
}

// ---------------------------------------------------------------------------
// The route
// ---------------------------------------------------------------------------

#[test]
fn the_approve_route_is_in_the_registry_as_a_human_route() {
    let entry = registry()
        .into_iter()
        .find(|r| r.method == "POST" && r.path == APPROVE_ROUTE)
        .unwrap_or_else(|| panic!("the registry has no route POST {APPROVE_ROUTE}"));
    assert_eq!(entry.access, Access::Human, "only the human approves");
}

#[test]
fn approving_is_the_humans_alone_and_an_api_key_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let doc = create_doc(f, client, &project, &changeset);
    add_paragraph(
        f,
        client,
        &project,
        &changeset,
        &doc,
        "Agents cannot approve",
    );
    save(f, client, &project, &changeset, &doc);
    submit(f, client, &project, &changeset);
    let before = changeset_of(f, client, &project, &changeset);

    let response = approve_response(f, client, &agent(f), &project, &changeset);
    assert_error(&response, 403, "forbidden");

    let after = changeset_of(f, client, &project, &changeset);
    assert_eq!(after["status"], "in_review", "{after}");
    assert_eq!(moves(&after), moves(&before), "the history is unchanged");
    assert_eq!(
        doc_of(f, client, &project, &doc)["mainline_version_id"],
        Value::Null,
        "nothing was merged"
    );
    assert!(
        events_of(&project, &changeset, "changeset.approved").is_empty(),
        "no approval event"
    );

    // The control: the human's session approves the same change set.
    let approved = approve(f, client, &project, &changeset);
    assert_eq!(approved["status"], "merged", "{approved}");
}

#[test]
fn approving_outside_in_review_is_refused_and_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let doc = create_doc(f, client, &project, &changeset);
    add_paragraph(f, client, &project, &changeset, &doc, "Not yet in review");
    save(f, client, &project, &changeset, &doc);

    // Still open: the agent has not submitted it.
    let before = changeset_of(f, client, &project, &changeset);
    let response = approve_response(f, client, &human(f), &project, &changeset);
    assert_error(&response, 409, "invalid_transition");
    let after = changeset_of(f, client, &project, &changeset);
    assert_eq!(after["status"], "open", "{after}");
    assert_eq!(moves(&after), moves(&before), "the history is unchanged");
    assert_eq!(
        approved_versions(&after),
        BTreeSet::new(),
        "no approval is recorded"
    );

    // Merged already: a second approval is refused too.
    submit(f, client, &project, &changeset);
    assert_eq!(approve(f, client, &project, &changeset)["status"], "merged");
    let merged = changeset_of(f, client, &project, &changeset);
    let response = approve_response(f, client, &human(f), &project, &changeset);
    assert_error(&response, 409, "invalid_transition");
    assert_eq!(
        moves(&changeset_of(f, client, &project, &changeset)),
        moves(&merged),
        "the history is unchanged"
    );

    // An unknown change set.
    let response = approve_response(f, client, &human(f), &project, UNKNOWN_ID);
    assert_error(&response, 404, "not_found");
}

// ---------------------------------------------------------------------------
// Recording the exact versions and merging
// ---------------------------------------------------------------------------

#[test]
fn approving_records_the_latest_saved_version_of_each_changed_doc_and_merges_them() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let first = create_doc(f, client, &project, &changeset);
    let second = create_doc(f, client, &project, &changeset);
    add_paragraph(f, client, &project, &changeset, &first, "Reviewed once");
    let early = save(f, client, &project, &changeset, &first);
    add_paragraph(f, client, &project, &changeset, &first, "Reviewed twice");
    let seen = save(f, client, &project, &changeset, &first);
    add_paragraph(f, client, &project, &changeset, &second, "The other doc");
    let other = save(f, client, &project, &changeset, &second);
    // An edit after the last save: the reviewer never saw it.
    let unseen = unique("Never reviewed");
    add_paragraph(f, client, &project, &changeset, &first, &unseen);
    submit(f, client, &project, &changeset);

    let answer = approve(f, client, &project, &changeset);
    let expected = pairs(&[(&first, &seen), (&second, &other)]);
    assert_eq!(
        approved_versions(&answer),
        expected,
        "the approval records the latest saved version of each changed doc: {answer}"
    );
    assert!(
        !approved_versions(&answer)
            .iter()
            .any(|(_, version)| *version == early),
        "an earlier version is not approved"
    );
    let shown = changeset_of(f, client, &project, &changeset);
    assert_eq!(
        approved_versions(&shown),
        expected,
        "the change set keeps its approval: {shown}"
    );
    let listed = must(
        call(
            client,
            "GET",
            &format!("{}/changesets", project_base(f, &project)),
            &agent(f),
            None,
        ),
        200,
        "list the change sets",
    );
    let in_list = listed["changesets"]
        .as_array()
        .expect("changesets")
        .iter()
        .find(|c| c["id"] == changeset.as_str())
        .cloned()
        .expect("the change set is listed");
    assert_eq!(approved_versions(&in_list), expected, "{in_list}");

    // Every base was the mainline (none: both docs are new), so it merged.
    assert_eq!(answer["status"], "merged", "{answer}");
    assert_eq!(shown["status"], "merged", "{shown}");
    let history = moves(&shown);
    assert_eq!(
        history[history.len() - 2..].to_vec(),
        vec![
            step("in_review", "approved", "human"),
            step("approved", "merged", "system"),
        ],
        "the history records the approval and the merge: {shown}"
    );

    // The approved versions are the new mainline versions, exactly as saved.
    for (doc, version) in [(&first, &seen), (&second, &other)] {
        let shown_doc = doc_of(f, client, &project, doc);
        assert_eq!(
            shown_doc["mainline_version_id"],
            version.as_str(),
            "{shown_doc}"
        );
        let read = mainline_html(f, client, &project, doc);
        assert_eq!(read["version_id"], version.as_str(), "{read}");
        assert_eq!(
            read["html"].as_str().expect("html"),
            version_html(f, client, &project, doc, version),
            "the mainline is the approved version"
        );
    }
    let read = mainline_html(f, client, &project, &first);
    let html = read["html"].as_str().expect("html");
    assert!(html.contains("Reviewed twice"), "{html}");
    assert!(
        !html.contains(&unseen),
        "an edit that was never saved is not merged: {html}"
    );
}

#[test]
fn approving_and_merging_emit_their_live_events_in_order() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let doc = create_doc(f, client, &project, &changeset);
    add_paragraph(f, client, &project, &changeset, &doc, "Live");
    save(f, client, &project, &changeset, &doc);
    submit(f, client, &project, &changeset);
    approve(f, client, &project, &changeset);

    let approved = events_of(&project, &changeset, "changeset.approved");
    assert_eq!(
        approved,
        vec![json!({
            "changeset_id": changeset,
            "from_status": "in_review",
            "status": "approved",
            "actor": "human",
            "feature_id": null
        })],
        "one changeset.approved event"
    );
    let merged = events_of(&project, &changeset, "changeset.merged");
    assert_eq!(
        merged,
        vec![json!({
            "changeset_id": changeset,
            "from_status": "approved",
            "status": "merged",
            "actor": "system",
            "feature_id": null
        })],
        "one changeset.merged event"
    );
    let order = (
        first_seq(&project, &changeset, "changeset.approved"),
        first_seq(&project, &changeset, "changeset.merged"),
    );
    assert!(
        matches!(order, (Some(a), Some(m)) if a < m),
        "the approval comes before the merge: {order:?}"
    );
    assert!(
        events_of(&project, &changeset, "changeset.needs_rebase").is_empty(),
        "a merge needs no rebase"
    );
}

#[test]
fn a_merged_version_is_the_base_of_the_next_draft() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, mainline) = merged_doc(f, client, &project);

    let next = new_changeset(f, client, &project);
    let draft = open_doc(f, client, &project, &next, &doc);
    assert_eq!(
        draft["base_version_id"],
        mainline.as_str(),
        "a new draft starts from the merged version: {draft}"
    );

    // And the next change set merges onto it in turn.
    add_paragraph(f, client, &project, &next, &doc, "Second round");
    let version = save(f, client, &project, &next, &doc);
    submit(f, client, &project, &next);
    let answer = approve(f, client, &project, &next);
    assert_eq!(answer["status"], "merged", "{answer}");
    assert_eq!(
        approved_versions(&answer),
        pairs(&[(&doc, &version)]),
        "{answer}"
    );
    assert_eq!(
        doc_of(f, client, &project, &doc)["mainline_version_id"],
        version.as_str()
    );
}

#[test]
fn a_doc_whose_draft_was_only_opened_is_not_part_of_the_approval() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (untouched, mainline) = merged_doc(f, client, &project);

    let changeset = new_changeset(f, client, &project);
    open_doc(f, client, &project, &changeset, &untouched);
    // Even an edit that was never saved leaves the doc out of the approval.
    add_paragraph(f, client, &project, &changeset, &untouched, "Unsaved");
    let fresh = create_doc(f, client, &project, &changeset);
    add_paragraph(f, client, &project, &changeset, &fresh, "Saved");
    let version = save(f, client, &project, &changeset, &fresh);
    submit(f, client, &project, &changeset);

    let answer = approve(f, client, &project, &changeset);
    assert_eq!(
        approved_versions(&answer),
        pairs(&[(&fresh, &version)]),
        "only the doc with a saved version is approved: {answer}"
    );
    assert_eq!(answer["status"], "merged", "{answer}");
    assert_eq!(
        doc_of(f, client, &project, &untouched)["mainline_version_id"],
        mainline.as_str(),
        "the doc that was only opened keeps its mainline"
    );
}

#[test]
fn a_removal_is_approved_and_merged_like_any_other_version() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, _) = merged_doc(f, client, &project);

    let changeset = new_changeset(f, client, &project);
    open_doc(f, client, &project, &changeset, &doc);
    let removal = remove(f, client, &project, &changeset, &doc);
    submit(f, client, &project, &changeset);

    let answer = approve(f, client, &project, &changeset);
    assert_eq!(
        approved_versions(&answer),
        pairs(&[(&doc, &removal)]),
        "the removal is the doc's approved version: {answer}"
    );
    assert_eq!(answer["status"], "merged", "{answer}");
    assert_eq!(
        doc_of(f, client, &project, &doc)["mainline_version_id"],
        removal.as_str(),
        "the removal is the doc's mainline version now"
    );
}

// ---------------------------------------------------------------------------
// Outdated bases
// ---------------------------------------------------------------------------

#[test]
fn an_outdated_base_moves_the_change_set_to_needs_rebase_and_merges_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, base) = merged_doc(f, client, &project);

    // Two change sets start from the same mainline version.
    let ahead = new_changeset(f, client, &project);
    let behind = new_changeset(f, client, &project);
    for changeset in [&ahead, &behind] {
        let draft = open_doc(f, client, &project, changeset, &doc);
        assert_eq!(draft["base_version_id"], base.as_str(), "{draft}");
    }
    add_paragraph(f, client, &project, &ahead, &doc, "Ahead");
    let merged_version = save(f, client, &project, &ahead, &doc);
    add_paragraph(f, client, &project, &behind, &doc, "Behind");
    let stale_version = save(f, client, &project, &behind, &doc);
    // The late change set also starts a doc of its own, whose base (none) is
    // still current: nothing of the change set merges all the same.
    let fresh = create_doc(f, client, &project, &behind);
    add_paragraph(f, client, &project, &behind, &fresh, "New doc");
    let fresh_version = save(f, client, &project, &behind, &fresh);
    submit(f, client, &project, &ahead);
    submit(f, client, &project, &behind);

    assert_eq!(approve(f, client, &project, &ahead)["status"], "merged");
    let answer = approve(f, client, &project, &behind);
    assert_eq!(
        answer["status"], "needs_rebase",
        "a base is no longer the mainline: {answer}"
    );
    let shown = changeset_of(f, client, &project, &behind);
    assert_eq!(shown["status"], "needs_rebase", "{shown}");
    let history = moves(&shown);
    assert_eq!(
        history[history.len() - 2..].to_vec(),
        vec![
            step("in_review", "approved", "human"),
            step("approved", "needs_rebase", "system"),
        ],
        "{shown}"
    );
    assert_eq!(
        approved_versions(&shown),
        pairs(&[(&doc, &stale_version), (&fresh, &fresh_version)]),
        "the approval is recorded even though it cannot merge yet: {shown}"
    );

    // Nothing merged: the mainline is the other change set's version, and the
    // new doc still has none.
    assert_eq!(
        doc_of(f, client, &project, &doc)["mainline_version_id"],
        merged_version.as_str()
    );
    assert_eq!(
        mainline_html(f, client, &project, &doc)["version_id"],
        merged_version.as_str()
    );
    assert_eq!(
        doc_of(f, client, &project, &fresh)["mainline_version_id"],
        Value::Null,
        "no doc of a change set that needs a rebase is merged"
    );

    assert_eq!(
        events_of(&project, &behind, "changeset.needs_rebase"),
        vec![json!({
            "changeset_id": behind,
            "from_status": "approved",
            "status": "needs_rebase",
            "actor": "system",
            "feature_id": null
        })],
        "one changeset.needs_rebase event"
    );
    assert_eq!(
        events_of(&project, &behind, "changeset.approved").len(),
        1,
        "the approval itself is announced"
    );
    assert!(
        events_of(&project, &behind, "changeset.merged").is_empty(),
        "no merge event"
    );
}

#[test]
fn saving_a_version_after_needs_rebase_clears_the_recorded_approval() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, _) = merged_doc(f, client, &project);
    let ahead = new_changeset(f, client, &project);
    let behind = new_changeset(f, client, &project);
    open_doc(f, client, &project, &ahead, &doc);
    open_doc(f, client, &project, &behind, &doc);
    add_paragraph(f, client, &project, &ahead, &doc, "Ahead");
    save(f, client, &project, &ahead, &doc);
    add_paragraph(f, client, &project, &behind, &doc, "Behind");
    save(f, client, &project, &behind, &doc);
    submit(f, client, &project, &ahead);
    submit(f, client, &project, &behind);
    approve(f, client, &project, &ahead);
    let answer = approve(f, client, &project, &behind);
    assert_eq!(answer["status"], "needs_rebase", "{answer}");
    assert!(!approved_versions(&answer).is_empty(), "{answer}");

    add_paragraph(f, client, &project, &behind, &doc, "Reworked");
    save(f, client, &project, &behind, &doc);
    let shown = changeset_of(f, client, &project, &behind);
    assert_eq!(shown["status"], "in_review", "{shown}");
    assert_eq!(
        approved_versions(&shown),
        BTreeSet::new(),
        "the approval is cleared: {shown}"
    );
}

#[test]
fn of_two_change_sets_approved_at_once_on_one_base_one_merges_and_the_other_needs_a_rebase() {
    let (f, client) = ready();
    for round in 0..3 {
        let project = new_project(f, client);
        let (doc, base) = merged_doc(f, client, &project);
        let changesets: Vec<String> = (0..2)
            .map(|side| {
                let changeset = new_changeset(f, client, &project);
                let draft = open_doc(f, client, &project, &changeset, &doc);
                assert_eq!(draft["base_version_id"], base.as_str(), "{draft}");
                add_paragraph(
                    f,
                    client,
                    &project,
                    &changeset,
                    &doc,
                    &format!("Round {round}, side {side}"),
                );
                save(f, client, &project, &changeset, &doc);
                submit(f, client, &project, &changeset);
                changeset
            })
            .collect();

        let barrier = std::sync::Barrier::new(2);
        let answers: Vec<Value> = std::thread::scope(|scope| {
            let handles: Vec<_> = changesets
                .iter()
                .map(|changeset| {
                    let barrier = &barrier;
                    let project = &project;
                    scope.spawn(move || {
                        barrier.wait();
                        approve(f, client, project, changeset)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("an approval"))
                .collect()
        });
        let mut statuses: Vec<&str> = answers
            .iter()
            .map(|a| a["status"].as_str().expect("a status"))
            .collect();
        statuses.sort_unstable();
        assert_eq!(
            statuses,
            vec!["merged", "needs_rebase"],
            "round {round}: exactly one merges onto the mainline: {answers:?}"
        );
        let winner = answers
            .iter()
            .find(|a| a["status"] == "merged")
            .expect("a merged change set");
        let (_, version) = approved_versions(winner)
            .into_iter()
            .next()
            .expect("an approved version");
        assert_eq!(
            doc_of(f, client, &project, &doc)["mainline_version_id"],
            version.as_str(),
            "round {round}: the mainline is the merged change set's version"
        );
    }
}
