//! US-031: editing designs with operations, through the API. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint two API keys through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; the batch route is that of
//! tests/live_designs.rs, the board reads those of tests/boards.rs, and the
//! tags those of docs/formats/design-tags.md):
//!
//! - `POST /docs/{doc_id}/ops` (agent) edits the head of a design doc with the
//!   operations of docs/formats/operations.md, on the design's nodes: `insert`
//!   (`parent`, `position`, `content`), `replace` (`node`, `rev`, `content`),
//!   `set` (`node`, `rev`, `properties`), `move` (`node`, `rev`, `parent`,
//!   `position`) and `delete` (`node`, `rev`). `content` is one tag in the tag
//!   format with its children. A batch is all or nothing and answers 200
//!   `{"results": [{"node", "revision", "created", "removed"}]}`, one result
//!   per operation.
//!   - An insert adds a new subtree under `parent` at `position`. Every node of
//!     the content is new, so a tag of the content that carries an `id` is
//!     refused: the server assigns the IDs and answers them in `created`, in
//!     document order. A new node is at revision 1.
//!   - A set changes properties of the node, named as the attributes of its
//!     tag; an attribute that the node's tag does not take, and `id`, are
//!     refused, naming it.
//!   - A move puts the node under a new parent; only the node's revision rises.
//!   - An ID that is in no node of the head (an operation's node or parent, an
//!     `id` in a replace's content, the `component` of an `<Instance>`) is 400
//!     `{"error": "invalid_operation", "index", "message"}`, named in the
//!     message. A refused batch changes and logs nothing.
//! - Batches of different agents on different boards of one design, sent at the
//!   same time, all apply.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::HashMap;
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us031_design_edits";
const TEST_BUCKET: &str = "omni-us031-design-edits";
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

async fn ensure_bucket() {
    let bucket = Bucket::new(TEST_BUCKET, s3_region(), s3_credentials())
        .expect("bucket settings")
        .with_path_style();
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
    /// Two API keys of the workspace.
    key: String,
    other_key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us031-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

/// Who sends a request: an agent (an API key with its run and role), or the
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

/// The agent that builds the designs and takes the claims in these tests.
fn holder(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-1",
        role: "designer",
    }
}

/// Another agent, with a key of its own.
fn other(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.other_key,
        run: "run-2",
        role: "illustrator",
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
        response.json()["error"],
        code,
        "the error code: {}",
        response.text()
    );
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

// ---------------------------------------------------------------------------
// The design
// ---------------------------------------------------------------------------

/// Node IDs by node name. Every name in a design is used once.
type Ids = HashMap<String, String>;

fn collect_ids(nodes: &Value, ids: &mut Ids) {
    for node in nodes.as_array().unwrap_or(&Vec::new()) {
        if let (Some(name), Some(id)) = (node["name"].as_str(), node["id"].as_str()) {
            ids.insert(name.to_string(), id.to_string());
        }
        collect_ids(&node["children"], ids);
    }
}

const CART: &str = r##"<Board name="Cart" x={0} y={0} width={360} height={640} dir="col" gap={8}>
  <Frame name="Summary" dir="col" gap={4}>
    <Text name="Total" content="Summa 10 kr" fontFamily="Inter" fontSize={16}/>
    <Text name="Shipping" content="Frakt ingår" fontFamily="Inter" fontSize={14}/>
  </Frame>
  <Frame name="Actions" dir="row" gap={8}/>
</Board>"##;

const PAYMENT: &str = r##"<Board name="Payment" x={400} y={0} width={360} height={640} dir="col">
  <Text name="Pay" content="Betala nu" fontFamily="Inter" fontSize={16}/>
</Board>"##;

/// A project with a design doc whose first page, here Checkout, has the
/// boards of `CART` and `PAYMENT`.
struct Design {
    project: String,
    doc: String,
    ids: Ids,
}

impl Design {
    fn base(&self, f: &Fixture) -> String {
        format!("/api/v1/w/{}/projects/{}", f.workspace, self.project)
    }

    fn doc_path(&self, f: &Fixture) -> String {
        format!("{}/docs/{}", self.base(f), self.doc)
    }

    fn id(&self, name: &str) -> String {
        self.ids
            .get(name)
            .unwrap_or_else(|| panic!("the node {name} has an ID: {:?}", self.ids))
            .clone()
    }
}

fn new_design(f: &Fixture, client: &Client) -> Design {
    let project = str_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &holder(f),
                Some(&json!({ "name": unique("design-edits") })),
            ),
            201,
            "create a project",
        )["project"],
        "id",
    )
    .to_string();
    let base = format!("/api/v1/w/{}/projects/{project}", f.workspace);
    let doc = str_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/docs"),
                &holder(f),
                Some(&json!({ "kind": "design", "pillar": "ui", "title": unique("design") })),
            ),
            201,
            "create a design doc",
        )["doc"],
        "id",
    )
    .to_string();
    let mut design = Design {
        project,
        doc,
        ids: Ids::new(),
    };
    let page = str_of(&head_pages(f, client, &design)[0], "id").to_string();
    applied(
        f,
        client,
        &design,
        json!([insert(&page, None, PAYMENT), insert(&page, None, CART)]),
    );
    collect_ids(&head_pages(f, client, &design), &mut design.ids);
    design.ids.insert("Checkout".to_string(), page);
    assert_eq!(
        design.ids.len(),
        9,
        "the first page's own name, then 8 nodes: {:?}",
        design.ids
    );
    design
}

/// Sends a batch of operations as `auth`.
fn edit(f: &Fixture, client: &Client, auth: &Auth, design: &Design, ops: Value) -> Response {
    call(
        client,
        "POST",
        &format!("{}/ops", design.doc_path(f)),
        auth,
        Some(&json!({ "ops": ops })),
    )
}

/// Sends a batch of the holder's that must apply; returns its results.
fn applied(f: &Fixture, client: &Client, design: &Design, ops: Value) -> Vec<Value> {
    let answer = must(
        edit(f, client, &holder(f), design, ops),
        200,
        "apply the batch",
    );
    answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer lists results: {answer}"))
        .clone()
}

/// One board of the head in the tag format.
fn board_tags(f: &Fixture, client: &Client, design: &Design, board: &str) -> String {
    let answer = must(
        call(
            client,
            "GET",
            &format!("{}/boards/{board}", design.doc_path(f)),
            &holder(f),
            None,
        ),
        200,
        "read a board of the head",
    );
    str_of(&answer, "tags").to_string()
}

/// The pages of the head as JSON nodes.
fn head_pages(f: &Fixture, client: &Client, design: &Design) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/html", design.doc_path(f)),
            &holder(f),
            None,
        ),
        200,
        "read the head",
    )["pages"]
        .clone()
}

/// The JSON node with this ID among `nodes` and their descendants.
fn find_node(nodes: &Value, id: &str) -> Option<Value> {
    for node in nodes.as_array()? {
        if node["id"] == id {
            return Some(node.clone());
        }
        if let Some(found) = find_node(&node["children"], id) {
            return Some(found);
        }
    }
    None
}

/// The IDs of a JSON node's children, in order.
fn child_ids(node: &Value) -> Vec<String> {
    node["children"]
        .as_array()
        .map(|children| {
            children
                .iter()
                .filter_map(|child| child["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn ids_of(result: &Value, field: &str) -> Vec<String> {
    result[field]
        .as_array()
        .unwrap_or_else(|| panic!("the result lists `{field}`: {result}"))
        .iter()
        .map(|id| id.as_str().expect("an ID").to_string())
        .collect()
}

fn text_tag(name: &str, content: &str) -> String {
    format!(r#"<Text name="{name}" content="{content}" fontFamily="Inter" fontSize={{14}}/>"#)
}

fn insert(parent: &str, after: Option<&str>, content: &str) -> Value {
    json!({ "op": "insert", "parent": parent, "position": { "after": after }, "content": content })
}

fn set(node: &str, rev: u64, properties: Value) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": properties })
}

fn set_content(node: &str, rev: u64, content: &str) -> Value {
    set(node, rev, json!({ "content": content }))
}

fn move_to(node: &str, rev: u64, parent: &str, after: Option<&str>) -> Value {
    json!({ "op": "move", "node": node, "rev": rev, "parent": parent, "position": { "after": after } })
}

fn delete(node: &str, rev: u64) -> Value {
    json!({ "op": "delete", "node": node, "rev": rev })
}

/// The number of rows in the log of the doc's writes.
fn log_len(design: &Design) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from doc_changes where doc_id = $1::uuid")
            .bind(&design.doc)
            .fetch_one(&mut conn)
            .await
            .expect("count the doc's log")
    })
}

/// Asserts that a refused operation names `id` in its message.
fn assert_invalid_naming(response: &Response, index: u64, id: &str) {
    assert_eq!(response.status, 400, "body: {}", response.text());
    let answer = response.json();
    assert_eq!(answer["error"], "invalid_operation", "{answer}");
    assert_eq!(answer["index"], index, "{answer}");
    let message = str_of(&answer, "message");
    assert!(message.contains(id), "the message names {id}: {message:?}");
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

#[test]
fn an_insert_puts_a_new_board_under_a_page() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let content = format!(
        "<Board name=\"Receipt\" x={{800}} y={{0}} width={{360}} height={{640}} dir=\"col\">\n  {}\n</Board>",
        text_tag("Thanks", "Tack för ditt köp")
    );
    let results = applied(
        f,
        client,
        &design,
        json!([insert(
            &design.id("Checkout"),
            Some(&design.id("Payment")),
            &content
        )]),
    );
    let created = ids_of(&results[0], "created");
    assert_eq!(created.len(), 2, "{results:?}");
    let board = &created[0];

    let pages = head_pages(f, client, &design);
    let page = find_node(&pages, &design.id("Checkout")).expect("the head has its page");
    assert_eq!(
        child_ids(&page),
        vec![design.id("Cart"), design.id("Payment"), board.clone()],
        "the new board is the page's third board: {page}"
    );
    let tags = board_tags(f, client, &design, board);
    assert!(
        tags.trim_start()
            .starts_with(&format!(r#"<Board id="{board}""#)),
        "the new board reads as a board: {tags}"
    );
    assert!(tags.contains(r#"name="Thanks""#), "{tags}");
    // A new node is at revision 1: a batch that expects that applies.
    applied(
        f,
        client,
        &design,
        json!([set_content(&created[1], 1, "Tack")]),
    );
}

#[test]
fn an_insert_whose_content_carries_an_id_is_refused_since_the_server_assigns_ids() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let before = log_len(&design);
    let response = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([insert(
            &design.id("Actions"),
            None,
            r#"<Text id="abcdefgh" name="Mine" content="Mitt eget ID" fontFamily="Inter" fontSize={14}/>"#
        )]),
    );
    assert_error(&response, 400, "invalid_operation");
    assert_eq!(log_len(&design), before, "a refused insert logs nothing");

    // Without the ID, the server gives the node one of its own.
    let results = applied(
        f,
        client,
        &design,
        json!([insert(
            &design.id("Actions"),
            None,
            r#"<Text name="Mine" content="Mitt eget ID" fontFamily="Inter" fontSize={14}/>"#
        )]),
    );
    let created = ids_of(&results[0], "created");
    assert_eq!(created.len(), 1, "{results:?}");
    assert_ne!(created[0], "abcdefgh", "the server assigns the ID");
}

#[test]
fn a_set_of_a_property_that_the_tag_does_not_take_is_refused_naming_it() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let unknown = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([set(&design.id("Total"), 1, json!({ "colour": "#ff0000" }))]),
    );
    assert_invalid_naming(&unknown, 0, "colour");
    let id = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([set(&design.id("Total"), 1, json!({ "id": "abcdefgh" }))]),
    );
    assert_error(&id, 400, "invalid_operation");
    // Nothing changed: the node is still at revision 1.
    applied(
        f,
        client,
        &design,
        json!([set_content(&design.id("Total"), 1, "Summa 12 kr")]),
    );
}

#[test]
fn a_move_puts_a_node_under_a_new_parent_and_raises_only_its_revision() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let results = applied(
        f,
        client,
        &design,
        json!([move_to(
            &design.id("Shipping"),
            1,
            &design.id("Actions"),
            None
        )]),
    );
    assert_eq!(results[0]["node"], design.id("Shipping"), "{results:?}");
    assert_eq!(results[0]["revision"], 2, "{results:?}");

    let pages = head_pages(f, client, &design);
    let summary = find_node(&pages, &design.id("Summary")).expect("Summary");
    let actions = find_node(&pages, &design.id("Actions")).expect("Actions");
    assert_eq!(child_ids(&summary), vec![design.id("Total")], "{summary}");
    assert_eq!(
        child_ids(&actions),
        vec![design.id("Shipping")],
        "{actions}"
    );
    // The old and the new parent are still at revision 1.
    applied(
        f,
        client,
        &design,
        json!([
            set(&design.id("Summary"), 1, json!({ "gap": 6 })),
            set(&design.id("Actions"), 1, json!({ "gap": 6 }))
        ]),
    );
}

#[test]
fn an_operation_that_refers_to_an_unknown_id_is_an_error_that_names_it() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let before = log_len(&design);
    let replace_content = format!(
        "<Frame id=\"{}\" name=\"Summary\" dir=\"col\">\n  <Text id=\"{UNKNOWN_NODE}\" name=\"Ghost\" content=\"Spöke\" fontFamily=\"Inter\" fontSize={{14}}/>\n</Frame>",
        design.id("Summary")
    );
    let batches = [
        json!([set_content(UNKNOWN_NODE, 1, "Nobody")]),
        json!([insert(
            UNKNOWN_NODE,
            None,
            &text_tag("Orphan", "Föräldralös")
        )]),
        json!([move_to(&design.id("Pay"), 1, UNKNOWN_NODE, None)]),
        json!([delete(UNKNOWN_NODE, 1)]),
        json!([{ "op": "replace", "node": design.id("Summary"), "rev": 1, "content": replace_content }]),
        json!([insert(
            &design.id("Actions"),
            None,
            &format!(r#"<Instance name="Button" component="{UNKNOWN_NODE}"/>"#)
        )]),
    ];
    for batch in batches {
        let ops = json!([
            set_content(&design.id("Pay"), 1, "Betala"),
            batch[0].clone()
        ]);
        let response = edit(f, client, &holder(f), &design, ops.clone());
        assert_invalid_naming(&response, 1, UNKNOWN_NODE);
        assert_eq!(log_len(&design), before, "nothing of {ops} is logged");
    }
    let tags = board_tags(f, client, &design, &design.id("Payment"));
    assert!(tags.contains("Betala nu"), "nothing changed: {tags}");
}

#[test]
fn two_agents_editing_different_boards_of_one_design_at_once_both_succeed() {
    let (f, client) = ready();
    let design = new_design(f, client);
    const ROUNDS: usize = 10;
    let barrier = std::sync::Barrier::new(2);
    let (actions, payment) = (design.id("Actions"), design.id("Payment"));
    let statuses: Vec<(String, u16)> = std::thread::scope(|scope| {
        let runs = [
            (holder(f), actions.clone(), "A"),
            (other(f), payment.clone(), "B"),
        ]
        .map(|(auth, parent, label)| {
            let barrier = &barrier;
            let design = &design;
            scope.spawn(move || {
                barrier.wait();
                (0..ROUNDS)
                    .map(|round| {
                        let name = format!("{label}{round}");
                        let response = edit(
                            f,
                            client,
                            &auth,
                            design,
                            json!([insert(&parent, None, &text_tag(&name, &name))]),
                        );
                        (format!("{name}: {}", response.text()), response.status)
                    })
                    .collect::<Vec<_>>()
            })
        });
        runs.into_iter()
            .flat_map(|run| run.join().expect("an agent's thread"))
            .collect()
    });
    for (what, status) in &statuses {
        assert_eq!(*status, 200, "{what}");
    }
    let cart = board_tags(f, client, &design, &design.id("Cart"));
    let pay = board_tags(f, client, &design, &payment);
    for round in 0..ROUNDS {
        assert!(cart.contains(&format!(r#"name="A{round}""#)), "{cart}");
        assert!(pay.contains(&format!(r#"name="B{round}""#)), "{pay}");
    }
}
