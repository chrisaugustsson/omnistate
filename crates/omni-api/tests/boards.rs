//! US-030: pages, boards and chunked storage through the API. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; docs and drafts are those
//! of US-022, versions of US-024, approvals of US-025 and design imports
//! (`PUT /changesets/{changeset_id}/docs/{doc_id}/design`) of US-029; the
//! tags are those of docs/formats/design-tags.md):
//!
//! - A design doc holds pages, and pages hold boards. A version of a design
//!   doc is a manifest: `GET /versions/{version_id}` of a design version
//!   answers `"manifest": {"pages": [{"id", "chunk", "boards": [{"id",
//!   "chunk"}]}]}`, the pages in order and the boards of each page in order,
//!   where `id` is the node ID and `chunk` the name of the stored chunk.
//! - Every page and every board is its own chunk in RustFS, at
//!   `ws/{workspace_id}/chunks/{chunk}`, where `chunk` is the BLAKE3 hash of
//!   the stored bytes in lowercase hex (64 characters). A board's chunk holds
//!   that board and its subtree and nothing of the other boards; a page's
//!   chunk holds none of its boards' content.
//! - The chunk index is the tenant table `chunks`, keyed by `(workspace_id,
//!   hash)`: one row per chunk of a workspace, whatever number of versions use
//!   it. The columns without a default are `workspace_id` and `hash`. Its rows
//!   are never changed, so the API role has no UPDATE privilege on it
//!   (crates/omni-core/tests/tenancy.rs).
//! - Unchanged boards (and pages) keep their chunk: a later version, in the
//!   same change set or another, lists the same chunk for them.
//! - The node index (`node_index`, US-024) is stored per chunk: the rows of a
//!   board's chunk are exactly the board and its descendants, written once and
//!   shared by every version that uses the chunk. A version's `nodes` are
//!   still every node of the design in document order with its parent, and a
//!   comparison of two versions still names only what changed.
//! - `GET /changesets/{changeset_id}/docs/{doc_id}/boards/{board_id}` (both)
//!   reads one board of the change set's draft, unsaved changes included, and
//!   `GET /docs/{doc_id}/boards/{board_id}` (both) one board of the mainline
//!   version, or with `?version=<v>` of that version of the doc. The answer is
//!   200 `{"doc_id", "page_id", "board_id", "version_id", "tags"}` (with
//!   `"changeset_id"` and `"draft_id"` for a draft, whose `version_id` is
//!   `null`): `tags` is the board alone in the tag format, starting with its
//!   `<Board id="...">` tag, with no `<Page>`. When reading a draft, every tag
//!   carries a `rev` attribute, the node's revision (`rev={3}` or `rev="3"`);
//!   a version or the mainline has none. An ID that names no board of the doc
//!   (an unknown node, a page, a frame) is 404 `{"error": "unknown_board"}`,
//!   a doc of another kind 400 `{"error": "not_a_design"}`, the mainline of a
//!   doc that has no mainline version 409 `{"error": "no_mainline_version"}`,
//!   and a version that is not one of the doc's 404 `{"error": "not_found"}`.
//! - Both routes are in the route registry as `both`.
//!
//! US-058: the design viewer and the change set review read a version board by
//! board, never its whole body at once:
//!
//! - `GET /versions/{version_id}?view=outline` of a design version answers the
//!   version without `pages` and `tags`, and with `"outline"`: its pages in
//!   order, each page as its own JSON with its boards in order under
//!   `children`, every board as its own JSON (its name, box, layout and
//!   fills) without its children. The `manifest` is the whole read's, and
//!   nothing below a board (no ID, no content) is in the answer, so `nodes`
//!   lists no node below a board either.
//! - The board of a version (`GET /docs/{doc_id}/boards/{board_id}`, with or
//!   without `?version=`) also answers `"node"`: the board and its subtree as
//!   JSON, as the whole read's `pages` hold it.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
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

const TEST_DB: &str = "omni_us030_boards";
const TEST_BUCKET: &str = "omni-us030-boards";
const DRAFT_BOARD_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/boards/{board_id}";
const BOARD_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/boards/{board_id}";
/// A node ID in the right format that no node has.
const UNKNOWN_NODE: &str = "zzzzzzzz";
/// A version ID that no version has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";

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
    /// A second workspace, which has no records of its own.
    other_workspace: String,
    alice_token: String,
    key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us030-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
        let (workspace, other_workspace) = block(async {
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
            let other = text(
                "insert into workspaces (name) values ('Beta') returning id::text",
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
            (workspace, other)
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
            other_workspace,
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

fn auth_headers(auth: &Auth) -> Vec<(&'static str, String)> {
    match auth {
        Auth::Agent(key) => vec![
            ("Authorization", format!("Bearer {key}")),
            ("X-Omni-Run", "run-30".to_string()),
            ("X-Omni-Role", "designer".to_string()),
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

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
}

fn changeset_path(f: &Fixture, project: &str, changeset: &str) -> String {
    format!("{}/changesets/{changeset}", project_base(f, project))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &agent(f),
                Some(&json!({ "name": unique("boards-project") })),
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
                Some(&json!({ "title": unique("change"), "run_tag": "run-30" })),
            ),
            201,
            "create a change set",
        )["changeset"],
    )
}

fn create_doc(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    kind: &str,
    module: &str,
) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", changeset_path(f, project, changeset)),
                &agent(f),
                Some(&json!({ "kind": kind, "module": module, "title": unique("doc") })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

/// Replaces the pages of the change set's draft of `doc`; returns the answer.
fn import_design(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    tags: &str,
) -> Value {
    must(
        call(
            client,
            "PUT",
            &format!(
                "{}/docs/{doc}/design",
                changeset_path(f, project, changeset)
            ),
            &agent(f),
            Some(&json!({ "tags": tags })),
        ),
        200,
        "import the design",
    )
}

/// Saves a version of the change set's draft of `doc`; returns the version.
fn save(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "POST",
            &format!(
                "{}/docs/{doc}/versions",
                changeset_path(f, project, changeset)
            ),
            &agent(f),
            Some(&json!({})),
        ),
        201,
        "save a version",
    )["version"]
        .clone()
}

fn read_version(f: &Fixture, client: &Client, project: &str, version: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/versions/{version}", project_base(f, project)),
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
            &format!("{}/versions/{from}/diff/{to}", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "compare two versions",
    )["diff"]
        .clone()
}

/// Submits the change set and has the human approve it, which merges it.
fn merge(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let path = changeset_path(f, project, changeset);
    must(
        call(client, "POST", &format!("{path}/submit"), &agent(f), None),
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
        "the human approves",
    );
    assert_eq!(approved["changeset"]["status"], "merged", "{approved}");
}

fn mainline_of(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    must(
        call(
            client,
            "GET",
            &format!("{}/docs/{doc}", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "read the doc",
    )["doc"]["mainline_version_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the doc {doc} has a mainline version"))
        .to_string()
}

fn draft_board_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    board: &str,
) -> Response {
    call(
        client,
        "GET",
        &format!(
            "{}/docs/{doc}/boards/{board}",
            changeset_path(f, project, changeset)
        ),
        &agent(f),
        None,
    )
}

/// `query` is empty for the mainline, or `?version=<v>`.
fn board_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    doc: &str,
    board: &str,
    query: &str,
) -> Response {
    call(
        client,
        "GET",
        &format!(
            "{}/docs/{doc}/boards/{board}{query}",
            project_base(f, project)
        ),
        auth,
        None,
    )
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
            .unwrap_or_else(|e| panic!("count with {sql}: {e}"))
    })
}

/// The node IDs that the node index lists for a chunk of the workspace.
fn indexed_nodes(f: &Fixture, chunk: &str) -> BTreeSet<String> {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar::<_, String>(
            "select node from node_index where workspace_id = $1::uuid and chunk = $2",
        )
        .bind(&f.workspace)
        .bind(chunk)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the node index of {chunk}: {e}"))
        .into_iter()
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

// ---------------------------------------------------------------------------
// The design
// ---------------------------------------------------------------------------

/// Node IDs by node name. Every name in the design is used once.
type Ids = HashMap<String, String>;

fn collect_ids(nodes: &Value, ids: &mut Ids) {
    for node in nodes.as_array().unwrap_or(&Vec::new()) {
        if let (Some(name), Some(id)) = (node["name"].as_str(), node["id"].as_str()) {
            ids.insert(name.to_string(), id.to_string());
        }
        collect_ids(&node["children"], ids);
    }
}

fn ids_in(nodes: &Value) -> Ids {
    let mut ids = Ids::new();
    collect_ids(nodes, &mut ids);
    ids
}

/// ` id="<id>"` for a node the draft already has, nothing for a new one.
fn id_attr(ids: &Ids, name: &str) -> String {
    ids.get(name)
        .map(|id| format!(r#" id="{id}""#))
        .unwrap_or_default()
}

fn id(ids: &Ids, name: &str) -> String {
    ids.get(name)
        .unwrap_or_else(|| panic!("the node {name} has an ID: {ids:?}"))
        .clone()
}

/// The names of the design's nodes in document order, each with its parent's
/// name.
const DOCUMENT_ORDER: [(&str, Option<&str>); 10] = [
    ("Checkout", None),
    ("Cart", Some("Checkout")),
    ("Summary", Some("Cart")),
    ("Total", Some("Summary")),
    ("Shipping", Some("Summary")),
    ("Payment", Some("Checkout")),
    ("Pay", Some("Payment")),
    ("Account", None),
    ("Profile", Some("Account")),
    ("Hello", Some("Profile")),
];

/// Two pages: Checkout with the boards Cart and Payment, Account with the
/// board Profile. `total` and `hello` are the texts of the Total and Hello
/// texts, which the tests change.
fn design_tags(ids: &Ids, total: &str, hello: &str) -> String {
    format!(
        r##"<Page{} name="Checkout">
  <Board{} name="Cart" x={{0}} y={{0}} width={{360}} height={{640}} dir="col" gap={{8}}>
    <Frame{} name="Summary" dir="col" gap={{4}}>
      <Text{} name="Total" content="{total}" fontFamily="Inter" fontSize={{16}}/>
      <Text{} name="Shipping" content="Frakt ingår" fontFamily="Inter" fontSize={{14}}/>
    </Frame>
  </Board>
  <Board{} name="Payment" x={{400}} y={{0}} width={{360}} height={{640}} dir="col">
    <Text{} name="Pay" content="Betala nu" fontFamily="Inter" fontSize={{16}}/>
  </Board>
</Page>
<Page{} name="Account">
  <Board{} name="Profile" x={{0}} y={{0}} width={{360}} height={{640}} dir="col">
    <Text{} name="Hello" content="{hello}" fontFamily="Inter" fontSize={{16}}/>
  </Board>
</Page>
"##,
        id_attr(ids, "Checkout"),
        id_attr(ids, "Cart"),
        id_attr(ids, "Summary"),
        id_attr(ids, "Total"),
        id_attr(ids, "Shipping"),
        id_attr(ids, "Payment"),
        id_attr(ids, "Pay"),
        id_attr(ids, "Account"),
        id_attr(ids, "Profile"),
        id_attr(ids, "Hello"),
    )
}

const TOTAL: &str = "Summa 10 kr";
const NEW_TOTAL: &str = "Summa 12 kr";
const HELLO: &str = "Hej Alice";
const NEW_HELLO: &str = "Hej Bob";

/// A project with a design doc started in an open change set and the design
/// imported into its draft.
struct Design {
    project: String,
    changeset: String,
    doc: String,
    ids: Ids,
}

fn new_design(f: &Fixture, client: &Client) -> Design {
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let doc = create_doc(f, client, &project, &changeset, "design", "ui");
    let imported = import_design(
        f,
        client,
        &project,
        &changeset,
        &doc,
        &design_tags(&Ids::new(), TOTAL, HELLO),
    );
    let ids = ids_in(&imported["pages"]);
    assert_eq!(ids.len(), DOCUMENT_ORDER.len(), "{imported}");
    Design {
        project,
        changeset,
        doc,
        ids,
    }
}

/// The page entries of a version's manifest.
fn manifest_pages(version: &Value) -> Vec<Value> {
    version["manifest"]["pages"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("a design version answers a manifest with its pages: {version:#}")
        })
        .clone()
}

/// The chunk of every page and board of a version's manifest, by node ID.
fn chunks_by_node(version: &Value) -> HashMap<String, String> {
    let mut chunks = HashMap::new();
    for page in manifest_pages(version) {
        chunks.insert(id_of(&page), chunk_of(&page));
        for board in page["boards"]
            .as_array()
            .unwrap_or_else(|| panic!("a page of the manifest lists its boards: {page}"))
        {
            chunks.insert(id_of(board), chunk_of(board));
        }
    }
    chunks
}

fn chunk_of(entry: &Value) -> String {
    let chunk = entry["chunk"]
        .as_str()
        .unwrap_or_else(|| panic!("a manifest entry names its chunk: {entry}"))
        .to_string();
    assert!(
        chunk.len() == 64
            && chunk
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "a chunk is named by a BLAKE3 hash in lowercase hex: {chunk:?}"
    );
    chunk
}

/// The chunk of the node named `name` in a version's manifest.
fn chunk_named(chunks: &HashMap<String, String>, ids: &Ids, name: &str) -> String {
    chunks
        .get(&id(ids, name))
        .unwrap_or_else(|| panic!("the manifest lists {name}: {chunks:?}"))
        .clone()
}

/// Every opening tag of `tags`, one per line as `tags::print` writes them.
fn opening_tags(tags: &str) -> Vec<&str> {
    tags.lines()
        .map(str::trim)
        .filter(|line| line.starts_with('<') && !line.starts_with("</"))
        .collect()
}

/// The value of the `rev` attribute of a tag, `rev={3}` or `rev="3"`.
fn rev_of(tag: &str) -> Option<u64> {
    let start = tag.find(" rev=")? + " rev=".len();
    let rest = &tag[start..];
    let (open, close) = match rest.chars().next()? {
        '{' => ('{', '}'),
        '"' => ('"', '"'),
        _ => return None,
    };
    let rest = rest.strip_prefix(open)?;
    let end = rest.find(close)?;
    rest[..end].parse().ok()
}

/// Asserts that `tags` is the board `board` alone: its tag first, the nodes
/// named in `inside` in it, and nothing of `outside`.
fn assert_board_alone(tags: &str, board: &str, inside: &[&str], outside: &[&str]) {
    let first = opening_tags(tags)
        .first()
        .copied()
        .unwrap_or_else(|| panic!("the answer has tags: {tags:?}"));
    assert!(
        first.starts_with(&format!(r#"<Board id="{board}""#)),
        "the tags start with the board's own tag: {tags}"
    );
    assert!(!tags.contains("<Page"), "no page is printed: {tags}");
    for needle in inside {
        assert!(tags.contains(needle), "the board has {needle:?}: {tags}");
    }
    for needle in outside {
        assert!(
            !tags.contains(needle),
            "{needle:?} is not in the board: {tags}"
        );
    }
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_board_routes_are_in_the_registry() {
    let routes = registry();
    for path in [DRAFT_BOARD_ROUTE, BOARD_ROUTE] {
        let entry = routes
            .iter()
            .find(|r| r.method == "GET" && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route GET {path}"));
        assert_eq!(entry.access, Access::Both, "GET {path}");
    }
}

// ---------------------------------------------------------------------------
// Pages, boards and their chunks
// ---------------------------------------------------------------------------

#[test]
fn a_design_version_is_a_manifest_of_its_pages_and_their_boards() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let saved = save(f, client, &d.project, &d.changeset, &d.doc);
    let version = read_version(f, client, &d.project, &id_of(&saved));

    let pages = manifest_pages(&version);
    let page_ids: Vec<String> = pages.iter().map(id_of).collect();
    assert_eq!(
        page_ids,
        [id(&d.ids, "Checkout"), id(&d.ids, "Account")],
        "the manifest lists the pages in order: {version:#}"
    );
    let boards = |page: &Value| -> Vec<String> {
        page["boards"]
            .as_array()
            .unwrap_or_else(|| panic!("a page of the manifest lists its boards: {page}"))
            .iter()
            .map(id_of)
            .collect()
    };
    assert_eq!(
        boards(&pages[0]),
        [id(&d.ids, "Cart"), id(&d.ids, "Payment")],
        "the first page holds its boards in order"
    );
    assert_eq!(boards(&pages[1]), [id(&d.ids, "Profile")]);

    let chunks = chunks_by_node(&version);
    let distinct: BTreeSet<&String> = chunks.values().collect();
    assert_eq!(
        distinct.len(),
        5,
        "every page and board is a chunk of its own: {chunks:?}"
    );
    // The full design is still there for every viewer.
    let tags = version["tags"].as_str().expect("the version's tags");
    for text in [TOTAL, "Betala nu", HELLO] {
        assert!(tags.contains(text), "{text:?} is in the version: {tags}");
    }
}

#[test]
fn every_page_and_board_is_stored_in_rustfs_named_by_the_blake3_hash_of_its_content() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let saved = save(f, client, &d.project, &d.changeset, &d.doc);
    let version = read_version(f, client, &d.project, &id_of(&saved));
    let chunks = chunks_by_node(&version);
    assert_eq!(chunks.len(), 5, "{chunks:?}");

    let mut contents = HashMap::new();
    for (node, chunk) in &chunks {
        let key = chunk_key(f, chunk);
        let bytes = stored(&key);
        assert_eq!(
            blake3::hash(&bytes).to_hex().as_str(),
            chunk.as_str(),
            "{key} (of node {node}) is named by the BLAKE3 hash of its bytes"
        );
        contents.insert(node.clone(), String::from_utf8_lossy(&bytes).to_string());
    }
    let content = |name: &str| contents[&id(&d.ids, name)].clone();

    // A board's chunk holds the board and its subtree, and nothing else.
    let cart = content("Cart");
    for name in ["Cart", "Summary", "Total", "Shipping"] {
        assert!(
            cart.contains(&id(&d.ids, name)),
            "the Cart chunk holds {name}: {cart}"
        );
    }
    assert!(cart.contains(TOTAL), "{cart}");
    for name in ["Payment", "Pay", "Profile", "Hello"] {
        assert!(
            !cart.contains(&id(&d.ids, name)),
            "the Cart chunk holds nothing of {name}: {cart}"
        );
    }
    let payment = content("Payment");
    assert!(payment.contains("Betala nu"), "{payment}");
    assert!(
        !payment.contains(TOTAL) && !payment.contains(HELLO),
        "{payment}"
    );

    // A page's chunk holds none of its boards' content.
    for page in ["Checkout", "Account"] {
        let text = content(page);
        for board_content in [TOTAL, "Frakt ingår", "Betala nu", HELLO] {
            assert!(
                !text.contains(board_content),
                "the {page} page's chunk holds no board content: {text}"
            );
        }
    }
}

#[test]
fn the_chunk_index_is_keyed_by_workspace_and_hash() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let saved = save(f, client, &d.project, &d.changeset, &d.doc);
    let first = read_version(f, client, &d.project, &id_of(&saved));
    let chunks = chunks_by_node(&first);

    for chunk in chunks.values() {
        assert_eq!(
            count(
                "select count(*) from chunks where workspace_id = $1::uuid and hash = $2",
                &[&f.workspace, chunk],
            ),
            1,
            "the chunk index has one row for {chunk} in the workspace"
        );
    }

    // Saving the same draft again uses the same chunks: still one row each.
    let again = save(f, client, &d.project, &d.changeset, &d.doc);
    let again = read_version(f, client, &d.project, &id_of(&again));
    assert_eq!(chunks_by_node(&again), chunks);
    for chunk in chunks.values() {
        assert_eq!(
            count(
                "select count(*) from chunks where workspace_id = $1::uuid and hash = $2",
                &[&f.workspace, chunk],
            ),
            1,
            "a chunk that two versions use is indexed once: {chunk}"
        );
    }

    // The key is the pair: the same hash may be indexed in another workspace,
    // but not twice in one.
    let cart = chunk_named(&chunks, &d.ids, "Cart");
    let insert = |workspace: &str| {
        block(async {
            let mut conn = admin_connection().await;
            sqlx::query("insert into chunks (workspace_id, hash) values ($1::uuid, $2)")
                .bind(workspace)
                .bind(&cart)
                .execute(&mut conn)
                .await
                .map(|_| ())
        })
    };
    insert(&f.other_workspace)
        .unwrap_or_else(|e| panic!("another workspace may index the same hash: {e}"));
    let twice = insert(&f.workspace).expect_err("a workspace indexes a hash once");
    assert!(
        twice.to_string().contains("duplicate key"),
        "the second row is refused as a duplicate of (workspace_id, hash): {twice}"
    );
}

#[test]
fn unchanged_boards_and_pages_keep_their_chunks_in_later_versions() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let first = save(f, client, &d.project, &d.changeset, &d.doc);
    let first = chunks_by_node(&read_version(f, client, &d.project, &id_of(&first)));

    // Only the Cart board changes.
    import_design(
        f,
        client,
        &d.project,
        &d.changeset,
        &d.doc,
        &design_tags(&d.ids, NEW_TOTAL, HELLO),
    );
    let second = save(f, client, &d.project, &d.changeset, &d.doc);
    let second_version = read_version(f, client, &d.project, &id_of(&second));
    let second = chunks_by_node(&second_version);
    let chunk = |chunks: &HashMap<String, String>, name: &str| chunk_named(chunks, &d.ids, name);
    assert_ne!(
        chunk(&first, "Cart"),
        chunk(&second, "Cart"),
        "the changed board has a new chunk"
    );
    for name in ["Payment", "Profile", "Account"] {
        assert_eq!(
            chunk(&first, name),
            chunk(&second, name),
            "the unchanged {name} keeps its chunk"
        );
    }
    let tags = second_version["tags"].as_str().expect("the version's tags");
    assert!(tags.contains(NEW_TOTAL) && tags.contains(HELLO), "{tags}");

    // A version of another change set shares the boards it does not change.
    merge(f, client, &d.project, &d.changeset);
    let later = new_changeset(f, client, &d.project);
    import_design(
        f,
        client,
        &d.project,
        &later,
        &d.doc,
        &design_tags(&d.ids, NEW_TOTAL, NEW_HELLO),
    );
    let third = save(f, client, &d.project, &later, &d.doc);
    let third = chunks_by_node(&read_version(f, client, &d.project, &id_of(&third)));
    for name in ["Cart", "Payment", "Checkout"] {
        assert_eq!(
            chunk(&second, name),
            chunk(&third, name),
            "the unchanged {name} keeps its chunk in the next change set"
        );
    }
    assert_ne!(chunk(&second, "Profile"), chunk(&third, "Profile"));
}

#[test]
fn the_node_index_is_stored_per_chunk_and_shared_by_the_versions_that_use_it() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let first = id_of(&save(f, client, &d.project, &d.changeset, &d.doc));
    let first_chunks = chunks_by_node(&read_version(f, client, &d.project, &first));
    let names = |names: &[&str]| -> BTreeSet<String> {
        names.iter().map(|name| id(&d.ids, name)).collect()
    };

    let cart = chunk_named(&first_chunks, &d.ids, "Cart");
    assert_eq!(
        indexed_nodes(f, &cart),
        names(&["Cart", "Summary", "Total", "Shipping"]),
        "the Cart chunk's index lists the board and its subtree"
    );
    let payment = chunk_named(&first_chunks, &d.ids, "Payment");
    assert_eq!(indexed_nodes(f, &payment), names(&["Payment", "Pay"]));

    import_design(
        f,
        client,
        &d.project,
        &d.changeset,
        &d.doc,
        &design_tags(&d.ids, NEW_TOTAL, HELLO),
    );
    let second = id_of(&save(f, client, &d.project, &d.changeset, &d.doc));
    let second_version = read_version(f, client, &d.project, &second);
    let second_chunks = chunks_by_node(&second_version);
    assert_eq!(
        chunk_named(&second_chunks, &d.ids, "Payment"),
        payment,
        "the unchanged board keeps its chunk"
    );
    assert_eq!(
        count(
            "select count(*) from node_index where workspace_id = $1::uuid and chunk = $2",
            &[&f.workspace, &payment],
        ),
        2,
        "the shared chunk's nodes are indexed once, for both versions"
    );
    let new_cart = chunk_named(&second_chunks, &d.ids, "Cart");
    assert_eq!(
        indexed_nodes(f, &new_cart),
        names(&["Cart", "Summary", "Total", "Shipping"])
    );

    // The version still lists every node in document order with its parent.
    let nodes = second_version["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("the version lists its nodes: {second_version:#}"));
    let listed: Vec<(String, Option<String>)> = nodes
        .iter()
        .map(|node| (id_of(node), node["parent"].as_str().map(str::to_string)))
        .collect();
    let expected: Vec<(String, Option<String>)> = DOCUMENT_ORDER
        .iter()
        .map(|(name, parent)| (id(&d.ids, name), parent.map(|p| id(&d.ids, p))))
        .collect();
    assert_eq!(listed, expected, "{second_version:#}");

    // A comparison names only the text that changed.
    let d2 = diff(f, client, &d.project, &first, &second);
    let changed: Vec<String> = d2["changed"]
        .as_array()
        .unwrap_or_else(|| panic!("{d2}"))
        .iter()
        .map(id_of)
        .collect();
    assert_eq!(changed, [id(&d.ids, "Total")], "{d2}");
    for list in ["added", "removed", "moved"] {
        assert_eq!(d2[list], json!([]), "{list}: {d2}");
    }
}

// ---------------------------------------------------------------------------
// Reading one board
// ---------------------------------------------------------------------------

#[test]
fn reading_a_board_of_a_draft_gives_its_tags_with_a_rev_on_every_tag() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let cart = id(&d.ids, "Cart");
    let answer = must(
        draft_board_response(f, client, &d.project, &d.changeset, &d.doc, &cart),
        200,
        "read a board of the draft",
    );
    assert_eq!(answer["doc_id"], d.doc.as_str(), "{answer}");
    assert_eq!(answer["changeset_id"], d.changeset.as_str(), "{answer}");
    assert_eq!(answer["board_id"], cart.as_str(), "{answer}");
    assert_eq!(
        answer["page_id"],
        id(&d.ids, "Checkout").as_str(),
        "{answer}"
    );
    assert_eq!(answer["version_id"], Value::Null, "{answer}");
    let tags = answer["tags"].as_str().expect("the board's tags");
    assert_board_alone(
        tags,
        &cart,
        &[
            &id(&d.ids, "Summary"),
            &id(&d.ids, "Total"),
            &id(&d.ids, "Shipping"),
            TOTAL,
        ],
        &[&id(&d.ids, "Payment"), "Betala nu", HELLO],
    );
    let opening = opening_tags(tags);
    assert_eq!(opening.len(), 4, "one tag per node of the board: {tags}");
    for tag in opening {
        let rev = rev_of(tag).unwrap_or_else(|| panic!("a draft's tag carries a rev: {tag}"));
        assert!(rev >= 1, "{tag}");
    }

    // Unsaved changes are read.
    import_design(
        f,
        client,
        &d.project,
        &d.changeset,
        &d.doc,
        &design_tags(&d.ids, NEW_TOTAL, HELLO),
    );
    let answer = must(
        draft_board_response(f, client, &d.project, &d.changeset, &d.doc, &cart),
        200,
        "read the board again",
    );
    let tags = answer["tags"].as_str().expect("the board's tags");
    assert!(tags.contains(NEW_TOTAL) && !tags.contains(TOTAL), "{tags}");
}

#[test]
fn reading_a_board_of_the_mainline_or_a_version_gives_its_tags_without_rev() {
    let (f, client) = ready();
    let d = new_design(f, client);
    save(f, client, &d.project, &d.changeset, &d.doc);
    merge(f, client, &d.project, &d.changeset);
    let mainline = mainline_of(f, client, &d.project, &d.doc);
    let later = new_changeset(f, client, &d.project);
    import_design(
        f,
        client,
        &d.project,
        &later,
        &d.doc,
        &design_tags(&d.ids, NEW_TOTAL, HELLO),
    );
    let newer = id_of(&save(f, client, &d.project, &later, &d.doc));
    let cart = id(&d.ids, "Cart");

    let answer = must(
        board_response(f, client, &agent(f), &d.project, &d.doc, &cart, ""),
        200,
        "read a board of the mainline",
    );
    assert_eq!(answer["version_id"], mainline.as_str(), "{answer}");
    assert_eq!(answer["board_id"], cart.as_str(), "{answer}");
    assert_eq!(
        answer["page_id"],
        id(&d.ids, "Checkout").as_str(),
        "{answer}"
    );
    let tags = answer["tags"].as_str().expect("the board's tags");
    assert_board_alone(tags, &cart, &[TOTAL], &[NEW_TOTAL, "Betala nu"]);
    assert!(
        !tags.contains("rev="),
        "a version's tags carry no rev: {tags}"
    );

    // The human reads it too.
    let by_human = must(
        board_response(f, client, &human(f), &d.project, &d.doc, &cart, ""),
        200,
        "the human reads a board of the mainline",
    );
    assert_eq!(by_human["tags"], answer["tags"]);

    let answer = must(
        board_response(
            f,
            client,
            &agent(f),
            &d.project,
            &d.doc,
            &cart,
            &format!("?version={newer}"),
        ),
        200,
        "read a board of a saved version",
    );
    assert_eq!(answer["version_id"], newer.as_str(), "{answer}");
    let tags = answer["tags"].as_str().expect("the board's tags");
    assert_board_alone(tags, &cart, &[NEW_TOTAL], &[TOTAL]);
    assert!(
        !tags.contains("rev="),
        "a version's tags carry no rev: {tags}"
    );

    // The draft of the later change set, with revs.
    let answer = must(
        draft_board_response(f, client, &d.project, &later, &d.doc, &cart),
        200,
        "read a board of the later draft",
    );
    let tags = answer["tags"].as_str().expect("the board's tags");
    assert_board_alone(tags, &cart, &[NEW_TOTAL], &[TOTAL]);
    assert!(
        opening_tags(tags).iter().all(|tag| rev_of(tag).is_some()),
        "{tags}"
    );
}

#[test]
fn an_id_that_names_no_board_of_the_doc_is_refused() {
    let (f, client) = ready();
    let d = new_design(f, client);
    save(f, client, &d.project, &d.changeset, &d.doc);
    merge(f, client, &d.project, &d.changeset);
    let later = new_changeset(f, client, &d.project);
    import_design(
        f,
        client,
        &d.project,
        &later,
        &d.doc,
        &design_tags(&d.ids, TOTAL, HELLO),
    );
    for node in [
        UNKNOWN_NODE.to_string(),
        id(&d.ids, "Checkout"),
        id(&d.ids, "Summary"),
        id(&d.ids, "Pay"),
    ] {
        assert_error(
            &board_response(f, client, &agent(f), &d.project, &d.doc, &node, ""),
            404,
            "unknown_board",
        );
        assert_error(
            &draft_board_response(f, client, &d.project, &later, &d.doc, &node),
            404,
            "unknown_board",
        );
    }
}

#[test]
fn reading_a_board_of_what_is_no_design_or_has_no_such_version_is_refused() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let cart = id(&d.ids, "Cart");

    // The doc has no mainline version until its change set merges.
    assert_error(
        &board_response(f, client, &agent(f), &d.project, &d.doc, &cart, ""),
        409,
        "no_mainline_version",
    );

    // A version that is not one of the doc's.
    assert_error(
        &board_response(
            f,
            client,
            &agent(f),
            &d.project,
            &d.doc,
            &cart,
            &format!("?version={UNKNOWN_ID}"),
        ),
        404,
        "not_found",
    );

    // An HTML doc has no boards.
    let html = create_doc(f, client, &d.project, &d.changeset, "html", "prd");
    assert_error(
        &draft_board_response(f, client, &d.project, &d.changeset, &html, &cart),
        400,
        "not_a_design",
    );
}

// ---------------------------------------------------------------------------
// US-058: reading a version board by board
// ---------------------------------------------------------------------------

fn outline_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    version: &str,
) -> Response {
    call(
        client,
        "GET",
        &format!(
            "{}/versions/{version}?view=outline",
            project_base(f, project)
        ),
        auth,
        None,
    )
}

/// A node's JSON without its children.
fn own_json(node: &Value) -> Value {
    let mut own = node.clone();
    if let Some(object) = own.as_object_mut() {
        object.remove("children");
    }
    own
}

fn children_of(node: &Value) -> Vec<Value> {
    node["children"].as_array().cloned().unwrap_or_default()
}

/// The board `board` of the whole read of a version.
fn whole_board(version: &Value, board: &str) -> Value {
    version["pages"]
        .as_array()
        .unwrap_or_else(|| panic!("the whole read has pages: {version:#}"))
        .iter()
        .flat_map(children_of)
        .find(|node| node["id"] == board)
        .unwrap_or_else(|| panic!("the whole read has the board {board}: {version:#}"))
}

#[test]
fn the_outline_of_a_design_version_lists_its_pages_and_boards_without_their_content() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let saved = save(f, client, &d.project, &d.changeset, &d.doc);
    let version_id = id_of(&saved);
    let whole = read_version(f, client, &d.project, &version_id);

    let response = outline_response(f, client, &agent(f), &d.project, &version_id);
    let text = response.text();
    let outline = must(response, 200, "read the version's outline")["version"].clone();
    assert_eq!(outline["id"], version_id.as_str(), "{outline:#}");
    assert!(
        outline.get("pages").is_none_or(Value::is_null),
        "the outline has no pages body: {outline:#}"
    );
    assert!(
        outline.get("tags").is_none_or(Value::is_null),
        "the outline has no tags: {outline:#}"
    );
    assert_eq!(
        outline["manifest"], whole["manifest"],
        "the outline carries the version's manifest"
    );

    let pages = outline["outline"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer has an `outline` of pages: {outline:#}"))
        .clone();
    let whole_pages = whole["pages"]
        .as_array()
        .expect("the whole read's pages")
        .clone();
    assert_eq!(
        pages.iter().map(id_of).collect::<Vec<_>>(),
        [id(&d.ids, "Checkout"), id(&d.ids, "Account")],
        "the pages in order: {outline:#}"
    );
    for (page, whole_page) in pages.iter().zip(&whole_pages) {
        assert_eq!(
            own_json(page),
            own_json(whole_page),
            "a page of the outline is the page's own JSON, its name included"
        );
        let boards = children_of(page);
        let whole_boards = children_of(whole_page);
        assert_eq!(
            boards.iter().map(id_of).collect::<Vec<_>>(),
            whole_boards.iter().map(id_of).collect::<Vec<_>>(),
            "every board of the page, in order"
        );
        for (board, whole_board) in boards.iter().zip(&whole_boards) {
            assert!(
                children_of(board).is_empty(),
                "a board of the outline is a placeholder without its children: {board}"
            );
            assert_eq!(
                own_json(board),
                own_json(whole_board),
                "a board of the outline has the board's own name, box, layout and fills"
            );
        }
    }

    // Nothing below a board is in the answer, `nodes` included.
    for name in ["Summary", "Total", "Shipping", "Pay", "Hello"] {
        assert!(
            !text.contains(&id(&d.ids, name)),
            "the outline holds no ID of {name}, which is below a board: {text}"
        );
    }
    for content in [TOTAL, "Frakt ingår", "Betala nu", HELLO] {
        assert!(
            !text.contains(content),
            "the outline holds no content of a board ({content:?}): {text}"
        );
    }
}

#[test]
fn the_human_reads_the_outline_too() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let saved = save(f, client, &d.project, &d.changeset, &d.doc);
    let outline = must(
        outline_response(f, client, &human(f), &d.project, &id_of(&saved)),
        200,
        "the viewer's session reads the outline",
    )["version"]
        .clone();
    assert_eq!(
        outline["outline"].as_array().map(Vec::len),
        Some(2),
        "the outline lists both pages: {outline:#}"
    );
}

#[test]
fn a_board_of_a_version_answers_its_subtree_as_json() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let saved = save(f, client, &d.project, &d.changeset, &d.doc);
    let version_id = id_of(&saved);
    let whole = read_version(f, client, &d.project, &version_id);
    let query = format!("?version={version_id}");

    for (board, page) in [
        ("Cart", "Checkout"),
        ("Payment", "Checkout"),
        ("Profile", "Account"),
    ] {
        let board_id = id(&d.ids, board);
        let body = must(
            board_response(f, client, &human(f), &d.project, &d.doc, &board_id, &query),
            200,
            "read a board of the version",
        );
        assert_eq!(body["page_id"], id(&d.ids, page).as_str(), "{body}");
        assert_eq!(
            body["node"],
            whole_board(&whole, &board_id),
            "the board {board} and its subtree as JSON, as the whole read holds it: {body:#}"
        );
    }

    // The mainline's board answers its node too.
    merge(f, client, &d.project, &d.changeset);
    let mainline = mainline_of(f, client, &d.project, &d.doc);
    assert_eq!(mainline, version_id);
    let cart = id(&d.ids, "Cart");
    let body = must(
        board_response(f, client, &agent(f), &d.project, &d.doc, &cart, ""),
        200,
        "read a board of the mainline",
    );
    assert_eq!(body["node"], whole_board(&whole, &cart), "{body:#}");
}
