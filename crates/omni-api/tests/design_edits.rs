//! US-031: editing designs with operations, and claims on boards, through the
//! API. Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint two API keys through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. The tests
//! of expiry use a second router over the same database whose claims last a
//! few seconds (`AppState::with_claim_ttl`), and the test of the log a third
//! one, which loads the draft from the database; every other test talks to one
//! router only, since each keeps its own drafts in memory.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; docs, drafts and the
//! operations route are those of US-022 (tests/docs.rs), claims those of
//! US-023 (tests/claims.rs), design imports and board reads those of US-029
//! and US-030 (tests/boards.rs); the tags are those of
//! docs/formats/design-tags.md):
//!
//! - `POST /changesets/{changeset_id}/docs/{doc_id}/ops` (agent) edits the
//!   change set's draft of a design doc with the operations of
//!   docs/formats/operations.md, on the design's nodes: `insert` (`parent`,
//!   `position`, `content`), `replace` (`node`, `rev`, `content`), `set`
//!   (`node`, `rev`, `properties`), `move` (`node`, `rev`, `parent`,
//!   `position`) and `delete` (`node`, `rev`). `content` is one tag in the tag
//!   format with its children. A batch is all or nothing and answers 200
//!   `{"results": [{"node", "revision", "created", "removed"}]}`, one result
//!   per operation, as for HTML docs.
//!   - An insert adds a new subtree under `parent` at `position`: a `<Board>`
//!     goes under a page, every other node under a board or a frame (a node
//!     of another kind under a page, or a board under a frame, is refused).
//!     Every node of the content is new, so a tag of the content that carries
//!     an `id` is refused: the server assigns the IDs (8 lowercase base32
//!     characters), new to the project, and answers them in `created`, in
//!     document order. A new node is at revision 1, and the parent's revision
//!     does not change.
//!   - A replace swaps the node's content in one step: the node keeps its ID
//!     and its revision rises by 1; a tag of the content with the ID of one of
//!     the node's descendants keeps that node, a descendant that the content
//!     leaves out is removed (`removed`), and a tag without `id` is new
//!     (`created`).
//!   - A set changes properties of the node, named as the attributes of its
//!     tag, with their values in JSON (`{"content": "Hej", "fontSize": 18}`);
//!     the node's revision rises by 1 and no other node's does. An attribute
//!     that the node's tag does not take, and `id`, are refused, naming it.
//!   - A move puts the node under a new parent; only the node's revision
//!     rises. A delete removes the node and its descendants (`removed`, in
//!     document order).
//!   - An operation whose `rev` is not the node's revision is 409 `{"error":
//!     "conflict", "conflict": {"index", "node", "expected",
//!     "current_revision", ...}}`. Every other refusal is 400 `{"error":
//!     "invalid_operation", "index", "message"}`, and an ID that is in no node
//!     of the draft (an operation's node or parent, an `id` in a replace's
//!     content, the `component` of an `<Instance>`) is named in the message.
//!     A refused batch changes, logs and emits nothing.
//!   - The body may carry `"board": "<board ID>"` (`omni design edit
//!     --board`): then every operation must stay inside that board (its node,
//!     and its parent for an insert or a move, are the board or in it); one
//!     that does not is 400 `invalid_operation` naming the node, and nothing
//!     changes. An ID that is no board of the doc is 404 `{"error":
//!     "unknown_board"}`.
//! - Every applied operation is a row of the draft's log (`draft_ops`) with
//!   the agent identity, so a router that loads the draft from the database
//!   reads the same design with the same revisions, and a saved version holds
//!   the edits. Each applied operation emits `op.applied` with `data`
//!   `{"doc_id", "changeset_id", "draft_id", "node", "revision", "created",
//!   "removed", "agent"}`.
//! - Batches of different agents on different boards of one draft, sent at
//!   the same time, all apply.
//! - Claims work on boards as on sections: `POST /changesets/{changeset_id}/
//!   claims` with the ID of a board claims it (201 `{"claim"}`, listed in
//!   `GET /docs/{doc_id}` and emitting `claim.taken`); a page, a frame or a
//!   text is no board, 400 `"error": "invalid_request"`. While the claim is
//!   held, another agent's claim on the board is 409 `claimed`, and another
//!   agent's batch that changes, moves or deletes a node of the board, inserts
//!   or moves a node into it, or replaces or deletes its page is 409
//!   `{"error": "claimed", "index", "message", "claim"}` with `message`
//!   naming the holder (their run and role); nothing changes. The holder's
//!   own operations, and everyone's on other boards, apply. Releasing emits
//!   `claim.released`; a claim expires after the claim TTL without an
//!   operation of its holder in the draft and emits `claim.expired`, and the
//!   holder's operations on the board renew it.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::{AppState, Config};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const TEST_DB: &str = "omni_us031_design_edits";
const TEST_BUCKET: &str = "omni-us031-design-edits";
/// A node ID in the right format that no node has.
const UNKNOWN_NODE: &str = "zzzzzzzz";
/// How long a claim lasts on the short router.
const SHORT_TTL: Duration = Duration::from_secs(3);
/// The characters of a node ID.
const NODE_ID_ALPHABET: &str = "0123456789abcdefghjkmnpqrstvwxyz";

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

/// A second router over the scratch database whose claims last `SHORT_TTL`.
struct ShortServer {
    client: Client,
    /// Keeps the router's runtime, and so the router, alive.
    _runtime: tokio::runtime::Runtime,
}

fn short() -> (&'static Fixture, &'static Client) {
    static SHORT: OnceLock<ShortServer> = OnceLock::new();
    let f = fixture();
    let server = SHORT.get_or_init(|| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("bind a local port");
        let port = listener.local_addr().unwrap().port();
        let router = {
            // The connection pool needs a runtime to be created in.
            let _guard = runtime.enter();
            let state = AppState::new(&config())
                .expect("build the API state")
                .with_claim_ttl(SHORT_TTL);
            omni_api::router(state)
        };
        runtime.spawn(async move {
            axum::serve(listener, router).await.expect("serve the API");
        });
        ShortServer {
            client: Client::local(port),
            _runtime: runtime,
        }
    });
    (f, &server.client)
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

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
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

/// One page, Checkout, with the boards Cart and Payment.
const DESIGN: &str = r##"<Page name="Checkout">
  <Board name="Cart" x={0} y={0} width={360} height={640} dir="col" gap={8}>
    <Frame name="Summary" dir="col" gap={4}>
      <Text name="Total" content="Summa 10 kr" fontFamily="Inter" fontSize={16}/>
      <Text name="Shipping" content="Frakt ingår" fontFamily="Inter" fontSize={14}/>
    </Frame>
    <Frame name="Actions" dir="row" gap={8}/>
  </Board>
  <Board name="Payment" x={400} y={0} width={360} height={640} dir="col">
    <Text name="Pay" content="Betala nu" fontFamily="Inter" fontSize={16}/>
  </Board>
</Page>
"##;

/// A project with a design doc started in an open change set and `DESIGN`
/// imported into its draft.
struct Design {
    project: String,
    changeset: String,
    doc: String,
    draft: String,
    ids: Ids,
}

impl Design {
    fn base(&self, f: &Fixture) -> String {
        format!("/api/v1/w/{}/projects/{}", f.workspace, self.project)
    }

    fn changeset_path(&self, f: &Fixture) -> String {
        format!("{}/changesets/{}", self.base(f), self.changeset)
    }

    fn ops_path(&self, f: &Fixture) -> String {
        format!("{}/docs/{}/ops", self.changeset_path(f), self.doc)
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
    let changeset = str_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/changesets"),
                &holder(f),
                Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
            ),
            201,
            "create a change set",
        )["changeset"],
        "id",
    )
    .to_string();
    let doc = str_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/changesets/{changeset}/docs"),
                &holder(f),
                Some(&json!({ "kind": "design", "module": "ui", "title": unique("design") })),
            ),
            201,
            "create a design doc",
        )["doc"],
        "id",
    )
    .to_string();
    let imported = must(
        call(
            client,
            "PUT",
            &format!("{base}/changesets/{changeset}/docs/{doc}/design"),
            &holder(f),
            Some(&json!({ "tags": DESIGN })),
        ),
        200,
        "import the design",
    );
    let mut ids = Ids::new();
    collect_ids(&imported["pages"], &mut ids);
    assert_eq!(ids.len(), 8, "{imported}");
    Design {
        project,
        changeset,
        doc,
        draft: str_of(&imported, "draft_id").to_string(),
        ids,
    }
}

/// Sends a batch of operations as `auth`.
fn edit(f: &Fixture, client: &Client, auth: &Auth, design: &Design, ops: Value) -> Response {
    call(
        client,
        "POST",
        &design.ops_path(f),
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

/// One board of the draft in the tag format, with a `rev` on every tag.
fn board_tags(f: &Fixture, client: &Client, design: &Design, board: &str) -> String {
    let answer = must(
        call(
            client,
            "GET",
            &format!(
                "{}/docs/{}/boards/{board}",
                design.changeset_path(f),
                design.doc
            ),
            &holder(f),
            None,
        ),
        200,
        "read a board of the draft",
    );
    str_of(&answer, "tags").to_string()
}

/// The pages of the draft as JSON nodes.
fn draft_pages(f: &Fixture, client: &Client, design: &Design) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/docs/{}", design.changeset_path(f), design.doc),
            &holder(f),
            None,
        ),
        200,
        "read the draft",
    )["draft"]["pages"]
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

/// The tag of the node `id` in `tags`, one per line as `tags::print` writes
/// them.
fn tag_of<'a>(tags: &'a str, id: &str) -> Option<&'a str> {
    let needle = format!(r#"id="{id}""#);
    tags.lines()
        .map(str::trim)
        .find(|line| line.starts_with('<') && line.contains(&needle))
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

/// The revision of the node `id` as a board read shows it.
fn rev_in(tags: &str, id: &str) -> u64 {
    let tag = tag_of(tags, id).unwrap_or_else(|| panic!("the board has the node {id}: {tags}"));
    rev_of(tag).unwrap_or_else(|| panic!("the tag of {id} has a rev: {tag}"))
}

fn assert_node_id(id: &Value) {
    let id = id
        .as_str()
        .unwrap_or_else(|| panic!("a node ID is a string: {id}"));
    assert!(
        id.len() == 8 && id.chars().all(|c| NODE_ID_ALPHABET.contains(c)),
        "{id:?} is not a node ID of 8 lowercase base32 characters"
    );
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

/// The number of rows in the draft's log.
fn log_len(design: &Design) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from draft_ops where draft_id = $1::uuid")
            .bind(&design.draft)
            .fetch_one(&mut conn)
            .await
            .expect("count the draft's log")
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
fn an_insert_adds_a_subtree_in_the_tag_format_with_ids_that_the_server_assigns() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let content = format!(
        "<Frame name=\"Buttons\" dir=\"row\" gap={{4}}>\n  {}\n  {}\n</Frame>",
        text_tag("Buy", "Köp"),
        text_tag("Later", "Spara till senare")
    );
    let results = applied(
        f,
        client,
        &design,
        json!([insert(&design.id("Actions"), None, &content)]),
    );
    assert_eq!(results.len(), 1, "{results:?}");
    let created = ids_of(&results[0], "created");
    assert_eq!(
        created.len(),
        3,
        "a frame and two texts are new: {results:?}"
    );
    for id in &created {
        assert_node_id(&json!(id));
        assert!(
            !design.ids.values().any(|known| known == id),
            "a new ID is new to the design: {id}"
        );
    }
    assert_eq!(
        created.iter().collect::<HashSet<_>>().len(),
        3,
        "every new node has its own ID: {created:?}"
    );
    assert_eq!(results[0]["node"], created[0].as_str(), "{results:?}");
    assert_eq!(results[0]["revision"], 1, "{results:?}");
    assert_eq!(ids_of(&results[0], "removed"), Vec::<String>::new());

    let tags = board_tags(f, client, &design, &design.id("Cart"));
    for (id, name) in created.iter().zip(["Buttons", "Buy", "Later"]) {
        let tag = tag_of(&tags, id).unwrap_or_else(|| panic!("the board has {name}: {tags}"));
        assert!(
            tag.contains(&format!(r#"name="{name}""#)),
            "the IDs are in document order: {tag}"
        );
        assert_eq!(rev_of(tag), Some(1), "a new node is at revision 1: {tag}");
    }
    assert_eq!(
        rev_in(&tags, &design.id("Actions")),
        1,
        "an insert does not change the parent's revision: {tags}"
    );
    let pages = draft_pages(f, client, &design);
    let actions = find_node(&pages, &design.id("Actions")).expect("the draft has Actions");
    assert_eq!(child_ids(&actions), vec![created[0].clone()], "{actions}");
}

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

    let pages = draft_pages(f, client, &design);
    let page = find_node(&pages, &design.id("Checkout")).expect("the draft has its page");
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
    assert_eq!(rev_in(&tags, &created[1]), 1, "{tags}");
}

#[test]
fn an_insert_keeps_boards_under_pages_and_other_nodes_under_boards_and_frames() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let before = log_len(&design);
    let frame_under_page = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([insert(
            &design.id("Checkout"),
            None,
            "<Frame name=\"Loose\" dir=\"row\"/>"
        )]),
    );
    assert_error(&frame_under_page, 400, "invalid_operation");
    let board_in_frame = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([insert(
            &design.id("Summary"),
            None,
            "<Board name=\"Nested\" width={100} height={100}/>"
        )]),
    );
    assert_error(&board_in_frame, 400, "invalid_operation");
    assert_eq!(log_len(&design), before, "refused inserts log nothing");

    // The same nodes where they belong: the board under the page, the frame
    // in a board.
    let results = applied(
        f,
        client,
        &design,
        json!([
            insert(
                &design.id("Checkout"),
                None,
                "<Board name=\"Nested\" x={0} y={800} width={100} height={100}/>"
            ),
            insert(
                &design.id("Cart"),
                None,
                "<Frame name=\"Loose\" dir=\"row\"/>"
            )
        ]),
    );
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(
        log_len(&design),
        before + 2,
        "two applied inserts are logged"
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
fn a_replace_swaps_a_node_keeping_the_descendants_its_content_names() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let content = format!(
        "<Frame id=\"{}\" name=\"Summary\" dir=\"row\" gap={{4}}>\n  <Text id=\"{}\" name=\"Total\" content=\"Summa 12 kr\" fontFamily=\"Inter\" fontSize={{16}}/>\n  {}\n</Frame>",
        design.id("Summary"),
        design.id("Total"),
        text_tag("Discount", "Rabatt 2 kr")
    );
    let results = applied(
        f,
        client,
        &design,
        json!([{ "op": "replace", "node": design.id("Summary"), "rev": 1, "content": content }]),
    );
    assert_eq!(results[0]["node"], design.id("Summary"), "{results:?}");
    assert_eq!(results[0]["revision"], 2, "{results:?}");
    assert_eq!(
        ids_of(&results[0], "removed"),
        vec![design.id("Shipping")],
        "the content leaves Shipping out: {results:?}"
    );
    let created = ids_of(&results[0], "created");
    assert_eq!(created.len(), 1, "Discount is new: {results:?}");

    let tags = board_tags(f, client, &design, &design.id("Cart"));
    let summary = tag_of(&tags, &design.id("Summary")).expect("Summary stays");
    assert!(summary.contains(r#"dir="row""#), "{summary}");
    let total = tag_of(&tags, &design.id("Total")).expect("Total keeps its ID");
    assert!(total.contains("Summa 12 kr"), "{total}");
    assert_eq!(rev_of(total), Some(2), "Total's content changed: {total}");
    assert!(
        tag_of(&tags, &design.id("Shipping")).is_none(),
        "Shipping is removed: {tags}"
    );
    assert!(
        tag_of(&tags, &created[0]).is_some_and(|tag| tag.contains("Rabatt 2 kr")),
        "{tags}"
    );
    assert_eq!(rev_in(&tags, &design.id("Cart")), 1, "{tags}");
}

#[test]
fn a_set_changes_properties_and_only_that_nodes_revision() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let results = applied(
        f,
        client,
        &design,
        json!([set(
            &design.id("Total"),
            1,
            json!({ "content": "Summa 12 kr", "fontSize": 18 })
        )]),
    );
    assert_eq!(results[0]["node"], design.id("Total"), "{results:?}");
    assert_eq!(results[0]["revision"], 2, "{results:?}");

    let tags = board_tags(f, client, &design, &design.id("Cart"));
    let total = tag_of(&tags, &design.id("Total")).expect("the board has Total");
    assert!(total.contains(r#"content="Summa 12 kr""#), "{total}");
    assert!(total.contains("fontSize={18}"), "{total}");
    assert_eq!(rev_of(total), Some(2), "{total}");
    for name in ["Cart", "Summary", "Shipping", "Actions"] {
        assert_eq!(
            rev_in(&tags, &design.id(name)),
            1,
            "a set changes no other node's revision ({name}): {tags}"
        );
    }
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
    let tags = board_tags(f, client, &design, &design.id("Cart"));
    assert_eq!(rev_in(&tags, &design.id("Total")), 1, "{tags}");
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

    let pages = draft_pages(f, client, &design);
    let summary = find_node(&pages, &design.id("Summary")).expect("Summary");
    let actions = find_node(&pages, &design.id("Actions")).expect("Actions");
    assert_eq!(child_ids(&summary), vec![design.id("Total")], "{summary}");
    assert_eq!(
        child_ids(&actions),
        vec![design.id("Shipping")],
        "{actions}"
    );
    let tags = board_tags(f, client, &design, &design.id("Cart"));
    assert_eq!(rev_in(&tags, &design.id("Summary")), 1, "{tags}");
    assert_eq!(rev_in(&tags, &design.id("Actions")), 1, "{tags}");
}

#[test]
fn a_delete_removes_a_node_and_its_descendants() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let results = applied(
        f,
        client,
        &design,
        json!([delete(&design.id("Summary"), 1)]),
    );
    assert_eq!(
        ids_of(&results[0], "removed"),
        vec![
            design.id("Summary"),
            design.id("Total"),
            design.id("Shipping")
        ],
        "the node and its descendants, in document order: {results:?}"
    );
    let tags = board_tags(f, client, &design, &design.id("Cart"));
    for name in ["Summary", "Total", "Shipping"] {
        assert!(tag_of(&tags, &design.id(name)).is_none(), "{name}: {tags}");
    }
    assert_eq!(rev_in(&tags, &design.id("Cart")), 1, "{tags}");
}

#[test]
fn a_batch_with_a_stale_revision_is_a_conflict_and_changes_nothing() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let cart = board_tags(f, client, &design, &design.id("Cart"));
    let payment = board_tags(f, client, &design, &design.id("Payment"));
    let before = log_len(&design);
    let events = events_of(&design.project, "op.applied").len();
    let response = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([
            set_content(&design.id("Total"), 1, "Summa 12 kr"),
            delete(&design.id("Pay"), 7)
        ]),
    );
    assert_eq!(response.status, 409, "body: {}", response.text());
    let answer = response.json();
    assert_eq!(answer["error"], "conflict", "{answer}");
    assert_eq!(answer["conflict"]["index"], 1, "{answer}");
    assert_eq!(answer["conflict"]["node"], design.id("Pay"), "{answer}");
    assert_eq!(answer["conflict"]["expected"], 7, "{answer}");
    assert_eq!(answer["conflict"]["current_revision"], 1, "{answer}");

    assert_eq!(board_tags(f, client, &design, &design.id("Cart")), cart);
    assert_eq!(
        board_tags(f, client, &design, &design.id("Payment")),
        payment
    );
    assert_eq!(log_len(&design), before, "a refused batch logs nothing");
    assert_eq!(
        events_of(&design.project, "op.applied").len(),
        events,
        "a refused batch emits nothing"
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
fn the_board_option_keeps_a_batch_inside_that_board() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let cart = design.id("Cart");
    let outside = call(
        client,
        "POST",
        &design.ops_path(f),
        &holder(f),
        Some(&json!({
            "board": cart,
            "ops": [
                set_content(&design.id("Total"), 1, "Summa 12 kr"),
                set_content(&design.id("Pay"), 1, "Betala")
            ]
        })),
    );
    assert_invalid_naming(&outside, 1, &design.id("Pay"));
    let moved_out = call(
        client,
        "POST",
        &design.ops_path(f),
        &holder(f),
        Some(&json!({
            "board": cart,
            "ops": [move_to(&design.id("Total"), 1, &design.id("Payment"), None)]
        })),
    );
    assert_eq!(moved_out.status, 400, "body: {}", moved_out.text());
    let tags = board_tags(f, client, &design, &cart);
    assert_eq!(
        rev_in(&tags, &design.id("Total")),
        1,
        "nothing changed: {tags}"
    );

    let inside = must(
        call(
            client,
            "POST",
            &design.ops_path(f),
            &holder(f),
            Some(&json!({
                "board": cart,
                "ops": [
                    set_content(&design.id("Total"), 1, "Summa 12 kr"),
                    insert(&design.id("Actions"), None, &text_tag("Buy", "Köp"))
                ]
            })),
        ),
        200,
        "a batch inside the board applies",
    );
    assert_eq!(
        inside["results"].as_array().map(Vec::len),
        Some(2),
        "{inside}"
    );

    for board in [UNKNOWN_NODE.to_string(), design.id("Summary")] {
        let response = call(
            client,
            "POST",
            &design.ops_path(f),
            &holder(f),
            Some(&json!({
                "board": board,
                "ops": [set_content(&design.id("Total"), 2, "Summa 13 kr")]
            })),
        );
        assert_error(&response, 404, "unknown_board");
    }
}

#[test]
fn applied_operations_are_logged_and_a_fresh_router_reads_the_same_draft() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let before = log_len(&design);
    let results = applied(
        f,
        client,
        &design,
        json!([
            set_content(&design.id("Total"), 1, "Summa 12 kr"),
            insert(&design.id("Actions"), None, &text_tag("Buy", "Köp")),
            move_to(&design.id("Shipping"), 1, &design.id("Actions"), None),
            delete(&design.id("Pay"), 1)
        ]),
    );
    assert_eq!(results.len(), 4, "{results:?}");
    assert_eq!(
        log_len(&design),
        before + 4,
        "one row per applied operation"
    );
    let agents: Vec<String> = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar(
            "select agent->>'run' from draft_ops where draft_id = $1::uuid and seq > $2 order by seq",
        )
        .bind(&design.draft)
        .bind(before)
        .fetch_all(&mut conn)
        .await
        .expect("read the draft's log")
    });
    assert_eq!(agents, vec!["run-1"; 4], "each row carries the agent");

    let cart = board_tags(f, client, &design, &design.id("Cart"));
    let payment = board_tags(f, client, &design, &design.id("Payment"));
    // A router of its own loads the draft from its base and its log.
    let fresh = TestServer::start(&config());
    for (board, tags) in [("Cart", &cart), ("Payment", &payment)] {
        let answer = must(
            call(
                &fresh.client,
                "GET",
                &format!(
                    "{}/docs/{}/boards/{}",
                    design.changeset_path(f),
                    design.doc,
                    design.id(board)
                ),
                &holder(f),
                None,
            ),
            200,
            "read the board on a fresh router",
        );
        assert_eq!(
            str_of(&answer, "tags"),
            tags.as_str(),
            "the log replays to the same {board}, revisions included"
        );
    }

    let version = must(
        call(
            client,
            "POST",
            &format!("{}/docs/{}/versions", design.changeset_path(f), design.doc),
            &holder(f),
            Some(&json!({})),
        ),
        201,
        "save a version",
    );
    let version = str_of(&version["version"], "id").to_string();
    let saved = must(
        call(
            client,
            "GET",
            &format!(
                "{}/docs/{}/boards/{}?version={version}",
                design.base(f),
                design.doc,
                design.id("Cart")
            ),
            &holder(f),
            None,
        ),
        200,
        "read the board of the version",
    );
    let saved = str_of(&saved, "tags");
    assert!(
        saved.contains("Summa 12 kr"),
        "the version holds the set: {saved}"
    );
    assert!(saved.contains(r#"name="Buy""#), "and the insert: {saved}");
}

#[test]
fn applied_operations_emit_op_applied() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let before = events_of(&design.project, "op.applied").len();
    let results = applied(
        f,
        client,
        &design,
        json!([
            set_content(&design.id("Total"), 1, "Summa 12 kr"),
            insert(&design.id("Actions"), None, &text_tag("Buy", "Köp"))
        ]),
    );
    let events = events_of(&design.project, "op.applied");
    assert_eq!(
        events.len(),
        before + 2,
        "one event per operation: {events:?}"
    );
    let events = &events[before..];
    for (event, result) in events.iter().zip(&results) {
        assert_eq!(event["doc_id"], design.doc.as_str(), "{event}");
        assert_eq!(event["changeset_id"], design.changeset.as_str(), "{event}");
        assert_eq!(event["draft_id"], design.draft.as_str(), "{event}");
        assert_eq!(event["node"], result["node"], "{event}");
        assert_eq!(event["revision"], result["revision"], "{event}");
        assert_eq!(event["created"], result["created"], "{event}");
        assert_eq!(event["agent"]["run"], "run-1", "{event}");
        assert_eq!(event["agent"]["role"], "designer", "{event}");
    }
    assert_eq!(events[0]["node"], design.id("Total"), "{:?}", events[0]);
}

#[test]
fn two_agents_editing_different_boards_of_one_draft_at_once_both_succeed() {
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

// ---------------------------------------------------------------------------
// Claims on boards
// ---------------------------------------------------------------------------

fn claim(f: &Fixture, client: &Client, auth: &Auth, design: &Design, node: &str) -> Response {
    call(
        client,
        "POST",
        &format!("{}/claims", design.changeset_path(f)),
        auth,
        Some(&json!({ "node": node })),
    )
}

fn release(f: &Fixture, client: &Client, auth: &Auth, design: &Design, node: &str) -> Response {
    call(
        client,
        "POST",
        &format!("{}/claims/release", design.changeset_path(f)),
        auth,
        Some(&json!({ "node": node })),
    )
}

/// Asserts that `response` is a refusal because `board` is claimed by the
/// holder (run-1/designer).
fn assert_claimed_by_holder(response: &Response, board: &str) {
    assert_eq!(response.status, 409, "body: {}", response.text());
    let answer = response.json();
    assert_eq!(answer["error"], "claimed", "{answer}");
    let message = str_of(&answer, "message");
    assert!(
        message.contains("run-1") && message.contains("designer"),
        "the message names the holder run-1/designer: {message:?}"
    );
    assert_eq!(answer["claim"]["node"], board, "{answer}");
    assert_eq!(answer["claim"]["agent"]["run"], "run-1", "{answer}");
}

#[test]
fn an_agent_claims_a_board_and_the_doc_lists_the_claim() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let cart = design.id("Cart");
    let taken = must(
        claim(f, client, &holder(f), &design, &cart),
        201,
        "claim the board",
    );
    assert_eq!(taken["claim"]["node"], cart.as_str(), "{taken}");
    assert_eq!(taken["claim"]["doc_id"], design.doc.as_str(), "{taken}");
    assert_eq!(taken["claim"]["draft_id"], design.draft.as_str(), "{taken}");
    let doc = must(
        call(
            client,
            "GET",
            &format!("{}/docs/{}", design.base(f), design.doc),
            &holder(f),
            None,
        ),
        200,
        "read the doc",
    );
    let claims = doc["doc"]["claims"].as_array().cloned().unwrap_or_default();
    assert_eq!(claims.len(), 1, "{doc}");
    assert_eq!(claims[0]["node"], cart.as_str(), "{doc}");
    let events = events_of(&design.project, "claim.taken");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["node"], cart.as_str(), "{:?}", events[0]);
}

#[test]
fn only_boards_of_a_design_can_be_claimed() {
    let (f, client) = ready();
    let design = new_design(f, client);
    for name in ["Checkout", "Summary", "Total"] {
        let response = claim(f, client, &holder(f), &design, &design.id(name));
        assert_error(&response, 400, "invalid_request");
    }
    assert!(events_of(&design.project, "claim.taken").is_empty());
}

#[test]
fn a_board_that_another_agent_holds_cannot_be_claimed() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let cart = design.id("Cart");
    must(
        claim(f, client, &holder(f), &design, &cart),
        201,
        "claim the board",
    );
    assert_claimed_by_holder(&claim(f, client, &other(f), &design, &cart), &cart);
    must(
        claim(f, client, &other(f), &design, &design.id("Payment")),
        201,
        "another agent claims another board",
    );
}

#[test]
fn editing_a_board_claimed_by_another_agent_fails_and_names_the_holder() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let cart = design.id("Cart");
    must(
        claim(f, client, &holder(f), &design, &cart),
        201,
        "claim the board",
    );
    let before = log_len(&design);
    let page_replacement = format!("<Page id=\"{}\" name=\"Checkout\"/>", design.id("Checkout"));
    let refused = [
        set_content(&design.id("Total"), 1, "Summa 12 kr"),
        insert(&design.id("Summary"), None, &text_tag("Sneaky", "Smyg")),
        move_to(&design.id("Pay"), 1, &design.id("Actions"), None),
        move_to(&design.id("Total"), 1, &design.id("Payment"), None),
        delete(&design.id("Summary"), 1),
        set(&cart, 1, json!({ "name": "Basket" })),
        delete(&design.id("Checkout"), 1),
        json!({ "op": "replace", "node": design.id("Checkout"), "rev": 1, "content": page_replacement }),
    ];
    for op in refused {
        let ops = json!([set_content(&design.id("Pay"), 1, "Betala"), op]);
        let response = edit(f, client, &other(f), &design, ops.clone());
        assert_claimed_by_holder(&response, &cart);
        assert_eq!(response.json()["index"], 1, "{}", response.text());
        assert_eq!(log_len(&design), before, "nothing of {ops} is logged");
    }

    let theirs = must(
        edit(
            f,
            client,
            &other(f),
            &design,
            json!([set_content(&design.id("Pay"), 1, "Betala")]),
        ),
        200,
        "another agent edits another board",
    );
    assert_eq!(theirs["results"][0]["revision"], 2, "{theirs}");
    must(
        edit(
            f,
            client,
            &holder(f),
            &design,
            json!([set_content(&design.id("Total"), 1, "Summa 12 kr")]),
        ),
        200,
        "the holder edits its board",
    );
}

#[test]
fn the_holder_releases_a_board_and_others_edit_it_again() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let cart = design.id("Cart");
    must(
        claim(f, client, &holder(f), &design, &cart),
        201,
        "claim the board",
    );
    let released = must(
        release(f, client, &holder(f), &design, &cart),
        200,
        "release the board",
    );
    assert_eq!(released["claim"]["node"], cart.as_str(), "{released}");
    let events = events_of(&design.project, "claim.released");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["node"], cart.as_str(), "{:?}", events[0]);
    must(
        edit(
            f,
            client,
            &other(f),
            &design,
            json!([set_content(&design.id("Total"), 1, "Summa 12 kr")]),
        ),
        200,
        "another agent edits the released board",
    );
}

#[test]
fn a_board_claim_expires_without_an_operation_from_its_holder() {
    let (f, client) = short();
    let design = new_design(f, client);
    let cart = design.id("Cart");
    let taken = must(
        claim(f, client, &holder(f), &design, &cart),
        201,
        "claim the board",
    );
    let total = design.id("Total");
    assert_claimed_by_holder(
        &edit(
            f,
            client,
            &other(f),
            &design,
            json!([set_content(&total, 1, "Too early")]),
        ),
        &cart,
    );

    std::thread::sleep(SHORT_TTL + Duration::from_millis(700));

    must(
        edit(
            f,
            client,
            &other(f),
            &design,
            json!([set_content(&total, 1, "After the claim expired")]),
        ),
        200,
        "another agent edits once the claim expired",
    );
    let expired = events_of(&design.project, "claim.expired");
    assert_eq!(expired.len(), 1, "{expired:?}");
    assert_eq!(
        expired[0]["claim_id"], taken["claim"]["id"],
        "{:?}",
        expired[0]
    );
    assert_eq!(expired[0]["node"], cart.as_str(), "{:?}", expired[0]);
}

#[test]
fn the_holders_operations_on_the_board_keep_its_claim_alive() {
    let (f, client) = short();
    let design = new_design(f, client);
    let cart = design.id("Cart");
    must(
        claim(f, client, &holder(f), &design, &cart),
        201,
        "claim the board",
    );
    std::thread::sleep(SHORT_TTL.mul_f64(0.6));
    must(
        edit(
            f,
            client,
            &holder(f),
            &design,
            json!([set_content(&design.id("Total"), 1, "Summa 12 kr")]),
        ),
        200,
        "the holder edits its board",
    );
    std::thread::sleep(SHORT_TTL.mul_f64(0.6));
    // Past the TTL since the claim was taken, but not since the holder's edit.
    assert_claimed_by_holder(
        &edit(
            f,
            client,
            &other(f),
            &design,
            json!([set_content(&design.id("Shipping"), 1, "Still claimed")]),
        ),
        &cart,
    );
    assert!(events_of(&design.project, "claim.expired").is_empty());
}
