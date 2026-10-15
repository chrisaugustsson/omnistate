//! US-024: versions and comparisons through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; docs, drafts and
//! operations are those of US-022, tests/docs.rs):
//!
//! - `POST /changesets/{changeset_id}/docs/{doc_id}/versions` (agent) takes
//!   `{}` and saves the change set's draft of the doc as a version: 201
//!   `{"version": Version}`. A change set without a draft of the doc, and an
//!   unknown doc, are 404 `{"error": "not_found"}` and save nothing. Saving a
//!   version does not touch the doc's mainline version: versions become
//!   mainline only when their change set merges (US-025).
//! - A Version is `{"id" (UUIDv7), "doc_id", "changeset_id", "draft_id",
//!   "removed", "created_at", "agent": {"run", "role", "model", "harness"},
//!   "nodes"}`. `nodes` is the version's node index: one `{"id", "type",
//!   "parent", "fingerprint"}` per node, in document order. `type` is the
//!   block's type as the HTML doc format stores it (`section`, `heading`,
//!   `paragraph`, ...), `parent` the parent's node ID (`null` at the top
//!   level) and `fingerprint` a non-empty string that covers the node's own
//!   content (its type, properties and text) and nothing else: neither its
//!   children nor its place. A version is unchangeable: whatever happens to
//!   the draft afterwards, the version's node index and its HTML (`GET
//!   /docs/{doc_id}/html?version=<id>`, US-022) stay exactly as saved.
//! - `GET /versions/{version_id}` (both) answers `{"version": Version}`.
//!   A version of another project, or none, is 404 `not_found`.
//! - `GET /versions/{version_id}/diff/{other_version_id}` (both) compares two
//!   versions of one doc, from the first to the second: 200 `{"diff": {"doc_id",
//!   "from", "to", "added", "removed", "changed", "moved"}}`. `added` lists the
//!   nodes only the second version has and `removed` those only the first has
//!   (as their node index entries); `changed` lists the nodes in both whose
//!   fingerprint differs (as in the second version); `moved` lists the nodes in
//!   both whose place differs, as `{"id", "type", "from_parent", "to_parent",
//!   "from_after", "to_after"}`, where `from_after` and `to_after` are the
//!   sibling the node follows in the first and in the second version (`null`
//!   for a first child). A node's place differs when its parent does, or when
//!   it keeps its parent but its order among the siblings that also keep that
//!   parent changes (a reorder with the `move` op). A reorder lists only the
//!   fewest nodes that account for it, those outside a longest common
//!   subsequence of the two orders: moving one node after another sibling
//!   lists just that node, not the siblings it passed, and an insertion or a
//!   deletion that shifts its siblings lists none of them. A node in none of
//!   the four lists is the same in both.
//!   Versions of two different docs are 400 `{"error": "invalid_request"}`.
//! - `POST /changesets/{changeset_id}/docs/{doc_id}/remove` (agent) takes `{}`
//!   and removes the doc in that change set: it saves a version marking the
//!   doc as removed, 201 `{"version": Version}` with `"removed": true` and no
//!   nodes. Nothing is deleted: the doc and every earlier version stay
//!   readable.
//! - Saving a version (a removal included) emits `version.saved` with `data`
//!   `{"version_id", "doc_id", "changeset_id", "draft_id", "removed",
//!   "agent"}`.
//! - Saving a version while the change set is `approved` or `needs_rebase`
//!   (other than through a rebase, US-026) clears the approval: the change set
//!   moves back to `in_review`, its history gains `{"from_status": <that
//!   status>, "to_status": "in_review", "actor": "system"}` and it emits
//!   `changeset.in_review` (US-017). In `open` or `in_review` the status stays.
//! - The human never saves or removes: a session is 403 `forbidden`.
//! - Versions live in the tenant table `versions`, whose only columns without
//!   a default are `workspace_id`, `doc_id` and `changeset_id`, and the node
//!   index in the tenant table `node_index`, whose only columns without a
//!   default are `workspace_id`, `chunk` (the stored content that the nodes
//!   belong to, so that later design versions can share it, US-028), `node`,
//!   `type` and `fingerprint`; tests/workspace_delete.rs and the isolation
//!   suite seed them. They and the operation log (`draft_ops`) are
//!   append-only: the API role has no UPDATE privilege on any of the three
//!   (FR-18).

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

const TEST_DB: &str = "omni_us024_versions";
const TEST_BUCKET: &str = "omni-us024-versions";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const API_ROLE: &str = "omni_api";

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
        // Versions may keep their content in RustFS, so the API gets a real bucket.
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
    format!("us024-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
    Agent {
        key: &'a str,
        run: &'a str,
        role: &'a str,
    },
    Session(&'a str),
}

fn agent(f: &Fixture) -> Auth<'_> {
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

fn version_path(f: &Fixture, project: &str, version: &str) -> String {
    format!("{}/versions/{version}", project_base(f, project))
}

fn diff_path(f: &Fixture, project: &str, from: &str, to: &str) -> String {
    format!("{}/diff/{to}", version_path(f, project, from))
}

/// A new project with an open change set; returns (project ID, change set ID).
fn project_with_changeset(f: &Fixture, client: &Client) -> (String, String) {
    let project = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.workspace),
            &agent(f),
            Some(&json!({ "name": unique("versions-project") })),
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

/// Creates an HTML PRD doc in the change set; returns the whole answer.
fn create_doc(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    must(
        call(
            client,
            "POST",
            &format!("{}/changesets/{changeset}/docs", project_base(f, project)),
            &agent(f),
            Some(&json!({ "kind": "html", "module": "prd", "title": unique("Checkout PRD") })),
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

/// Applies a batch of operations, which must succeed; returns its results.
fn edit(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    ops: &Value,
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
        .expect("results")
        .clone()
}

fn save_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    doc: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/versions", draft_path(f, project, changeset, doc)),
        auth,
        Some(&json!({})),
    )
}

/// Saves the change set's draft of the doc as a version; returns the Version.
fn save(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> Value {
    must(
        save_response(f, client, &agent(f), project, changeset, doc),
        201,
        "save a version",
    )["version"]
        .clone()
}

fn remove_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    doc: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/remove", draft_path(f, project, changeset, doc)),
        auth,
        Some(&json!({})),
    )
}

fn get_version(f: &Fixture, client: &Client, project: &str, version: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &version_path(f, project, version),
            &agent(f),
            None,
        ),
        200,
        "read a version",
    )["version"]
        .clone()
}

fn diff(f: &Fixture, client: &Client, project: &str, from: &str, to: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &diff_path(f, project, from, to),
            &agent(f),
            None,
        ),
        200,
        "compare two versions",
    )["diff"]
        .clone()
}

/// The HTML of a saved version, as `omni doc read --version` reads it.
fn version_html(f: &Fixture, client: &Client, project: &str, doc: &str, version: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/html?version={version}", doc_path(f, project, doc)),
            &agent(f),
            None,
        ),
        200,
        "read the version's HTML",
    )
}

fn draft_html(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> String {
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
    )["draft"]["html"]
        .as_str()
        .expect("the draft has its html")
        .to_string()
}

fn changeset_of(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/changesets/{changeset}", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "show the change set",
    )["changeset"]
        .clone()
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

fn insert(parent: Option<&str>, after: Option<&str>, content: &str) -> Value {
    json!({ "op": "insert", "parent": parent, "position": { "after": after }, "content": content })
}

fn set_html(node: &str, rev: u64, html: &str) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": { "html": html } })
}

fn node_ids(nodes: &Value) -> Vec<String> {
    nodes
        .as_array()
        .unwrap_or_else(|| panic!("a list of nodes: {nodes}"))
        .iter()
        .map(id_of)
        .collect()
}

fn id_set(nodes: &Value) -> BTreeSet<String> {
    node_ids(nodes).into_iter().collect()
}

/// A version's node index by node ID.
fn index_of(version: &Value) -> BTreeMap<String, Value> {
    version["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("a version has its nodes: {version}"))
        .iter()
        .map(|node| (id_of(node), node.clone()))
        .collect()
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
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

/// Puts a change set into `status` directly, as the approval of US-025 and the
/// mainline checks of US-026 will.
fn force_status(changeset: &str, status: &str) {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query("update changesets set status = $2 where id = $1::uuid")
            .bind(changeset)
            .bind(status)
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("put the change set into {status}: {e}"));
    });
}

/// The doc of the comparison tests, saved twice. Version 1 is
///
/// ```text
/// section            (top level)
///   heading "Scope"
///   paragraph "Keep"
///   paragraph "Edit me"
///   paragraph "Drop me"
/// paragraph "Mover"  (top level)
/// ```
///
/// and version 2 changes "Edit me", deletes "Drop me", moves "Mover" into the
/// section after "Keep", and adds a top-level paragraph "Fresh".
struct Story {
    project: String,
    changeset: String,
    doc: String,
    draft: String,
    section: String,
    heading: String,
    keep: String,
    edited: String,
    dropped: String,
    mover: String,
    fresh: String,
    v1: Value,
    v2: Value,
}

fn story(f: &Fixture, client: &Client) -> Story {
    let (project, changeset) = project_with_changeset(f, client);
    let created = create_doc(f, client, &project, &changeset);
    let doc = id_of(&created["doc"]);
    let draft = id_of(&created["draft"]);
    let results = edit(
        f,
        client,
        &project,
        &changeset,
        &doc,
        &json!([insert(
            None,
            None,
            "<section><h2>Scope</h2><p>Keep</p><p>Edit me</p><p>Drop me</p></section>"
        )]),
    );
    let created_ids: Vec<String> = results[0]["created"]
        .as_array()
        .expect("created")
        .iter()
        .map(|id| id.as_str().expect("an ID").to_string())
        .collect();
    let [section, heading, keep, edited, dropped] = <[String; 5]>::try_from(created_ids)
        .unwrap_or_else(|ids| panic!("five new nodes in document order: {ids:?}"));
    let results = edit(
        f,
        client,
        &project,
        &changeset,
        &doc,
        &json!([insert(None, Some(&section), "<p>Mover</p>")]),
    );
    let mover = results[0]["node"].as_str().expect("a node").to_string();
    let v1 = save(f, client, &project, &changeset, &doc);

    let results = edit(
        f,
        client,
        &project,
        &changeset,
        &doc,
        &json!([
            set_html(&edited, 1, "Edited"),
            { "op": "delete", "node": dropped, "rev": 1 },
            { "op": "move", "node": mover, "rev": 1, "parent": section, "position": { "after": keep } },
            insert(None, Some(&section), "<p>Fresh</p>"),
        ]),
    );
    let fresh = results[3]["node"].as_str().expect("a node").to_string();
    let v2 = save(f, client, &project, &changeset, &doc);
    Story {
        project,
        changeset,
        doc,
        draft,
        section,
        heading,
        keep,
        edited,
        dropped,
        mover,
        fresh,
        v1,
        v2,
    }
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_version_routes_are_in_the_registry_with_their_access() {
    let wanted = [
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/versions",
            Access::Agent,
        ),
        (
            "POST",
            "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/remove",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}",
            Access::Both,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/diff/{other_version_id}",
            Access::Both,
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
// omni version save
// ---------------------------------------------------------------------------

#[test]
fn saving_a_draft_records_each_nodes_id_type_parent_and_fingerprint() {
    let (f, client) = ready();
    let s = story(f, client);
    let v1 = &s.v1;
    assert_uuid_v7(&v1["id"]);
    assert_eq!(v1["doc_id"], s.doc.as_str(), "{v1}");
    assert_eq!(v1["changeset_id"], s.changeset.as_str(), "{v1}");
    assert_eq!(v1["draft_id"], s.draft.as_str(), "{v1}");
    assert_eq!(v1["removed"], false, "{v1}");
    assert!(v1["created_at"].is_string(), "{v1}");
    assert_eq!(v1["agent"]["run"], "run-1", "{v1}");
    assert_eq!(v1["agent"]["role"], "writer", "{v1}");

    assert_eq!(
        node_ids(&v1["nodes"]),
        vec![
            s.section.clone(),
            s.heading.clone(),
            s.keep.clone(),
            s.edited.clone(),
            s.dropped.clone(),
            s.mover.clone(),
        ],
        "one node index entry per node, in document order: {v1}"
    );
    let index = index_of(v1);
    let expected = [
        (&s.section, "section", None),
        (&s.heading, "heading", Some(&s.section)),
        (&s.keep, "paragraph", Some(&s.section)),
        (&s.edited, "paragraph", Some(&s.section)),
        (&s.dropped, "paragraph", Some(&s.section)),
        (&s.mover, "paragraph", None),
    ];
    for (id, kind, parent) in expected {
        let node = &index[id];
        assert_eq!(node["type"], kind, "{node}");
        assert_eq!(
            node["parent"],
            parent.map_or(Value::Null, |p| json!(p)),
            "{node}"
        );
        assert!(
            node["fingerprint"].as_str().is_some_and(|f| !f.is_empty()),
            "every node has a content fingerprint: {node}"
        );
    }
    assert_ne!(
        index[&s.keep]["fingerprint"], index[&s.edited]["fingerprint"],
        "paragraphs with different text have different fingerprints"
    );

    // The version reads back as it was saved, for the agent and the human.
    for auth in [agent(f), Auth::Session(&f.alice_token)] {
        let read = must(
            call(
                client,
                "GET",
                &version_path(f, &s.project, &id_of(v1)),
                &auth,
                None,
            ),
            200,
            "read the version",
        );
        assert_eq!(&read["version"], v1, "{read}");
    }
}

#[test]
fn the_fingerprint_follows_the_nodes_own_content() {
    let (f, client) = ready();
    let s = story(f, client);
    let (v1, v2) = (index_of(&s.v1), index_of(&s.v2));
    for same in [&s.section, &s.heading, &s.keep, &s.mover] {
        assert_eq!(
            v1[same]["fingerprint"], v2[same]["fingerprint"],
            "{same}'s own content did not change (a child or its place did)"
        );
    }
    assert_ne!(
        v1[&s.edited]["fingerprint"], v2[&s.edited]["fingerprint"],
        "the edited paragraph's fingerprint changes"
    );
    assert_eq!(v2[&s.mover]["parent"], s.section.as_str(), "{}", s.v2);
    assert!(!v2.contains_key(&s.dropped), "{}", s.v2);
    assert_eq!(v2[&s.fresh]["parent"], Value::Null, "{}", s.v2);
}

#[test]
fn a_saved_version_does_not_change_when_the_draft_does() {
    let (f, client) = ready();
    let s = story(f, client);
    let v1 = id_of(&s.v1);

    // After v1 the draft changed (that made v2), and it changes again now.
    edit(
        f,
        client,
        &s.project,
        &s.changeset,
        &s.doc,
        &json!([set_html(&s.keep, 1, "Kept, but changed after both saves")]),
    );
    assert!(
        draft_html(f, client, &s.project, &s.changeset, &s.doc).contains("after both saves"),
        "the draft stays editable after a save"
    );

    assert_eq!(
        get_version(f, client, &s.project, &v1),
        s.v1,
        "the node index of a saved version never changes"
    );
    let read = version_html(f, client, &s.project, &s.doc, &v1);
    assert_eq!(read["doc_id"], s.doc.as_str(), "{read}");
    assert_eq!(read["version_id"], v1.as_str(), "{read}");
    let html = read["html"].as_str().expect("the version's html");
    for text in ["Scope", "Keep", "Edit me", "Drop me", "Mover"] {
        assert!(html.contains(text), "{text} is in version 1: {html}");
    }
    for text in ["Edited", "Fresh", "after both saves"] {
        assert!(!html.contains(text), "{text} came after version 1: {html}");
    }
    let tags = tagged(html);
    assert_eq!(
        tags.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
        node_ids(&s.v1["nodes"]),
        "every block of the version carries its node ID: {html}"
    );
    assert!(
        tags.iter().all(|(_, rev)| rev.is_none()),
        "a version has no revisions: {html}"
    );
}

#[test]
fn saving_a_version_leaves_the_mainline_alone() {
    let (f, client) = ready();
    let s = story(f, client);
    let doc = must(
        call(
            client,
            "GET",
            &doc_path(f, &s.project, &s.doc),
            &agent(f),
            None,
        ),
        200,
        "show the doc",
    )["doc"]
        .clone();
    assert_eq!(
        doc["mainline_version_id"],
        Value::Null,
        "a version becomes mainline only when its change set merges: {doc}"
    );
    assert_ne!(s.v1["id"], s.v2["id"], "every save is a new version");
}

#[test]
fn a_version_is_saved_only_from_a_draft_that_the_change_set_has() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let other = new_changeset(f, client, &project);
    let response = save_response(f, client, &agent(f), &project, &other, &doc);
    assert_error(&response, 404, "not_found");
    let response = save_response(f, client, &agent(f), &project, &changeset, UNKNOWN_ID);
    assert_error(&response, 404, "not_found");
    let response = call(
        client,
        "GET",
        &version_path(f, &project, UNKNOWN_ID),
        &agent(f),
        None,
    );
    assert_error(&response, 404, "not_found");
    // The control: the change set that has the draft saves it.
    save(f, client, &project, &changeset, &doc);
    let saved: i64 = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from versions where doc_id = $1::uuid")
            .bind(&doc)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the doc's versions: {e}"))
    });
    assert_eq!(saved, 1, "only the control saved a version");
}

#[test]
fn a_version_of_another_project_is_not_found() {
    let (f, client) = ready();
    let s = story(f, client);
    let (elsewhere, _) = project_with_changeset(f, client);
    let response = call(
        client,
        "GET",
        &version_path(f, &elsewhere, &id_of(&s.v1)),
        &agent(f),
        None,
    );
    assert_error(&response, 404, "not_found");
}

#[test]
fn the_human_never_saves_a_version_or_removes_a_doc() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let human = Auth::Session(&f.alice_token);
    let response = save_response(f, client, &human, &project, &changeset, &doc);
    assert_error(&response, 403, "forbidden");
    let response = remove_response(f, client, &human, &project, &changeset, &doc);
    assert_error(&response, 403, "forbidden");
}

#[test]
fn saving_a_version_emits_version_saved() {
    let (f, client) = ready();
    let s = story(f, client);
    let events = events_of(&s.project, "version.saved");
    assert_eq!(events.len(), 2, "one event per saved version: {events:?}");
    for (event, version) in events.iter().zip([&s.v1, &s.v2]) {
        assert_eq!(event["version_id"], version["id"], "{event}");
        assert_eq!(event["doc_id"], s.doc.as_str(), "{event}");
        assert_eq!(event["changeset_id"], s.changeset.as_str(), "{event}");
        assert_eq!(event["draft_id"], s.draft.as_str(), "{event}");
        assert_eq!(event["removed"], false, "{event}");
        assert_eq!(event["agent"]["run"], "run-1", "{event}");
        assert_eq!(event["agent"]["role"], "writer", "{event}");
    }
}

// ---------------------------------------------------------------------------
// Append-only storage
// ---------------------------------------------------------------------------

#[test]
fn the_api_role_has_no_update_privilege_on_versions_the_log_or_the_node_index() {
    let (f, client) = ready();
    let s = story(f, client);
    block(async {
        let mut conn = admin_connection().await;
        for table in ["versions", "draft_ops", "node_index"] {
            let exists: bool = sqlx::query_scalar("select to_regclass($1) is not null")
                .bind(format!("public.{table}"))
                .fetch_one(&mut conn)
                .await
                .expect("look for the table");
            assert!(exists, "the table {table} exists");
            for (privilege, expected) in [("SELECT", true), ("INSERT", true), ("UPDATE", false)] {
                let granted: bool = sqlx::query_scalar("select has_table_privilege($1, $2, $3)")
                    .bind(API_ROLE)
                    .bind(format!("public.{table}"))
                    .bind(privilege)
                    .fetch_one(&mut conn)
                    .await
                    .expect("look up the privilege");
                assert_eq!(
                    granted, expected,
                    "{API_ROLE} {privilege} on {table}: expected {expected}"
                );
            }
        }

        // As the API role, a saved version cannot be changed.
        let mut tx = conn.begin().await.expect("begin");
        sqlx::query(&format!("set local role {API_ROLE}"))
            .execute(&mut *tx)
            .await
            .expect("act as the API role");
        sqlx::query("select set_config('app.workspace_id', $1, true)")
            .bind(&f.workspace)
            .execute(&mut *tx)
            .await
            .expect("set app.workspace_id");
        let error = sqlx::query("update versions set doc_id = doc_id where id = $1::uuid")
            .bind(id_of(&s.v1))
            .execute(&mut *tx)
            .await
            .expect_err("the API role changed a saved version");
        let code = error
            .as_database_error()
            .and_then(|e| e.code().map(|c| c.into_owned()))
            .unwrap_or_default();
        assert_eq!(code, "42501", "{error}");
    });
}

#[test]
fn versions_and_the_node_index_are_tenant_tables_of_the_workspace() {
    let (f, client) = ready();
    let s = story(f, client);
    let (versions, nodes): (Vec<String>, i64) = block(async {
        let mut conn = admin_connection().await;
        let versions = sqlx::query_scalar::<_, String>(
            "select workspace_id::text from versions where doc_id = $1::uuid",
        )
        .bind(&s.doc)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the versions: {e}"));
        let nodes = sqlx::query_scalar::<_, i64>(
            "select count(*) from node_index where workspace_id = $1::uuid",
        )
        .bind(&f.workspace)
        .fetch_one(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the node index: {e}"));
        (versions, nodes)
    });
    assert_eq!(versions.len(), 2, "the two saved versions: {versions:?}");
    for owner in &versions {
        assert_eq!(owner, &f.workspace);
    }
    assert!(nodes > 0, "the node index has rows of the workspace");
}

// ---------------------------------------------------------------------------
// omni version diff
// ---------------------------------------------------------------------------

#[test]
fn a_diff_lists_added_removed_changed_and_moved_nodes() {
    let (f, client) = ready();
    let s = story(f, client);
    let (v1, v2) = (id_of(&s.v1), id_of(&s.v2));
    for auth in [agent(f), Auth::Session(&f.alice_token)] {
        must(
            call(
                client,
                "GET",
                &diff_path(f, &s.project, &v1, &v2),
                &auth,
                None,
            ),
            200,
            "compare as the agent and as the human",
        );
    }
    let d = diff(f, client, &s.project, &v1, &v2);
    assert_eq!(d["doc_id"], s.doc.as_str(), "{d}");
    assert_eq!(d["from"], v1.as_str(), "{d}");
    assert_eq!(d["to"], v2.as_str(), "{d}");

    assert_eq!(
        id_set(&d["added"]),
        BTreeSet::from([s.fresh.clone()]),
        "{d}"
    );
    assert_eq!(d["added"][0]["type"], "paragraph", "{d}");
    assert_eq!(d["added"][0]["parent"], Value::Null, "{d}");

    assert_eq!(
        id_set(&d["removed"]),
        BTreeSet::from([s.dropped.clone()]),
        "{d}"
    );
    assert_eq!(d["removed"][0]["type"], "paragraph", "{d}");
    assert_eq!(d["removed"][0]["parent"], s.section.as_str(), "{d}");

    assert_eq!(
        id_set(&d["changed"]),
        BTreeSet::from([s.edited.clone()]),
        "only the paragraph whose own content changed: {d}"
    );
    assert_eq!(
        d["changed"][0]["fingerprint"],
        index_of(&s.v2)[&s.edited]["fingerprint"],
        "a changed node is listed as it is in the second version: {d}"
    );

    assert_eq!(
        id_set(&d["moved"]),
        BTreeSet::from([s.mover.clone()]),
        "{d}"
    );
    let moved = &d["moved"][0];
    assert_eq!(moved["type"], "paragraph", "{moved}");
    assert_eq!(moved["from_parent"], Value::Null, "{moved}");
    assert_eq!(moved["to_parent"], s.section.as_str(), "{moved}");
    assert_eq!(moved["from_after"], s.section.as_str(), "{moved}");
    assert_eq!(moved["to_after"], s.keep.as_str(), "{moved}");
}

#[test]
fn a_diff_the_other_way_round_swaps_added_and_removed() {
    let (f, client) = ready();
    let s = story(f, client);
    let d = diff(f, client, &s.project, &id_of(&s.v2), &id_of(&s.v1));
    assert_eq!(
        id_set(&d["added"]),
        BTreeSet::from([s.dropped.clone()]),
        "{d}"
    );
    assert_eq!(
        id_set(&d["removed"]),
        BTreeSet::from([s.fresh.clone()]),
        "{d}"
    );
    assert_eq!(
        id_set(&d["changed"]),
        BTreeSet::from([s.edited.clone()]),
        "{d}"
    );
    assert_eq!(
        id_set(&d["moved"]),
        BTreeSet::from([s.mover.clone()]),
        "{d}"
    );
    assert_eq!(d["moved"][0]["from_parent"], s.section.as_str(), "{d}");
    assert_eq!(d["moved"][0]["to_parent"], Value::Null, "{d}");
    assert_eq!(d["moved"][0]["from_after"], s.keep.as_str(), "{d}");
    assert_eq!(d["moved"][0]["to_after"], s.section.as_str(), "{d}");
}

#[test]
fn a_diff_lists_a_node_moved_after_another_sibling_of_the_same_parent() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let results = edit(
        f,
        client,
        &project,
        &changeset,
        &doc,
        &json!([insert(
            None,
            None,
            "<section><p>A</p><p>B</p><p>C</p><p>D</p></section>"
        )]),
    );
    let created_ids: Vec<String> = results[0]["created"]
        .as_array()
        .expect("created")
        .iter()
        .map(|id| id.as_str().expect("an ID").to_string())
        .collect();
    let [section, a, b, c, d] = <[String; 5]>::try_from(created_ids)
        .unwrap_or_else(|ids| panic!("five new nodes in document order: {ids:?}"));
    let v1 = save(f, client, &project, &changeset, &doc);

    // A moves after C inside the same section: A B C D becomes B C A D.
    edit(
        f,
        client,
        &project,
        &changeset,
        &doc,
        &json!([{ "op": "move", "node": a, "rev": 1, "parent": section, "position": { "after": c } }]),
    );
    let v2 = save(f, client, &project, &changeset, &doc);
    assert_eq!(
        node_ids(&v2["nodes"]),
        vec![section.clone(), b.clone(), c.clone(), a.clone(), d.clone()],
        "the second version has the new order: {v2}"
    );
    assert_eq!(index_of(&v2)[&a]["parent"], section.as_str(), "{v2}");

    let diff_ab = diff(f, client, &project, &id_of(&v1), &id_of(&v2));
    for list in ["added", "removed", "changed"] {
        assert_eq!(diff_ab[list], json!([]), "{list}: {diff_ab}");
    }
    assert_eq!(
        node_ids(&diff_ab["moved"]),
        vec![a.clone()],
        "a reorder inside one parent is a move of just the node that moved, \
         not of the siblings it passed: {diff_ab}"
    );
    let moved = &diff_ab["moved"][0];
    assert_eq!(moved["type"], "paragraph", "{moved}");
    assert_eq!(moved["from_parent"], section.as_str(), "{moved}");
    assert_eq!(moved["to_parent"], section.as_str(), "{moved}");
    assert_eq!(moved["from_after"], Value::Null, "{moved}");
    assert_eq!(moved["to_after"], c.as_str(), "{moved}");

    // The other way round, A moves back to the front.
    let diff_ba = diff(f, client, &project, &id_of(&v2), &id_of(&v1));
    assert_eq!(node_ids(&diff_ba["moved"]), vec![a.clone()], "{diff_ba}");
    assert_eq!(diff_ba["moved"][0]["from_after"], c.as_str(), "{diff_ba}");
    assert_eq!(diff_ba["moved"][0]["to_after"], Value::Null, "{diff_ba}");
}

#[test]
fn a_version_compared_with_itself_has_no_differences() {
    let (f, client) = ready();
    let s = story(f, client);
    let v2 = id_of(&s.v2);
    let d = diff(f, client, &s.project, &v2, &v2);
    for list in ["added", "removed", "changed", "moved"] {
        assert_eq!(d[list], json!([]), "{list}: {d}");
    }
}

#[test]
fn versions_of_two_different_docs_cannot_be_compared() {
    let (f, client) = ready();
    let s = story(f, client);
    let other = id_of(&create_doc(f, client, &s.project, &s.changeset)["doc"]);
    edit(
        f,
        client,
        &s.project,
        &s.changeset,
        &other,
        &json!([insert(None, None, "<p>Another doc</p>")]),
    );
    let other_version = save(f, client, &s.project, &s.changeset, &other);
    let response = call(
        client,
        "GET",
        &diff_path(f, &s.project, &id_of(&s.v1), &id_of(&other_version)),
        &agent(f),
        None,
    );
    assert_error(&response, 400, "invalid_request");
    let response = call(
        client,
        "GET",
        &diff_path(f, &s.project, &id_of(&s.v1), UNKNOWN_ID),
        &agent(f),
        None,
    );
    assert_error(&response, 404, "not_found");
}

// ---------------------------------------------------------------------------
// Removing a doc
// ---------------------------------------------------------------------------

#[test]
fn removing_a_doc_saves_a_version_marking_it_removed_and_the_history_stays() {
    let (f, client) = ready();
    let s = story(f, client);
    let removal = must(
        remove_response(f, client, &agent(f), &s.project, &s.changeset, &s.doc),
        201,
        "remove the doc",
    )["version"]
        .clone();
    assert_uuid_v7(&removal["id"]);
    assert_eq!(removal["removed"], true, "{removal}");
    assert_eq!(removal["doc_id"], s.doc.as_str(), "{removal}");
    assert_eq!(removal["changeset_id"], s.changeset.as_str(), "{removal}");
    assert_eq!(removal["nodes"], json!([]), "{removal}");
    assert_ne!(removal["id"], s.v1["id"]);
    assert_ne!(removal["id"], s.v2["id"]);
    assert_eq!(
        get_version(f, client, &s.project, &id_of(&removal)),
        removal,
        "the removal is a version like any other"
    );

    // The history stays: the doc and its earlier versions are all still there.
    must(
        call(
            client,
            "GET",
            &doc_path(f, &s.project, &s.doc),
            &agent(f),
            None,
        ),
        200,
        "the removed doc can still be shown",
    );
    assert_eq!(get_version(f, client, &s.project, &id_of(&s.v1)), s.v1);
    assert_eq!(get_version(f, client, &s.project, &id_of(&s.v2)), s.v2);
    let read = version_html(f, client, &s.project, &s.doc, &id_of(&s.v1));
    assert!(
        read["html"].as_str().is_some_and(|h| h.contains("Drop me")),
        "{read}"
    );
    let (docs, versions): (i64, i64) = block(async {
        let mut conn = admin_connection().await;
        let docs = sqlx::query_scalar::<_, i64>("select count(*) from docs where id = $1::uuid")
            .bind(&s.doc)
            .fetch_one(&mut conn)
            .await
            .expect("count the doc");
        let versions =
            sqlx::query_scalar::<_, i64>("select count(*) from versions where doc_id = $1::uuid")
                .bind(&s.doc)
                .fetch_one(&mut conn)
                .await
                .expect("count the versions");
        (docs, versions)
    });
    assert_eq!(docs, 1, "the doc is never deleted");
    assert_eq!(versions, 3, "two saved versions and the removal");

    // Compared with the removal, every node of the last version is removed.
    let d = diff(f, client, &s.project, &id_of(&s.v2), &id_of(&removal));
    assert_eq!(id_set(&d["removed"]), id_set(&s.v2["nodes"]), "{d}");
    for list in ["added", "changed", "moved"] {
        assert_eq!(d[list], json!([]), "{list}: {d}");
    }
}

#[test]
fn removing_a_doc_emits_version_saved_marked_removed() {
    let (f, client) = ready();
    let s = story(f, client);
    let removal = must(
        remove_response(f, client, &agent(f), &s.project, &s.changeset, &s.doc),
        201,
        "remove the doc",
    )["version"]
        .clone();
    let events = events_of(&s.project, "version.saved");
    let event = events
        .iter()
        .find(|e| e["version_id"] == removal["id"])
        .unwrap_or_else(|| panic!("the removal emits version.saved: {events:?}"));
    assert_eq!(event["removed"], true, "{event}");
    assert_eq!(event["doc_id"], s.doc.as_str(), "{event}");
    assert_eq!(event["changeset_id"], s.changeset.as_str(), "{event}");
}

#[test]
fn a_doc_is_removed_only_in_a_change_set_that_has_its_draft() {
    let (f, client) = ready();
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    let other = new_changeset(f, client, &project);
    // The doc has no mainline version, so no other change set can open it.
    let response = remove_response(f, client, &agent(f), &project, &other, &doc);
    assert_error(&response, 404, "not_found");
    let response = remove_response(f, client, &agent(f), &project, &changeset, UNKNOWN_ID);
    assert_error(&response, 404, "not_found");
    // The control: the change set that has the draft removes the doc.
    let removal = must(
        remove_response(f, client, &agent(f), &project, &changeset, &doc),
        201,
        "remove the doc in its own change set",
    );
    assert_eq!(removal["version"]["removed"], true, "{removal}");
}

// ---------------------------------------------------------------------------
// Saving clears an approval
// ---------------------------------------------------------------------------

/// A change set in `status` whose draft has one paragraph, saved once.
fn saved_once_in(f: &Fixture, client: &Client, status: &str) -> (String, String, String) {
    let (project, changeset) = project_with_changeset(f, client);
    let doc = id_of(&create_doc(f, client, &project, &changeset)["doc"]);
    edit(
        f,
        client,
        &project,
        &changeset,
        &doc,
        &json!([insert(None, None, "<p>Approved text</p>")]),
    );
    save(f, client, &project, &changeset, &doc);
    if status != "open" {
        force_status(&changeset, status);
    }
    (project, changeset, doc)
}

fn assert_back_in_review(f: &Fixture, client: &Client, project: &str, changeset: &str, from: &str) {
    let shown = changeset_of(f, client, project, changeset);
    assert_eq!(
        shown["status"], "in_review",
        "saving a version cleared the approval: {shown}"
    );
    let history = shown["history"].as_array().expect("history");
    let last = history.last().expect("an entry");
    assert_eq!(last["from_status"], from, "{shown}");
    assert_eq!(last["to_status"], "in_review", "{shown}");
    assert_eq!(last["actor"], "system", "{shown}");

    let events = events_of(project, "changeset.in_review");
    let event = events
        .iter()
        .find(|e| e["changeset_id"] == changeset && e["from_status"] == from)
        .unwrap_or_else(|| panic!("the move back emits changeset.in_review: {events:?}"));
    assert_eq!(event["status"], "in_review", "{event}");
    assert_eq!(event["actor"], "system", "{event}");
}

#[test]
fn saving_a_version_of_an_approved_change_set_moves_it_back_to_in_review() {
    let (f, client) = ready();
    let (project, changeset, doc) = saved_once_in(f, client, "approved");
    save(f, client, &project, &changeset, &doc);
    assert_back_in_review(f, client, &project, &changeset, "approved");
}

#[test]
fn saving_a_version_of_a_change_set_that_needs_a_rebase_moves_it_back_to_in_review() {
    let (f, client) = ready();
    let (project, changeset, doc) = saved_once_in(f, client, "needs_rebase");
    save(f, client, &project, &changeset, &doc);
    assert_back_in_review(f, client, &project, &changeset, "needs_rebase");
}

#[test]
fn removing_a_doc_of_an_approved_change_set_moves_it_back_to_in_review() {
    let (f, client) = ready();
    let (project, changeset, doc) = saved_once_in(f, client, "approved");
    must(
        remove_response(f, client, &agent(f), &project, &changeset, &doc),
        201,
        "remove the doc",
    );
    assert_back_in_review(f, client, &project, &changeset, "approved");
}

#[test]
fn saving_a_version_of_an_open_or_in_review_change_set_keeps_its_status() {
    let (f, client) = ready();
    let (project, changeset, doc) = saved_once_in(f, client, "open");
    let before = changeset_of(f, client, &project, &changeset);
    assert_eq!(before["status"], "open", "{before}");
    save(f, client, &project, &changeset, &doc);
    let after = changeset_of(f, client, &project, &changeset);
    assert_eq!(after["status"], "open", "{after}");
    assert_eq!(after["history"], before["history"], "no status change");

    must(
        call(
            client,
            "POST",
            &format!(
                "{}/changesets/{changeset}/submit",
                project_base(f, &project)
            ),
            &agent(f),
            None,
        ),
        200,
        "submit",
    );
    let before = changeset_of(f, client, &project, &changeset);
    assert_eq!(before["status"], "in_review", "{before}");
    save(f, client, &project, &changeset, &doc);
    let after = changeset_of(f, client, &project, &changeset);
    assert_eq!(after["status"], "in_review", "{after}");
    assert_eq!(after["history"], before["history"], "no status change");
}
