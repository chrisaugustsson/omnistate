//! US-022: docs and their drafts through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint two API keys through the API (two agents),
//! create a scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; the HTML is the format of
//! docs/formats/html-docs.md and the operations are those of
//! docs/formats/operations.md):
//!
//! - `POST /changesets/{changeset_id}/docs` (agent) takes `{"kind", "pillar",
//!   "title"}` and starts a new doc in that change set: 201 `{"doc": Doc,
//!   "draft": Draft}`. `kind` is `html`, `design` or `design-system`, `pillar`
//!   is one of the five pillars `spec`, `ui`, `architecture`, `db` and
//!   `research` (US-101; the old `module` field and its value `prd` are gone),
//!   and `title` is not blank; anything else is 400 `{"error":
//!   "invalid_request"}` and creates nothing. An HTML doc goes in any pillar; a
//!   `design` or `design-system` doc outside `ui` is 422 and creates nothing.
//!   The doc's first version comes through the change set (Q18): the new doc
//!   has no mainline version, and the change set has its draft, which is empty
//!   and based on no version. Creating a doc emits `doc.created` with `data`
//!   `{"doc_id", "changeset_id", "kind", "pillar", "title"}`.
//! - `GET /docs?pillar=<pillar>` (both) lists only that pillar's docs; a
//!   pillar that is not one of the five is 400 `invalid_request`.
//! - The migration of US-101 renames the `docs` column `module` to `pillar`,
//!   turns `prd` into `spec` and keeps the other values, and the column only
//!   accepts the five pillar names.
//! - A Doc is `{"id" (UUIDv7), "project_id", "kind", "pillar", "title",
//!   "created_at", "mainline_version_id", "drafts", "claims"}`:
//!   `mainline_version_id` is `null` until a version is merged, `drafts` lists
//!   the doc's drafts (one per change set) and `claims` its claims (US-023; an
//!   empty array until then). A Draft is `{"id" (UUIDv7), "doc_id",
//!   "changeset_id", "base_version_id"}`: the version it started from, `null`
//!   for a new doc.
//! - `GET /docs/{doc_id}` (both) answers `{"doc": Doc}`.
//! - `GET /docs/{doc_id}/html` (both) answers the mainline version as
//!   `{"doc_id", "version_id", "html"}`, with `data-omni-id` on every block and
//!   no `data-rev`. A doc without a mainline version answers `"version_id":
//!   null` and `"html": ""`. With `?version=<id>` it answers that version
//!   instead; a version that the doc does not have is 404 `not_found`.
//! - `POST /changesets/{changeset_id}/docs/{doc_id}/open` (agent) answers 200
//!   or 201 `{"draft": Draft}`: the change set's draft of the doc, made from the
//!   doc's mainline version, or the one the change set already has (same `id`,
//!   never a second one). A doc without a mainline version exists only in the
//!   change set that created it: opening it in another change set is 409
//!   `{"error": "no_mainline_version"}` and makes no draft.
//! - `GET /changesets/{changeset_id}/docs/{doc_id}` (both) answers `{"draft":
//!   Draft + "html"}`: the draft as HTML with `data-omni-id` and `data-rev`
//!   (the node's revision) on every block.
//! - `POST /changesets/{changeset_id}/docs/{doc_id}/ops` (agent) takes `{"ops":
//!   [...]}`, a batch of operations, and applies it to the change set's draft,
//!   all or nothing. 200 `{"results": [...]}` has one result per operation,
//!   in order: `{"node", "revision", "created", "removed"}`, where `created`
//!   lists the IDs that the server assigned to new nodes (8 lowercase base32
//!   characters, unique within the project). A revision mismatch is 409
//!   `{"error": "conflict", "conflict": {"index", "node", "expected",
//!   "current_revision", "current_html"}}`; any other refusal is 400 `{"error":
//!   "invalid_operation", "index", "message"}`, and a message about an ID that
//!   is not in the draft names that ID. Either way nothing changes. The human
//!   never edits: a session is 403 `forbidden`.
//! - The agent identity of a request is in the headers `X-Omni-Run`,
//!   `X-Omni-Role`, `X-Omni-Model` and `X-Omni-Harness` (US-015). Every applied
//!   operation is one row of the draft's log, the tenant table `draft_ops`
//!   (`workspace_id`, `draft_id`, `seq`, `op` (the operation's JSON), `agent`
//!   (`{"run", "role", "model", "harness"}`), `applied_at`), with `seq` rising
//!   by one per operation of the draft, and emits one `op.applied` event with
//!   `data` `{"doc_id", "changeset_id", "draft_id", "node", "revision",
//!   "created", "removed", "agent"}`. A refused batch logs and emits nothing.
//! - The story's tenant tables are `docs`, `drafts` and `draft_ops`; the
//!   isolation suite and the deletion tests seed them.

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

const TEST_DB: &str = "omni_us022_docs";
const TEST_BUCKET: &str = "omni-us022-docs";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
/// Node IDs in the right format that no node has.
const UNKNOWN_NODE: &str = "zzzzzzzz";
const OTHER_UNKNOWN_NODE: &str = "yyyyyyyy";

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
        // Drafts may keep checkpoints in RustFS, so the API gets a real bucket.
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
    /// Two API keys of the workspace: two agents.
    key: String,
    other_key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us022-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
        let mint = || -> String {
            let response = call(
                &server.client,
                "POST",
                &format!("/api/v1/w/{workspace}/api-keys"),
                &Auth::Session(&session_token("alice")),
                Some(&json!({})),
            );
            assert_eq!(response.status, 201, "mint a key: {}", response.text());
            response.json()["secret"]
                .as_str()
                .expect("the new key has a secret")
                .to_string()
        };
        let key = mint();
        let other_key = mint();
        Fixture {
            workspace,
            alice_token: session_token("alice"),
            key,
            other_key,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

/// Who sends a request: an agent (an API key with its run and role), the
/// human (a session cookie) or nobody.
#[derive(Clone, Copy)]
enum Auth<'a> {
    Agent {
        key: &'a str,
        run: &'a str,
        role: &'a str,
    },
    Session(&'a str),
}

fn agent<'a>(f: &'a Fixture) -> Auth<'a> {
    Auth::Agent {
        key: &f.key,
        run: "run-1",
        role: "writer",
    }
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let headers: Vec<(&str, String)> = match auth {
        Auth::Agent { key, run, role } => vec![
            ("Authorization", format!("Bearer {key}")),
            ("X-Omni-Run", run.to_string()),
            ("X-Omni-Role", role.to_string()),
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

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
}

fn doc_path(f: &Fixture, project: &str, doc: &str) -> String {
    format!("{}/docs/{doc}", project_base(f, project))
}

fn draft_path(f: &Fixture, project: &str, changeset: &str, doc: &str) -> String {
    format!(
        "{}/changesets/{changeset}/docs/{doc}",
        project_base(f, project)
    )
}

/// A new project with an open change set; returns (project ID, change set ID).
fn project_with_changeset(f: &Fixture, client: &Client) -> (String, String) {
    let project = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.workspace),
            &agent(f),
            Some(&json!({ "name": unique("docs-project") })),
        ),
        201,
        "create a project",
    )["project"]["id"]
        .as_str()
        .expect("a project id")
        .to_string();
    let changeset = new_changeset(f, client, &project);
    (project, changeset)
}

fn new_changeset(f: &Fixture, client: &Client, project: &str) -> String {
    must(
        call(
            client,
            "POST",
            &format!("{}/changesets", project_base(f, project)),
            &agent(f),
            Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
        ),
        201,
        "create a change set",
    )["changeset"]["id"]
        .as_str()
        .expect("a change set id")
        .to_string()
}

fn create_doc_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/changesets/{changeset}/docs", project_base(f, project)),
        &agent(f),
        Some(body),
    )
}

/// Creates an HTML Spec doc in the change set; returns the whole answer.
fn create_doc(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    must(
        create_doc_response(
            f,
            client,
            project,
            changeset,
            &json!({ "kind": "html", "pillar": "spec", "title": unique("Checkout PRD") }),
        ),
        201,
        "create a doc",
    )
}

fn id_of(value: &Value) -> String {
    value["id"]
        .as_str()
        .unwrap_or_else(|| panic!("there is an `id`: {value}"))
        .to_string()
}

fn edit(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    doc: &str,
    ops: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/ops", draft_path(f, project, changeset, doc)),
        auth,
        Some(&json!({ "ops": ops })),
    )
}

/// The draft as the API reads it back.
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

/// The value of `name` in one start tag, if it has the attribute.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_string())
}

/// Every start tag of `html` that carries `data-omni-id`, as (ID, the tag's
/// `data-rev` if it has one), in document order.
fn tagged(html: &str) -> Vec<(String, Option<String>)> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start..start + end];
        if let Some(id) = attribute(tag, "data-omni-id") {
            found.push((id, attribute(tag, "data-rev")));
        }
        rest = &rest[start + end + 1..];
    }
    found
}

/// The revision of `node` in the HTML of a draft.
fn rev_in(html: &str, node: &str) -> Option<u64> {
    tagged(html)
        .into_iter()
        .find(|(id, _)| id == node)
        .and_then(|(_, rev)| rev)
        .and_then(|rev| rev.parse().ok())
}

fn insert_top(content: &str) -> Value {
    json!({ "op": "insert", "parent": null, "position": { "after": null }, "content": content })
}

fn insert_after(after: &str, content: &str) -> Value {
    json!({ "op": "insert", "parent": null, "position": { "after": after }, "content": content })
}

fn set_html(node: &str, rev: u64, html: &str) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": { "html": html } })
}

/// Inserts two top-level paragraphs into a draft; returns their IDs.
fn two_paragraphs(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
) -> (String, String) {
    let answer = must(
        edit(
            f,
            client,
            &agent(f),
            project,
            changeset,
            doc,
            &json!([insert_top("<p>First</p>")]),
        ),
        200,
        "insert the first paragraph",
    );
    let first = answer["results"][0]["node"]
        .as_str()
        .expect("the new node")
        .to_string();
    let answer = must(
        edit(
            f,
            client,
            &agent(f),
            project,
            changeset,
            doc,
            &json!([insert_after(&first, "<p>Second</p>")]),
        ),
        200,
        "insert the second paragraph",
    );
    let second = answer["results"][0]["node"]
        .as_str()
        .expect("the new node")
        .to_string();
    (first, second)
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

/// The rows of a draft's log: (seq, op, agent, seconds since applied_at).
fn log_of(draft: &str) -> Vec<(i64, Value, Value, f64)> {
    block(async {
        let mut conn = admin_connection().await;
        let rows = sqlx::query_as::<_, (i64, String, String, f64)>(
            "select seq::bigint, op::jsonb::text, agent::jsonb::text, \
             extract(epoch from (now() - applied_at))::float8 \
             from draft_ops where draft_id = $1::uuid order by seq",
        )
        .bind(draft)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the draft's log in draft_ops: {e}"));
        rows.into_iter()
            .map(|(seq, op, agent, age)| (seq, parse(&op), parse(&agent), age))
            .collect()
    })
}

/// The live events of a project named `event`, oldest first, as their `data`.
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

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_doc_and_draft_routes_are_in_the_registry_with_their_access() {
    let wanted = [
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}",
            Access::Both,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/html",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/open",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}",
            Access::Both,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/ops",
            Access::Agent,
        ),
    ];
    let routes = registry();
    for (method, path, access) in wanted {
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
    }
}

// ---------------------------------------------------------------------------
// omni doc create
// ---------------------------------------------------------------------------

#[test]
fn a_new_doc_starts_in_its_change_set_without_a_mainline_version() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let title = unique("Checkout PRD");
    let answer = must(
        create_doc_response(
            f,
            client,
            &project,
            &changeset,
            &json!({ "kind": "html", "pillar": "spec", "title": title }),
        ),
        201,
        "create a doc",
    );
    let doc = &answer["doc"];
    assert_uuid_v7(&doc["id"]);
    assert_eq!(doc["project_id"], project.as_str(), "{doc}");
    assert_eq!(doc["kind"], "html", "{doc}");
    assert_eq!(doc["pillar"], "spec", "{doc}");
    assert!(
        doc.get("module").is_none(),
        "the pillar replaces the old module: {doc}"
    );
    assert_eq!(doc["title"], title.as_str(), "{doc}");
    assert!(doc["created_at"].is_string(), "{doc}");
    assert_eq!(
        doc["mainline_version_id"],
        Value::Null,
        "the first version comes through the change set, not straight into mainline: {doc}"
    );
    let draft = &answer["draft"];
    assert_uuid_v7(&draft["id"]);
    assert_eq!(draft["doc_id"], doc["id"], "{draft}");
    assert_eq!(draft["changeset_id"], changeset.as_str(), "{draft}");
    assert_eq!(draft["base_version_id"], Value::Null, "{draft}");

    // The draft is empty, and mainline has nothing of the doc yet.
    let doc_id = id_of(doc);
    let read = read_draft(f, client, &project, &changeset, &doc_id);
    assert_eq!(read["id"], draft["id"], "{read}");
    assert_eq!(read["html"].as_str().map(str::trim), Some(""), "{read}");
    let mainline = must(
        call(
            client,
            "GET",
            &format!("{}/html", doc_path(f, &project, &doc_id)),
            &agent(f),
            None,
        ),
        200,
        "read mainline",
    );
    assert_eq!(mainline["doc_id"], doc_id.as_str(), "{mainline}");
    assert_eq!(mainline["version_id"], Value::Null, "{mainline}");
    assert_eq!(mainline["html"], "", "{mainline}");
}

// US-101: the five pillars.
#[test]
fn an_html_doc_starts_in_each_of_the_five_pillars() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    for pillar in ["spec", "ui", "architecture", "db", "research"] {
        let answer = must(
            create_doc_response(
                f,
                client,
                &project,
                &changeset,
                &json!({ "kind": "html", "pillar": pillar, "title": unique(pillar) }),
            ),
            201,
            &format!("create an html doc in {pillar}"),
        );
        assert_eq!(answer["doc"]["kind"], "html", "{answer}");
        assert_eq!(answer["doc"]["pillar"], pillar, "{answer}");
        assert_eq!(answer["draft"]["changeset_id"], changeset.as_str());
        let shown = must(
            call(
                client,
                "GET",
                &doc_path(f, &project, &id_of(&answer["doc"])),
                &agent(f),
                None,
            ),
            200,
            "show the doc",
        );
        assert_eq!(shown["doc"]["pillar"], pillar, "{shown}");
    }
}

#[test]
fn design_docs_start_in_the_ui_pillar() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    for kind in ["design", "design-system"] {
        let answer = must(
            create_doc_response(
                f,
                client,
                &project,
                &changeset,
                &json!({ "kind": kind, "pillar": "ui", "title": unique(kind) }),
            ),
            201,
            &format!("create a {kind} doc in ui"),
        );
        assert_eq!(answer["doc"]["kind"], kind, "{answer}");
        assert_eq!(answer["doc"]["pillar"], "ui", "{answer}");
    }
}

#[test]
fn design_docs_outside_the_ui_pillar_are_422_and_create_nothing() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    for kind in ["design", "design-system"] {
        for pillar in ["spec", "architecture", "db", "research"] {
            let response = create_doc_response(
                f,
                client,
                &project,
                &changeset,
                &json!({ "kind": kind, "pillar": pillar, "title": unique(kind) }),
            );
            assert_eq!(
                response.status,
                422,
                "a {kind} doc in {pillar}: {}",
                response.text()
            );
            assert!(
                response.json()["error"].is_string(),
                "the refusal names its reason: {}",
                response.text()
            );
        }
    }
    let docs: i64 = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from docs where project_id = $1::uuid")
            .bind(&project)
            .fetch_one(&mut conn)
            .await
            .expect("count the project's docs")
    });
    assert_eq!(docs, 0, "a refused doc is not stored");
}

#[test]
fn a_doc_needs_a_known_kind_and_pillar_and_a_title() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    for body in [
        json!({ "kind": "pdf", "pillar": "spec", "title": "T" }),
        json!({ "kind": "html", "pillar": "marketing", "title": "T" }),
        json!({ "kind": "html", "pillar": "other", "title": "T" }),
        // The old module value and the old field are no pillar.
        json!({ "kind": "html", "pillar": "prd", "title": "T" }),
        json!({ "kind": "html", "module": "spec", "title": "T" }),
        json!({ "kind": "html", "pillar": "spec", "title": "   " }),
        json!({ "pillar": "spec", "title": "T" }),
        json!({ "kind": "html", "title": "T" }),
        json!({ "kind": "html", "pillar": "spec" }),
    ] {
        let response = create_doc_response(f, client, &project, &changeset, &body);
        assert_error(&response, 400, "invalid_request");
    }
    let docs: i64 = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from docs where project_id = $1::uuid")
            .bind(&project)
            .fetch_one(&mut conn)
            .await
            .expect("count the project's docs")
    });
    assert_eq!(docs, 0, "a refused doc is not stored");
}

#[test]
fn a_doc_starts_only_in_a_change_set_of_its_project() {
    let (f, client) = ready();
    let (project, _changeset) = project_with_changeset(f, client);
    let (_other_project, other_changeset) = project_with_changeset(f, client);
    for changeset in [UNKNOWN_ID, other_changeset.as_str()] {
        let response = create_doc_response(
            f,
            client,
            &project,
            changeset,
            &json!({ "kind": "html", "pillar": "spec", "title": "T" }),
        );
        assert_error(&response, 404, "not_found");
    }
}

#[test]
fn creating_a_doc_emits_doc_created() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let answer = create_doc(f, client, &project, &changeset);
    let doc = id_of(&answer["doc"]);
    let events = events_of(&project, "doc.created");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["doc_id"], doc.as_str(), "{}", events[0]);
    assert_eq!(
        events[0]["changeset_id"],
        changeset.as_str(),
        "{}",
        events[0]
    );
    assert_eq!(events[0]["kind"], "html", "{}", events[0]);
    assert_eq!(events[0]["pillar"], "spec", "{}", events[0]);
    assert_eq!(events[0]["title"], answer["doc"]["title"], "{}", events[0]);
}

// ---------------------------------------------------------------------------
// omni doc open
// ---------------------------------------------------------------------------

#[test]
fn opening_a_doc_in_its_change_set_reuses_the_draft_it_has() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let answer = create_doc(f, client, &project, &changeset);
    let doc = id_of(&answer["doc"]);
    let draft = id_of(&answer["draft"]);
    for attempt in 0..2 {
        let response = call(
            client,
            "POST",
            &format!("{}/open", draft_path(f, &project, &changeset, &doc)),
            &agent(f),
            None,
        );
        assert!(
            matches!(response.status, 200 | 201),
            "open #{attempt}: {}",
            response.text()
        );
        let opened = response.json()["draft"].clone();
        assert_eq!(opened["id"], draft.as_str(), "the same draft: {opened}");
        assert_eq!(opened["changeset_id"], changeset.as_str(), "{opened}");
        assert_eq!(opened["doc_id"], doc.as_str(), "{opened}");
        assert_eq!(opened["base_version_id"], Value::Null, "{opened}");
    }
    let shown = must(
        call(client, "GET", &doc_path(f, &project, &doc), &agent(f), None),
        200,
        "show the doc",
    );
    let drafts = shown["doc"]["drafts"].as_array().expect("drafts").clone();
    assert_eq!(drafts.len(), 1, "never a second draft: {drafts:?}");
}

#[test]
fn a_doc_without_a_mainline_version_cannot_be_opened_in_another_change_set() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let other = new_changeset(f, client, &project);
    let response = call(
        client,
        "POST",
        &format!("{}/open", draft_path(f, &project, &other, &doc)),
        &agent(f),
        None,
    );
    assert_error(&response, 409, "no_mainline_version");
    let shown = must(
        call(client, "GET", &doc_path(f, &project, &doc), &agent(f), None),
        200,
        "show the doc",
    );
    let drafts = shown["doc"]["drafts"].as_array().expect("drafts").clone();
    assert_eq!(drafts.len(), 1, "no draft was made: {drafts:?}");
    assert_eq!(drafts[0]["changeset_id"], changeset.as_str());
}

#[test]
fn opening_an_unknown_doc_or_change_set_is_not_found() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    for (changeset, doc) in [(changeset.as_str(), UNKNOWN_ID), (UNKNOWN_ID, doc.as_str())] {
        let response = call(
            client,
            "POST",
            &format!("{}/open", draft_path(f, &project, changeset, doc)),
            &agent(f),
            None,
        );
        assert_error(&response, 404, "not_found");
    }
}

// ---------------------------------------------------------------------------
// omni doc read and show
// ---------------------------------------------------------------------------

#[test]
fn reading_a_draft_gives_every_block_its_node_id_and_revision() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let answer = must(
        edit(
            f,
            client,
            &agent(f),
            &project,
            &changeset,
            &doc,
            &json!([insert_top(
                "<section><h2>Goals</h2><p>Faster checkout</p><ul><li>One</li><li>Two</li></ul></section>"
            )]),
        ),
        200,
        "insert a section",
    );
    let created: Vec<String> = answer["results"][0]["created"]
        .as_array()
        .expect("created")
        .iter()
        .map(|id| id.as_str().expect("an ID").to_string())
        .collect();
    assert_eq!(
        created.len(),
        6,
        "section, heading, paragraph, list, two items"
    );

    let html = draft_html(f, client, &project, &changeset, &doc);
    let tags = tagged(&html);
    assert_eq!(
        tags.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
        created,
        "every block carries its data-omni-id, in document order: {html}"
    );
    for (id, rev) in &tags {
        assert_eq!(rev.as_deref(), Some("1"), "{id} carries data-rev: {html}");
    }
    assert!(html.contains("Faster checkout"), "{html}");

    // The human reads the same draft.
    let response = call(
        client,
        "GET",
        &draft_path(f, &project, &changeset, &doc),
        &Auth::Session(&f.alice_token),
        None,
    );
    let human = must(response, 200, "the human reads the draft");
    assert_eq!(human["draft"]["html"], html.as_str());
}

#[test]
fn a_change_set_without_a_draft_of_the_doc_has_nothing_to_read() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let other = new_changeset(f, client, &project);
    let response = call(
        client,
        "GET",
        &draft_path(f, &project, &other, &doc),
        &agent(f),
        None,
    );
    assert_error(&response, 404, "not_found");
}

#[test]
fn reading_a_version_the_doc_does_not_have_is_not_found() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let response = call(
        client,
        "GET",
        &format!("{}/html?version={UNKNOWN_ID}", doc_path(f, &project, &doc)),
        &agent(f),
        None,
    );
    assert_error(&response, 404, "not_found");
}

#[test]
fn showing_a_doc_gives_its_metadata_mainline_drafts_and_claims() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let created = create_doc(f, client, &project, &changeset);
    let doc = id_of(&created["doc"]);
    for auth in [agent(f), Auth::Session(&f.alice_token)] {
        let shown = must(
            call(client, "GET", &doc_path(f, &project, &doc), &auth, None),
            200,
            "show the doc",
        )["doc"]
            .clone();
        for field in ["id", "project_id", "kind", "pillar", "title", "created_at"] {
            assert_eq!(shown[field], created["doc"][field], "{field}: {shown}");
        }
        assert_eq!(shown["mainline_version_id"], Value::Null, "{shown}");
        let drafts = shown["drafts"].as_array().expect("drafts");
        assert_eq!(drafts.len(), 1, "{shown}");
        assert_eq!(drafts[0]["id"], created["draft"]["id"], "{shown}");
        assert_eq!(drafts[0]["changeset_id"], changeset.as_str(), "{shown}");
        assert_eq!(shown["claims"], json!([]), "no claims yet: {shown}");
    }
    let response = call(
        client,
        "GET",
        &doc_path(f, &project, UNKNOWN_ID),
        &agent(f),
        None,
    );
    assert_error(&response, 404, "not_found");
}

// ---------------------------------------------------------------------------
// omni doc edit
// ---------------------------------------------------------------------------

#[test]
fn an_edit_returns_the_new_revisions_and_the_ids_of_new_nodes() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let (first, second) = two_paragraphs(f, client, &project, &changeset, &doc);
    assert!(
        is_node_id(&first) && is_node_id(&second),
        "{first} {second}"
    );

    let answer = must(
        edit(
            f,
            client,
            &agent(f),
            &project,
            &changeset,
            &doc,
            &json!([
                set_html(&first, 1, "First, <em>rewritten</em>"),
                insert_after(&second, "<ul><li>Alpha</li><li>Beta</li></ul>"),
                { "op": "delete", "node": second, "rev": 1 }
            ]),
        ),
        200,
        "edit",
    );
    let results = answer["results"].as_array().expect("results").clone();
    assert_eq!(results.len(), 3, "one result per operation: {answer}");
    assert_eq!(results[0]["node"], first.as_str());
    assert_eq!(results[0]["revision"], 2, "{answer}");
    assert_eq!(results[0]["created"], json!([]), "{answer}");
    let created: Vec<&str> = results[1]["created"]
        .as_array()
        .expect("created")
        .iter()
        .map(|id| id.as_str().expect("an ID"))
        .collect();
    assert_eq!(created.len(), 3, "the list and its two items: {answer}");
    assert_eq!(results[1]["node"], created[0], "{answer}");
    assert_eq!(results[1]["revision"], 1, "{answer}");
    for id in &created {
        assert!(is_node_id(id), "the server assigns 8-character IDs: {id}");
    }
    assert_eq!(results[2]["node"], second.as_str());
    assert_eq!(results[2]["removed"], json!([second]), "{answer}");

    let html = draft_html(f, client, &project, &changeset, &doc);
    assert_eq!(rev_in(&html, &first), Some(2), "{html}");
    assert!(html.contains("<em>rewritten</em>"), "{html}");
    assert!(!html.contains(&second), "the deleted node is gone: {html}");
    for id in &created {
        assert_eq!(rev_in(&html, id), Some(1), "{id}: {html}");
    }
}

#[test]
fn the_server_assigns_node_ids_unique_within_the_project() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let section = format!(
        "<section><h2>Many</h2>{}</section>",
        (0..60)
            .map(|i| format!("<p>Paragraph {i}</p>"))
            .collect::<String>()
    );
    let mut seen = BTreeSet::new();
    let mut total = 0;
    for _ in 0..2 {
        let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
        let answer = must(
            edit(
                f,
                client,
                &agent(f),
                &project,
                &changeset,
                &doc,
                &json!([insert_top(&section), insert_top(&section)]),
            ),
            200,
            "insert many blocks",
        );
        for result in answer["results"].as_array().expect("results") {
            for id in result["created"].as_array().expect("created") {
                let id = id.as_str().expect("an ID");
                assert!(
                    is_node_id(id),
                    "{id:?} is not 8 lowercase base32 characters"
                );
                seen.insert(id.to_string());
                total += 1;
            }
        }
    }
    assert_eq!(total, 4 * 62, "every block got an ID");
    assert_eq!(seen.len(), total, "no ID is used twice in the project");
}

/// US-109: an agent-supplied `data-omni-id` that is valid and unique is kept
/// (it was refused before US-109).
#[test]
fn content_may_choose_its_own_valid_unique_node_id() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let response = edit(
        f,
        client,
        &agent(f),
        &project,
        &changeset,
        &doc,
        &json!([insert_top("<p data-omni-id=\"k3f9x2qa\">Mine</p>")]),
    );
    assert_eq!(response.status, 200, "{}", response.text());
    let answer = response.json();
    assert_eq!(answer["results"][0]["node"], "k3f9x2qa", "{answer}");
    let html = draft_html(f, client, &project, &changeset, &doc);
    assert_eq!(rev_in(&html, "k3f9x2qa"), Some(1), "{html}");
}

#[test]
fn referring_to_an_unknown_node_is_an_error_that_names_it_and_changes_nothing() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let (first, _second) = two_paragraphs(f, client, &project, &changeset, &doc);
    let draft = id_of(&read_draft(f, client, &project, &changeset, &doc));
    let html_before = draft_html(f, client, &project, &changeset, &doc);
    let log_before = log_of(&draft).len();
    let events_before = events_of(&project, "op.applied").len();

    let cases = [
        (json!([set_html(UNKNOWN_NODE, 1, "x")]), 0, UNKNOWN_NODE),
        (
            json!([{ "op": "delete", "node": UNKNOWN_NODE, "rev": 1 }]),
            0,
            UNKNOWN_NODE,
        ),
        (
            json!([{
                "op": "insert",
                "parent": OTHER_UNKNOWN_NODE,
                "position": { "after": null },
                "content": "<p>x</p>"
            }]),
            0,
            OTHER_UNKNOWN_NODE,
        ),
        (
            json!([insert_after(OTHER_UNKNOWN_NODE, "<p>x</p>")]),
            0,
            OTHER_UNKNOWN_NODE,
        ),
        // A valid first operation does not apply when a later one fails.
        (
            json!([
                set_html(&first, 1, "Changed"),
                set_html(UNKNOWN_NODE, 1, "x")
            ]),
            1,
            UNKNOWN_NODE,
        ),
    ];
    for (ops, index, named) in cases {
        let response = edit(f, client, &agent(f), &project, &changeset, &doc, &ops);
        assert_eq!(response.status, 400, "{ops}: {}", response.text());
        let body = response.json();
        assert_eq!(body["error"], "invalid_operation", "{body}");
        assert_eq!(body["index"], index, "{ops}: {body}");
        let message = body["message"].as_str().unwrap_or_else(|| panic!("{body}"));
        assert!(
            message.contains(named),
            "the error names the unknown ID {named}: {message:?}"
        );
    }
    assert_eq!(
        draft_html(f, client, &project, &changeset, &doc),
        html_before,
        "nothing changed"
    );
    assert_eq!(log_of(&draft).len(), log_before, "nothing was logged");
    assert_eq!(
        events_of(&project, "op.applied").len(),
        events_before,
        "nothing was emitted"
    );
}

#[test]
fn a_stale_revision_is_a_conflict_with_the_current_state_and_changes_nothing() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let (first, _second) = two_paragraphs(f, client, &project, &changeset, &doc);
    must(
        edit(
            f,
            client,
            &agent(f),
            &project,
            &changeset,
            &doc,
            &json!([set_html(&first, 1, "Changed by someone else")]),
        ),
        200,
        "the first change",
    );
    let before = draft_html(f, client, &project, &changeset, &doc);
    let response = edit(
        f,
        client,
        &agent(f),
        &project,
        &changeset,
        &doc,
        &json!([set_html(&first, 1, "My stale change")]),
    );
    assert_eq!(response.status, 409, "{}", response.text());
    let body = response.json();
    assert_eq!(body["error"], "conflict", "{body}");
    let conflict = &body["conflict"];
    assert_eq!(conflict["index"], 0, "{body}");
    assert_eq!(conflict["node"], first.as_str(), "{body}");
    assert_eq!(conflict["expected"], 1, "{body}");
    assert_eq!(conflict["current_revision"], 2, "{body}");
    assert!(
        conflict["current_html"]
            .as_str()
            .is_some_and(|html| html.contains("Changed by someone else")),
        "{body}"
    );
    assert_eq!(draft_html(f, client, &project, &changeset, &doc), before);
}

#[test]
fn the_human_never_edits_a_draft() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let response = edit(
        f,
        client,
        &Auth::Session(&f.alice_token),
        &project,
        &changeset,
        &doc,
        &json!([insert_top("<p>The human's words</p>")]),
    );
    assert_error(&response, 403, "forbidden");
    assert_eq!(draft_html(f, client, &project, &changeset, &doc).trim(), "");
}

#[test]
fn operations_from_different_agents_on_different_nodes_all_apply() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let (first, second) = two_paragraphs(f, client, &project, &changeset, &doc);
    const ROUNDS: u64 = 25;
    let agents = [
        (
            Auth::Agent {
                key: &f.key,
                run: "run-a",
                role: "writer",
            },
            first.clone(),
        ),
        (
            Auth::Agent {
                key: &f.other_key,
                run: "run-b",
                role: "reviewer",
            },
            second.clone(),
        ),
    ];
    std::thread::scope(|scope| {
        for (auth, node) in &agents {
            let (project, changeset, doc) = (&project, &changeset, &doc);
            scope.spawn(move || {
                for round in 1..=ROUNDS {
                    let text = format!("{node} round {round}");
                    let response = edit(
                        f,
                        client,
                        auth,
                        project,
                        changeset,
                        doc,
                        &json!([
                            set_html(node, round, &text),
                            insert_top(&format!("<p>{node} adds {round}</p>"))
                        ]),
                    );
                    assert_eq!(
                        response.status,
                        200,
                        "{node} round {round}: {}",
                        response.text()
                    );
                    assert_eq!(response.json()["results"][0]["revision"], round + 1);
                }
            });
        }
    });
    let html = draft_html(f, client, &project, &changeset, &doc);
    for node in [&first, &second] {
        assert_eq!(rev_in(&html, node), Some(ROUNDS + 1), "{node}: {html}");
        assert!(
            html.contains(&format!("{node} round {ROUNDS}")),
            "{node}'s last change is in the draft"
        );
        for round in 1..=ROUNDS {
            assert!(
                html.contains(&format!("{node} adds {round}<")),
                "{node}'s insert {round} is in the draft"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The draft's log and op.applied
// ---------------------------------------------------------------------------

#[test]
fn every_applied_operation_is_logged_with_the_agent_identity_and_time() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let created = create_doc(f, client, &project, &changeset);
    let doc = id_of(&created["doc"]);
    let draft = id_of(&created["draft"]);
    assert!(log_of(&draft).is_empty(), "a new draft has an empty log");

    let writer = Auth::Agent {
        key: &f.key,
        run: "run-41",
        role: "writer",
    };
    let answer = must(
        edit(
            f,
            client,
            &writer,
            &project,
            &changeset,
            &doc,
            &json!([insert_top("<p>One</p>"), insert_top("<p>Two</p>")]),
        ),
        200,
        "the writer's batch",
    );
    let one = answer["results"][0]["node"]
        .as_str()
        .expect("a node")
        .to_string();
    let editor = Auth::Agent {
        key: &f.other_key,
        run: "run-42",
        role: "editor",
    };
    must(
        edit(
            f,
            client,
            &editor,
            &project,
            &changeset,
            &doc,
            &json!([set_html(&one, 1, "One, edited")]),
        ),
        200,
        "the editor's batch",
    );

    let log = log_of(&draft);
    assert_eq!(log.len(), 3, "one row per applied operation: {log:?}");
    let seqs: Vec<i64> = log.iter().map(|(seq, ..)| *seq).collect();
    assert!(
        seqs.windows(2).all(|w| w[1] == w[0] + 1),
        "seq rises by one: {seqs:?}"
    );
    let ops: Vec<&str> = log
        .iter()
        .map(|(_, op, ..)| op["op"].as_str().unwrap_or("?"))
        .collect();
    assert_eq!(ops, ["insert", "insert", "set"], "the operations, in order");
    assert_eq!(log[2].1["node"], one.as_str(), "{:?}", log[2].1);
    for (row, (run, role)) in log.iter().zip([
        ("run-41", "writer"),
        ("run-41", "writer"),
        ("run-42", "editor"),
    ]) {
        let agent = &row.2;
        assert_eq!(agent["run"], run, "{agent}");
        assert_eq!(agent["role"], role, "{agent}");
        assert_eq!(agent["model"], "test-model", "{agent}");
        assert_eq!(agent["harness"], "test-harness", "{agent}");
        assert!(
            (0.0..300.0).contains(&row.3),
            "applied_at is the time of the request: {} seconds ago",
            row.3
        );
    }
}

#[test]
fn every_applied_operation_emits_op_applied() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let created = create_doc(f, client, &project, &changeset);
    let doc = id_of(&created["doc"]);
    let draft = id_of(&created["draft"]);
    let writer = Auth::Agent {
        key: &f.key,
        run: "run-51",
        role: "writer",
    };
    let answer = must(
        edit(
            f,
            client,
            &writer,
            &project,
            &changeset,
            &doc,
            &json!([insert_top("<p>One</p>"), insert_top("<ol><li>A</li></ol>")]),
        ),
        200,
        "a batch of two",
    );
    let one = answer["results"][0]["node"]
        .as_str()
        .expect("a node")
        .to_string();
    must(
        edit(
            f,
            client,
            &writer,
            &project,
            &changeset,
            &doc,
            &json!([set_html(&one, 1, "Changed")]),
        ),
        200,
        "a set",
    );

    let events = events_of(&project, "op.applied");
    assert_eq!(
        events.len(),
        3,
        "one event per applied operation: {events:?}"
    );
    for event in &events {
        assert_eq!(event["doc_id"], doc.as_str(), "{event}");
        assert_eq!(event["changeset_id"], changeset.as_str(), "{event}");
        assert_eq!(event["draft_id"], draft.as_str(), "{event}");
        assert_eq!(event["agent"]["run"], "run-51", "{event}");
        assert_eq!(event["agent"]["role"], "writer", "{event}");
    }
    assert_eq!(events[0]["node"], one.as_str(), "{}", events[0]);
    assert_eq!(events[0]["created"], json!([one]), "{}", events[0]);
    assert_eq!(events[1]["created"].as_array().map(Vec::len), Some(2));
    assert_eq!(events[2]["node"], one.as_str(), "{}", events[2]);
    assert_eq!(events[2]["revision"], 2, "{}", events[2]);
    assert_eq!(events[2]["removed"], json!([]), "{}", events[2]);

    // A conflict emits nothing.
    let response = edit(
        f,
        client,
        &writer,
        &project,
        &changeset,
        &doc,
        &json!([set_html(&one, 1, "Stale")]),
    );
    assert_eq!(response.status, 409, "{}", response.text());
    assert_eq!(events_of(&project, "op.applied").len(), 3);
}

// ---------------------------------------------------------------------------
// US-101: the docs of a pillar, and the migration to pillars
// ---------------------------------------------------------------------------

/// The titles of the docs that `GET .../docs<query>` lists.
fn listed_titles(f: &Fixture, client: &Client, project: &str, query: &str) -> Vec<String> {
    let answer = must(
        call(
            client,
            "GET",
            &format!("{}/docs{query}", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        &format!("list the docs{query}"),
    );
    answer["docs"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of docs: {answer}"))
        .iter()
        .map(|doc| doc["title"].as_str().expect("a title").to_string())
        .collect()
}

#[test]
fn the_docs_of_one_pillar_are_listed_with_the_pillar_filter() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let mut titles = Vec::new();
    for (kind, pillar, title) in [
        ("html", "spec", "Checkout spec"),
        ("html", "research", "Interview notes"),
        ("design", "ui", "Checkout screens"),
        ("html", "spec", "Refund spec"),
        ("html", "db", "Orders schema"),
    ] {
        let title = unique(title);
        must(
            create_doc_response(
                f,
                client,
                &project,
                &changeset,
                &json!({ "kind": kind, "pillar": pillar, "title": title }),
            ),
            201,
            &format!("create a {kind} doc in {pillar}"),
        );
        titles.push(title);
    }
    assert_eq!(
        listed_titles(f, client, &project, "?pillar=spec"),
        [titles[0].clone(), titles[3].clone()],
        "only the Spec docs, oldest first"
    );
    assert_eq!(
        listed_titles(f, client, &project, "?pillar=research"),
        [titles[1].clone()],
        "only the Research docs"
    );
    assert_eq!(
        listed_titles(f, client, &project, "?pillar=architecture"),
        Vec::<String>::new(),
        "a pillar without docs lists none"
    );
    assert_eq!(
        listed_titles(f, client, &project, "").len(),
        5,
        "without the filter every doc is listed"
    );
    for pillar in ["other", "prd"] {
        let response = call(
            client,
            "GET",
            &format!("{}/docs?pillar={pillar}", project_base(f, &project)),
            &agent(f),
            None,
        );
        assert_error(&response, 400, "invalid_request");
    }
}

/// The version of the last migration before US-101; the pillar migration
/// comes after it.
const LAST_MIGRATION_WITH_MODULES: i64 = 20261024000000;
const PILLARS_DB: &str = "omni_us101_pillars";

fn migrations_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

#[test]
fn the_pillar_migration_renames_module_maps_prd_to_spec_and_accepts_only_the_five_pillars() {
    use sqlx::migrate::Migrator;
    block(async {
        let mut admin = PgConnection::connect_with(&admin_options())
            .await
            .unwrap_or_else(|e| panic!("cannot connect to Postgres as the admin role: {e}"));
        sqlx::query(&format!(
            "DROP DATABASE IF EXISTS {PILLARS_DB} WITH (FORCE)"
        ))
        .execute(&mut admin)
        .await
        .expect("drop the scratch database");
        sqlx::query(&format!("CREATE DATABASE {PILLARS_DB}"))
            .execute(&mut admin)
            .await
            .expect("create the scratch database");
        let mut conn = PgConnection::connect_with(&admin_options().database(PILLARS_DB))
            .await
            .expect("connect to the scratch database");

        // The schema as it was before US-101, with a doc of every old module.
        let all = Migrator::new(migrations_dir())
            .await
            .expect("read migrations/");
        let mut before = Migrator::new(migrations_dir())
            .await
            .expect("read migrations/");
        before.migrations = all
            .iter()
            .filter(|m| m.version <= LAST_MIGRATION_WITH_MODULES)
            .cloned()
            .collect::<Vec<_>>()
            .into();
        assert!(
            all.iter().any(|m| m.version > LAST_MIGRATION_WITH_MODULES),
            "a new migration comes after {LAST_MIGRATION_WITH_MODULES}"
        );
        before
            .run(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("applying the migrations before US-101 failed: {e}"));
        let workspace: String = sqlx::query_scalar(
            "insert into workspaces (name) values ('Pillars') returning id::text",
        )
        .fetch_one(&mut conn)
        .await
        .expect("seed a workspace");
        let project: String = sqlx::query_scalar(
            "insert into projects (workspace_id, name) values ($1::uuid, 'shop') returning id::text",
        )
        .bind(&workspace)
        .fetch_one(&mut conn)
        .await
        .expect("seed a project");
        for (kind, module) in [
            ("html", "prd"),
            ("html", "architecture"),
            ("html", "db"),
            ("design", "ui"),
        ] {
            sqlx::query(
                "insert into docs (workspace_id, project_id, kind, module, title) \
                 values ($1::uuid, $2::uuid, $3, $4, $4)",
            )
            .bind(&workspace)
            .bind(&project)
            .bind(kind)
            .bind(module)
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("seed a {module} doc: {e}"));
        }

        all.run(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("applying the pillar migration failed: {e}"));

        let columns: Vec<String> = sqlx::query_scalar(
            "select column_name::text from information_schema.columns \
             where table_schema = 'public' and table_name = 'docs'",
        )
        .fetch_all(&mut conn)
        .await
        .expect("read the columns of docs");
        assert!(
            columns.iter().any(|c| c == "pillar"),
            "docs has the column pillar: {columns:?}"
        );
        assert!(
            !columns.iter().any(|c| c == "module"),
            "docs has no column module any more: {columns:?}"
        );

        // The title of each seeded doc is its old module.
        let pillars: Vec<(String, String)> = sqlx::query_as(
            "select title, pillar::text from docs where project_id = $1::uuid order by title",
        )
        .bind(&project)
        .fetch_all(&mut conn)
        .await
        .expect("read the docs' pillars");
        assert_eq!(
            pillars,
            [
                ("architecture".to_string(), "architecture".to_string()),
                ("db".to_string(), "db".to_string()),
                ("prd".to_string(), "spec".to_string()),
                ("ui".to_string(), "ui".to_string()),
            ],
            "prd becomes spec and the other modules keep their names"
        );

        for pillar in ["spec", "ui", "architecture", "db", "research"] {
            sqlx::query(
                "insert into docs (workspace_id, project_id, kind, pillar, title) \
                 values ($1::uuid, $2::uuid, 'html', $3, $3)",
            )
            .bind(&workspace)
            .bind(&project)
            .bind(pillar)
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("the column accepts the pillar {pillar}: {e}"));
        }
        for pillar in ["prd", "other", "Spec", ""] {
            let refused = sqlx::query(
                "insert into docs (workspace_id, project_id, kind, pillar, title) \
                 values ($1::uuid, $2::uuid, 'html', $3, 'refused')",
            )
            .bind(&workspace)
            .bind(&project)
            .bind(pillar)
            .execute(&mut conn)
            .await;
            let error = refused
                .err()
                .unwrap_or_else(|| panic!("the column accepts {pillar:?}, which is no pillar"));
            assert_eq!(
                error
                    .as_database_error()
                    .and_then(|e| e.code().map(|c| c.into_owned()))
                    .as_deref(),
                Some("23514"),
                "{pillar:?} is refused by a check constraint: {error}"
            );
        }
        drop(conn);
        sqlx::query(&format!(
            "DROP DATABASE IF EXISTS {PILLARS_DB} WITH (FORCE)"
        ))
        .execute(&mut admin)
        .await
        .expect("drop the scratch database");
    });
}

// ---------------------------------------------------------------------------
// Tenancy
// ---------------------------------------------------------------------------

#[test]
fn docs_drafts_and_the_log_are_tenant_tables_of_the_workspace() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let created = create_doc(f, client, &project, &changeset);
    let doc = id_of(&created["doc"]);
    must(
        edit(
            f,
            client,
            &agent(f),
            &project,
            &changeset,
            &doc,
            &json!([insert_top("<p>Tenant</p>")]),
        ),
        200,
        "edit",
    );
    let owners: Vec<(String, String)> = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_as(
            "select 'docs', workspace_id::text from docs where id = $1::uuid \
             union all select 'drafts', workspace_id::text from drafts where doc_id = $1::uuid \
             union all select 'draft_ops', o.workspace_id::text from draft_ops o \
               join drafts d on d.id = o.draft_id where d.doc_id = $1::uuid",
        )
        .bind(&doc)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read docs, drafts and draft_ops: {e}"))
    });
    let tables: BTreeSet<&str> = owners.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        tables,
        BTreeSet::from(["docs", "drafts", "draft_ops"]),
        "{owners:?}"
    );
    for (table, owner) in &owners {
        assert_eq!(owner, &f.workspace, "{table}");
    }
}
