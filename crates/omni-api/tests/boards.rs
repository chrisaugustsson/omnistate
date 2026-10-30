//! US-030: pages, boards and chunked storage through the API. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; the tags are those of
//! docs/formats/design-tags.md):
//!
//! - A design doc holds pages, and pages hold boards. Every page and every
//!   board is its own chunk in RustFS, at `ws/{workspace_id}/chunks/{chunk}`,
//!   where `chunk` is the BLAKE3 hash of the stored bytes in lowercase hex (64
//!   characters).
//! - The chunk index is the tenant table `chunks`, keyed by `(workspace_id,
//!   hash)`: one row per chunk of a workspace. The columns without a default
//!   are `workspace_id` and `hash`. Its rows are never changed, so the API role
//!   has no UPDATE privilege on it (crates/omni-core/tests/tenancy.rs).
//! - Unchanged boards keep their chunk: a later head lists the same chunk for
//!   them.
//! - The node index (`node_index`) is stored per chunk: the rows of a board's
//!   chunk are exactly the board and its descendants, written once and shared
//!   by every head that uses the chunk.
//! - `GET /docs/{doc_id}/boards/{board_id}` (both) reads one board of the head.
//!   The answer is 200 `{"doc_id", "page_id", "board_id", "tags", "node"}`:
//!   `tags` is the board alone in the tag format, starting with its `<Board
//!   id="...">` tag, with no `<Page>`, and `node` the board and its subtree as
//!   JSON, as the whole read's `pages` hold it. An ID that names no board of
//!   the doc (an unknown node, a page, a frame) is 404 `{"error":
//!   "unknown_board"}` and a doc of another kind 400 `{"error":
//!   "not_a_design"}`.
//! - `GET /docs/{doc_id}/html?view=outline` of a design answers its head
//!   without `pages` and `tags`, and with `"outline"`: its pages in order, each
//!   page as its own JSON with its boards in order under `children`, every
//!   board as its own JSON (its name, box, layout and fills) without its
//!   children, and `"boards"`: the boards in page order, each `{"id", "chunk"}`.
//!   A width or height that is not fixed (a board that hugs its content) is the
//!   fixed size that the board is laid out in with its content, so that its
//!   placeholder takes the board's room on the canvas; fixed sizes are as
//!   stored. Nothing below a board (no ID, no content) is in the answer.

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

const TEST_DB: &str = "omni_us030_boards";
const TEST_BUCKET: &str = "omni-us030-boards";
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
        // Heads keep their content in RustFS, so the API gets a real bucket.
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

/// Creates an empty design doc; a new design has one page.
fn new_design_doc(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", project_base(f, project)),
                &agent(f),
                Some(&json!({ "kind": "design", "pillar": "ui", "title": unique("doc") })),
            ),
            201,
            "create a design doc",
        )["doc"],
    )
}

/// Applies a batch of operations to the doc's head.
fn edit(f: &Fixture, client: &Client, project: &str, doc: &str, ops: &Value) -> Value {
    must(
        call(
            client,
            "POST",
            &format!("{}/docs/{doc}/ops", project_base(f, project)),
            &agent(f),
            Some(&json!({ "ops": ops })),
        ),
        200,
        "edit the doc's head",
    )
}

/// The whole read of the doc's head.
fn read_head(f: &Fixture, client: &Client, project: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/docs/{doc}/html", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "read the doc's head",
    )
}

fn first_page(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    id_of(&read_head(f, client, project, doc)["pages"][0])
}

/// The outline read of the doc's head.
fn read_outline(f: &Fixture, client: &Client, project: &str, doc: &str) -> Response {
    call(
        client,
        "GET",
        &format!("{}/docs/{doc}/html?view=outline", project_base(f, project)),
        &agent(f),
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

fn id(ids: &Ids, name: &str) -> String {
    ids.get(name)
        .unwrap_or_else(|| panic!("the node {name} has an ID: {ids:?}"))
        .clone()
}

const TOTAL: &str = "Summa 10 kr";
const NEW_TOTAL: &str = "Summa 12 kr";
const HELLO: &str = "Hej Alice";

/// A project with a design doc of two pages: its first page, here Checkout,
/// with the boards Cart and Payment, and Account with the board Profile.
struct Design {
    project: String,
    doc: String,
    ids: Ids,
}

fn new_design(f: &Fixture, client: &Client) -> Design {
    let project = new_project(f, client);
    let doc = new_design_doc(f, client, &project);
    let page = first_page(f, client, &project, &doc);
    let cart = format!(
        r#"<Board name="Cart" x={{0}} y={{0}} width={{360}} height={{640}} dir="col" gap={{8}}>
  <Frame name="Summary" dir="col" gap={{4}}>
    <Text name="Total" content="{TOTAL}" fontFamily="Inter" fontSize={{16}}/>
    <Text name="Shipping" content="Frakt ingår" fontFamily="Inter" fontSize={{14}}/>
  </Frame>
</Board>"#
    );
    let payment = r#"<Board name="Payment" x={400} y={0} width={360} height={640} dir="col">
  <Text name="Pay" content="Betala nu" fontFamily="Inter" fontSize={16}/>
</Board>"#;
    let account = format!(
        r#"<Page name="Account">
  <Board name="Profile" x={{0}} y={{0}} width={{360}} height={{640}} dir="col">
    <Text name="Hello" content="{HELLO}" fontFamily="Inter" fontSize={{16}}/>
  </Board>
</Page>"#
    );
    edit(
        f,
        client,
        &project,
        &doc,
        &json!([
            { "op": "insert", "parent": page, "position": { "after": null }, "content": payment },
            { "op": "insert", "parent": page, "position": { "after": null }, "content": cart },
            { "op": "insert", "parent": null, "position": { "after": page }, "content": account }
        ]),
    );
    let mut ids = ids_in(&read_head(f, client, &project, &doc)["pages"]);
    ids.insert("Checkout".to_string(), page);
    assert_eq!(
        ids.len(),
        11,
        "the first page's own name, then 10 nodes: {ids:?}"
    );
    Design { project, doc, ids }
}

/// A design doc with the boards Cart
/// (Frame Summary holding Text Total) and Payment inserted into its head.
struct LiveDesign {
    project: String,
    doc: String,
    page: String,
    cart: String,
    summary: String,
    total: String,
}

fn live_design(f: &Fixture, client: &Client) -> LiveDesign {
    let project = new_project(f, client);
    let base = project_base(f, &project);
    let doc = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/docs"),
                &agent(f),
                Some(&json!({ "kind": "design", "pillar": "ui", "title": unique("doc") })),
            ),
            201,
            "create a design",
        )["doc"],
    );
    let head = must(
        call(
            client,
            "GET",
            &format!("{base}/docs/{doc}/html"),
            &agent(f),
            None,
        ),
        200,
        "read the doc's head",
    );
    let page = id_of(&head["pages"][0]);
    let cart = format!(
        r#"<Board name="Cart" x={{0}} y={{0}} width={{360}} height={{640}} dir="col" gap={{8}}>
  <Frame name="Summary" dir="col" gap={{4}}>
    <Text name="Total" content="{TOTAL}" fontFamily="Inter" fontSize={{16}}/>
  </Frame>
</Board>"#
    );
    let payment = r#"<Board name="Payment" x={400} y={0} width={360} height={640} dir="col">
  <Text name="Pay" content="Betala nu" fontFamily="Inter" fontSize={16}/>
</Board>"#;
    let insert = |content: &str| json!({ "op": "insert", "parent": page, "position": { "after": null }, "content": content });
    let answer = must(
        call(
            client,
            "POST",
            &format!("{base}/docs/{doc}/ops"),
            &agent(f),
            Some(&json!({ "ops": [insert(&cart), insert(payment)] })),
        ),
        200,
        "insert the boards into the head",
    );
    let created: Vec<String> = answer["results"][0]["created"]
        .as_array()
        .unwrap_or_else(|| panic!("the insert lists what it created: {answer}"))
        .iter()
        .map(|id| id.as_str().expect("an ID").to_string())
        .collect();
    assert_eq!(created.len(), 3, "Cart, Summary and Total: {answer}");
    LiveDesign {
        project,
        doc,
        page,
        cart: created[0].clone(),
        summary: created[1].clone(),
        total: created[2].clone(),
    }
}

fn chunk_of(entry: &Value) -> String {
    let chunk = entry["chunk"]
        .as_str()
        .unwrap_or_else(|| panic!("a board names its chunk: {entry}"))
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

/// The chunk of every board of the head, by board ID.
fn chunks_by_board(f: &Fixture, client: &Client, design: &Design) -> HashMap<String, String> {
    let outline = must(
        read_outline(f, client, &design.project, &design.doc),
        200,
        "read the head's outline",
    );
    outline["boards"]
        .as_array()
        .unwrap_or_else(|| panic!("the outline lists its boards: {outline:#}"))
        .iter()
        .map(|board| (id_of(board), chunk_of(board)))
        .collect()
}

/// The chunk of the board named `name`.
fn chunk_named(chunks: &HashMap<String, String>, ids: &Ids, name: &str) -> String {
    chunks
        .get(&id(ids, name))
        .unwrap_or_else(|| panic!("the head lists {name}: {chunks:?}"))
        .clone()
}

/// Every opening tag of `tags`, one per line as `tags::print` writes them.
fn opening_tags(tags: &str) -> Vec<&str> {
    tags.lines()
        .map(str::trim)
        .filter(|line| line.starts_with('<') && !line.starts_with("</"))
        .collect()
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
// Pages, boards and their chunks
// ---------------------------------------------------------------------------

#[test]
fn the_chunk_index_is_keyed_by_workspace_and_hash() {
    let (f, _) = ready();
    let hash = blake3::hash(unique("chunk").as_bytes())
        .to_hex()
        .to_string();

    // The key is the pair: the same hash may be indexed in another workspace,
    // but not twice in one.
    let insert = |workspace: &str| {
        block(async {
            let mut conn = admin_connection().await;
            sqlx::query("insert into chunks (workspace_id, hash) values ($1::uuid, $2)")
                .bind(workspace)
                .bind(&hash)
                .execute(&mut conn)
                .await
                .map(|_| ())
        })
    };
    insert(&f.workspace).unwrap_or_else(|e| panic!("a workspace indexes a new hash: {e}"));
    insert(&f.other_workspace)
        .unwrap_or_else(|e| panic!("another workspace may index the same hash: {e}"));
    let twice = insert(&f.workspace).expect_err("a workspace indexes a hash once");
    assert!(
        twice.to_string().contains("duplicate key"),
        "the second row is refused as a duplicate of (workspace_id, hash): {twice}"
    );
}

#[test]
fn the_node_index_is_stored_per_chunk_and_shared_by_the_heads_that_use_it() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let first_chunks = chunks_by_board(f, client, &d);
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

    edit(
        f,
        client,
        &d.project,
        &d.doc,
        &json!([{
            "op": "set",
            "node": id(&d.ids, "Total"),
            "rev": 1,
            "properties": { "content": NEW_TOTAL }
        }]),
    );
    let second_chunks = chunks_by_board(f, client, &d);
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
        "the shared chunk's nodes are indexed once, for both heads"
    );
    let new_cart = chunk_named(&second_chunks, &d.ids, "Cart");
    assert_ne!(new_cart, cart, "the changed board has a new chunk");
    assert_eq!(
        indexed_nodes(f, &new_cart),
        names(&["Cart", "Summary", "Total", "Shipping"])
    );
}

// ---------------------------------------------------------------------------
// Reading one board
// ---------------------------------------------------------------------------

#[test]
fn reading_a_board_of_the_head_gives_that_board_alone() {
    let (f, client) = ready();
    let d = live_design(f, client);
    for auth in [agent(f), human(f)] {
        let answer = must(
            board_response(f, client, &auth, &d.project, &d.doc, &d.cart, ""),
            200,
            "read a board of the head",
        );
        assert_eq!(answer["board_id"], d.cart.as_str(), "{answer}");
        assert_eq!(answer["page_id"], d.page.as_str(), "{answer}");
        let tags = answer["tags"].as_str().expect("the board's tags");
        assert_board_alone(
            tags,
            &d.cart,
            &[&d.summary, &d.total, TOTAL],
            &["Betala nu"],
        );
    }
}

#[test]
fn an_id_that_names_no_board_of_the_doc_is_refused() {
    let (f, client) = ready();
    let d = live_design(f, client);
    for node in [UNKNOWN_NODE, &d.page, &d.summary, &d.total] {
        assert_error(
            &board_response(f, client, &agent(f), &d.project, &d.doc, node, ""),
            404,
            "unknown_board",
        );
    }
}

#[test]
fn reading_a_board_of_a_doc_that_is_no_design_is_refused() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let html = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", project_base(f, &project)),
                &agent(f),
                Some(&json!({ "kind": "html", "pillar": "spec", "title": unique("doc") })),
            ),
            201,
            "create an HTML doc",
        )["doc"],
    );
    assert_error(
        &board_response(f, client, &agent(f), &project, &html, UNKNOWN_NODE, ""),
        400,
        "not_a_design",
    );
}

// ---------------------------------------------------------------------------
// Reading a design board by board
// ---------------------------------------------------------------------------

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

/// The board `board` of the whole read of a head.
fn whole_board(head: &Value, board: &str) -> Value {
    head["pages"]
        .as_array()
        .unwrap_or_else(|| panic!("the whole read has pages: {head:#}"))
        .iter()
        .flat_map(children_of)
        .find(|node| node["id"] == board)
        .unwrap_or_else(|| panic!("the whole read has the board {board}: {head:#}"))
}

#[test]
fn the_outline_of_a_design_lists_its_pages_and_boards_without_their_content() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let whole = read_head(f, client, &d.project, &d.doc);

    let response = read_outline(f, client, &d.project, &d.doc);
    let text = response.text();
    let outline = must(response, 200, "read the head's outline");
    assert_eq!(outline["doc_id"], d.doc.as_str(), "{outline:#}");
    assert!(
        outline.get("pages").is_none_or(Value::is_null),
        "the outline has no pages body: {outline:#}"
    );
    assert!(
        outline.get("tags").is_none_or(Value::is_null),
        "the outline has no tags: {outline:#}"
    );
    let listed: Vec<String> = outline["boards"]
        .as_array()
        .unwrap_or_else(|| panic!("the outline lists its boards: {outline:#}"))
        .iter()
        .map(id_of)
        .collect();
    assert_eq!(
        listed,
        [
            id(&d.ids, "Cart"),
            id(&d.ids, "Payment"),
            id(&d.ids, "Profile")
        ],
        "the outline carries the chunk of each board, in page order: {outline:#}"
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

    // Nothing below a board is in the answer.
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
fn a_placeholder_of_a_board_that_hugs_its_content_has_the_box_of_that_content() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = new_design_doc(f, client, &project);
    let page = first_page(f, client, &project, &doc);
    // Status hugs its width: 12 + 60 + 12 = 84 wide, 32 tall as fixed.
    // Outline hugs both: 10 + 120 + 10 = 140 wide, 10 + 40 + 8 + 30 + 10 = 98 tall.
    let status = r##"<Board name="Status" x={2000} y={0} width="hug" height={32} dir="row" pad={{left: 12, right: 12}}>
  <Frame name="Dot" width={60} height={20}/>
</Board>"##;
    let hugging = r##"<Board name="Outline" x={0} y={300000} width="hug" height="hug" dir="col" gap={8} pad={10}>
  <Frame name="Head" width={120} height={40}/>
  <Frame name="Body" width={80} height={30}/>
</Board>"##;
    edit(
        f,
        client,
        &project,
        &doc,
        &json!([
            { "op": "insert", "parent": page, "position": { "after": null }, "content": hugging },
            { "op": "insert", "parent": page, "position": { "after": null }, "content": status }
        ]),
    );
    let whole = read_head(f, client, &project, &doc);
    let outline = must(
        read_outline(f, client, &project, &doc),
        200,
        "read the head's outline",
    );
    let boards = children_of(&outline["outline"][0]);
    let whole_boards = children_of(&whole["pages"][0]);
    assert_eq!(boards.len(), 2, "{outline:#}");
    for (board, whole_board, width, height) in [
        (&boards[0], &whole_boards[0], 84, 32),
        (&boards[1], &whole_boards[1], 140, 98),
    ] {
        assert_eq!(board["width"], json!({"fixed": width}), "{board}");
        assert_eq!(board["height"], json!({"fixed": height}), "{board}");
        let mut rest = own_json(board);
        let mut whole_rest = own_json(whole_board);
        for key in ["width", "height"] {
            rest.as_object_mut().map(|object| object.remove(key));
            whole_rest.as_object_mut().map(|object| object.remove(key));
        }
        assert_eq!(
            rest, whole_rest,
            "the placeholder is otherwise the board's own JSON: its place, name and fills"
        );
    }
}

#[test]
fn a_board_answers_its_subtree_as_json() {
    let (f, client) = ready();
    let d = new_design(f, client);
    let whole = read_head(f, client, &d.project, &d.doc);

    for (board, page) in [
        ("Cart", "Checkout"),
        ("Payment", "Checkout"),
        ("Profile", "Account"),
    ] {
        let board_id = id(&d.ids, board);
        let body = must(
            board_response(f, client, &human(f), &d.project, &d.doc, &board_id, ""),
            200,
            "read a board of the head",
        );
        assert_eq!(body["page_id"], id(&d.ids, page).as_str(), "{body}");
        assert_eq!(
            body["node"],
            whole_board(&whole, &board_id),
            "the board {board} and its subtree as JSON, as the whole read holds it: {body:#}"
        );
    }
}
