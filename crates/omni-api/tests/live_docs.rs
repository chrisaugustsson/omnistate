//! US-102: live docs, created and read. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API (the agent),
//! create a scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`):
//!
//! - `POST /docs` (agent) creates a doc at the app level and `POST
//!   /features/{feature_id}/docs` (agent) one in that feature, with the body
//!   of US-101 (`{"kind", "pillar", "title"}`): 201 `{"doc": Doc}`, where the
//!   Doc is the one `GET /docs/{doc_id}` answers, with `feature_id` `null` at
//!   the app level and the feature's ID in a feature. The refusals are 400
//!   `invalid_request` for a missing or unknown kind, pillar or title, 422 `design_needs_ui_pillar` for a design kind
//!   outside `ui`, 422 `design_system_is_global` for a design system in a
//!   feature, 409 `design_system_exists` for a second design system, and 404
//!   `not_found` for a project or feature that is not there. A refusal
//!   creates nothing. A session is 403 `forbidden`: the human never edits.
//! - Both routes are in the route registry as `agent`.
//! - Every doc has a head: the column `docs.head`, the name of a chunk (64
//!   lowercase hex characters, the BLAKE3 hash of its bytes) stored in RustFS
//!   at `ws/{workspace_id}/chunks/{head}`. A new HTML doc's head is the empty
//!   doc, a new design's head is a manifest with one page and no boards.
//! - The doc is readable at once. `GET /docs/{doc_id}/html` reads the head:
//!   `"html": ""` for a new HTML doc, and for a design also `"pages"` (the
//!   head's pages as JSON nodes) and `"tags"` (the same pages in the tag
//!   format), which for a new design is one page without boards.
//! - `doc_changes` is the append-only tenant table of the writes to docs:
//!   `workspace_id`, `doc_id`, `seq` (rising by one per doc, from 1), `agent`
//!   (`{"run", "role", "model", "harness"}` as the agent declared them),
//!   `operations` (JSON), `head_before`, `head_after` and `created_at`. The
//!   columns without a default are `workspace_id`, `doc_id`, `seq`, `agent`,
//!   `operations` and `head_after`. Creating a doc writes its first row: `seq`
//!   1, `head_before` null and `head_after` the new head. Its checklist items
//!   are checked by crates/omni-core/tests/tenancy.rs (which lists it in
//!   `APPEND_ONLY_TABLES`), the deletion tests seed it and the isolation suite
//!   attacks the new routes.

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

const TEST_DB: &str = "omni_us102_live_docs";
const TEST_BUCKET: &str = "omni-us102-live-docs";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const PROJECT_DOCS_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/docs";
const FEATURE_DOCS_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/docs";

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
    format!("us102-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

/// Who sends a request: an agent (an API key) or the human (a session cookie).
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
            ("X-Omni-Run", "run-102".to_string()),
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
                Some(&json!({ "name": unique("live-docs-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

fn new_feature(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/features", project_base(f, project)),
                &agent(f),
                Some(&json!({ "name": unique("Checkout") })),
            ),
            201,
            "create a feature",
        )["feature"],
    )
}

/// Where a live doc is created: the app level or a feature's.
fn scope_docs_path(f: &Fixture, project: &str, feature: Option<&str>) -> String {
    match feature {
        None => format!("{}/docs", project_base(f, project)),
        Some(feature) => format!("{}/features/{feature}/docs", project_base(f, project)),
    }
}

fn create_live_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    feature: Option<&str>,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &scope_docs_path(f, project, feature),
        auth,
        Some(body),
    )
}

/// Creates a live doc as the agent; returns the doc of the answer.
fn create_live(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: Option<&str>,
    kind: &str,
    pillar: &str,
    title: &str,
) -> Value {
    must(
        create_live_response(
            f,
            client,
            &agent(f),
            project,
            feature,
            &json!({ "kind": kind, "pillar": pillar, "title": title }),
        ),
        201,
        "create a doc",
    )["doc"]
        .clone()
}

fn read_html(f: &Fixture, client: &Client, auth: &Auth, project: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/html", doc_path(f, project, doc)),
            auth,
            None,
        ),
        200,
        "read the doc's HTML",
    )
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
struct Change {
    seq: i64,
    agent: Value,
    operations: Option<String>,
    head_before: Option<String>,
    head_after: Option<String>,
    has_created_at: bool,
}

/// `seq`, `agent`, `operations`, `head_before`, `head_after` and whether the
/// row has its `created_at`.
type ChangeRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
);

fn changes_of(doc: &str) -> Vec<Change> {
    block(async {
        let mut conn = admin_connection().await;
        let rows: Vec<ChangeRow> = sqlx::query_as(
            "SELECT seq::bigint, agent::text, operations::text, head_before::text, \
                 head_after::text, created_at IS NOT NULL \
                 FROM doc_changes WHERE doc_id = $1::uuid ORDER BY seq",
        )
        .bind(doc)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the doc changes of {doc}: {e}"));
        rows.into_iter()
            .map(
                |(seq, agent, operations, head_before, head_after, has_created_at)| Change {
                    seq,
                    agent: serde_json::from_str(&agent).expect("the agent is JSON"),
                    operations,
                    head_before,
                    head_after,
                    has_created_at,
                },
            )
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

fn chunk_key(f: &Fixture, chunk: &str) -> String {
    format!("ws/{}/chunks/{chunk}", f.workspace)
}

/// The doc's head is a content-addressed chunk in RustFS.
fn assert_head_is_stored(f: &Fixture, doc: &str) -> String {
    let head = head_of(doc);
    assert!(
        is_chunk_name(&head),
        "the head of {doc} names a chunk (64 lowercase hex characters): {head:?}"
    );
    let bytes = stored(&chunk_key(f, &head));
    assert_eq!(
        blake3::hash(&bytes).to_hex().to_string(),
        head,
        "the head of {doc} is named by the BLAKE3 hash of its bytes"
    );
    head
}

/// The doc's first change is its creation: the only row so far.
fn assert_created_by_first_change(doc: &str) {
    let changes = changes_of(doc);
    assert_eq!(
        changes.len(),
        1,
        "the creation of {doc} is its one row of doc_changes so far"
    );
    let first = &changes[0];
    assert_eq!(first.seq, 1, "the creation is the first change of {doc}");
    assert_eq!(first.head_before, None, "a new doc had no head before");
    assert_eq!(
        first.head_after.as_deref(),
        Some(head_of(doc).as_str()),
        "the creation's head after is the doc's head"
    );
    assert_eq!(first.agent["run"], "run-102", "{}", first.agent);
    assert_eq!(first.agent["role"], "writer", "{}", first.agent);
    assert!(
        first.operations.is_some(),
        "the change records its operations"
    );
    assert!(first.has_created_at, "the change has its time");
}

fn listed_ids(f: &Fixture, client: &Client, project: &str, query: &str) -> Vec<String> {
    let listed = must(
        call(
            client,
            "GET",
            &format!("{}/docs{query}", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "list the docs",
    );
    listed["docs"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of docs: {listed}"))
        .iter()
        .map(id_of)
        .collect()
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_scope_level_create_routes_are_agent_routes_of_the_registry() {
    for path in [PROJECT_DOCS_ROUTE, FEATURE_DOCS_ROUTE] {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == "POST" && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route POST {path}"));
        assert_eq!(entry.access, Access::Agent, "POST {path}");
    }
}

// ---------------------------------------------------------------------------
// Creating a doc
// ---------------------------------------------------------------------------

#[test]
fn an_agent_creates_an_html_doc_at_the_app_level() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let title = unique("Checkout plan");
    let doc = create_live(f, client, &project, None, "html", "spec", &title);
    assert_uuid_v7(&doc["id"]);
    assert_eq!(doc["project_id"], project.as_str(), "{doc}");
    assert_eq!(doc["kind"], "html", "{doc}");
    assert_eq!(doc["pillar"], "spec", "{doc}");
    assert_eq!(doc["title"], title.as_str(), "{doc}");
    assert_eq!(doc["feature_id"], Value::Null, "an app-level doc: {doc}");
    let id = id_of(&doc);

    let shown = must(
        call(client, "GET", &doc_path(f, &project, &id), &agent(f), None),
        200,
        "show the doc",
    );
    assert_eq!(shown["doc"]["id"], id.as_str(), "{shown}");
    assert_eq!(shown["doc"]["title"], title.as_str(), "{shown}");
    assert_eq!(shown["doc"]["feature_id"], Value::Null, "{shown}");

    for auth in [agent(f), human(f)] {
        let read = read_html(f, client, &auth, &project, &id);
        assert_eq!(read["doc_id"], id.as_str(), "{read}");
        assert_eq!(read["html"], "", "a new HTML doc is the empty doc: {read}");
    }

    assert!(
        listed_ids(f, client, &project, "?feature=global").contains(&id),
        "the doc is one of the app level's"
    );
}

#[test]
fn an_agent_creates_a_doc_in_a_feature() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let doc = create_live(
        f,
        client,
        &project,
        Some(&feature),
        "html",
        "architecture",
        "Checkout architecture",
    );
    assert_uuid_v7(&doc["id"]);
    assert_eq!(doc["feature_id"], feature.as_str(), "{doc}");
    assert_eq!(doc["pillar"], "architecture", "{doc}");
    let id = id_of(&doc);

    let shown = must(
        call(client, "GET", &doc_path(f, &project, &id), &agent(f), None),
        200,
        "show the doc",
    );
    assert_eq!(shown["doc"]["feature_id"], feature.as_str(), "{shown}");
    let read = read_html(f, client, &agent(f), &project, &id);
    assert_eq!(read["html"], "", "a new HTML doc is the empty doc: {read}");

    assert!(
        listed_ids(f, client, &project, &format!("?feature={feature}")).contains(&id),
        "the doc is one of the feature's"
    );
    assert!(
        !listed_ids(f, client, &project, "?feature=global").contains(&id),
        "the doc is not one of the app level's"
    );
}

#[test]
fn a_new_design_reads_as_an_empty_design_with_one_page() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    for scope in [None, Some(feature.as_str())] {
        let doc = create_live(
            f,
            client,
            &project,
            scope,
            "design",
            "ui",
            "Checkout screens",
        );
        assert_eq!(doc["kind"], "design", "{doc}");
        let id = id_of(&doc);
        let read = read_html(f, client, &agent(f), &project, &id);
        let pages = read["pages"]
            .as_array()
            .unwrap_or_else(|| panic!("a design reads with its pages: {read}"));
        assert_eq!(pages.len(), 1, "a new design has one page: {read}");
        let page = &pages[0];
        assert_eq!(page["type"], "page", "{read}");
        assert!(
            page["id"].as_str().is_some_and(is_node_id),
            "the page has a node ID: {read}"
        );
        assert!(
            page["children"].as_array().is_none_or(Vec::is_empty),
            "the page has no boards yet: {read}"
        );
        let tags = read["tags"]
            .as_str()
            .unwrap_or_else(|| panic!("a design reads with its tags: {read}"));
        assert_eq!(
            tags.matches("<Page").count(),
            1,
            "the tags hold the one page: {tags:?}"
        );
        assert!(!tags.contains("<Board"), "and no board: {tags:?}");
    }
}

#[test]
fn the_creation_of_a_doc_is_its_first_row_of_doc_changes() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let app_doc = id_of(&create_live(
        f, client, &project, None, "html", "spec", "Plan",
    ));
    let feature_doc = id_of(&create_live(
        f,
        client,
        &project,
        Some(&feature),
        "html",
        "research",
        "Interviews",
    ));
    let design = id_of(&create_live(
        f, client, &project, None, "design", "ui", "Screens",
    ));
    for doc in [&app_doc, &feature_doc, &design] {
        assert_created_by_first_change(doc);
    }
}

#[test]
fn the_head_of_a_new_doc_is_a_stored_chunk_named_by_its_hash() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let html = id_of(&create_live(
        f, client, &project, None, "html", "db", "Schema",
    ));
    let design = id_of(&create_live(
        f, client, &project, None, "design", "ui", "Screens",
    ));
    let html_head = assert_head_is_stored(f, &html);
    let design_head = assert_head_is_stored(f, &design);
    assert_ne!(
        html_head, design_head,
        "an empty design (a manifest with one page) is not the empty HTML doc"
    );
}

#[test]
fn a_refused_create_makes_no_doc_and_no_change() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let docs_of_project = || {
        count(
            "SELECT count(*) FROM docs WHERE project_id = $1::uuid",
            &[&project],
        )
    };
    let changes_of_project = || {
        count(
            "SELECT count(*) FROM doc_changes c JOIN docs d ON d.id = c.doc_id \
             WHERE d.project_id = $1::uuid",
            &[&project],
        )
    };
    // One design system for the project, so a second one is refused.
    create_live(f, client, &project, None, "design-system", "ui", "Tokens");
    let docs_before = docs_of_project();
    let changes_before = changes_of_project();

    for scope in [None, Some(feature.as_str())] {
        for body in [
            json!({ "pillar": "spec", "title": "No kind" }),
            json!({ "kind": "pdf", "pillar": "spec", "title": "Bad kind" }),
            json!({ "kind": "html", "title": "No pillar" }),
            json!({ "kind": "html", "pillar": "prd", "title": "Old pillar" }),
            json!({ "kind": "html", "pillar": "spec" }),
            json!({ "kind": "html", "pillar": "spec", "title": "   " }),
        ] {
            assert_error(
                &create_live_response(f, client, &agent(f), &project, scope, &body),
                400,
                "invalid_request",
            );
        }
        assert_error(
            &create_live_response(
                f,
                client,
                &agent(f),
                &project,
                scope,
                &json!({ "kind": "design", "pillar": "spec", "title": "Misplaced" }),
            ),
            422,
            "design_needs_ui_pillar",
        );
    }
    assert_error(
        &create_live_response(
            f,
            client,
            &agent(f),
            &project,
            Some(&feature),
            &json!({ "kind": "design-system", "pillar": "ui", "title": "Feature tokens" }),
        ),
        422,
        "design_system_is_global",
    );
    assert_error(
        &create_live_response(
            f,
            client,
            &agent(f),
            &project,
            None,
            &json!({ "kind": "design-system", "pillar": "ui", "title": "Second tokens" }),
        ),
        409,
        "design_system_exists",
    );
    let fine = json!({ "kind": "html", "pillar": "spec", "title": "Fine" });
    assert_error(
        &create_live_response(f, client, &agent(f), &project, Some(UNKNOWN_ID), &fine),
        404,
        "not_found",
    );
    assert_error(
        &create_live_response(f, client, &agent(f), UNKNOWN_ID, None, &fine),
        404,
        "not_found",
    );

    assert_eq!(docs_of_project(), docs_before, "a refusal creates no doc");
    assert_eq!(
        changes_of_project(),
        changes_before,
        "a refusal writes no doc change"
    );
}

#[test]
fn the_human_cannot_create_a_doc() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let body = json!({ "kind": "html", "pillar": "spec", "title": "By hand" });
    for scope in [None, Some(feature.as_str())] {
        assert_error(
            &create_live_response(f, client, &human(f), &project, scope, &body),
            403,
            "forbidden",
        );
    }
    assert_eq!(
        count(
            "SELECT count(*) FROM docs WHERE project_id = $1::uuid",
            &[&project],
        ),
        0,
        "the human created no doc"
    );
}

// ---------------------------------------------------------------------------
// Reads follow the head
// ---------------------------------------------------------------------------

