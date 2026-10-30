//! US-106: comments on live docs, through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. Docs are
//! created and written with the live routes of US-102 to US-104 (no change
//! set). The CLI side is in crates/omni-cli/tests/comments.rs and
//! stack_comments.rs.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`):
//!
//! - A comment belongs to a doc and a node address of the doc's head, with no
//!   version and no status (V17). `POST /docs/{doc_id}/comments` (both) takes
//!   `{"node", "text"}` and answers 201 `{"comment": Comment}`. `node` is the
//!   ID of a node that the doc's head has (a block of an HTML doc; a page, a
//!   board or any node in a board of a design), or, for a node inside a
//!   component instance of a design, `<instanceId>/<componentNodeId>`, where
//!   the instance is an `Instance` node of the head and the component node is
//!   a node of that instance's component in the design system's head. A node
//!   address that the head does not have (an unknown node, a node of another
//!   doc, a node that an operation deleted, a component node on its own, an
//!   unknown node inside an instance, a component node under a node that is
//!   no instance, a `/` address in an HTML doc) is 422 `{"error":
//!   "node_not_in_head"}`. A missing node, or a missing or blank text, is 400
//!   `{"error": "invalid_request"}`; an unknown doc, or a doc of another
//!   project, is 404 `not_found`. A refused call stores nothing and emits
//!   nothing.
//! - A Comment has at least `{"id" (UUIDv7), "project_id", "doc_id", "node",
//!   "feature_id", "author", "text", "replies", "waiting_on_you",
//!   "created_at"}`: `node` is the address as sent; `feature_id` the doc's
//!   feature, `null` at the app level; `author` `{"kind": "human",
//!   "user_id"}` for the human and `{"kind": "agent", "run", "role", "model",
//!   "harness"}` for an agent; `replies` the replies, oldest first, each
//!   `{"id", "author", "text", "created_at"}`; and `waiting_on_you` is `true`
//!   exactly when the latest reply is an agent's (a comment without replies,
//!   or whose latest reply is the human's, is not waiting).
//! - `GET /comments` (both) lists the project's comments, oldest first: 200
//!   `{"project_id", "comments": [Comment, ...]}`. `doc=<id>` narrows it to
//!   one doc's comments and `feature=<id>` to the comments on the docs of one
//!   feature. `GET /comments/{comment_id}` (both) answers 200 `{"project_id",
//!   "comment": Comment}`.
//! - `POST /comments/{comment_id}/replies` (both) takes `{"text"}` (not blank,
//!   else 400 `invalid_request`) and answers 201 `{"comment": Comment}` with
//!   the new reply last. An unknown comment is 404.
//! - `DELETE /comments/{comment_id}` (human) deletes a comment: 204. The row
//!   stays in the database with `deleted_at` set (the API role has no DELETE
//!   privilege); the comment is left out of every list, `GET
//!   /comments/{comment_id}` and a reply to it are 404, and deleting it again
//!   is 404. An API key is 403 `{"error": "forbidden"}` and changes nothing.
//! - Creating a comment emits `comment.created` with `data` at least
//!   `{"comment_id", "doc_id", "node"}`, a reply `comment.replied` with at
//!   least `{"comment_id", "reply_id", "doc_id"}` and a delete
//!   `comment.deleted` with at least `{"comment_id", "doc_id"}`. A refused
//!   call emits nothing.
//! - There is no status any more: the `addressed`, `resolve` and `reopen`
//!   routes are gone from the registry.
//! - Storage: comments live in the tenant table `comments`, whose columns
//!   without a default that must be given are `workspace_id`, `doc_id`,
//!   `node`, `author` (jsonb) and `body`, plus the nullable `deleted_at`; it
//!   has no `status` column. Replies stay in the append-only `comment_replies`.
//!   tests/workspace_delete.rs and the isolation suite seed both.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
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

const TEST_DB: &str = "omni_us106_comments";
const TEST_BUCKET: &str = "omni-us106-comments";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
/// A node ID in the right format that no node has.
const UNKNOWN_NODE: &str = "zzzzzzzz";
const ROUTE_BASE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}";

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
    /// Alice's user ID, the human's in `author`.
    alice: String,
    alice_token: String,
    key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us106-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
        let (workspace, alice) = block(async {
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
            (workspace, user)
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
            alice,
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

/// The agent that writes the docs and answers comments: `run-1/author`.
fn author(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-1",
        role: "author",
    }
}

/// Another agent, which reviews: `run-1/critic`.
fn critic(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-1",
        role: "critic",
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

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
}

fn comment_path(f: &Fixture, project: &str, comment: &str) -> String {
    format!("{}/comments/{comment}", project_base(f, project))
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

// ---------------------------------------------------------------------------
// Projects, features and live docs
// ---------------------------------------------------------------------------

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &author(f),
                Some(&json!({ "name": unique("comments-project") })),
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
                &author(f),
                Some(&json!({ "name": unique("Checkout") })),
            ),
            201,
            "create a feature",
        )["feature"],
    )
}

/// Creates a doc (US-102), at the app level or in
/// `feature`; returns its ID.
fn new_doc(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: Option<&str>,
    kind: &str,
    pillar: &str,
) -> String {
    let path = match feature {
        None => format!("{}/docs", project_base(f, project)),
        Some(feature) => format!("{}/features/{feature}/docs", project_base(f, project)),
    };
    id_of(
        &must(
            call(
                client,
                "POST",
                &path,
                &author(f),
                Some(&json!({ "kind": kind, "pillar": pillar, "title": unique(kind) })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

/// Applies a batch to the doc's head as the author; returns the results.
fn edit(f: &Fixture, client: &Client, project: &str, doc: &str, ops: Value) -> Vec<Value> {
    let answer = must(
        call(
            client,
            "POST",
            &format!("{}/docs/{doc}/ops", project_base(f, project)),
            &author(f),
            Some(&json!({ "ops": ops })),
        ),
        200,
        "apply a batch to the head",
    );
    answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer lists results: {answer}"))
        .clone()
}

fn insert(parent: Option<&str>, content: &str) -> Value {
    json!({ "op": "insert", "parent": parent, "position": { "after": null }, "content": content })
}

/// Adds a paragraph to an HTML doc's head; returns its node ID.
fn paragraph(f: &Fixture, client: &Client, project: &str, doc: &str, text: &str) -> String {
    let results = edit(
        f,
        client,
        project,
        doc,
        json!([insert(None, &format!("<p>{text}</p>"))]),
    );
    results[0]["node"]
        .as_str()
        .unwrap_or_else(|| panic!("an insert names its node: {results:?}"))
        .to_string()
}

/// An HTML doc at the app level of `project` (or in `feature`) with one
/// paragraph: (doc, the paragraph's node).
fn paper_in(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: Option<&str>,
) -> (String, String) {
    let doc = new_doc(f, client, project, feature, "html", "spec");
    let node = paragraph(f, client, project, &doc, "Orders ship in two days.");
    (doc, node)
}

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

/// The head read of a doc (`GET /docs/{doc_id}/html`).
fn read_head(f: &Fixture, client: &Client, project: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/docs/{doc}/html", project_base(f, project)),
            &author(f),
            None,
        ),
        200,
        "read the doc's head",
    )
}

/// A live design whose board holds an instance of the design system's
/// Button and a text of its own.
struct Design {
    doc: String,
    page: String,
    board: String,
    text: String,
    instance: String,
    /// The Button component of the design system's head.
    button: String,
    /// The ID of the Button's label in the component (not a node of the
    /// design).
    label: String,
}

/// A live design system with a Button component, and a design in `project`
/// that places it.
fn design_in(f: &Fixture, client: &Client, project: &str) -> Design {
    let (button, label) = design_system_in(f, client, project);
    design_using(f, client, project, &button, &label)
}

/// A live design system in `project` with a Button component: (the Button,
/// its label).
fn design_system_in(f: &Fixture, client: &Client, project: &str) -> (String, String) {
    new_doc(f, client, project, None, "design-system", "ui");
    must(
        call(
            client,
            "PUT",
            &format!("{}/tokens", project_base(f, project)),
            &author(f),
            Some(&json!({ "tokens": {
                "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
                "color": {
                    "$type": "color",
                    "surface": {
                        "$value": "#ffffff",
                        "$extensions": {"omnistate": {"modes": {"light": "#ffffff", "dark": "#0f172a"}}}
                    }
                }
            } })),
        ),
        200,
        "import the tokens into the design system's head",
    );
    let components = must(
        call(
            client,
            "PUT",
            &format!("{}/components", project_base(f, project)),
            &author(f),
            Some(
                &json!({ "tags": r##"<Component name="Button" dir="row" pad={12} fill="#2563eb" radius={8}>
  <Text name="Label" content="Buy" fontFamily="Inter" fontSize={14} color="#ffffff"/>
</Component>
"## }),
            ),
        ),
        200,
        "import the components into the design system's head",
    );
    let parts = ids_in(&components["components"]);
    let button = parts.get("Button").expect("the Button's ID").clone();
    let label = parts.get("Label").expect("the label's ID").clone();
    (button, label)
}

/// A live design in `project` whose board places the Button.
fn design_using(f: &Fixture, client: &Client, project: &str, button: &str, label: &str) -> Design {
    let doc = new_doc(f, client, project, None, "design", "ui");
    let page = read_head(f, client, project, &doc)["pages"][0]["id"]
        .as_str()
        .expect("a new design has a page")
        .to_string();
    edit(
        f,
        client,
        project,
        &doc,
        json!([insert(
            Some(&page),
            &format!(
                r#"<Board name="Checkout" x={{0}} y={{0}} width={{375}} height={{812}} dir="col">
  <Instance name="Pay button" component="{button}"/>
  <Text name="Total" content="Total: 42 kr" fontFamily="Inter" fontSize={{14}}/>
</Board>"#
            )
        )]),
    );
    let nodes = ids_in(&read_head(f, client, project, &doc)["pages"]);
    let id = |name: &str| {
        nodes
            .get(name)
            .unwrap_or_else(|| panic!("the head has {name}: {nodes:?}"))
            .clone()
    };
    Design {
        board: id("Checkout"),
        text: id("Total"),
        instance: id("Pay button"),
        doc,
        page,
        button: button.to_string(),
        label: label.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

fn comment_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    doc: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/docs/{doc}/comments", project_base(f, project)),
        auth,
        Some(body),
    )
}

/// `auth` pins `text` to `node` of the doc's head; returns the comment.
fn comment(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    doc: &str,
    node: &str,
    text: &str,
) -> Value {
    must(
        comment_response(
            f,
            client,
            auth,
            project,
            doc,
            &json!({ "node": node, "text": text }),
        ),
        201,
        &format!("comment on {node}"),
    )["comment"]
        .clone()
}

fn show_response(f: &Fixture, client: &Client, auth: &Auth, project: &str, id: &str) -> Response {
    call(client, "GET", &comment_path(f, project, id), auth, None)
}

fn show(f: &Fixture, client: &Client, project: &str, id: &str) -> Value {
    must(
        show_response(f, client, &author(f), project, id),
        200,
        "read the comment",
    )
}

/// The comments that `GET /comments{query}` lists to `auth`.
fn list(f: &Fixture, client: &Client, auth: &Auth, project: &str, query: &str) -> Vec<Value> {
    let answer = must(
        call(
            client,
            "GET",
            &format!("{}/comments{query}", project_base(f, project)),
            auth,
            None,
        ),
        200,
        &format!("list the comments{query}"),
    );
    assert_eq!(answer["project_id"], project, "{answer}");
    answer["comments"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer lists comments: {answer}"))
        .clone()
}

fn ids(comments: &[Value]) -> Vec<String> {
    comments.iter().map(id_of).collect()
}

fn reply_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    id: &str,
    text: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/replies", comment_path(f, project, id)),
        auth,
        Some(&json!({ "text": text })),
    )
}

fn reply(f: &Fixture, client: &Client, auth: &Auth, project: &str, id: &str, text: &str) -> Value {
    must(
        reply_response(f, client, auth, project, id, text),
        201,
        "reply to the comment",
    )["comment"]
        .clone()
}

fn delete_response(f: &Fixture, client: &Client, auth: &Auth, project: &str, id: &str) -> Response {
    call(client, "DELETE", &comment_path(f, project, id), auth, None)
}

// ---------------------------------------------------------------------------
// The database
// ---------------------------------------------------------------------------

/// The live events of a project named `event`, oldest first, as their
/// `data`.
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

/// How many live events of a project have a name that starts with
/// `comment.`.
fn comment_events(project: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar(
            "select count(*) from live_events \
             where project_id = $1::uuid and event like 'comment.%'",
        )
        .bind(project)
        .fetch_one(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("count the live events: {e}"))
    })
}

/// How many rows of `comments` belong to `doc`, as the admin sees them; read
/// through `to_jsonb` so that a table without a `doc_id` column counts 0.
fn stored_on(doc: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from comments c where to_jsonb(c) ->> 'doc_id' = $1")
            .bind(doc)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the comments: {e}"))
    })
}

/// The stored row of a comment as JSON, as the admin sees it; `None` when
/// there is no such row.
fn stored_row(id: &str) -> Option<Value> {
    block(async {
        let mut conn = admin_connection().await;
        let row: Option<String> =
            sqlx::query_scalar("select to_jsonb(c)::text from comments c where c.id = $1::uuid")
                .bind(id)
                .fetch_optional(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("read the comment's row: {e}"));
        row.map(|text| parse(&text))
    })
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_comment_routes_are_in_the_registry_with_their_access() {
    let routes = registry();
    for (method, path, access) in [
        ("POST", "/docs/{doc_id}/comments", Access::Both),
        ("GET", "/comments", Access::Both),
        ("GET", "/comments/{comment_id}", Access::Both),
        ("POST", "/comments/{comment_id}/replies", Access::Both),
        ("DELETE", "/comments/{comment_id}", Access::Human),
    ] {
        let full = format!("{ROUTE_BASE}{path}");
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == full)
            .unwrap_or_else(|| panic!("{method} {full} is in the route registry"));
        assert_eq!(entry.access, access, "{method} {full}");
    }
    for action in ["addressed", "resolve", "reopen"] {
        let full = format!("{ROUTE_BASE}/comments/{{comment_id}}/{action}");
        assert!(
            !routes.iter().any(|r| r.path == full),
            "comments have no status any more, so {full} is gone"
        );
    }
}

// ---------------------------------------------------------------------------
// Pinning a comment to a node of the head
// ---------------------------------------------------------------------------

#[test]
fn a_comment_on_a_block_of_an_html_head_records_its_doc_node_author_and_text() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, node) = paper_in(f, client, &project, None);

    let made = comment(
        f,
        client,
        &human(f),
        &project,
        &doc,
        &node,
        "Say which days count.",
    );
    assert_uuid_v7(&made["id"]);
    assert_eq!(made["project_id"], project.as_str(), "{made}");
    assert_eq!(made["doc_id"], doc.as_str(), "{made}");
    assert_eq!(made["node"], node.as_str(), "{made}");
    assert_eq!(made["feature_id"], Value::Null, "an app-level doc: {made}");
    assert_eq!(
        made["author"],
        json!({ "kind": "human", "user_id": f.alice }),
        "{made}"
    );
    assert_eq!(made["text"], "Say which days count.", "{made}");
    assert_eq!(made["replies"], json!([]), "{made}");
    assert_eq!(made["waiting_on_you"], false, "no reply yet: {made}");
    assert!(made["created_at"].is_string(), "{made}");

    let read = show(f, client, &project, &id_of(&made));
    assert_eq!(read["project_id"], project.as_str(), "{read}");
    assert_eq!(
        read["comment"], made,
        "the read answers the comment as made"
    );
    assert_eq!(stored_on(&doc), 1, "the comment is stored on the doc");
}

#[test]
fn an_agent_may_comment_too_and_a_comment_on_a_feature_doc_names_the_feature() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let (doc, node) = paper_in(f, client, &project, Some(&feature));

    let made = comment(
        f,
        client,
        &critic(f),
        &project,
        &doc,
        &node,
        "Two days is too slow.",
    );
    assert_eq!(made["doc_id"], doc.as_str(), "{made}");
    assert_eq!(made["feature_id"], feature.as_str(), "{made}");
    assert_eq!(
        made["author"],
        json!({
            "kind": "agent",
            "run": "run-1",
            "role": "critic",
            "model": "test-model",
            "harness": "test-harness"
        }),
        "{made}"
    );
}

#[test]
fn a_comment_may_pin_any_node_of_a_designs_head_including_one_inside_an_instance() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let design = design_in(f, client, &project);

    let inside = format!("{}/{}", design.instance, design.label);
    for node in [
        &design.page,
        &design.board,
        &design.text,
        &design.instance,
        &inside,
    ] {
        let made = comment(
            f,
            client,
            &human(f),
            &project,
            &design.doc,
            node,
            "Look here",
        );
        assert_eq!(
            made["node"],
            node.as_str(),
            "the comment lands on the exact node: {made}"
        );
        assert_eq!(made["doc_id"], design.doc.as_str(), "{made}");
        assert_eq!(
            show(f, client, &project, &id_of(&made))["comment"]["node"],
            node.as_str()
        );
    }
    assert_eq!(stored_on(&design.doc), 5, "every comment is stored");
}

#[test]
fn a_node_address_that_is_not_in_the_head_is_422_and_stores_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, kept) = paper_in(f, client, &project, None);
    let (other_doc, other_node) = paper_in(f, client, &project, None);
    // A paragraph that an operation deleted from the head again.
    let results = edit(
        f,
        client,
        &project,
        &doc,
        json!([insert(None, "<p>Soon gone.</p>")]),
    );
    let gone = results[0]["node"].as_str().expect("a node").to_string();
    let rev = results[0]["revision"].as_u64().expect("a revision");
    edit(
        f,
        client,
        &project,
        &doc,
        json!([{ "op": "delete", "node": gone, "rev": rev }]),
    );
    let design = design_in(f, client, &project);
    let other_design = design_using(f, client, &project, &design.button, &design.label);

    for (what, target, node) in [
        ("an unknown node", &doc, UNKNOWN_NODE.to_string()),
        ("a node of another doc", &doc, other_node.clone()),
        ("a node that an operation deleted", &doc, gone.clone()),
        (
            "an address inside an instance in an HTML doc",
            &doc,
            format!("{kept}/{UNKNOWN_NODE}"),
        ),
        (
            "a component node on its own",
            &design.doc,
            design.label.clone(),
        ),
        (
            "a node that the instance's component does not have",
            &design.doc,
            format!("{}/{UNKNOWN_NODE}", design.instance),
        ),
        (
            "a component node under a node that is no instance",
            &design.doc,
            format!("{}/{}", design.board, design.label),
        ),
        (
            "a node of another design",
            &design.doc,
            other_design.text.clone(),
        ),
    ] {
        let response = comment_response(
            f,
            client,
            &human(f),
            &project,
            target,
            &json!({ "node": node, "text": "Look here" }),
        );
        assert_error(&response, 422, "node_not_in_head", what);
    }
    for stored in [&doc, &other_doc, &design.doc, &other_design.doc] {
        assert_eq!(stored_on(stored), 0, "a refused comment stores nothing");
    }
    assert_eq!(
        comment_events(&project),
        0,
        "a refused comment emits nothing"
    );
}

#[test]
fn a_comment_needs_a_node_and_a_text_and_a_doc_of_the_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let other_project = new_project(f, client);
    let (doc, node) = paper_in(f, client, &project, None);
    let (foreign, foreign_node) = paper_in(f, client, &other_project, None);

    for (what, body) in [
        ("no text", json!({ "node": node })),
        ("a blank text", json!({ "node": node, "text": "  \n " })),
        ("no node", json!({ "text": "Look here" })),
    ] {
        let response = comment_response(f, client, &human(f), &project, &doc, &body);
        assert_error(&response, 400, "invalid_request", what);
    }
    let response = comment_response(
        f,
        client,
        &human(f),
        &project,
        UNKNOWN_ID,
        &json!({ "node": node, "text": "Look here" }),
    );
    assert_error(&response, 404, "not_found", "an unknown doc");
    let response = comment_response(
        f,
        client,
        &human(f),
        &project,
        &foreign,
        &json!({ "node": foreign_node, "text": "Look here" }),
    );
    assert_error(&response, 404, "not_found", "a doc of another project");
    assert_eq!(stored_on(&doc), 0);
    assert_eq!(stored_on(&foreign), 0);
    assert_eq!(
        comment_events(&project),
        0,
        "a refused comment emits nothing"
    );
}

// ---------------------------------------------------------------------------
// Replies and waiting on the human
// ---------------------------------------------------------------------------

#[test]
fn a_comment_has_replies_from_agents_and_the_human_oldest_first() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, node) = paper_in(f, client, &project, None);
    let made = comment(
        f,
        client,
        &human(f),
        &project,
        &doc,
        &node,
        "Say which days.",
    );
    let id = id_of(&made);

    let first = reply(
        f,
        client,
        &author(f),
        &project,
        &id,
        "Working days, as in the SLA.",
    );
    assert_eq!(
        first["replies"].as_array().map(Vec::len),
        Some(1),
        "{first}"
    );
    reply(f, client, &human(f), &project, &id, "Say so in the text.");
    let last = reply(f, client, &critic(f), &project, &id, "Done.");

    let replies = last["replies"].as_array().expect("replies").clone();
    let texts: Vec<&str> = replies.iter().filter_map(|r| r["text"].as_str()).collect();
    assert_eq!(
        texts,
        [
            "Working days, as in the SLA.",
            "Say so in the text.",
            "Done."
        ],
        "the replies, oldest first, the new one last: {last}"
    );
    assert_eq!(replies[0]["author"]["kind"], "agent", "{last}");
    assert_eq!(replies[0]["author"]["role"], "author", "{last}");
    assert_eq!(
        replies[1]["author"],
        json!({ "kind": "human", "user_id": f.alice })
    );
    assert_eq!(replies[2]["author"]["role"], "critic", "{last}");
    for reply in &replies {
        assert_uuid_v7(&reply["id"]);
        assert!(reply["created_at"].is_string(), "{reply}");
    }
    assert_eq!(
        show(f, client, &project, &id)["comment"]["replies"],
        last["replies"]
    );

    let response = reply_response(f, client, &author(f), &project, &id, " \n ");
    assert_error(&response, 400, "invalid_request", "a blank reply");
    let response = reply_response(f, client, &author(f), &project, UNKNOWN_ID, "Hello");
    assert_error(&response, 404, "not_found", "a reply to an unknown comment");
    assert_eq!(
        show(f, client, &project, &id)["comment"]["replies"]
            .as_array()
            .map(Vec::len),
        Some(3),
        "a refused reply adds nothing"
    );
}

#[test]
fn a_comment_waits_on_you_when_its_latest_reply_is_an_agents() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, node) = paper_in(f, client, &project, None);
    let new = |text: &str| id_of(&comment(f, client, &human(f), &project, &doc, &node, text));
    let unanswered = new("Nobody answered this.");
    let answered = new("An agent answered this.");
    let replied_to = new("The human answered the agent here.");
    let answered_again = new("The agent answered the human's answer.");
    reply(f, client, &author(f), &project, &answered, "Fixed.");
    reply(f, client, &author(f), &project, &replied_to, "Fixed.");
    reply(f, client, &human(f), &project, &replied_to, "Not quite.");
    reply(f, client, &human(f), &project, &answered_again, "And?");
    reply(
        f,
        client,
        &critic(f),
        &project,
        &answered_again,
        "Now it is.",
    );

    let listed = list(f, client, &human(f), &project, &format!("?doc={doc}"));
    let waiting = |id: &str| {
        listed
            .iter()
            .find(|c| c["id"] == id)
            .unwrap_or_else(|| panic!("{id} is listed: {listed:?}"))["waiting_on_you"]
            .clone()
    };
    assert_eq!(waiting(&unanswered), false, "no reply: not waiting");
    assert_eq!(waiting(&answered), true, "an agent replied last: waiting");
    assert_eq!(waiting(&replied_to), false, "the human replied last");
    assert_eq!(
        waiting(&answered_again),
        true,
        "an agent replied last again"
    );
    assert_eq!(
        show(f, client, &project, &answered)["comment"]["waiting_on_you"],
        true,
        "the read of one comment says so too"
    );
    assert_eq!(
        list(f, client, &author(f), &project, &format!("?doc={doc}")),
        listed,
        "an agent's list is the same list"
    );
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

#[test]
fn comments_are_listed_oldest_first_by_project_doc_and_feature() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let other_project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let (app_doc, app_node) = paper_in(f, client, &project, None);
    let (feature_doc, feature_node) = paper_in(f, client, &project, Some(&feature));
    let (elsewhere, elsewhere_node) = paper_in(f, client, &other_project, None);

    let first = id_of(&comment(
        f,
        client,
        &human(f),
        &project,
        &app_doc,
        &app_node,
        "First",
    ));
    let second = id_of(&comment(
        f,
        client,
        &human(f),
        &project,
        &feature_doc,
        &feature_node,
        "Second",
    ));
    let third = id_of(&comment(
        f,
        client,
        &author(f),
        &project,
        &app_doc,
        &app_node,
        "Third",
    ));
    reply(f, client, &author(f), &project, &first, "A reply.");
    let foreign = id_of(&comment(
        f,
        client,
        &human(f),
        &other_project,
        &elsewhere,
        &elsewhere_node,
        "Elsewhere",
    ));

    let all = list(f, client, &human(f), &project, "");
    assert_eq!(
        ids(&all),
        [first.clone(), second.clone(), third.clone()],
        "every comment of the project, oldest first, and none of another project"
    );
    assert!(!ids(&all).contains(&foreign));
    assert_eq!(
        all[0]["replies"][0]["text"], "A reply.",
        "a list entry carries its replies: {all:?}"
    );
    for entry in &all {
        assert_eq!(
            entry,
            &show(f, client, &project, &id_of(entry))["comment"],
            "a list entry is the comment as its read answers it"
        );
    }
    assert_eq!(
        ids(&list(
            f,
            client,
            &author(f),
            &project,
            &format!("?doc={app_doc}")
        )),
        [first.clone(), third.clone()],
        "one doc's comments"
    );
    assert_eq!(
        ids(&list(
            f,
            client,
            &author(f),
            &project,
            &format!("?feature={feature}")
        )),
        [second.clone()],
        "the comments on the docs of one feature"
    );
}

// ---------------------------------------------------------------------------
// Deleting
// ---------------------------------------------------------------------------

#[test]
fn the_human_deletes_a_comment_which_leaves_every_list_and_stays_in_the_database() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let (doc, node) = paper_in(f, client, &project, Some(&feature));
    let kept = id_of(&comment(
        f,
        client,
        &human(f),
        &project,
        &doc,
        &node,
        "Keep me.",
    ));
    let doomed = id_of(&comment(
        f,
        client,
        &human(f),
        &project,
        &doc,
        &node,
        "Delete me.",
    ));
    reply(f, client, &author(f), &project, &doomed, "Done.");

    let response = delete_response(f, client, &human(f), &project, &doomed);
    assert_eq!(response.status, 204, "delete: {}", response.text());

    for query in [
        String::new(),
        format!("?doc={doc}"),
        format!("?feature={feature}"),
    ] {
        for auth in [human(f), author(f)] {
            assert_eq!(
                ids(&list(f, client, &auth, &project, &query)),
                [kept.clone()],
                "the deleted comment is left out of GET /comments{query}"
            );
        }
    }
    assert_error(
        &show_response(f, client, &human(f), &project, &doomed),
        404,
        "not_found",
        "the read of a deleted comment",
    );
    assert_error(
        &reply_response(f, client, &author(f), &project, &doomed, "Hello?"),
        404,
        "not_found",
        "a reply to a deleted comment",
    );
    assert_error(
        &delete_response(f, client, &human(f), &project, &doomed),
        404,
        "not_found",
        "deleting a deleted comment again",
    );
    assert_error(
        &delete_response(f, client, &human(f), &project, UNKNOWN_ID),
        404,
        "not_found",
        "deleting an unknown comment",
    );

    let row = stored_row(&doomed).expect("the deleted comment stays in the database");
    assert!(
        row["deleted_at"].is_string(),
        "the row records when it was deleted: {row}"
    );
    assert_eq!(row["body"], "Delete me.", "the row keeps its text: {row}");
    assert_eq!(
        stored_row(&kept).expect("the kept comment")["deleted_at"],
        Value::Null
    );
}

// ---------------------------------------------------------------------------
// Live events
// ---------------------------------------------------------------------------

#[test]
fn commenting_replying_and_deleting_emit_their_events() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, node) = paper_in(f, client, &project, None);
    let id = id_of(&comment(
        f,
        client,
        &human(f),
        &project,
        &doc,
        &node,
        "Look here",
    ));
    let replied = reply(f, client, &author(f), &project, &id, "Looked.");
    let reply_id = id_of(&replied["replies"][0]);
    let response = delete_response(f, client, &human(f), &project, &id);
    assert_eq!(response.status, 204, "delete: {}", response.text());

    let created = events_of(&project, "comment.created");
    assert_eq!(created.len(), 1, "{created:?}");
    assert_eq!(created[0]["comment_id"], id.as_str(), "{created:?}");
    assert_eq!(created[0]["doc_id"], doc.as_str(), "{created:?}");
    assert_eq!(created[0]["node"], node.as_str(), "{created:?}");
    let replies = events_of(&project, "comment.replied");
    assert_eq!(replies.len(), 1, "{replies:?}");
    assert_eq!(replies[0]["comment_id"], id.as_str(), "{replies:?}");
    assert_eq!(replies[0]["reply_id"], reply_id.as_str(), "{replies:?}");
    assert_eq!(replies[0]["doc_id"], doc.as_str(), "{replies:?}");
    let deleted = events_of(&project, "comment.deleted");
    assert_eq!(deleted.len(), 1, "{deleted:?}");
    assert_eq!(deleted[0]["comment_id"], id.as_str(), "{deleted:?}");
    assert_eq!(deleted[0]["doc_id"], doc.as_str(), "{deleted:?}");
}
