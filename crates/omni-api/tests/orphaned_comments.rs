//! US-107: orphaned comments, through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. Docs are
//! created and written with the live routes of US-102 to US-104 (no change
//! set), and comments are made with the routes of US-106
//! (tests/comments.rs). The CLI side is in crates/omni-cli/tests/comments.rs
//! and stack_comments.rs.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`):
//!
//! - A comment stores a `snapshot`: the content of the node it was pinned to,
//!   as it was when the comment was made (an HTML doc's block as HTML, a
//!   design's node in tag form, and for `<instanceId>/<componentNodeId>` the
//!   component node). Every comment that `GET /comments` lists carries
//!   `"snapshot"` (a string) and `"orphaned"` (a boolean). Later edits of the
//!   node do not change the snapshot.
//! - `orphaned` is computed against the doc's head when the comment is listed:
//!   it is `true` exactly when the head no longer has the comment's node
//!   address (an operation deleted the node or a node above it, or a replace
//!   left it out), and `false` while the node is in the head, also after a
//!   `replace` or a `set` that keeps its ID.
//! - Commented nodes do not stop a write (V18): `POST /docs/{doc_id}/ops`
//!   that deletes a commented node answers 200 like any other batch, and
//!   the comment stays listed, orphaned, with its snapshot, until the human
//!   deletes it.
//! - An orphaned comment is like any other: an agent may reply to it, the
//!   human deletes it with `DELETE /comments/{comment_id}` (204, it leaves
//!   every list, its read is 404, `comment.deleted` is emitted, the row stays
//!   with `deleted_at` set) and an API key cannot (403 `forbidden`).

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

const TEST_DB: &str = "omni_us107_orphans";
const TEST_BUCKET: &str = "omni-us107-orphans";

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
    format!("us107-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

/// The agent that writes the docs and answers comments: `run-1/author`.
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

/// The human pins `text` to `node` of the doc's head; returns the comment's
/// ID.
fn human_comment(
    f: &Fixture,
    client: &Client,
    project: &str,
    doc: &str,
    node: &str,
    text: &str,
) -> String {
    id_of(&comment(f, client, &human(f), project, doc, node, text))
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

// ---------------------------------------------------------------------------
// Operations and orphans
// ---------------------------------------------------------------------------

fn delete_op(node: &str, rev: u64) -> Value {
    json!({ "op": "delete", "node": node, "rev": rev })
}

fn replace_op(node: &str, rev: u64, content: &str) -> Value {
    json!({ "op": "replace", "node": node, "rev": rev, "content": content })
}

fn set_op(node: &str, rev: u64, properties: Value) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": properties })
}

/// The IDs that a result lists under `key` (`created` or `removed`).
fn listed_ids(result: &Value, key: &str) -> Vec<String> {
    result[key]
        .as_array()
        .unwrap_or_else(|| panic!("the result lists {key}: {result}"))
        .iter()
        .map(|id| id.as_str().expect("an ID").to_string())
        .collect()
}

/// Adds a section with a heading and a paragraph to an HTML doc's head:
/// (the section, the paragraph).
fn section(f: &Fixture, client: &Client, project: &str, doc: &str) -> (String, String) {
    let results = edit(
        f,
        client,
        project,
        doc,
        json!([insert(
            None,
            "<section><h2>Shipping</h2><p>Parcels leave the warehouse at noon.</p></section>"
        )]),
    );
    let created = listed_ids(&results[0], "created");
    assert_eq!(created.len(), 3, "section, heading, paragraph: {results:?}");
    (created[0].clone(), created[2].clone())
}

/// The comment `id` as `GET /comments{query}` lists it to `auth`.
fn listed(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    query: &str,
    id: &str,
) -> Value {
    let comments = list(f, client, auth, project, query);
    comments
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("GET /comments{query} lists {id}: {comments:?}"))
        .clone()
}

/// The snapshot of a listed comment.
fn snapshot(comment: &Value) -> &str {
    comment["snapshot"]
        .as_str()
        .unwrap_or_else(|| panic!("a listed comment has a snapshot: {comment}"))
}

/// Checks that every list that has comment `id` lists it with `orphaned`, to
/// the human and to an agent.
fn assert_orphaned(
    f: &Fixture,
    client: &Client,
    project: &str,
    doc: &str,
    id: &str,
    orphaned: bool,
) {
    for query in [String::new(), format!("?doc={doc}")] {
        for auth in [human(f), author(f)] {
            let comment = listed(f, client, &auth, project, &query, id);
            assert_eq!(
                comment["orphaned"],
                json!(orphaned),
                "GET /comments{query} lists the comment with orphaned {orphaned}: {comment}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Deleting a commented node
// ---------------------------------------------------------------------------

#[test]
fn deleting_a_commented_block_succeeds_and_its_comment_is_listed_orphaned_with_its_snapshot() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, doomed) = paper_in(f, client, &project, None);
    let kept = paragraph(f, client, &project, &doc, "Returns are free for 30 days.");
    let on_doomed = human_comment(f, client, &project, &doc, &doomed, "Say which days count.");
    let on_kept = human_comment(f, client, &project, &doc, &kept, "Thirty is generous.");
    assert_orphaned(f, client, &project, &doc, &on_doomed, false);

    // The write goes through although the node has a comment.
    let response = call(
        client,
        "POST",
        &format!("{}/docs/{doc}/ops", project_base(f, &project)),
        &author(f),
        Some(&json!({ "ops": [delete_op(&doomed, 1)] })),
    );
    let answer = must(response, 200, "delete the commented paragraph");
    assert_eq!(
        listed_ids(&answer["results"][0], "removed"),
        [doomed.clone()],
        "{answer}"
    );
    let head = read_head(f, client, &project, &doc);
    let html = head["html"].as_str().expect("the head's HTML");
    assert!(
        !html.contains("Orders ship in two days."),
        "the paragraph left the head: {html}"
    );

    assert_orphaned(f, client, &project, &doc, &on_doomed, true);
    assert_orphaned(f, client, &project, &doc, &on_kept, false);
    let orphan = listed(f, client, &human(f), &project, "", &on_doomed);
    assert_eq!(orphan["node"], doomed.as_str(), "{orphan}");
    assert_eq!(orphan["text"], "Say which days count.", "{orphan}");
    assert!(
        snapshot(&orphan).contains("Orders ship in two days."),
        "the snapshot shows what the comment pointed at: {orphan}"
    );
    assert!(
        snapshot(&listed(f, client, &human(f), &project, "", &on_kept))
            .contains("Returns are free for 30 days."),
        "a comment that is not orphaned has its snapshot too"
    );
    assert_eq!(
        ids(&list(
            f,
            client,
            &human(f),
            &project,
            &format!("?doc={doc}")
        )),
        [on_doomed.clone(), on_kept.clone()],
        "no comment vanished"
    );
}

#[test]
fn deleting_a_commented_board_orphans_the_comments_on_its_nodes_and_inside_its_instance() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let design = design_in(f, client, &project);
    let inside = format!("{}/{}", design.instance, design.label);
    let on_page = human_comment(f, client, &project, &design.doc, &design.page, "Look here");
    let on_board = human_comment(f, client, &project, &design.doc, &design.board, "Look here");
    let on_text = human_comment(f, client, &project, &design.doc, &design.text, "Look here");
    let on_instance = human_comment(
        f,
        client,
        &project,
        &design.doc,
        &design.instance,
        "Look here",
    );
    let on_inside = human_comment(f, client, &project, &design.doc, &inside, "Look here");
    assert_orphaned(f, client, &project, &design.doc, &on_inside, false);

    let results = edit(
        f,
        client,
        &project,
        &design.doc,
        json!([delete_op(&design.board, 1)]),
    );
    assert!(
        listed_ids(&results[0], "removed").contains(&design.text),
        "the board went with its nodes: {results:?}"
    );

    assert_orphaned(f, client, &project, &design.doc, &on_page, false);
    for orphan in [&on_board, &on_text, &on_instance, &on_inside] {
        assert_orphaned(f, client, &project, &design.doc, orphan, true);
    }
    let text = listed(f, client, &human(f), &project, "", &on_text);
    assert!(
        snapshot(&text).contains("Total: 42 kr"),
        "a design node's snapshot is its tag form: {text}"
    );
    assert!(
        snapshot(&listed(f, client, &human(f), &project, "", &on_board)).contains("Checkout"),
        "the board's snapshot names it"
    );
    let inside = listed(f, client, &human(f), &project, "", &on_inside);
    assert!(
        snapshot(&inside).contains("Buy"),
        "the snapshot of a node inside an instance is the component's node: {inside}"
    );
}

// ---------------------------------------------------------------------------
// Replacing a commented node
// ---------------------------------------------------------------------------

#[test]
fn a_comment_on_a_block_that_is_replaced_or_set_but_keeps_its_id_is_not_orphaned() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, replaced) = paper_in(f, client, &project, None);
    let changed = paragraph(f, client, &project, &doc, "Pay by card.");
    let on_replaced = human_comment(f, client, &project, &doc, &replaced, "Too slow.");
    let on_changed = human_comment(f, client, &project, &doc, &changed, "And invoice?");

    let results = edit(
        f,
        client,
        &project,
        &doc,
        json!([
            replace_op(&replaced, 1, "<p>Orders ship the same day.</p>"),
            set_op(&changed, 1, json!({ "html": "Pay by card or invoice." })),
        ]),
    );
    assert_eq!(results[0]["node"], replaced.as_str(), "{results:?}");
    assert_eq!(results[1]["node"], changed.as_str(), "{results:?}");

    assert_orphaned(f, client, &project, &doc, &on_replaced, false);
    assert_orphaned(f, client, &project, &doc, &on_changed, false);
    let comment = listed(f, client, &human(f), &project, "", &on_replaced);
    assert!(
        snapshot(&comment).contains("Orders ship in two days.")
            && !snapshot(&comment).contains("the same day"),
        "the snapshot is what the node held when the comment was made: {comment}"
    );
    let comment = listed(f, client, &human(f), &project, "", &on_changed);
    assert!(
        snapshot(&comment).contains("Pay by card.") && !snapshot(&comment).contains("invoice"),
        "{comment}"
    );
}

#[test]
fn a_replace_that_leaves_a_commented_block_out_orphans_only_that_comment() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, _) = paper_in(f, client, &project, None);
    let (section, inner) = section(f, client, &project, &doc);
    let on_section = human_comment(f, client, &project, &doc, &section, "Shorten this.");
    let on_inner = human_comment(f, client, &project, &doc, &inner, "Noon or 11?");

    let results = edit(
        f,
        client,
        &project,
        &doc,
        json!([replace_op(
            &section,
            1,
            "<section><h2>Shipping</h2></section>"
        )]),
    );
    assert_eq!(results[0]["node"], section.as_str(), "{results:?}");
    assert!(
        listed_ids(&results[0], "removed").contains(&inner),
        "the paragraph was left out: {results:?}"
    );

    assert_orphaned(f, client, &project, &doc, &on_section, false);
    assert_orphaned(f, client, &project, &doc, &on_inner, true);
}

// ---------------------------------------------------------------------------
// Deleting an orphaned comment
// ---------------------------------------------------------------------------
