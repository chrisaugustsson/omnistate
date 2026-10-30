//! US-104: an agent writes designs, tokens and components directly. Run with `--features stack-tests` in the Compose test container,
//! which reaches Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API (the agent),
//! create a scratch RustFS bucket and serve the API's router on a local port.
//! Docs are created with the live routes of US-102; the test of the chunks a
//! write stores seeds a workspace of its own, so that no other test writes
//! chunks under its prefix while it counts them.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; the operations are those
//! of tests/design_edits.rs and docs/formats/operations.md, the tags those of
//! docs/formats/design-tags.md and the tokens those of docs/formats/tokens.md):
//!
//! - `POST /docs/{doc_id}/ops` (US-103) applies a batch to a design doc's head
//!   as it does to an HTML doc's: all or nothing, 200 `{"results": [...]}`
//!   (`node`, `revision`, `created`, `removed`), and the next read of the
//!   design (`GET /docs/{doc_id}/html`, `GET /docs/{doc_id}/boards/{board}`)
//!   has the change. The body may carry `"board"`, which keeps the batch inside
//!   that board: an operation outside it is 400
//!   `invalid_operation` naming the node, and an ID that is no board of the
//!   head is 404 `unknown_board`.
//! - A design's head is a manifest chunk (`{"pages": [{"id", "chunk",
//!   "boards": [{"id", "chunk"}]}]}`, src/boards.rs) stored in RustFS at
//!   `ws/{workspace_id}/chunks/{head}` with every page and board in a chunk of
//!   its own, each named by the BLAKE3 hash of its bytes. A write stores only
//!   the chunks of the boards it changes and a new manifest: editing one board
//!   of a 100-board design adds exactly two objects to the workspace's chunks,
//!   and the other 99 boards keep their chunk in the new manifest.
//! - An operation whose `rev` is not the node's revision is 409 `{"error":
//!   "conflict", "conflict": {"index", "node", "expected",
//!   "current_revision"}}`, and nothing changes. Revisions are durable: an API
//!   started afresh checks against the revisions that were written.
//! - Every applied batch adds one `doc_changes` row (the agent, the operations
//!   as sent, `head_before`, `head_after`) and emits `doc.changed` `{"doc_id",
//!   "head"}`, as for HTML docs. The row's `boards` column lists each board
//!   whose chunk the batch changed as `{"board", "before", "after"}`: the
//!   board's node ID and its chunk in the manifest before and after the batch,
//!   `before` being `null` for a new board and `after` `null` for a removed
//!   one. A board the batch did not change is not listed.
//! - `PUT /tokens` (agent, `{"tokens": <tree>}`) replaces the token tree of the
//!   project's design system head, and `PUT /components` (agent, `{"tags":
//!   "<Component ...>..."}`) replaces its components, all or nothing (a tag with an `id` keeps that component's ID; the answer lists
//!   the components as JSON nodes under `components`). Each write makes a new
//!   head of the design system doc and adds one `doc_changes` row. `GET
//!   /tokens` reads the tree of the design system's head.
//! - Designs resolve instances and tokens against the design system's head. An
//!   `<Instance>` of a component that the head has is accepted, one of a
//!   component it does not have is 400 `invalid_operation` naming the ID. A
//!   design's head read (`GET /docs/{doc_id}/html`) answers `tokens` (the
//!   design system head's whole token tree) and `components` (the JSON of the
//!   components the design's instances use, as the head defines them), so a
//!   token or component change shows up the next time any design that uses it
//!   is read, without the design's own head changing. `GET
//!   /components/usage?component=<id>` lists the boards of the designs' heads
//!   that use it.
//! - Both new routes are in the route registry as `agent`.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us104_live_designs";
const TEST_BUCKET: &str = "omni-us104-live-designs";
/// A node ID in the right format that no node has.
const UNKNOWN_NODE: &str = "zzzzzzzz";

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

/// A workspace with one member, a live session of theirs and an API key.
struct Ws {
    workspace: String,
    token: String,
    key: String,
}

struct Fixture {
    ws: Ws,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us104-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
}

/// Seeds a workspace whose one member is `name`, with a session, and mints an
/// API key for it through the API.
fn seed_workspace(client: &Client, name: &str) -> Ws {
    let token = session_token(name);
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
            "insert into workspaces (name) values ($1) returning id::text",
            &[name],
        )
        .await;
        let user = text(
            "insert into users (email) values ($1) returning id::text",
            &[&format!("{name}@example.test")],
        )
        .await;
        text(
            "insert into sessions (user_id, token_hash, expires_at) \
             values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
             returning id::text",
            &[&user, &token],
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
    let response = call(
        client,
        "POST",
        &format!("/api/v1/w/{workspace}/api-keys"),
        &Auth::Session(&token),
        Some(&json!({})),
    );
    assert_eq!(response.status, 201, "mint a key: {}", response.text());
    let key = response.json()["secret"]
        .as_str()
        .expect("the new key has a secret")
        .to_string();
    Ws {
        workspace,
        token,
        key,
    }
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
        let server = TestServer::start(&config());
        let ws = seed_workspace(&server.client, "alice");
        Fixture { ws, server }
    })
}

fn ready() -> (&'static Ws, &'static Client) {
    let fixture = fixture();
    (&fixture.ws, &fixture.server.client)
}

/// Who sends a request: an agent (an API key, with the run it declares) or
/// the human (a session cookie).
#[derive(Clone, Copy)]
enum Auth<'a> {
    Agent(&'a str, &'a str),
    Session(&'a str),
}

fn agent(ws: &Ws) -> Auth<'_> {
    Auth::Agent(&ws.key, "run-104")
}

fn human(ws: &Ws) -> Auth<'_> {
    Auth::Session(&ws.token)
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let headers: Vec<(&str, String)> = match auth {
        Auth::Agent(key, run) => vec![
            ("Authorization", format!("Bearer {key}")),
            ("X-Omni-Run", run.to_string()),
            ("X-Omni-Role", "designer".to_string()),
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

/// The name of a chunk: 64 lowercase hexadecimal characters.
fn is_chunk_name(name: &str) -> bool {
    name.len() == 64 && name.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn str_of<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("there is a `{field}`: {value}"))
}

fn project_base(ws: &Ws, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", ws.workspace)
}

fn doc_path(ws: &Ws, project: &str, doc: &str) -> String {
    format!("{}/docs/{doc}", project_base(ws, project))
}

fn new_project(ws: &Ws, client: &Client) -> String {
    str_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", ws.workspace),
                &agent(ws),
                Some(&json!({ "name": unique("live-designs-project") })),
            ),
            201,
            "create a project",
        )["project"],
        "id",
    )
    .to_string()
}

fn new_feature(ws: &Ws, client: &Client, project: &str) -> String {
    str_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/features", project_base(ws, project)),
                &agent(ws),
                Some(&json!({ "name": unique("Checkout") })),
            ),
            201,
            "create a feature",
        )["feature"],
        "id",
    )
    .to_string()
}

/// Creates a doc of `kind` in the UI pillar (US-102), at
/// the app level or in `feature`; returns its ID.
fn new_ui_doc(
    ws: &Ws,
    client: &Client,
    project: &str,
    feature: Option<&str>,
    kind: &str,
) -> String {
    let path = match feature {
        None => format!("{}/docs", project_base(ws, project)),
        Some(feature) => format!("{}/features/{feature}/docs", project_base(ws, project)),
    };
    str_of(
        &must(
            call(
                client,
                "POST",
                &path,
                &agent(ws),
                Some(&json!({ "kind": kind, "pillar": "ui", "title": unique(kind) })),
            ),
            201,
            "create a doc",
        )["doc"],
        "id",
    )
    .to_string()
}

fn edit_response(ws: &Ws, client: &Client, project: &str, doc: &str, body: &Value) -> Response {
    call(
        client,
        "POST",
        &format!("{}/ops", doc_path(ws, project, doc)),
        &agent(ws),
        Some(body),
    )
}

/// Applies a batch to the design's head as the agent; returns the results.
fn edit(ws: &Ws, client: &Client, project: &str, doc: &str, ops: &Value) -> Vec<Value> {
    let answer = must(
        edit_response(ws, client, project, doc, &json!({ "ops": ops })),
        200,
        "apply a batch to the design",
    );
    answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer lists results: {answer}"))
        .clone()
}

/// The head read of a doc: `GET /docs/{doc_id}/html`.
fn read_head_on(client: &Client, ws: &Ws, auth: &Auth, project: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/html", doc_path(ws, project, doc)),
            auth,
            None,
        ),
        200,
        "read the doc's head",
    )
}

fn read_head(ws: &Ws, client: &Client, project: &str, doc: &str) -> Value {
    read_head_on(client, ws, &agent(ws), project, doc)
}

/// The ID of the design's first page, as its head read shows it.
fn first_page(ws: &Ws, client: &Client, project: &str, doc: &str) -> String {
    let read = read_head(ws, client, project, doc);
    read["pages"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a new design has a page: {read}"))
        .to_string()
}

/// One board of the design's head in the tag format.
fn board_tags(ws: &Ws, client: &Client, project: &str, doc: &str, board: &str) -> String {
    let read = must(
        call(
            client,
            "GET",
            &format!("{}/boards/{board}", doc_path(ws, project, doc)),
            &agent(ws),
            None,
        ),
        200,
        "read a board of the head",
    );
    str_of(&read, "tags").to_string()
}

fn board_tag(name: &str, x: u32, text: &str) -> String {
    format!(
        "<Board name=\"{name}\" x={{{x}}} y={{0}} width={{200}} height={{100}} dir=\"col\">\n  <Text name=\"{name} label\" content=\"{text}\" fontFamily=\"Inter\" fontSize={{14}}/>\n</Board>"
    )
}

fn insert(parent: &str, content: &str) -> Value {
    json!({ "op": "insert", "parent": parent, "position": { "after": null }, "content": content })
}

fn set_content(node: &str, rev: u64, content: &str) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": { "content": content } })
}

fn delete(node: &str, rev: u64) -> Value {
    json!({ "op": "delete", "node": node, "rev": rev })
}

/// A board inserted by `edit`: its node and its text's node.
struct Board {
    id: String,
    text: String,
}

fn boards_of(results: &[Value]) -> Vec<Board> {
    results
        .iter()
        .map(|result| {
            let created = result["created"]
                .as_array()
                .unwrap_or_else(|| panic!("an insert lists what it created: {result}"));
            assert_eq!(created.len(), 2, "a board and its text: {result}");
            Board {
                id: created[0].as_str().expect("an ID").to_string(),
                text: created[1].as_str().expect("an ID").to_string(),
            }
        })
        .collect()
}

/// A design with `count` boards on its first page, each with one text, made
/// in one batch; returns the design and its boards in the order inserted.
fn design_with_boards(ws: &Ws, client: &Client, project: &str, count: u32) -> (String, Vec<Board>) {
    let doc = new_ui_doc(ws, client, project, None, "design");
    let page = first_page(ws, client, project, &doc);
    let ops: Vec<Value> = (0..count)
        .map(|i| {
            insert(
                &page,
                &board_tag(&format!("Board {i}"), i * 240, &format!("Text {i}")),
            )
        })
        .collect();
    let results = edit(ws, client, project, &doc, &Value::Array(ops));
    assert_eq!(results.len(), count as usize, "{results:?}");
    (doc, boards_of(&results))
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
    /// The `boards` column; `Value::Null` when the row has none (or the table
    /// has no such column).
    boards: Value,
}

type ChangeRow = (
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn changes_of(doc: &str) -> Vec<Change> {
    block(async {
        let mut conn = admin_connection().await;
        // `to_jsonb(dc) -> 'boards'` is null, not an error, while the table
        // has no `boards` column.
        let rows: Vec<ChangeRow> = sqlx::query_as(
            "SELECT seq::bigint, agent::text, operations::text, head_before::text, \
                 head_after::text, (to_jsonb(dc) -> 'boards')::text \
                 FROM doc_changes dc WHERE doc_id = $1::uuid ORDER BY seq",
        )
        .bind(doc)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the doc changes of {doc}: {e}"));
        rows.into_iter()
            .map(
                |(seq, agent, operations, head_before, head_after, boards)| Change {
                    seq,
                    agent: serde_json::from_str(&agent).expect("the agent is JSON"),
                    operations: serde_json::from_str(&operations).expect("the operations are JSON"),
                    head_before,
                    head_after,
                    boards: boards
                        .map(|text| serde_json::from_str(&text).expect("the boards are JSON"))
                        .unwrap_or(Value::Null),
                },
            )
            .collect()
    })
}

/// The `data` of every `doc.changed` event of the project that names `doc`.
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

/// Every key in the bucket under `prefix`.
fn keys_under(prefix: &str) -> BTreeSet<String> {
    let pages = block(bucket().list(prefix.to_string(), None))
        .unwrap_or_else(|e| panic!("cannot list {prefix}: {e}"));
    pages
        .into_iter()
        .flat_map(|page| page.contents)
        .map(|object| object.key)
        .collect()
}

fn chunk_key(ws: &Ws, chunk: &str) -> String {
    format!("ws/{}/chunks/{chunk}", ws.workspace)
}

/// The design's head chunk, read from RustFS and checked to be named by the
/// BLAKE3 hash of its bytes, as a manifest.
fn manifest_of(ws: &Ws, doc: &str) -> Value {
    let head = head_of(doc);
    assert!(is_chunk_name(&head), "the head names a chunk: {head:?}");
    let bytes = stored(&chunk_key(ws, &head));
    assert_eq!(
        blake3::hash(&bytes).to_hex().to_string(),
        head,
        "the head of {doc} is named by the BLAKE3 hash of its bytes"
    );
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("the head of a design is a JSON manifest ({e})"))
}

/// The boards of the manifest's first page, in order, as (board, chunk).
fn manifest_boards(manifest: &Value) -> Vec<(String, String)> {
    manifest["pages"][0]["boards"]
        .as_array()
        .unwrap_or_else(|| panic!("the manifest lists the page's boards: {manifest}"))
        .iter()
        .map(|entry| {
            (
                str_of(entry, "id").to_string(),
                str_of(entry, "chunk").to_string(),
            )
        })
        .collect()
}

/// The chunk of every board of the manifest, by board.
fn chunk_by_board(manifest: &Value) -> HashMap<String, String> {
    manifest_boards(manifest).into_iter().collect()
}

/// The `boards` of a doc change, sorted by board, as (board, before, after).
fn changed_boards(change: &Change) -> Vec<(String, Option<String>, Option<String>)> {
    let entries = change
        .boards
        .as_array()
        .unwrap_or_else(|| panic!("the change lists its boards: {change:?}"));
    let mut boards: Vec<_> = entries
        .iter()
        .map(|entry| {
            (
                str_of(entry, "board").to_string(),
                entry["before"].as_str().map(str::to_string),
                entry["after"].as_str().map(str::to_string),
            )
        })
        .collect();
    boards.sort();
    boards
}

// ---------------------------------------------------------------------------
// The design system
// ---------------------------------------------------------------------------

fn token_tree(brand: &str) -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "$type": "color",
            "brand": {
                "$value": brand,
                "$extensions": {"omnistate": {"modes": {"light": brand, "dark": "#0f172a"}}}
            }
        }
    })
}

fn put_tokens(ws: &Ws, client: &Client, project: &str, tree: &Value) -> Value {
    must(
        call(
            client,
            "PUT",
            &format!("{}/tokens", project_base(ws, project)),
            &agent(ws),
            Some(&json!({ "tokens": tree })),
        ),
        200,
        "import the tokens into the design system's head",
    )
}

/// A Button component: its `id` is kept when given, and its label says
/// `label`.
fn button_tags(id: Option<&str>, label: &str) -> String {
    let id = id.map(|id| format!(" id=\"{id}\"")).unwrap_or_default();
    format!(
        "<Component{id} name=\"Button\" dir=\"row\" pad={{8}} fill=\"$color.brand\">\n  <Text name=\"Button label\" content=\"{label}\" fontFamily=\"Inter\" fontSize={{14}}/>\n</Component>"
    )
}

fn put_components(ws: &Ws, client: &Client, project: &str, tags: &str) -> Value {
    must(
        call(
            client,
            "PUT",
            &format!("{}/components", project_base(ws, project)),
            &agent(ws),
            Some(&json!({ "tags": tags })),
        ),
        200,
        "import the components into the design system's head",
    )
}

/// A project with a live design system that holds `token_tree(brand)` and a
/// Button labelled `Buy`; returns (design system, the Button's ID).
fn design_system(ws: &Ws, client: &Client, project: &str, brand: &str) -> (String, String) {
    let doc = new_ui_doc(ws, client, project, None, "design-system");
    put_tokens(ws, client, project, &token_tree(brand));
    let imported = put_components(ws, client, project, &button_tags(None, "Buy"));
    let button = imported["components"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the import lists the Button with its ID: {imported}"))
        .to_string();
    (doc, button)
}

/// A board that places the Button and fills a frame with the brand token.
fn board_using(button: &str, name: &str) -> String {
    format!(
        "<Board name=\"{name}\" x={{0}} y={{0}} width={{320}} height={{200}} dir=\"col\">\n  <Instance name=\"{name} pay\" component=\"{button}\"/>\n  <Frame name=\"{name} banner\" width={{320}} height={{40}} fill=\"$color.brand\"/>\n</Board>"
    )
}

// ---------------------------------------------------------------------------
// A design edit applies to the head and stores only the boards it changes
// ---------------------------------------------------------------------------

#[test]
fn a_batch_applies_to_a_designs_head_and_the_next_read_has_the_change() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, boards) = design_with_boards(ws, client, &project, 2);
    let changed = unique("Summa 12 kr");
    let results = edit(
        ws,
        client,
        &project,
        &doc,
        &json!([set_content(&boards[0].text, 1, &changed)]),
    );
    assert_eq!(results[0]["node"], boards[0].text.as_str(), "{results:?}");
    assert_eq!(results[0]["revision"], 2, "{results:?}");
    for auth in [agent(ws), human(ws)] {
        let read = read_head_on(client, ws, &auth, &project, &doc);
        let tags = str_of(&read, "tags");
        assert!(
            tags.contains(&changed),
            "the head read has the edit: {tags}"
        );
    }
    assert!(
        board_tags(ws, client, &project, &doc, &boards[0].id).contains(&changed),
        "the board read has the edit"
    );
    assert!(
        board_tags(ws, client, &project, &doc, &boards[1].id).contains("Text 1"),
        "the other board reads as it was"
    );
}

#[test]
fn editing_one_board_of_a_hundred_writes_one_board_chunk_and_a_new_manifest() {
    let (_, client) = ready();
    // A workspace of its own, so that only this test writes chunks under its
    // prefix.
    let ws = &seed_workspace(client, &unique("chunks"));
    let project = new_project(ws, client);
    let (doc, boards) = design_with_boards(ws, client, &project, 100);

    let before = manifest_of(ws, &doc);
    let head_before = head_of(&doc);
    let boards_before = manifest_boards(&before);
    assert_eq!(boards_before.len(), 100, "{before}");
    let mut ids_before: Vec<&str> = boards_before.iter().map(|(id, _)| id.as_str()).collect();
    ids_before.sort();
    let mut inserted: Vec<&str> = boards.iter().map(|b| b.id.as_str()).collect();
    inserted.sort();
    assert_eq!(ids_before, inserted, "the manifest lists every board");
    let chunks = chunk_key(ws, "");
    for (board, chunk) in &boards_before {
        assert!(is_chunk_name(chunk), "{board}: {chunk:?}");
        let bytes = stored(&format!("{chunks}{chunk}"));
        assert_eq!(
            blake3::hash(&bytes).to_hex().to_string(),
            *chunk,
            "the chunk of {board} is named by the BLAKE3 hash of its bytes"
        );
    }
    let keys_before = keys_under(&chunks);

    let edited = &boards[42];
    let changed = unique("Changed");
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([set_content(&edited.text, 1, &changed)]),
    );

    let head_after = head_of(&doc);
    assert_ne!(head_after, head_before, "the edit made a new head");
    let keys_after = keys_under(&chunks);
    let added: Vec<&String> = keys_after.difference(&keys_before).collect();
    assert_eq!(
        added.len(),
        2,
        "the edit stored one board chunk and one manifest, nothing else: {added:?}"
    );
    assert!(
        added.contains(&&format!("{chunks}{head_after}")),
        "one of them is the new manifest: {added:?}"
    );

    let after = manifest_of(ws, &doc);
    assert_eq!(
        after["pages"][0]["id"], before["pages"][0]["id"],
        "the same page"
    );
    assert_eq!(
        after["pages"][0]["chunk"], before["pages"][0]["chunk"],
        "the page's own chunk did not change"
    );
    let boards_after = manifest_boards(&after);
    let order = |boards: &[(String, String)]| -> Vec<String> {
        boards.iter().map(|(id, _)| id.clone()).collect()
    };
    assert_eq!(
        order(&boards_after),
        order(&boards_before),
        "the same boards in the same order"
    );
    let (was, now) = (chunk_by_board(&before), chunk_by_board(&after));
    let mut unchanged = 0;
    for board in &boards {
        if board.id == edited.id {
            assert_ne!(
                now[&board.id], was[&board.id],
                "the edited board has a new chunk"
            );
            assert!(
                added.contains(&&format!("{chunks}{}", now[&board.id])),
                "the edited board's new chunk is the other object stored: {added:?}"
            );
            let bytes = stored(&format!("{chunks}{}", now[&board.id]));
            assert!(
                String::from_utf8_lossy(&bytes).contains(&changed),
                "the new chunk holds the edit"
            );
        } else {
            assert_eq!(
                now[&board.id], was[&board.id],
                "board {} kept its chunk",
                board.id
            );
            unchanged += 1;
        }
    }
    assert_eq!(unchanged, 99, "99 boards kept their chunk");
    assert!(
        board_tags(ws, client, &project, &doc, &edited.id).contains(&changed),
        "the edited board reads with the edit"
    );
    assert!(
        board_tags(ws, client, &project, &doc, &boards[7].id).contains("Text 7"),
        "an untouched board reads as it was"
    );
}

#[test]
fn the_board_option_keeps_a_live_batch_inside_that_board() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, boards) = design_with_boards(ws, client, &project, 2);
    let head = head_of(&doc);
    let changes = changes_of(&doc).len();

    let response = edit_response(
        ws,
        client,
        &project,
        &doc,
        &json!({
            "board": boards[0].id,
            "ops": [
                set_content(&boards[0].text, 1, "Inside"),
                set_content(&boards[1].text, 1, "Outside")
            ]
        }),
    );
    assert_eq!(response.status, 400, "{}", response.text());
    let body = response.json();
    assert_eq!(body["error"], "invalid_operation", "{body}");
    assert_eq!(body["index"], 1, "{body}");
    assert!(
        str_of(&body, "message").contains(&boards[1].text),
        "the message names the node outside the board: {body}"
    );
    let response = edit_response(
        ws,
        client,
        &project,
        &doc,
        &json!({ "board": UNKNOWN_NODE, "ops": [set_content(&boards[0].text, 1, "x")] }),
    );
    assert_eq!(response.status, 404, "{}", response.text());
    assert_eq!(response.json()["error"], "unknown_board");
    assert_eq!(head_of(&doc), head, "a refused batch leaves the head");
    assert_eq!(changes_of(&doc).len(), changes, "and logs nothing");

    let results = edit_response(
        ws,
        client,
        &project,
        &doc,
        &json!({ "board": boards[0].id, "ops": [set_content(&boards[0].text, 1, "Inside")] }),
    );
    assert_eq!(results.status, 200, "{}", results.text());
    assert!(board_tags(ws, client, &project, &doc, &boards[0].id).contains("Inside"));
}

// ---------------------------------------------------------------------------
// Conflicts
// ---------------------------------------------------------------------------

#[test]
fn a_design_edit_with_a_stale_rev_is_a_conflict_and_changes_nothing() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, boards) = design_with_boards(ws, client, &project, 2);
    let current = unique("Changed by someone else");
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([set_content(&boards[0].text, 1, &current)]),
    );
    let head = head_of(&doc);
    let changes = changes_of(&doc).len();
    let events = doc_changed_events(&project, &doc).len();
    let tags = str_of(&read_head(ws, client, &project, &doc), "tags").to_string();

    let response = edit_response(
        ws,
        client,
        &project,
        &doc,
        &json!({ "ops": [
            set_content(&boards[1].text, 1, "Never applied"),
            set_content(&boards[0].text, 1, "Stale")
        ] }),
    );
    assert_eq!(response.status, 409, "{}", response.text());
    let body = response.json();
    assert_eq!(body["error"], "conflict", "{body}");
    assert_eq!(body["conflict"]["index"], 1, "the second operation: {body}");
    assert_eq!(body["conflict"]["node"], boards[0].text.as_str(), "{body}");
    assert_eq!(body["conflict"]["expected"], 1, "{body}");
    assert_eq!(body["conflict"]["current_revision"], 2, "{body}");

    assert_eq!(head_of(&doc), head, "a conflict leaves the head");
    assert_eq!(changes_of(&doc).len(), changes, "a conflict logs nothing");
    assert_eq!(
        doc_changed_events(&project, &doc).len(),
        events,
        "a conflict emits nothing"
    );
    assert_eq!(
        str_of(&read_head(ws, client, &project, &doc), "tags"),
        tags,
        "the design reads as before"
    );

    // With the current revision the same change applies.
    let results = edit(
        ws,
        client,
        &project,
        &doc,
        &json!([set_content(&boards[0].text, 2, "Current")]),
    );
    assert_eq!(results[0]["revision"], 3, "{results:?}");
}

#[test]
fn the_revisions_of_a_design_survive_an_api_restart() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, boards) = design_with_boards(ws, client, &project, 1);
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([set_content(&boards[0].text, 1, "Second")]),
    );

    let fresh = TestServer::start(&config());
    let stale = call(
        &fresh.client,
        "POST",
        &format!("{}/ops", doc_path(ws, &project, &doc)),
        &agent(ws),
        Some(&json!({ "ops": [set_content(&boards[0].text, 1, "Stale")] })),
    );
    assert_eq!(
        stale.status,
        409,
        "a fresh API checks against the written revision: {}",
        stale.text()
    );
    assert_eq!(
        stale.json()["conflict"]["current_revision"],
        2,
        "{}",
        stale.text()
    );
    let applied = must(
        call(
            &fresh.client,
            "POST",
            &format!("{}/ops", doc_path(ws, &project, &doc)),
            &agent(ws),
            Some(&json!({ "ops": [set_content(&boards[0].text, 2, "Third")] })),
        ),
        200,
        "edit through the fresh API",
    );
    assert_eq!(applied["results"][0]["revision"], 3, "{applied}");
    assert!(
        board_tags(ws, client, &project, &doc, &boards[0].id).contains("Third"),
        "the running API reads the fresh API's write"
    );
}

// ---------------------------------------------------------------------------
// The log of writes
// ---------------------------------------------------------------------------

#[test]
fn a_design_edit_logs_each_changed_board_with_its_chunk_before_and_after() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, boards) = design_with_boards(ws, client, &project, 3);
    let page = first_page(ws, client, &project, &doc);
    let before = chunk_by_board(&manifest_of(ws, &doc));
    let head_before = head_of(&doc);
    let rows = changes_of(&doc).len();
    let events = doc_changed_events(&project, &doc).len();

    // Board 0 changes, a new board comes and board 2 goes; board 1 stays.
    let ops = json!([
        set_content(&boards[0].text, 1, &unique("Changed")),
        insert(&page, &board_tag("Receipt", 900, "Tack")),
        delete(&boards[2].id, 1)
    ]);
    let results = edit(ws, client, &project, &doc, &ops);
    let new_board = str_of(&results[1], "node").to_string();
    let after = chunk_by_board(&manifest_of(ws, &doc));
    let head_after = head_of(&doc);
    assert!(
        !after.contains_key(&boards[2].id),
        "the deleted board is gone"
    );

    let changes = changes_of(&doc);
    assert_eq!(
        changes.len(),
        rows + 1,
        "one row for the batch: {changes:?}"
    );
    let change = changes.last().expect("a row");
    assert_eq!(change.seq, rows as i64 + 1, "{change:?}");
    assert_eq!(change.agent["run"], "run-104", "{change:?}");
    assert_eq!(change.agent["role"], "designer", "{change:?}");
    assert_eq!(
        change.operations, ops,
        "the row has the operations as the agent sent them"
    );
    assert_eq!(change.head_before.as_deref(), Some(head_before.as_str()));
    assert_eq!(change.head_after.as_deref(), Some(head_after.as_str()));

    let mut expected = vec![
        (
            boards[0].id.clone(),
            Some(before[&boards[0].id].clone()),
            Some(after[&boards[0].id].clone()),
        ),
        (
            boards[2].id.clone(),
            Some(before[&boards[2].id].clone()),
            None,
        ),
        (new_board.clone(), None, Some(after[&new_board].clone())),
    ];
    expected.sort();
    assert_eq!(
        changed_boards(change),
        expected,
        "the row lists each changed board with its chunk before and after, and not board 1: {:?}",
        change.boards
    );

    let sent = doc_changed_events(&project, &doc);
    assert_eq!(sent.len(), events + 1, "one doc.changed: {sent:?}");
    assert_eq!(sent.last().unwrap()["head"], head_after.as_str());
}

// ---------------------------------------------------------------------------
// Tokens and components of the design system's head
// ---------------------------------------------------------------------------

#[test]
fn token_and_component_imports_write_the_design_systems_head() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let doc = new_ui_doc(ws, client, &project, None, "design-system");
    let created = head_of(&doc);
    let rows = changes_of(&doc).len();

    put_tokens(ws, client, &project, &token_tree("#2563eb"));
    let after_tokens = head_of(&doc);
    assert_ne!(after_tokens, created, "the import made a new head");
    assert!(is_chunk_name(&after_tokens), "{after_tokens:?}");
    stored(&chunk_key(ws, &after_tokens));
    for auth in [agent(ws), human(ws)] {
        let read = must(
            call(
                client,
                "GET",
                &format!("{}/tokens", project_base(ws, &project)),
                &auth,
                None,
            ),
            200,
            "read the design system's tokens",
        );
        assert_eq!(read["tokens"], token_tree("#2563eb"), "{read}");
    }

    let imported = put_components(ws, client, &project, &button_tags(None, "Buy"));
    let after_components = head_of(&doc);
    assert_ne!(after_components, after_tokens, "the import made a new head");
    assert!(
        imported["components"][0]["id"].is_string(),
        "the Button has an ID: {imported}"
    );
    let read = must(
        call(
            client,
            "GET",
            &format!("{}/tokens", project_base(ws, &project)),
            &agent(ws),
            None,
        ),
        200,
        "read the tokens again",
    );
    assert_eq!(
        read["tokens"],
        token_tree("#2563eb"),
        "a components import keeps the tokens: {read}"
    );

    let changes = changes_of(&doc);
    assert_eq!(changes.len(), rows + 2, "one row per import: {changes:?}");
    assert_eq!(changes[rows].head_before.as_deref(), Some(created.as_str()));
    assert_eq!(
        changes[rows].head_after.as_deref(),
        Some(after_tokens.as_str())
    );
    assert_eq!(
        changes[rows + 1].head_after.as_deref(),
        Some(after_components.as_str())
    );
}

#[test]
fn designs_resolve_instances_and_tokens_against_the_design_systems_head() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (_, button) = design_system(ws, client, &project, "#2563eb");
    let feature = new_feature(ws, client, &project);

    // Two designs that use the Button and the brand token: one at the app
    // level and one in a feature.
    let mut designs = Vec::new();
    for (scope, name) in [(None, "App"), (Some(feature.as_str()), "Feature")] {
        let doc = new_ui_doc(ws, client, &project, scope, "design");
        let page = first_page(ws, client, &project, &doc);
        let results = edit(
            ws,
            client,
            &project,
            &doc,
            &json!([insert(&page, &board_using(&button, name))]),
        );
        designs.push((doc, str_of(&results[0], "node").to_string()));
    }
    let heads: Vec<String> = designs.iter().map(|(doc, _)| head_of(doc)).collect();

    for (doc, _) in &designs {
        let read = read_head(ws, client, &project, doc);
        assert_eq!(
            read["tokens"],
            token_tree("#2563eb"),
            "a design reads the design system head's tokens: {read}"
        );
        let components = read["components"].to_string();
        assert!(
            components.contains(&button) && components.contains("\"Buy\""),
            "a design reads the components its instances use: {read}"
        );
    }

    // The design system changes; the designs do not.
    put_tokens(ws, client, &project, &token_tree("#16a34a"));
    put_components(ws, client, &project, &button_tags(Some(&button), "Buy now"));
    for ((doc, _), head) in designs.iter().zip(&heads) {
        let read = read_head(ws, client, &project, doc);
        assert_eq!(
            read["tokens"],
            token_tree("#16a34a"),
            "the token change shows up at the next read: {read}"
        );
        let components = read["components"].to_string();
        assert!(
            components.contains(&button)
                && components.contains("Buy now")
                && !components.contains("\"Buy\""),
            "the component change shows up at the next read: {read}"
        );
        assert_eq!(&head_of(doc), head, "the design's own head did not change");
    }

    // The usage of the Button lists the boards of both designs' heads.
    let usage = must(
        call(
            client,
            "GET",
            &format!(
                "{}/components/usage?component={button}",
                project_base(ws, &project)
            ),
            &agent(ws),
            None,
        ),
        200,
        "list the boards that use the Button",
    );
    let listed: BTreeSet<(String, String)> = usage["boards"]
        .as_array()
        .unwrap_or_else(|| panic!("the usage lists boards: {usage}"))
        .iter()
        .map(|entry| {
            (
                str_of(entry, "doc_id").to_string(),
                str_of(entry, "board_id").to_string(),
            )
        })
        .collect();
    for (doc, board) in &designs {
        assert!(
            listed.contains(&(doc.clone(), board.clone())),
            "the usage lists board {board} of {doc}: {usage}"
        );
    }
}

#[test]
fn an_instance_of_a_component_the_design_systems_head_lacks_is_refused() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    design_system(ws, client, &project, "#2563eb");
    let doc = new_ui_doc(ws, client, &project, None, "design");
    let page = first_page(ws, client, &project, &doc);
    let head = head_of(&doc);
    let response = edit_response(
        ws,
        client,
        &project,
        &doc,
        &json!({ "ops": [insert(&page, &board_using(UNKNOWN_NODE, "Ghost"))] }),
    );
    assert_eq!(response.status, 400, "{}", response.text());
    let body = response.json();
    assert_eq!(body["error"], "invalid_operation", "{body}");
    assert!(
        str_of(&body, "message").contains(UNKNOWN_NODE),
        "the message names the unknown component: {body}"
    );
    assert_eq!(head_of(&doc), head, "a refused batch leaves the head");
}
