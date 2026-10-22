//! US-046: the change set review, through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. The web
//! app's side is web/src/changeset-review.test.tsx.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; change sets are those of
//! US-016 and US-025, docs, drafts and versions those of US-022 and US-024,
//! the node diff that of US-024 (`GET /versions/{a}/diff/{b}`), and decisions
//! and their links to change sets those of US-037):
//!
//! - `GET /changesets/{changeset_id}/review` (human) answers what the
//!   reviewer needs to approve a change set, in one call: 200
//!   `{"changeset", "docs", "decisions"}`. An API key is 403
//!   `{"error": "forbidden"}`; a change set that does not exist, or that
//!   belongs to another project, is 404 `{"error": "not_found"}`.
//! - `changeset` is the Changeset that `GET /changesets/{changeset_id}`
//!   answers the human.
//! - `docs` lists each doc that the change set changes: every doc with a
//!   version saved in the change set (removals included), oldest doc first. A
//!   doc that the change set only opened (a draft without a saved version) is
//!   not listed, nor is a doc that only other change sets change. Each item is
//!   `{"doc_id", "kind", "module", "title", "base_version_id", "version_id",
//!   "removed", "changes"}`:
//!   - `base_version_id`: the version that the change set's draft of the doc
//!     is based on (the mainline version it was opened from or last rebased
//!     onto); `null` for a doc that the change set created.
//!   - `version_id`: the latest version of the doc saved in the change set,
//!     the one an approval would cover; `removed` is that version's `removed`.
//!   - `changes`: `{"added", "removed", "changed", "moved"}`, the node changes
//!     from the base version to `version_id`, each list as the diff route
//!     answers it (Node entries `{"id", "type", "parent", "fingerprint"}`, and
//!     Moved nodes for `moved`). Without a base every node of the version is
//!     `added`; for a removal every node of the base is `removed`.
//! - `decisions` lists the decisions linked to the change set, in the order
//!   they were linked, each the Decision that `GET /decisions/{decision_id}`
//!   answers the human, with its status now.
//!
//! The conflicts of the change set's rebases come from the existing `GET
//! /changesets/{changeset_id}/conflicts` (US-026), and the versions' content
//! from `GET /versions/{version_id}` and `GET /docs/{doc_id}/html?version=`.

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

const TEST_DB: &str = "omni_us046_changeset_review";
const TEST_BUCKET: &str = "omni-us046-changeset-review";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const REVIEW_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/review";

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
    format!("us046-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

/// Who sends a request: an agent (the API key with a run and role) or the
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

impl Auth<'_> {
    fn headers(&self) -> Vec<(&'static str, String)> {
        match self {
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
}

/// The agent that writes docs, starts threads and proposes decisions:
/// `run-1/author`.
fn author(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-1",
        role: "author",
    }
}

fn human(f: &Fixture) -> Auth<'_> {
    Auth::Session(&f.alice_token)
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let owned = auth.headers();
    let headers: Vec<(&str, &str)> = owned.iter().map(|(n, v)| (*n, v.as_str())).collect();
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

fn changeset_path(f: &Fixture, project: &str, changeset: &str) -> String {
    format!("{}/changesets/{changeset}", project_base(f, project))
}

fn thread_path(f: &Fixture, project: &str, thread: &str) -> String {
    format!("{}/threads/{thread}", project_base(f, project))
}

/// What `auth` reads at `path` under the project, which must answer 200.
fn read(f: &Fixture, client: &Client, auth: &Auth, project: &str, path: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}{path}", project_base(f, project)),
            auth,
            None,
        ),
        200,
        &format!("read {path}"),
    )
}
// ---------------------------------------------------------------------------
// Projects, change sets, docs and versions
// ---------------------------------------------------------------------------

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &author(f),
                Some(&json!({ "name": unique("review-project") })),
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
                &author(f),
                Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
            ),
            201,
            "create a change set",
        )["changeset"],
    )
}

/// Starts an HTML PRD doc in the change set; returns its ID.
fn create_doc(f: &Fixture, client: &Client, project: &str, changeset: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", changeset_path(f, project, changeset)),
                &author(f),
                Some(&json!({ "kind": "html", "module": "prd", "title": unique("Checkout PRD") })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

fn draft_path(f: &Fixture, project: &str, changeset: &str, doc: &str) -> String {
    format!("{}/docs/{doc}", changeset_path(f, project, changeset))
}

/// Makes the change set's draft of a doc that has a mainline version.
fn open_doc(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) {
    let response = call(
        client,
        "POST",
        &format!("{}/open", draft_path(f, project, changeset, doc)),
        &author(f),
        Some(&json!({})),
    );
    assert!(
        response.status == 200 || response.status == 201,
        "open the doc in the change set: {} {}",
        response.status,
        response.text()
    );
}

/// Applies one batch of operations to the change set's draft of the doc;
/// returns the results.
fn edit(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    ops: Value,
) -> Value {
    must(
        call(
            client,
            "POST",
            &format!("{}/ops", draft_path(f, project, changeset, doc)),
            &author(f),
            Some(&json!({ "ops": ops })),
        ),
        200,
        "edit the draft",
    )["results"]
        .clone()
}

/// Inserts a top-level paragraph with `text` into the draft; returns its node.
fn add_paragraph(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    text: &str,
) -> String {
    let results = edit(
        f,
        client,
        project,
        changeset,
        doc,
        json!([{
            "op": "insert",
            "parent": null,
            "position": { "after": null },
            "content": format!("<p>{text}</p>")
        }]),
    );
    results[0]["node"]
        .as_str()
        .unwrap_or_else(|| panic!("an insert names its node: {results}"))
        .to_string()
}

/// The revision of `node` in the change set's draft of the doc (its
/// `data-rev` in the draft's HTML).
fn rev_of(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    node: &str,
) -> u64 {
    let draft = must(
        call(
            client,
            "GET",
            &draft_path(f, project, changeset, doc),
            &author(f),
            None,
        ),
        200,
        "read the draft",
    );
    let html = draft["draft"]["html"]
        .as_str()
        .unwrap_or_else(|| panic!("the draft has its html: {draft}"));
    let needle = format!("data-node-id=\"{node}\"");
    let at = html
        .find(&needle)
        .unwrap_or_else(|| panic!("the draft has {node}: {html}"));
    let tag_start = html[..at].rfind('<').expect("a start tag");
    let tag_end = at + html[at..].find('>').expect("the tag's end");
    let tag = &html[tag_start..tag_end];
    let start = tag.find("data-rev=\"").expect("a data-rev") + "data-rev=\"".len();
    let end = start + tag[start..].find('"').expect("the end of data-rev");
    tag[start..end].parse().expect("a revision number")
}

/// Saves the change set's draft of the doc as a version; returns its ID.
fn save(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/versions", draft_path(f, project, changeset, doc)),
                &author(f),
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
                &author(f),
                Some(&json!({})),
            ),
            201,
            "remove the doc",
        )["version"],
    )
}

/// Submits the change set and has the human approve it, which merges it.
fn merge(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let path = changeset_path(f, project, changeset);
    must(
        call(client, "POST", &format!("{path}/submit"), &author(f), None),
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
    assert_eq!(approved["changeset"]["status"], "merged", "{approved}");
}

/// The node IDs of a saved version, in document order.
fn version_nodes(f: &Fixture, client: &Client, project: &str, version: &str) -> Vec<String> {
    let read = read(
        f,
        client,
        &human(f),
        project,
        &format!("/versions/{version}"),
    );
    read["version"]["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("the version lists its nodes: {read}"))
        .iter()
        .map(id_of)
        .collect()
}

/// A merged PRD doc with the top-level paragraphs `texts`: (doc, mainline
/// version, the paragraphs' nodes in the order of `texts`).
fn merged_doc(
    f: &Fixture,
    client: &Client,
    project: &str,
    texts: &[&str],
) -> (String, String, Vec<String>) {
    let changeset = new_changeset(f, client, project);
    let doc = create_doc(f, client, project, &changeset);
    let nodes = texts
        .iter()
        .map(|text| add_paragraph(f, client, project, &changeset, &doc, text))
        .collect();
    let version = save(f, client, project, &changeset, &doc);
    merge(f, client, project, &changeset);
    (doc, version, nodes)
}

// ---------------------------------------------------------------------------
// Threads and decisions
// ---------------------------------------------------------------------------

/// The author starts a thread in the change set; returns its ID.
fn new_thread(f: &Fixture, client: &Client, project: &str, changeset: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/threads", changeset_path(f, project, changeset)),
                &author(f),
                Some(&json!({
                    "question": unique("How fast do we ship?"),
                    "criteria": ["Cheap to run"],
                    "sealed": false
                })),
            ),
            201,
            "start a thread",
        )["thread"],
    )
}

/// The author proposes a decision for the thread; returns its ID.
fn propose(f: &Fixture, client: &Client, project: &str, thread: &str) -> String {
    let body = json!({
        "direction": unique("Ship every order within one day"),
        "chosen": "One day",
        "criteria": ["Cheap to run"],
        "options": [
            { "name": "One day", "scores": { "Cheap to run": 6 }, "lost_because": null },
            {
                "name": "Two days",
                "scores": { "Cheap to run": 8 },
                "lost_because": "Customers leave for faster shops."
            }
        ],
        "assumptions": [],
        "open_questions": [],
        "dissent": [],
        "posts": []
    });
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/decisions", thread_path(f, project, thread)),
                &author(f),
                Some(&body),
            ),
            201,
            "propose a decision",
        )["decision"],
    )
}

/// The author links the decision to the change set (US-037).
fn link(f: &Fixture, client: &Client, project: &str, changeset: &str, decision: &str) {
    must(
        call(
            client,
            "POST",
            &format!(
                "{}/decisions/{decision}",
                changeset_path(f, project, changeset)
            ),
            &author(f),
            None,
        ),
        200,
        "link the decision to the change set",
    );
}

// ---------------------------------------------------------------------------
// The review
// ---------------------------------------------------------------------------

fn review_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
) -> Response {
    call(
        client,
        "GET",
        &format!("{}/review", changeset_path(f, project, changeset)),
        auth,
        None,
    )
}

/// The change set's review as the human reads it.
fn review(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    must(
        review_response(f, client, &human(f), project, changeset),
        200,
        "the human reads the change set review",
    )
}

/// The review's list `name` (`docs` or `decisions`).
fn list_of(review: &Value, name: &str) -> Vec<Value> {
    review[name]
        .as_array()
        .unwrap_or_else(|| panic!("the review has a list `{name}`: {review}"))
        .clone()
}

/// The doc IDs that the review lists, in order.
fn doc_ids(review: &Value) -> Vec<String> {
    list_of(review, "docs")
        .iter()
        .map(|doc| {
            doc["doc_id"]
                .as_str()
                .unwrap_or_else(|| panic!("a reviewed doc has a `doc_id`: {doc}"))
                .to_string()
        })
        .collect()
}

/// The review's item for `doc`.
fn doc_item(review: &Value, doc: &str) -> Value {
    list_of(review, "docs")
        .into_iter()
        .find(|item| item["doc_id"] == doc)
        .unwrap_or_else(|| panic!("the review lists the doc {doc}: {review}"))
}

/// The node IDs of the list `name` of a reviewed doc's `changes`.
fn changed_ids(item: &Value, name: &str) -> BTreeSet<String> {
    item["changes"][name]
        .as_array()
        .unwrap_or_else(|| panic!("the doc's changes have a list `{name}`: {item}"))
        .iter()
        .map(id_of)
        .collect()
}

fn set_of(ids: &[&String]) -> BTreeSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

// ---------------------------------------------------------------------------
// The route
// ---------------------------------------------------------------------------

#[test]
fn the_review_route_is_a_human_route_in_the_registry() {
    let entry = registry()
        .into_iter()
        .find(|r| r.method == "GET" && r.path == REVIEW_ROUTE)
        .unwrap_or_else(|| panic!("the registry has no route GET {REVIEW_ROUTE}"));
    assert_eq!(entry.access, Access::Human, "GET {REVIEW_ROUTE}");
}

#[test]
fn an_api_key_is_refused_the_review_and_an_unknown_change_set_is_not_found() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let other_project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let doc = create_doc(f, client, &project, &changeset);
    add_paragraph(
        f,
        client,
        &project,
        &changeset,
        &doc,
        "Orders ship in one day.",
    );
    save(f, client, &project, &changeset, &doc);

    assert_error(
        &review_response(f, client, &author(f), &project, &changeset),
        403,
        "forbidden",
        "an API key reads the change set review",
    );
    assert_error(
        &review_response(f, client, &human(f), &project, UNKNOWN_ID),
        404,
        "not_found",
        "the human reads the review of a change set that does not exist",
    );
    assert_error(
        &review_response(f, client, &human(f), &other_project, &changeset),
        404,
        "not_found",
        "the human reads the review of a change set through another project",
    );
}

#[test]
fn the_review_carries_the_change_set_as_its_own_read_answers_it() {
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
        "Orders ship in one day.",
    );
    save(f, client, &project, &changeset, &doc);
    must(
        call(
            client,
            "POST",
            &format!("{}/submit", changeset_path(f, &project, &changeset)),
            &author(f),
            None,
        ),
        200,
        "submit the change set",
    );

    let answer = review(f, client, &project, &changeset);
    let own = read(
        f,
        client,
        &human(f),
        &project,
        &format!("/changesets/{changeset}"),
    );
    assert_eq!(answer["changeset"], own["changeset"], "{answer}");
    assert_eq!(answer["changeset"]["status"], "in_review", "{answer}");
}

// ---------------------------------------------------------------------------
// The docs the change set changes
// ---------------------------------------------------------------------------

#[test]
fn the_review_lists_each_doc_with_a_saved_version_oldest_doc_first() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (older, _, _) = merged_doc(f, client, &project, &["Orders ship in two days."]);
    let (only_opened, _, _) = merged_doc(f, client, &project, &["Refunds take a week."]);
    let (elsewhere, _, _) = merged_doc(f, client, &project, &["Gift wrap is extra."]);

    let changeset = new_changeset(f, client, &project);
    // The new doc is saved first, but the older doc is listed first.
    let newer = create_doc(f, client, &project, &changeset);
    add_paragraph(f, client, &project, &changeset, &newer, "Returns are free.");
    save(f, client, &project, &changeset, &newer);
    open_doc(f, client, &project, &changeset, &older);
    add_paragraph(
        f,
        client,
        &project,
        &changeset,
        &older,
        "Orders ship in one day.",
    );
    save(f, client, &project, &changeset, &older);
    // Opened and edited, never saved: the change set does not change it yet.
    open_doc(f, client, &project, &changeset, &only_opened);
    add_paragraph(
        f,
        client,
        &project,
        &changeset,
        &only_opened,
        "Refunds take a day.",
    );
    // Another change set changes this one.
    let other = new_changeset(f, client, &project);
    open_doc(f, client, &project, &other, &elsewhere);
    add_paragraph(
        f,
        client,
        &project,
        &other,
        &elsewhere,
        "Gift wrap is free.",
    );
    save(f, client, &project, &other, &elsewhere);

    let answer = review(f, client, &project, &changeset);
    assert_eq!(
        doc_ids(&answer),
        vec![older.clone(), newer.clone()],
        "the docs with a version saved in the change set, oldest doc first: {answer}"
    );
    let doc = read(f, client, &human(f), &project, &format!("/docs/{older}"));
    let item = doc_item(&answer, &older);
    assert_eq!(item["title"], doc["doc"]["title"], "{item}");
    assert_eq!(item["kind"], "html", "{item}");
    assert_eq!(item["module"], "prd", "{item}");
}

#[test]
fn a_changed_doc_lists_the_nodes_added_changed_and_removed_since_its_base() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, base, nodes) = merged_doc(
        f,
        client,
        &project,
        &[
            "Orders ship in two days.",
            "Gift wrap is extra.",
            "Refunds take a week.",
        ],
    );
    let (rewritten, dropped, kept) = (&nodes[0], &nodes[1], &nodes[2]);

    let changeset = new_changeset(f, client, &project);
    open_doc(f, client, &project, &changeset, &doc);
    let rev = rev_of(f, client, &project, &changeset, &doc, rewritten);
    edit(
        f,
        client,
        &project,
        &changeset,
        &doc,
        json!([{
            "op": "set",
            "node": rewritten,
            "rev": rev,
            "properties": { "html": "Orders ship in one day." }
        }]),
    );
    let rev = rev_of(f, client, &project, &changeset, &doc, dropped);
    edit(
        f,
        client,
        &project,
        &changeset,
        &doc,
        json!([{ "op": "delete", "node": dropped, "rev": rev }]),
    );
    let first_added = add_paragraph(f, client, &project, &changeset, &doc, "Returns are free.");
    save(f, client, &project, &changeset, &doc);
    // Saved again: the review covers the latest version.
    let later_added = add_paragraph(f, client, &project, &changeset, &doc, "Coupons stack.");
    let latest = save(f, client, &project, &changeset, &doc);

    let answer = review(f, client, &project, &changeset);
    let item = doc_item(&answer, &doc);
    assert_eq!(
        item["base_version_id"], base,
        "the base is the mainline version it was opened from: {item}"
    );
    assert_eq!(
        item["version_id"], latest,
        "the latest version saved in the change set: {item}"
    );
    assert_eq!(item["removed"], false, "{item}");
    assert_eq!(
        changed_ids(&item, "added"),
        set_of(&[&first_added, &later_added]),
        "the paragraphs the change set added: {item}"
    );
    assert_eq!(
        changed_ids(&item, "changed"),
        set_of(&[rewritten]),
        "the paragraph whose text it changed: {item}"
    );
    assert_eq!(
        changed_ids(&item, "removed"),
        set_of(&[dropped]),
        "the paragraph it deleted: {item}"
    );
    for list in ["added", "changed", "removed"] {
        assert!(
            !changed_ids(&item, list).contains(kept),
            "the untouched paragraph is not {list}: {item}"
        );
    }
    let added = item["changes"]["added"]
        .as_array()
        .expect("the added nodes")
        .iter()
        .find(|node| node["id"] == first_added.as_str())
        .cloned()
        .unwrap_or_else(|| panic!("{first_added} is added: {item}"));
    assert_eq!(
        added["type"], "paragraph",
        "a node entry has its type: {added}"
    );
    assert!(
        added["fingerprint"].is_string(),
        "a node entry has its fingerprint: {added}"
    );
}

#[test]
fn a_new_doc_has_no_base_and_every_node_is_added() {
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
        "Orders ship in one day.",
    );
    add_paragraph(f, client, &project, &changeset, &doc, "Returns are free.");
    let version = save(f, client, &project, &changeset, &doc);
    let nodes = version_nodes(f, client, &project, &version);
    assert!(!nodes.is_empty(), "the version has nodes");

    let answer = review(f, client, &project, &changeset);
    let item = doc_item(&answer, &doc);
    assert_eq!(
        item["base_version_id"],
        Value::Null,
        "a new doc has no base: {item}"
    );
    assert_eq!(item["version_id"], version, "{item}");
    assert_eq!(item["removed"], false, "{item}");
    assert_eq!(
        changed_ids(&item, "added"),
        nodes.iter().cloned().collect::<BTreeSet<_>>(),
        "every node of the new doc is added: {item}"
    );
    for list in ["removed", "changed", "moved"] {
        assert!(
            changed_ids(&item, list).is_empty(),
            "a new doc has nothing {list}: {item}"
        );
    }
}

#[test]
fn a_removed_doc_is_marked_removed_and_every_node_of_its_base_is_removed() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, base, _) = merged_doc(
        f,
        client,
        &project,
        &["Orders ship in two days.", "Refunds take a week."],
    );
    let base_nodes = version_nodes(f, client, &project, &base);

    let changeset = new_changeset(f, client, &project);
    open_doc(f, client, &project, &changeset, &doc);
    let removal = remove(f, client, &project, &changeset, &doc);

    let answer = review(f, client, &project, &changeset);
    assert_eq!(doc_ids(&answer), vec![doc.clone()], "{answer}");
    let item = doc_item(&answer, &doc);
    assert_eq!(item["removed"], true, "{item}");
    assert_eq!(
        item["version_id"], removal,
        "the removal is the version under review: {item}"
    );
    assert_eq!(item["base_version_id"], base, "{item}");
    assert_eq!(
        changed_ids(&item, "removed"),
        base_nodes.iter().cloned().collect::<BTreeSet<_>>(),
        "every node of the base is removed: {item}"
    );
    assert!(changed_ids(&item, "added").is_empty(), "{item}");
    assert!(changed_ids(&item, "changed").is_empty(), "{item}");
}

// ---------------------------------------------------------------------------
// Linked decisions
// ---------------------------------------------------------------------------

#[test]
fn the_review_lists_the_linked_decisions_in_link_order_with_their_status_now() {
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
        "Orders ship in one day.",
    );
    save(f, client, &project, &changeset, &doc);
    let first_thread = new_thread(f, client, &project, &changeset);
    let second_thread = new_thread(f, client, &project, &changeset);
    let first = propose(f, client, &project, &first_thread);
    let second = propose(f, client, &project, &second_thread);
    // A decision of the project that is not linked is not listed.
    let unlinked_thread = new_thread(f, client, &project, &changeset);
    propose(f, client, &project, &unlinked_thread);
    link(f, client, &project, &changeset, &second);
    link(f, client, &project, &changeset, &first);

    let answer = review(f, client, &project, &changeset);
    let ids: Vec<String> = list_of(&answer, "decisions").iter().map(id_of).collect();
    assert_eq!(
        ids,
        vec![second.clone(), first.clone()],
        "in link order: {answer}"
    );
    for decision in list_of(&answer, "decisions") {
        let id = id_of(&decision);
        let own = read(f, client, &human(f), &project, &format!("/decisions/{id}"));
        assert_eq!(decision, own["decision"], "each as its own read answers it");
        assert_eq!(decision["status"], "proposed", "{decision}");
        assert!(decision["direction"].is_string(), "{decision}");
    }

    must(
        call(
            client,
            "POST",
            &format!("{}/decisions/{first}/approve", project_base(f, &project)),
            &human(f),
            Some(&json!({})),
        ),
        200,
        "the human approves the first decision",
    );
    let answer = review(f, client, &project, &changeset);
    let status_of = |id: &str| -> Value {
        list_of(&answer, "decisions")
            .into_iter()
            .find(|d| d["id"] == id)
            .unwrap_or_else(|| panic!("the review lists the decision {id}: {answer}"))["status"]
            .clone()
    };
    assert_eq!(status_of(&first), "approved", "{answer}");
    assert_eq!(status_of(&second), "proposed", "{answer}");
}
