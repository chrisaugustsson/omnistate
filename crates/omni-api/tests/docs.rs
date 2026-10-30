//! US-022: docs through the API. Run with `--features stack-tests` in the
//! Compose test container, which reaches Postgres at `postgres:5432` and RustFS
//! at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API (the agent),
//! create a scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; the HTML is the format of
//! docs/formats/html-docs.md and the operations are those of
//! docs/formats/operations.md; the routes are those of tests/live_docs.rs and
//! tests/live_edits.rs):
//!
//! - `pillar` is one of the five pillars `spec`, `ui`, `architecture`, `db` and
//!   `research` (US-101; the old `module` field and its value `prd` are gone).
//!   An HTML doc goes in any pillar. Creating a doc emits `doc.created` with
//!   `data` `{"doc_id", "feature_id", "kind", "pillar", "title"}`.
//! - `GET /docs?pillar=<pillar>` (both) lists only that pillar's docs; a
//!   pillar that is not one of the five is 400 `invalid_request`.
//! - The migration of US-101 renames the `docs` column `module` to `pillar`,
//!   turns `prd` into `spec` and keeps the other values, and the column only
//!   accepts the five pillar names.
//! - `GET /docs/{doc_id}/html` (both) answers the head as `{"doc_id", "html"}`,
//!   with `data-omni-id` and `data-rev` (the node's revision) on every block.
//! - `POST /docs/{doc_id}/ops` (agent) assigns every new node an ID of 8
//!   lowercase base32 characters, unique within the project. An operation that
//!   names a node the head does not have is 400 `{"error":
//!   "invalid_operation", "index", "message"}` whose message names that ID, and
//!   nothing changes.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
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
        // Heads keep their content in RustFS, so the API gets a real bucket.
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

/// Creates a doc in the project.
/// A new project; returns its ID.
fn new_project(f: &Fixture, client: &Client) -> String {
    must(
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
        .to_string()
}

fn create_live_response(f: &Fixture, client: &Client, project: &str, body: &Value) -> Response {
    call(
        client,
        "POST",
        &format!("{}/docs", project_base(f, project)),
        &agent(f),
        Some(body),
    )
}

fn id_of(value: &Value) -> String {
    value["id"]
        .as_str()
        .unwrap_or_else(|| panic!("there is an `id`: {value}"))
        .to_string()
}

/// Creates an HTML Spec doc; returns the whole answer.
fn create_doc(f: &Fixture, client: &Client, project: &str) -> Value {
    must(
        create_live_response(
            f,
            client,
            project,
            &json!({ "kind": "html", "pillar": "spec", "title": unique("Checkout PRD") }),
        ),
        201,
        "create a doc",
    )
}

fn edit(f: &Fixture, client: &Client, project: &str, doc: &str, ops: &Value) -> Response {
    call(
        client,
        "POST",
        &format!("{}/ops", doc_path(f, project, doc)),
        &agent(f),
        Some(&json!({ "ops": ops })),
    )
}

/// The head of the doc as HTML, as `auth` reads it.
fn head_html(f: &Fixture, client: &Client, auth: &Auth, project: &str, doc: &str) -> String {
    must(
        call(
            client,
            "GET",
            &format!("{}/html", doc_path(f, project, doc)),
            auth,
            None,
        ),
        200,
        "read the doc's head",
    )["html"]
        .as_str()
        .expect("the head has its html")
        .to_string()
}

/// How many writes the doc's log holds.
fn logged_writes(doc: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from doc_changes where doc_id = $1::uuid")
            .bind(doc)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the doc's log: {e}"))
    })
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

fn insert_top(content: &str) -> Value {
    json!({ "op": "insert", "parent": null, "position": { "after": null }, "content": content })
}

fn insert_after(after: &str, content: &str) -> Value {
    json!({ "op": "insert", "parent": null, "position": { "after": after }, "content": content })
}

fn set_html(node: &str, rev: u64, html: &str) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": { "html": html } })
}

/// Inserts a top-level paragraph into the doc; returns its ID.
fn a_paragraph(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    let answer = must(
        edit(
            f,
            client,
            project,
            doc,
            &json!([insert_top("<p>First</p>")]),
        ),
        200,
        "insert a paragraph",
    );
    answer["results"][0]["node"]
        .as_str()
        .expect("the new node")
        .to_string()
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

// ---------------------------------------------------------------------------
// Creating a doc
// ---------------------------------------------------------------------------

// US-101: the five pillars.
#[test]
fn an_html_doc_starts_in_each_of_the_five_pillars() {
    let (f, client) = ready();
    let project = new_project(f, client);
    for pillar in ["spec", "ui", "architecture", "db", "research"] {
        let answer = must(
            create_live_response(
                f,
                client,
                &project,
                &json!({ "kind": "html", "pillar": pillar, "title": unique(pillar) }),
            ),
            201,
            &format!("create an html doc in {pillar}"),
        );
        assert_eq!(answer["doc"]["kind"], "html", "{answer}");
        assert_eq!(answer["doc"]["pillar"], pillar, "{answer}");
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
fn creating_a_doc_emits_doc_created() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let answer = create_doc(f, client, &project);
    let doc = id_of(&answer["doc"]);
    let events = events_of(&project, "doc.created");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["doc_id"], doc.as_str(), "{}", events[0]);
    assert_eq!(events[0]["kind"], "html", "{}", events[0]);
    assert_eq!(events[0]["pillar"], "spec", "{}", events[0]);
    assert_eq!(events[0]["title"], answer["doc"]["title"], "{}", events[0]);
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// omni doc read and show
// ---------------------------------------------------------------------------

#[test]
fn reading_a_doc_gives_every_block_its_node_id_and_revision() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = id_of(&create_doc(f, client, &project)["doc"]);
    let answer = must(
        edit(
            f,
            client,
            &project,
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

    let html = head_html(f, client, &agent(f), &project, &doc);
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

    // The human reads the same head.
    let human = head_html(f, client, &Auth::Session(&f.alice_token), &project, &doc);
    assert_eq!(human, html);
}

// ---------------------------------------------------------------------------
// omni doc edit
// ---------------------------------------------------------------------------

#[test]
fn the_server_assigns_node_ids_unique_within_the_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let section = format!(
        "<section><h2>Many</h2>{}</section>",
        (0..60)
            .map(|i| format!("<p>Paragraph {i}</p>"))
            .collect::<String>()
    );
    let mut seen = BTreeSet::new();
    let mut total = 0;
    for _ in 0..2 {
        let doc = id_of(&create_doc(f, client, &project)["doc"]);
        let answer = must(
            edit(
                f,
                client,
                &project,
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

#[test]
fn referring_to_an_unknown_node_is_an_error_that_names_it_and_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = id_of(&create_doc(f, client, &project)["doc"]);
    let first = a_paragraph(f, client, &project, &doc);
    let html_before = head_html(f, client, &agent(f), &project, &doc);
    let log_before = logged_writes(&doc);
    let events_before = events_of(&project, "doc.changed").len();

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
        let response = edit(f, client, &project, &doc, &ops);
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
        head_html(f, client, &agent(f), &project, &doc),
        html_before,
        "nothing changed"
    );
    assert_eq!(logged_writes(&doc), log_before, "nothing was logged");
    assert_eq!(
        events_of(&project, "doc.changed").len(),
        events_before,
        "nothing was emitted"
    );
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
    let project = new_project(f, client);
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
            create_live_response(
                f,
                client,
                &project,
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
