//! US-111: the activity list of a doc. Run with `--features stack-tests` in
//! the Compose test container, which reaches Postgres at `postgres:5432` and
//! RustFS at `rustfs:9000`.

const TEST_DB: &str = "omni_us111_activity";
const TEST_BUCKET: &str = "omni-us111-activity";

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
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

/// Creates a doc of `kind` at the app level (US-102);
/// returns its ID.
fn new_doc(ws: &Ws, client: &Client, project: &str, kind: &str) -> String {
    new_titled_doc(ws, client, project, kind, &unique(kind))
}

fn new_titled_doc(ws: &Ws, client: &Client, project: &str, kind: &str, title: &str) -> String {
    let pillar = if kind == "html" { "spec" } else { "ui" };
    str_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", project_base(ws, project)),
                &agent(ws),
                Some(&json!({ "kind": kind, "pillar": pillar, "title": title })),
            ),
            201,
            "create a doc",
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
fn move_to(node: &str, rev: u64, parent: Option<&str>, after: Option<&str>) -> Value {
    json!({ "op": "move", "node": node, "rev": rev, "parent": parent, "position": { "after": after } })
}

fn insert_into(parent: &str, after: Option<&str>, content: &str) -> Value {
    json!({ "op": "insert", "parent": parent, "position": { "after": after }, "content": content })
}

/// A paragraph as the activity shows it: its outer HTML with `data-omni-id`.
fn paragraph(node: &str, text: &str) -> String {
    format!("<p data-omni-id=\"{node}\">{text}</p>")
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

// The design system -----------------------------------------------------------

fn put_tokens(ws: &Ws, client: &Client, project: &str, tree: &Value) {
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
    );
}

/// A token with a value for the modes light and dark.
fn token(light: &str, dark: &str) -> Value {
    json!({
        "$value": light,
        "$extensions": { "omnistate": { "modes": { "light": light, "dark": dark } } }
    })
}

/// A Button component whose label says `label`; its `id` is kept when given.
fn button_tags(id: Option<&str>, label: &str) -> String {
    let id = id.map(|id| format!(" id=\"{id}\"")).unwrap_or_default();
    format!(
        "<Component{id} name=\"Button\" dir=\"row\" pad={{8}} fill=\"#2563eb\">\n  <Text name=\"Button label\" content=\"{label}\" fontFamily=\"Inter\" fontSize={{14}}/>\n</Component>"
    )
}

/// Imports the components into the design system's head; returns the ID of
/// the first one.
fn put_components(ws: &Ws, client: &Client, project: &str, tags: &str) -> String {
    let imported = must(
        call(
            client,
            "PUT",
            &format!("{}/components", project_base(ws, project)),
            &agent(ws),
            Some(&json!({ "tags": tags })),
        ),
        200,
        "import the components into the design system's head",
    );
    str_of(&imported["components"][0], "id").to_string()
}

fn component_at(ws: &Ws, client: &Client, project: &str, component: &str, at: &str) -> Response {
    call(
        client,
        "GET",
        &format!(
            "{}/components/{component}?at={at}",
            project_base(ws, project)
        ),
        &agent(ws),
        None,
    )
}

// ---------------------------------------------------------------------------
// The list: one entry per write, with its kind, the agent, the time and a summary
// ---------------------------------------------------------------------------

#[test]
fn the_activity_lists_one_entry_per_write_newest_first_with_its_kind_the_agent_the_time_and_a_summary()
 {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let first_title = unique("Checkout draft");
    let doc = new_titled_doc(ws, client, &project, "html", &first_title);
    let results = edit(
        ws,
        client,
        &project,
        &doc,
        &json!([
            insert_paragraph("One"),
            insert_paragraph("Two"),
            insert_paragraph("Three")
        ]),
    );
    let nodes: Vec<String> = results.iter().map(node_of).collect();
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
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([move_to(&nodes[0], 2, None, None)]),
    );
    let title = unique("Checkout plan");
    rename(ws, client, &project, &doc, &title);

    let list = activity(ws, client, &project, &doc, "");
    let all = entries(&list);
    assert_eq!(seqs(&list), [5, 4, 3, 2, 1], "newest first: {list}");
    assert_eq!(
        all.iter()
            .map(|entry| entry["kind"].clone())
            .collect::<Vec<_>>(),
        ["rename", "edit", "edit", "edit", "create"],
        "{list}"
    );
    assert_eq!(
        summaries(&list),
        [
            format!("Renamed from '{first_title}' to '{title}'"),
            "1 moved".to_string(),
            "3 nodes changed, 1 inserted".to_string(),
            "3 inserted".to_string(),
            "Created".to_string(),
        ],
        "{list}"
    );
    assert_eq!(
        all[0]["agent"],
        json!({ "run": "run-111", "role": "designer", "model": "test-model", "harness": "test-harness" }),
        "{list}"
    );
    assert!(
        is_utc_time(str_of(&all[0], "created_at")),
        "the time is RFC 3339 UTC: {list}"
    );
    assert!(
        list["next_before"].is_null(),
        "nothing older remains: {list}"
    );
}

#[test]
fn the_activity_is_paged_with_limit_and_before() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, nodes) = doc_with_paragraphs(ws, client, &project, &["Paged"]);
    for rev in 1..=49 {
        edit(
            ws,
            client,
            &project,
            &doc,
            &json!([set_html(&nodes[0], rev, &format!("Paged {rev}"))]),
        );
    }
    // 51 writes: the creation, the insert and 49 changes.
    let default = activity(ws, client, &project, &doc, "");
    assert_eq!(entries(&default).len(), 50, "{default}");
    assert_eq!(seqs(&default)[0], 51, "{default}");
    assert_eq!(default["next_before"], 2, "{default}");
    let most = activity(ws, client, &project, &doc, "?limit=200");
    assert_eq!(entries(&most).len(), 51, "{most}");
    assert!(most["next_before"].is_null(), "{most}");

    let first = activity(ws, client, &project, &doc, "?limit=2");
    assert_eq!(seqs(&first), [51, 50], "{first}");
    assert_eq!(first["next_before"], 50, "{first}");
    let second = activity(ws, client, &project, &doc, "?limit=2&before=50");
    assert_eq!(seqs(&second), [49, 48], "{second}");
    assert_eq!(second["next_before"], 48, "{second}");
    let last = activity(ws, client, &project, &doc, "?limit=2&before=2");
    assert_eq!(seqs(&last), [1], "{last}");
    assert!(last["next_before"].is_null(), "{last}");
    for query in ["?limit=0", "?limit=201", "?limit=many"] {
        assert_error(
            &activity_on(client, ws, &agent(ws), &project, &doc, query),
            400,
            "invalid_request",
        );
    }
}

// ---------------------------------------------------------------------------
// The detail of a write to an HTML doc: each element it acted on, before and after
// ---------------------------------------------------------------------------

#[test]
fn a_batch_that_deletes_a_node_is_an_edit_that_shows_the_element_before_and_nothing_after() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, nodes) = doc_with_paragraphs(ws, client, &project, &["Doomed"]);
    edit(ws, client, &project, &doc, &json!([delete(&nodes[0], 1)]));

    let list = activity(ws, client, &project, &doc, "?limit=1");
    assert_eq!(entries(&list)[0]["kind"], "edit", "{list}");
    assert_eq!(summaries(&list), ["1 deleted"], "{list}");
    let write = detail(ws, client, &project, &doc, 3);
    assert_eq!(write["kind"], "edit", "{write}");
    assert_eq!(
        write["items"],
        json!([{ "node": nodes[0], "before": paragraph(&nodes[0], "Doomed"), "after": null }]),
        "{write}"
    );
}

#[test]
fn an_html_write_shows_each_node_an_operation_acted_on_once_in_the_order_it_touched_them() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let doc = new_doc(ws, client, &project, "html");
    let results = edit(
        ws,
        client,
        &project,
        &doc,
        &json!([
            insert_paragraph("Gamma one"),
            insert_paragraph("Delta one"),
            {
                "op": "insert",
                "parent": null,
                "position": { "after": null },
                "content": "<section><p>Alpha one</p><p>Beta one</p></section>"
            },
        ]),
    );
    let (gamma, delta) = (node_of(&results[0]), node_of(&results[1]));
    let created = &results[2]["created"];
    let section = created[0].as_str().expect("the section's ID").to_string();
    let alpha = created[1]
        .as_str()
        .expect("the first paragraph's ID")
        .to_string();
    let beta = created[2]
        .as_str()
        .expect("the second paragraph's ID")
        .to_string();

    // Delta moves into the section after Beta, Alpha changes inside it, a
    // paragraph is inserted and deleted again, Epsilon is inserted at the top
    // of the section, and Gamma both changes and moves.
    let results = edit(
        ws,
        client,
        &project,
        &doc,
        &json!([
            move_to(&delta, 1, Some(&section), Some(&beta)),
            set_html(&alpha, 1, "Alpha two"),
            insert_into(
                &section,
                Some(&alpha),
                "<p data-omni-id=\"zz000001\">Brief</p>"
            ),
            delete("zz000001", 1),
            insert_into(&section, None, "<p>Epsilon</p>"),
            set_html(&gamma, 1, "Gamma two"),
            move_to(&gamma, 2, None, None),
        ]),
    );
    let epsilon = node_of(&results[4]);

    let write = detail(ws, client, &project, &doc, 3);
    assert_eq!(
        write["summary"], "2 nodes changed, 2 inserted, 1 deleted, 1 moved",
        "{write}"
    );
    assert_eq!(
        write["items"],
        json!([
            { "node": delta, "before": paragraph(&delta, "Delta one"), "after": paragraph(&delta, "Delta one") },
            { "node": alpha, "before": paragraph(&alpha, "Alpha one"), "after": paragraph(&alpha, "Alpha two") },
            { "node": epsilon, "before": null, "after": paragraph(&epsilon, "Epsilon") },
            { "node": gamma, "before": paragraph(&gamma, "Gamma one"), "after": paragraph(&gamma, "Gamma two") },
        ]),
        "the section, Beta and the paragraph that came and went get no item: {write}"
    );
}

#[test]
fn a_set_to_the_same_value_counts_as_one_node_changed() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, nodes) = doc_with_paragraphs(ws, client, &project, &["Same"]);
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([set_html(&nodes[0], 1, "Same")]),
    );

    let write = detail(ws, client, &project, &doc, 3);
    assert_eq!(write["summary"], "1 node changed", "{write}");
    assert_eq!(
        write["items"],
        json!([{ "node": nodes[0], "before": paragraph(&nodes[0], "Same"), "after": paragraph(&nodes[0], "Same") }]),
        "{write}"
    );
}

#[test]
fn a_creation_shows_the_whole_doc_and_a_rename_the_old_and_new_title() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (before, after) = (unique("Old title"), unique("New title"));
    let doc = new_titled_doc(ws, client, &project, "html", &before);
    rename(ws, client, &project, &doc, &after);

    let created = detail(ws, client, &project, &doc, 1);
    assert_eq!(created["kind"], "create", "{created}");
    assert_eq!(
        created["items"],
        json!([{ "node": null, "before": null, "after": "" }]),
        "{created}"
    );
    assert!(created.get("title").is_none(), "{created}");
    let renamed = detail(ws, client, &project, &doc, 2);
    assert_eq!(renamed["kind"], "rename", "{renamed}");
    assert_eq!(
        renamed["title"],
        json!({ "before": before, "after": after }),
        "{renamed}"
    );
    assert_eq!(renamed["items"], json!([]), "{renamed}");
}

// ---------------------------------------------------------------------------
// The detail of a write to a design: the changed boards, readable at a seq
// ---------------------------------------------------------------------------

#[test]
fn a_batch_that_removes_a_board_is_an_edit_that_lists_only_that_board() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, _, boards) = design_with_three_boards(ws, client, &project);
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([delete(&boards[2].id, 1)]),
    );

    let write = detail(ws, client, &project, &doc, 3);
    assert_eq!(write["kind"], "edit", "{write}");
    assert_eq!(write["summary"], "1 removed", "{write}");
    assert_eq!(
        write["items"],
        json!([{ "board": boards[2].id, "before_at": 2, "after_at": null }]),
        "{write}"
    );
}

#[test]
fn a_design_write_that_only_moves_a_board_lists_no_board_and_says_no_changes() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let (doc, page, boards) = design_with_three_boards(ws, client, &project);
    edit(
        ws,
        client,
        &project,
        &doc,
        &json!([move_to(&boards[0].id, 1, Some(&page), Some(&boards[2].id))]),
    );
    let head = must(
        call(
            client,
            "GET",
            &format!("{}/html", doc_path(ws, &project, &doc)),
            &agent(ws),
            None,
        ),
        200,
        "read the design's head",
    );
    let order: Vec<&str> = head["pages"][0]["children"]
        .as_array()
        .unwrap_or_else(|| panic!("the page lists its boards: {head}"))
        .iter()
        .map(|board| str_of(board, "id"))
        .collect();
    assert_eq!(
        order,
        [&boards[2].id, &boards[0].id, &boards[1].id],
        "each insert went first, then Board 0 moved after Board 2"
    );

    let write = detail(ws, client, &project, &doc, 3);
    assert_eq!(write["kind"], "edit", "{write}");
    assert_eq!(write["summary"], "No changes", "{write}");
    assert_eq!(write["items"], json!([]), "{write}");
}

#[test]
fn a_design_write_lists_each_board_it_changed_or_added_with_the_seqs_to_read_it_at() {
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
            insert_board(&page, "Board 3", 720, "Text 3"),
        ]),
    );
    let new = board_of(&results[1]);

    let write = detail(ws, client, &project, &doc, 3);
    assert_eq!(write["summary"], "1 board changed, 1 added", "{write}");
    let items = &write["items"];
    assert_eq!(items.as_array().map(Vec::len), Some(2), "{write}");
    assert_eq!(
        *listed(items, "board", &boards[0].id),
        json!({ "board": boards[0].id, "before_at": 2, "after_at": 3 })
    );
    assert_eq!(
        *listed(items, "board", &new.id),
        json!({ "board": new.id, "before_at": null, "after_at": 3 })
    );
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

    // A later write leaves the earlier seq as it was; the read without `at`
    // is the head.
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

    for at in ["0", "5", "x"] {
        assert_error(
            &board_at(ws, client, &project, &doc, &boards[0].id, at),
            404,
            "not_found",
        );
    }
}

// ---------------------------------------------------------------------------
// The detail of a write to a design system: components and tokens
// ---------------------------------------------------------------------------

#[test]
fn a_token_change_lists_only_the_tokens_it_added_changed_or_removed_in_their_typed_groups() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let system = new_doc(ws, client, &project, "design-system");
    let modes = json!({ "omnistate": { "modes": ["light", "dark"] } });
    put_tokens(
        ws,
        client,
        &project,
        &json!({ "$extensions": modes, "color": { "$type": "color", "brand": token("#ff0000", "#110000") } }),
    );
    put_tokens(
        ws,
        client,
        &project,
        &json!({
            "$extensions": modes,
            "color": { "$type": "color", "brand": token("#ff0000", "#220000"), "ink": token("#000000", "#ffffff") }
        }),
    );
    put_tokens(
        ws,
        client,
        &project,
        &json!({
            "$extensions": modes,
            "color": { "$type": "color", "ink": token("#000000", "#ffffff"), "mist": token("#eeeeee", "#333333") }
        }),
    );

    let created = detail(ws, client, &project, &system, 1);
    assert_eq!(
        created["tokens"],
        json!({ "before": null, "after": {} }),
        "{created}"
    );
    let first = detail(ws, client, &project, &system, 2);
    assert_eq!(
        first["tokens"]["after"],
        json!({ "color": { "$type": "color", "brand": token("#ff0000", "#110000") } }),
        "{first}"
    );
    let changed = detail(ws, client, &project, &system, 3);
    assert_eq!(
        changed["tokens"],
        json!({
            "before": { "color": { "$type": "color", "brand": token("#ff0000", "#110000") } },
            "after": { "color": { "$type": "color", "brand": token("#ff0000", "#220000"), "ink": token("#000000", "#ffffff") } }
        }),
        "an added token is only in after: {changed}"
    );
    let removed = detail(ws, client, &project, &system, 4);
    assert_eq!(removed["summary"], "2 tokens changed", "{removed}");
    assert_eq!(
        removed["tokens"],
        json!({
            "before": { "color": { "$type": "color", "brand": token("#ff0000", "#220000") } },
            "after": { "color": { "$type": "color", "mist": token("#eeeeee", "#333333") } }
        }),
        "a removed token is only in before, an unchanged one in neither: {removed}"
    );
    assert_eq!(removed["items"], json!([]), "{removed}");
}

#[test]
fn a_component_change_lists_the_component_and_it_reads_as_each_write_left_it_with_at() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let system = new_doc(ws, client, &project, "design-system");
    let button = put_components(ws, client, &project, &button_tags(None, "Buy"));
    put_components(ws, client, &project, &button_tags(Some(&button), "Pay"));

    let added = detail(ws, client, &project, &system, 2);
    assert_eq!(
        added["items"],
        json!([{ "component": button, "before_at": null, "after_at": 2 }]),
        "{added}"
    );
    let changed = detail(ws, client, &project, &system, 3);
    assert_eq!(changed["summary"], "1 component changed", "{changed}");
    assert_eq!(
        changed["items"],
        json!([{ "component": button, "before_at": 2, "after_at": 3 }]),
        "{changed}"
    );
    assert!(
        changed.get("tokens").is_none(),
        "no token changed: {changed}"
    );

    let before = must(
        component_at(ws, client, &project, &button, "2"),
        200,
        "read the component at seq 2",
    );
    assert!(
        str_of(&before, "tags").contains("\"Buy\""),
        "the component before the write: {before}"
    );
    let after = must(
        component_at(ws, client, &project, &button, "3"),
        200,
        "read the component at seq 3",
    );
    assert!(
        str_of(&after, "tags").contains("\"Pay\""),
        "the component after the write: {after}"
    );
    assert_error(
        &component_at(ws, client, &project, &button, "1"),
        404,
        "unknown_component",
    );
}

// ---------------------------------------------------------------------------
// Docs and writes that are not there
// ---------------------------------------------------------------------------

#[test]
fn the_activity_of_a_deleted_doc_an_unknown_seq_or_a_doc_of_another_project_is_not_found() {
    let (ws, client) = ready();
    let project = new_project(ws, client);
    let other = new_project(ws, client);
    let (doc, _) = doc_with_paragraphs(ws, client, &project, &["Elsewhere"]);
    for seq in ["0", "3", "x"] {
        assert_error(
            &detail_response(ws, client, &project, &doc, seq),
            404,
            "not_found",
        );
    }
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
