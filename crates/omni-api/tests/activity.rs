//! US-111: the activity list of a doc, who wrote what and when. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API (the agent),
//! create a scratch RustFS bucket and serve the API's router on a local port.
//! Docs are created and written with the live routes of US-102..US-104 and
//! renamed with the route of US-110.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`):
//!
//! - `GET /docs/{doc_id}/activity` (both: an agent and the human read it, in
//!   the route registry) lists the doc's writes, one entry per row of
//!   `doc_changes`, newest first: 200 `{"entries": [Entry], "next": <seq> |
//!   null}`. An entry is `{"seq", "agent", "created_at", "summary"}`: `seq`
//!   is the row's number (1 for the creation), `agent` the agent the write
//!   declared (`run`, `role`, `model`, `harness`), `created_at` the time in
//!   RFC 3339 UTC (`...Z`), and `summary` one line about the write.
//! - The list is paged: `?limit=<n>` (1 to 200, default 50) lists at most `n`
//!   entries and `?before=<seq>` only the entries below that seq. `next` is
//!   the `before` of the next page: the last seq listed when older entries
//!   remain, else null. A `limit` that is no number from 1 to 200 is 400
//!   `invalid_request`.
//! - The summary compares the head before and after the write by node: the
//!   nodes whose own content changed, the inserted, the removed and the moved
//!   ones. It lists the counts that are not zero in that order, separated by
//!   ", ", and the first one names the nodes: `1 node changed`, `3 nodes
//!   changed, 1 inserted`, `2 nodes removed`, `1 node changed, 2 inserted, 2
//!   removed`. The creation of a doc reads `created` and a rename `renamed to
//!   "<title>"`. A design counts the nodes of its pages and boards the same
//!   way (a board and its text are two nodes).
//! - `GET /docs/{doc_id}/activity/{seq}` (both) answers one write: 200
//!   `{"entry": Entry, "nodes": [Node], "boards": [Board]}`. For an HTML doc
//!   `nodes` lists every node that the write changed, inserted, removed or
//!   moved as `{"node", "kind", "change", "before", "after"}`: `change` is
//!   `changed`, `inserted`, `removed` or `moved`, and `before` and `after`
//!   are the element's HTML in the head before and after the write, `null`
//!   where it is not there. A node the write did not touch is not listed. For
//!   a design `boards` lists every board whose chunk the write changed as
//!   `{"board", "change"}`, `change` being `changed`, `inserted` or `removed`;
//!   a board it did not change is not listed. A seq the doc does not have is
//!   404 `not_found`.
//! - `GET /docs/{doc_id}/boards/{board_id}?at=<seq>` reads the board in the
//!   head that write `seq` left, so a write's board reads as it was before at
//!   `seq - 1` and as it is after at `seq`. A board that the head of that seq
//!   does not have is 404 `unknown_board`, and a seq the doc does not have is
//!   404 `not_found`.
//! - A doc that is deleted, or not in the project, is 404 on both routes.

const TEST_DB: &str = "omni_us111_activity";
const TEST_BUCKET: &str = "omni-us111-activity";

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

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
    format!("us111-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
    Auth::Agent(&ws.key, "run-111")
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
                Some(&json!({ "name": unique("activity-project") })),
            ),
            201,
            "create a project",
        )["project"],
        "id",
    )
    .to_string()
}

/// Creates a doc of `kind` at the app level without a change set (US-102);
/// returns its ID.
fn new_doc(ws: &Ws, client: &Client, project: &str, kind: &str) -> String {
    let pillar = if kind == "html" { "spec" } else { "ui" };
    str_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", project_base(ws, project)),
                &agent(ws),
                Some(&json!({ "kind": kind, "pillar": pillar, "title": unique(kind) })),
            ),
            201,
            "create a doc without a change set",
        )["doc"],
        "id",
    )
    .to_string()
}

/// Applies a batch to the doc's head as the agent; returns the results.
fn edit(ws: &Ws, client: &Client, project: &str, doc: &str, ops: &Value) -> Vec<Value> {
    let answer = must(
        call(
            client,
            "POST",
            &format!("{}/ops", doc_path(ws, project, doc)),
            &agent(ws),
            Some(&json!({ "ops": ops })),
        ),
        200,
        "apply a batch to the doc",
    );
    answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer lists results: {answer}"))
        .clone()
}

fn rename(ws: &Ws, client: &Client, project: &str, doc: &str, title: &str) {
    must(
        call(
            client,
            "PATCH",
            &doc_path(ws, project, doc),
            &agent(ws),
            Some(&json!({ "title": title })),
        ),
        200,
        "rename the doc",
    );
}

fn insert_paragraph(text: &str) -> Value {
    json!({
        "op": "insert",
        "parent": null,
        "position": { "after": null },
        "content": format!("<p>{text}</p>")
    })
}

fn set_html(node: &str, rev: u64, text: &str) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": { "html": text } })
}

fn delete(node: &str, rev: u64) -> Value {
    json!({ "op": "delete", "node": node, "rev": rev })
}

fn node_of(result: &Value) -> String {
    str_of(result, "node").to_string()
}

/// An HTML doc with a paragraph of each text, written in one batch (seq 2);
/// returns the doc and the paragraphs' nodes, in the order of `texts`.
fn doc_with_paragraphs(
    ws: &Ws,
    client: &Client,
    project: &str,
    texts: &[&str],
) -> (String, Vec<String>) {
    let doc = new_doc(ws, client, project, "html");
    let ops: Vec<Value> = texts.iter().map(|text| insert_paragraph(text)).collect();
    let results = edit(ws, client, project, &doc, &Value::Array(ops));
    (doc, results.iter().map(node_of).collect())
}

fn activity_on(
    client: &Client,
    ws: &Ws,
    auth: &Auth,
    project: &str,
    doc: &str,
    query: &str,
) -> Response {
    call(
        client,
        "GET",
        &format!("{}/activity{query}", doc_path(ws, project, doc)),
        auth,
        None,
    )
}

/// The activity list of the doc as the agent reads it, with `query` (empty, or
/// `?...`).
fn activity(ws: &Ws, client: &Client, project: &str, doc: &str, query: &str) -> Value {
    must(
        activity_on(client, ws, &agent(ws), project, doc, query),
        200,
        "read the doc's activity",
    )
}

fn entries(list: &Value) -> Vec<Value> {
    list["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("the activity lists entries: {list}"))
        .clone()
}

fn seqs(list: &Value) -> Vec<i64> {
    entries(list)
        .iter()
        .map(|entry| {
            entry["seq"]
                .as_i64()
                .unwrap_or_else(|| panic!("an entry has a seq: {entry}"))
        })
        .collect()
}

fn summaries(list: &Value) -> Vec<String> {
    entries(list)
        .iter()
        .map(|entry| str_of(entry, "summary").to_string())
        .collect()
}

fn detail_response(ws: &Ws, client: &Client, project: &str, doc: &str, seq: &str) -> Response {
    call(
        client,
        "GET",
        &format!("{}/activity/{seq}", doc_path(ws, project, doc)),
        &agent(ws),
        None,
    )
}

/// One write of the doc: `GET /docs/{doc_id}/activity/{seq}`.
fn detail(ws: &Ws, client: &Client, project: &str, doc: &str, seq: i64) -> Value {
    must(
        detail_response(ws, client, project, doc, &seq.to_string()),
        200,
        "read one write of the doc",
    )
}

/// The entry of `list` (the `nodes` or `boards` of a detail) whose `field` is
/// `id`.
fn listed<'a>(list: &'a Value, field: &str, id: &str) -> &'a Value {
    list.as_array()
        .unwrap_or_else(|| panic!("a list: {list}"))
        .iter()
        .find(|item| item[field] == id)
        .unwrap_or_else(|| panic!("{id} is listed: {list}"))
}

fn assert_error(response: &Response, status: u16, code: &str) {
    assert_eq!(response.status, status, "{}", response.text());
    assert_eq!(response.json()["error"], code, "{}", response.text());
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
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    })
}

/// The time of an entry as the API answers it: RFC 3339 in UTC, `...Z`.
fn is_utc_time(text: &str) -> bool {
    text.len() >= 20
        && text.ends_with('Z')
        && text.as_bytes()[4] == b'-'
        && text.as_bytes()[7] == b'-'
        && text.as_bytes()[10] == b'T'
}

// Designs ---------------------------------------------------------------------

/// The ID of the design's first page, as its head read shows it.
fn first_page(ws: &Ws, client: &Client, project: &str, doc: &str) -> String {
    let read = must(
        call(
            client,
            "GET",
            &format!("{}/html", doc_path(ws, project, doc)),
            &agent(ws),
            None,
        ),
        200,
        "read the design's head",
    );
    read["pages"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a new design has a page: {read}"))
        .to_string()
}

fn board_tag(name: &str, x: u32, text: &str) -> String {
    format!(
        "<Board name=\"{name}\" x={{{x}}} y={{0}} width={{200}} height={{100}} dir=\"col\">\n  <Text name=\"{name} label\" content=\"{text}\" fontFamily=\"Inter\" fontSize={{14}}/>\n</Board>"
    )
}

fn insert_board(page: &str, name: &str, x: u32, text: &str) -> Value {
    json!({ "op": "insert", "parent": page, "position": { "after": null }, "content": board_tag(name, x, text) })
}

fn set_content(node: &str, rev: u64, content: &str) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": { "content": content } })
}

/// A board inserted by `edit`: its node and its text's node.
struct Board {
    id: String,
    text: String,
}

fn board_of(result: &Value) -> Board {
    let created = result["created"]
        .as_array()
        .unwrap_or_else(|| panic!("an insert lists what it created: {result}"));
    assert_eq!(created.len(), 2, "a board and its text: {result}");
    Board {
        id: created[0].as_str().expect("an ID").to_string(),
        text: created[1].as_str().expect("an ID").to_string(),
    }
}

/// A design with three boards `Board i`, each with the text `Text i`,
/// inserted in one batch (seq 2); returns the design, its page and the boards.
fn design_with_three_boards(
    ws: &Ws,
    client: &Client,
    project: &str,
) -> (String, String, Vec<Board>) {
    let doc = new_doc(ws, client, project, "design");
    let page = first_page(ws, client, project, &doc);
    let ops: Vec<Value> = (0..3)
        .map(|i| insert_board(&page, &format!("Board {i}"), i * 240, &format!("Text {i}")))
        .collect();
    let results = edit(ws, client, project, &doc, &Value::Array(ops));
    (doc, page, results.iter().map(board_of).collect())
}

fn board_at(ws: &Ws, client: &Client, project: &str, doc: &str, board: &str, at: &str) -> Response {
    call(
        client,
        "GET",
        &format!("{}/boards/{board}?at={at}", doc_path(ws, project, doc)),
        &agent(ws),
        None,
    )
}

/// The tags of a board in the head that write `at` left.
fn board_tags_at(
    ws: &Ws,
    client: &Client,
    project: &str,
    doc: &str,
    board: &str,
    at: i64,
) -> String {
    let read = must(
        board_at(ws, client, project, doc, board, &at.to_string()),
        200,
        "read a board as a write left it",
    );
    str_of(&read, "tags").to_string()
}

// ---------------------------------------------------------------------------
// The route registry
// ---------------------------------------------------------------------------

#[test]
fn the_activity_routes_are_in_the_registry_for_agents_and_the_human() {
    for path in [
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/activity",
        "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/activity/{seq}",
    ] {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == "GET" && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route GET {path}"));
        assert_eq!(entry.access, Access::Both, "GET {path}");
    }
}

// ---------------------------------------------------------------------------
// The list: one entry per write, with the agent, the time and a summary
// ---------------------------------------------------------------------------

#[test]
fn the_activity_lists_one_entry_per_write_newest_first_with_the_agent_the_time_and_a_summary() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    // seq 1 creates the doc, seq 2 inserts three paragraphs.
    let (doc, nodes) = doc_with_paragraphs(ws, client, &project, &["One", "Two", "Three"]);
    // seq 3 changes the three and inserts a fourth.
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([
            set_html(&nodes[0], 1, "One again"),
            set_html(&nodes[1], 1, "Two again"),
            set_html(&nodes[2], 1, "Three again"),
            insert_paragraph("Four"),
        ]),
    );
    // seq 4 removes one, seq 5 renames the doc.
    edit(ws, client, &project, &doc, &json!([delete(&nodes[1], 2)]));
    let title = unique("Checkout plan");
    rename(ws, client, &project, &doc, &title);

    let list = activity(ws, client, &project, &doc, "");
    assert_eq!(seqs(&list), [5, 4, 3, 2, 1], "newest first: {list}");
    assert_eq!(
        entries(&list).len() as i64,
        count(
            "SELECT count(*) FROM doc_changes WHERE doc_id = $1::uuid",
            &[&doc]
        ),
        "one entry per row of doc_changes: {list}"
    );
    assert_eq!(
        summaries(&list),
        [
            format!("renamed to \"{title}\""),
            "1 node removed".to_string(),
            "3 nodes changed, 1 inserted".to_string(),
            "3 nodes inserted".to_string(),
            "created".to_string(),
        ],
        "{list}"
    );
    let mut previous: Option<String> = None;
    for entry in entries(&list) {
        assert_eq!(
            entry["agent"]["role"], "designer",
            "the agent's role: {entry}"
        );
        assert_eq!(entry["agent"]["run"], "run-111", "the agent's run: {entry}");
        let at = str_of(&entry, "created_at").to_string();
        assert!(is_utc_time(&at), "the time is RFC 3339 UTC: {entry}");
        if let Some(newer) = &previous {
            assert!(at <= *newer, "newest first: {list}");
        }
        assert!(
            !str_of(&entry, "summary").contains('\n'),
            "the summary is one line: {entry}"
        );
        previous = Some(at);
    }
    assert!(list["next"].is_null(), "nothing older remains: {list}");
}

#[test]
fn the_human_reads_the_same_activity_as_the_agent() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, _) = doc_with_paragraphs(ws, client, &project, &["Seen by both"]);
    let by_agent = activity(ws, client, &project, &doc, "");
    let by_human = must(
        activity_on(client, ws, &human(ws), &project, &doc, ""),
        200,
        "the human reads the doc's activity",
    );
    assert_eq!(seqs(&by_human), [2, 1], "{by_human}");
    assert_eq!(by_human, by_agent);
}

#[test]
fn the_activity_is_paged_with_limit_and_before() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, nodes) = doc_with_paragraphs(ws, client, &project, &["Paged"]);
    for rev in 1..=3 {
        edit(
            ws,
            client,
            &project,
            &doc,
            &json!([set_html(&nodes[0], rev, &format!("Paged {rev}"))]),
        );
    }
    // Five writes: the creation, the insert and three changes.
    let first = activity(ws, client, &project, &doc, "?limit=2");
    assert_eq!(seqs(&first), [5, 4], "{first}");
    assert_eq!(first["next"], 4, "{first}");
    let second = activity(ws, client, &project, &doc, "?limit=2&before=4");
    assert_eq!(seqs(&second), [3, 2], "{second}");
    assert_eq!(second["next"], 2, "{second}");
    let last = activity(ws, client, &project, &doc, "?limit=2&before=2");
    assert_eq!(seqs(&last), [1], "{last}");
    assert!(last["next"].is_null(), "{last}");
    assert_eq!(
        seqs(&activity(ws, client, &project, &doc, "?before=4")),
        [3, 2, 1]
    );
    for query in ["?limit=0", "?limit=201", "?limit=many"] {
        assert_error(
            &activity_on(client, ws, &agent(ws), &project, &doc, query),
            400,
            "invalid_request",
        );
    }
}

#[test]
fn a_design_write_is_summarised_by_the_nodes_of_its_boards() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, page, boards) = design_with_three_boards(ws, client, &project);
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([set_content(&boards[0].text, 1, "Updated 0")]),
    );
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([
            delete(&boards[2].id, 1),
            insert_board(&page, "Board 3", 720, "Text 3"),
        ]),
    );
    let list = activity(ws, client, &project, &doc, "");
    assert_eq!(seqs(&list), [4, 3, 2, 1], "{list}");
    assert_eq!(
        summaries(&list),
        [
            "2 nodes inserted, 2 removed",
            "1 node changed",
            "6 nodes inserted",
            "created",
        ],
        "{list}"
    );
}

// ---------------------------------------------------------------------------
// The detail of a write to an HTML doc: each changed element, before and after
// ---------------------------------------------------------------------------

#[test]
fn an_html_write_shows_the_html_of_each_element_it_touched_before_and_after() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, nodes) = doc_with_paragraphs(
        ws,
        client,
        &project,
        &["Alpha one", "Beta one", "Gamma one"],
    );
    let results = edit(
        ws,
        client,
        &project,
        &doc,
        &json!([
            set_html(&nodes[0], 1, "Alpha two"),
            delete(&nodes[1], 1),
            insert_paragraph("Delta one"),
        ]),
    );
    let inserted = node_of(&results[2]);

    let write = detail(ws, client, &project, &doc, 3);
    assert_eq!(write["entry"]["seq"], 3, "{write}");
    assert_eq!(write["entry"]["agent"]["role"], "designer", "{write}");
    assert_eq!(
        write["entry"]["summary"], "1 node changed, 1 inserted, 1 removed",
        "{write}"
    );
    let touched = &write["nodes"];
    assert_eq!(
        touched.as_array().map(Vec::len),
        Some(3),
        "only the three touched elements are listed: {write}"
    );

    let changed = listed(touched, "node", &nodes[0]);
    assert_eq!(changed["change"], "changed", "{changed}");
    assert_eq!(changed["kind"], "paragraph", "{changed}");
    let before = str_of(changed, "before");
    let after = str_of(changed, "after");
    assert!(
        before.starts_with("<p") && before.contains("Alpha one") && !before.contains("Alpha two"),
        "the element before the write: {changed}"
    );
    assert!(
        after.starts_with("<p") && after.contains("Alpha two") && !after.contains("Alpha one"),
        "the element after the write: {changed}"
    );

    let removed = listed(touched, "node", &nodes[1]);
    assert_eq!(removed["change"], "removed", "{removed}");
    assert!(
        str_of(removed, "before").contains("Beta one"),
        "a removed element shows what it was: {removed}"
    );
    assert!(removed["after"].is_null(), "{removed}");

    let new = listed(touched, "node", &inserted);
    assert_eq!(new["change"], "inserted", "{new}");
    assert!(new["before"].is_null(), "{new}");
    assert!(
        str_of(new, "after").contains("Delta one"),
        "an inserted element shows what it is: {new}"
    );

    assert!(
        !touched.to_string().contains("Gamma one"),
        "an element the write did not touch is not listed: {write}"
    );
}

#[test]
fn the_creation_of_a_doc_is_its_first_write_and_an_unknown_seq_is_not_found() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, _) = doc_with_paragraphs(ws, client, &project, &["First"]);
    let created = detail(ws, client, &project, &doc, 1);
    assert_eq!(created["entry"]["seq"], 1, "{created}");
    assert_eq!(created["entry"]["summary"], "created", "{created}");
    assert_eq!(created["nodes"], json!([]), "{created}");
    let inserted = detail(ws, client, &project, &doc, 2);
    assert_eq!(
        inserted["entry"]["summary"], "1 node inserted",
        "{inserted}"
    );
    for seq in ["3", "0", "99"] {
        assert_error(
            &detail_response(ws, client, &project, &doc, seq),
            404,
            "not_found",
        );
    }
}

// ---------------------------------------------------------------------------
// The detail of a write to a design: the changed boards, readable at a seq
// ---------------------------------------------------------------------------

#[test]
fn a_design_write_lists_the_boards_it_changed() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, page, boards) = design_with_three_boards(ws, client, &project);
    let results = edit(
        ws,
        client,
        &project,
        &doc,
        &json!([
            set_content(&boards[0].text, 1, "Updated 0"),
            delete(&boards[2].id, 1),
            insert_board(&page, "Board 3", 720, "Text 3"),
        ]),
    );
    let new = board_of(&results[2]);

    let write = detail(ws, client, &project, &doc, 3);
    assert_eq!(write["entry"]["seq"], 3, "{write}");
    assert_eq!(
        write["entry"]["summary"], "1 node changed, 2 inserted, 2 removed",
        "{write}"
    );
    let listed_boards = &write["boards"];
    assert_eq!(
        listed_boards.as_array().map(Vec::len),
        Some(3),
        "only the three changed boards are listed: {write}"
    );
    assert_eq!(
        listed(listed_boards, "board", &boards[0].id)["change"],
        "changed"
    );
    assert_eq!(
        listed(listed_boards, "board", &boards[2].id)["change"],
        "removed"
    );
    assert_eq!(
        listed(listed_boards, "board", &new.id)["change"],
        "inserted"
    );
    assert!(
        !listed_boards.to_string().contains(&boards[1].id),
        "a board the write did not change is not listed: {write}"
    );

    let inserted = detail(ws, client, &project, &doc, 2);
    let all = &inserted["boards"];
    assert_eq!(all.as_array().map(Vec::len), Some(3), "{inserted}");
    for board in &boards {
        assert_eq!(listed(all, "board", &board.id)["change"], "inserted");
    }
}

#[test]
fn a_board_reads_as_each_write_left_it_with_at() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, page, boards) = design_with_three_boards(ws, client, &project);
    let results = edit(
        ws,
        client,
        &project,
        &doc,
        &json!([
            set_content(&boards[0].text, 1, "Updated 0"),
            delete(&boards[2].id, 1),
            insert_board(&page, "Board 3", 720, "Text 3"),
        ]),
    );
    let new = board_of(&results[2]);

    // A changed board: before at seq 2, after at seq 3.
    let before = board_tags_at(ws, client, &project, &doc, &boards[0].id, 2);
    let after = board_tags_at(ws, client, &project, &doc, &boards[0].id, 3);
    assert!(
        before.contains("Text 0") && !before.contains("Updated 0"),
        "the board before the write: {before}"
    );
    assert!(
        after.contains("Updated 0") && !after.contains("Text 0"),
        "the board after the write: {after}"
    );

    // A removed board is there before and gone after; a new one the other way.
    let gone = board_tags_at(ws, client, &project, &doc, &boards[2].id, 2);
    assert!(gone.contains("Text 2"), "{gone}");
    assert_error(
        &board_at(ws, client, &project, &doc, &boards[2].id, "3"),
        404,
        "unknown_board",
    );
    let added = board_tags_at(ws, client, &project, &doc, &new.id, 3);
    assert!(added.contains("Text 3"), "{added}");
    assert_error(
        &board_at(ws, client, &project, &doc, &new.id, "2"),
        404,
        "unknown_board",
    );

    // The new design's head holds no board, and a later write still reads
    // back; the head read without `at` is the latest.
    assert_error(
        &board_at(ws, client, &project, &doc, &boards[0].id, "1"),
        404,
        "unknown_board",
    );
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([set_content(&boards[0].text, 2, "Updated again")]),
    );
    let still = board_tags_at(ws, client, &project, &doc, &boards[0].id, 3);
    assert!(
        still.contains("Updated 0") && !still.contains("Updated again"),
        "{still}"
    );
    let latest = must(
        call(
            client,
            "GET",
            &format!("{}/boards/{}", doc_path(ws, &project, &doc), boards[0].id),
            &agent(ws),
            None,
        ),
        200,
        "read the board of the head",
    );
    assert!(
        str_of(&latest, "tags").contains("Updated again"),
        "{latest}"
    );

    assert_error(
        &board_at(ws, client, &project, &doc, &boards[0].id, "99"),
        404,
        "not_found",
    );
}

// ---------------------------------------------------------------------------
// Docs that are not there
// ---------------------------------------------------------------------------

#[test]
fn the_activity_of_a_deleted_doc_or_one_of_another_project_is_not_found() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let other = new_project(ws, client);
    let (doc, _) = doc_with_paragraphs(ws, client, &project, &["Elsewhere"]);
    assert_error(
        &activity_on(client, ws, &agent(ws), &other, &doc, ""),
        404,
        "not_found",
    );
    assert_error(
        &detail_response(ws, client, &other, &doc, "1"),
        404,
        "not_found",
    );

    let response = call(
        client,
        "DELETE",
        &doc_path(ws, &project, &doc),
        &agent(ws),
        None,
    );
    assert_eq!(response.status, 204, "delete the doc: {}", response.text());
    for auth in [agent(ws), human(ws)] {
        assert_error(
            &activity_on(client, ws, &auth, &project, &doc, ""),
            404,
            "not_found",
        );
    }
    assert_error(
        &detail_response(ws, client, &project, &doc, "1"),
        404,
        "not_found",
    );
}
