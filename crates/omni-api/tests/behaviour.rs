//! US-201: the behaviour tree of a feature, through the API. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API and serve the
//! API's router on a local port against that database.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour`):
//!
//! - `POST /nodes` with `{title, slug?, summary?, parent_id?}` answers 201
//!   `{node}`. The first node of a feature also creates the root, titled with
//!   the feature's name, at path `""` and level 0. No `parent_id` makes the
//!   node a child of the root (a user story, level 1); below a story is level
//!   2. A missing slug comes from the title: lowercase, every run of other
//!   characters one `-`, cut to 60 characters, `-2`, `-3` on a clash among
//!   siblings. A new node is `open` at `version` 1 and stores its author:
//!   `{kind: "agent", run, role, ...}` for an API key, `{kind: "human",
//!   user_id}` for a session.
//! - Refused with nothing written: a blank title (422 `invalid_title`), a
//!   slug outside `^[a-z0-9]+(-[a-z0-9]+)*$` or over 80 characters (422
//!   `invalid_slug`), a slug a sibling has (409 `slug_taken`), an API key
//!   without `X-Omni-Run` (400 `identity_required`), a `parent_id` of another
//!   feature's node (404).
//! - `GET /nodes/{id}` answers `{node, chain}`, `chain` the nodes above it,
//!   root first; a node of another feature is 404. `GET /` lists every node
//!   in tree order (depth first, siblings by position) without the
//!   `situation` and `behaviour` text.
//! - Each created node, the root included, emits `behaviour.node_created`
//!   with `{feature_id, node_id, parent_id, path}`.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us201_behaviour";

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
        // Nothing here reaches RustFS: the tree only uses Postgres.
        s3_endpoint: "http://127.0.0.1:1".to_string(),
        s3_access_key: "none".to_string(),
        s3_secret_key: "none".to_string(),
        s3_bucket: "omni".to_string(),
    }
}

async fn admin_connection() -> PgConnection {
    PgConnection::connect_with(&admin_options().database(TEST_DB))
        .await
        .expect("connect to the scratch database as the admin role")
}

struct Fixture {
    workspace: String,
    alice: String,
    alice_token: String,
    key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us201-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
        let response = server.client.request(
            "POST",
            &format!("/api/v1/w/{workspace}/api-keys"),
            &[("Cookie", &cookie(&session_token("alice")))],
            Some("{}"),
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

/// Who sends a request: the grill agent `t1/grill` with the API key, the key
/// without a run, or alice with her session.
#[derive(Clone, Copy)]
enum Auth {
    Agent,
    AgentWithoutRun,
    Session,
}

fn call(f: &Fixture, method: &str, path: &str, auth: Auth, body: Option<&Value>) -> Response {
    let bearer = format!("Bearer {}", f.key);
    let headers: Vec<(&str, String)> = match auth {
        Auth::Agent => vec![
            ("Authorization", bearer),
            ("X-Omni-Run", "t1".to_string()),
            ("X-Omni-Role", "grill".to_string()),
        ],
        Auth::AgentWithoutRun => vec![
            ("Authorization", bearer),
            ("X-Omni-Role", "grill".to_string()),
        ],
        Auth::Session => vec![("Cookie", cookie(&f.alice_token))],
    };
    let headers: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let body = body.map(Value::to_string);
    f.server
        .client
        .request(method, path, &headers, body.as_deref())
}

fn must(response: Response, status: u16, what: &str) -> Value {
    assert_eq!(response.status, status, "{what}: {}", response.text());
    response.json()
}

fn assert_error(response: &Response, status: u16, code: &str) {
    assert_eq!(response.status, status, "body: {}", response.text());
    assert_eq!(response.json(), json!({ "error": code }));
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

/// A new project and a feature in it, named `feature_name`: `(project, feature)`.
fn new_feature(f: &Fixture, feature_name: &str) -> (String, String) {
    let created = must(
        call(
            f,
            "POST",
            &format!("/api/v1/w/{}/projects", f.workspace),
            Auth::Agent,
            Some(&json!({ "name": unique("behaviour-project") })),
        ),
        201,
        "create a project",
    );
    let project = created["project"]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    let created = must(
        call(
            f,
            "POST",
            &format!("/api/v1/w/{}/projects/{project}/features", f.workspace),
            Auth::Agent,
            Some(&json!({ "name": feature_name })),
        ),
        201,
        "create a feature",
    );
    let feature = created["feature"]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    (project, feature)
}

fn tree_path(f: &Fixture, project: &str, feature: &str) -> String {
    format!(
        "/api/v1/w/{}/projects/{project}/features/{feature}/behaviour",
        f.workspace
    )
}

fn post_node(f: &Fixture, project: &str, feature: &str, auth: Auth, body: &Value) -> Response {
    call(
        f,
        "POST",
        &format!("{}/nodes", tree_path(f, project, feature)),
        auth,
        Some(body),
    )
}

/// Adds a node as the agent; returns the `node` of the answer.
fn add(f: &Fixture, project: &str, feature: &str, body: &Value) -> Value {
    must(
        post_node(f, project, feature, Auth::Agent, body),
        201,
        "add a node",
    )["node"]
        .clone()
}

fn id_of(node: &Value) -> String {
    node["id"].as_str().expect("the node has an id").to_string()
}

fn read(f: &Fixture, project: &str, feature: &str, node: &str) -> Response {
    call(
        f,
        "GET",
        &format!("{}/nodes/{node}", tree_path(f, project, feature)),
        Auth::Agent,
        None,
    )
}

fn listed(f: &Fixture, project: &str, feature: &str) -> Vec<Value> {
    must(
        call(f, "GET", &tree_path(f, project, feature), Auth::Agent, None),
        200,
        "list the tree",
    )["nodes"]
        .as_array()
        .expect("`nodes` is a list")
        .clone()
}

fn paths_of(nodes: &[Value]) -> Vec<Value> {
    nodes.iter().map(|node| node["path"].clone()).collect()
}

/// The `data` of the live events of a project named `event`, oldest first.
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

const STORY: &str = "As a customer, I want to buy a product so that I receive what I ordered";

// ---------------------------------------------------------------------------
// Adding and reading
// ---------------------------------------------------------------------------

#[test]
fn a_situation_reads_back_at_level_2_with_the_root_named_for_the_feature_and_its_story_above() {
    let (f, _) = ready();
    let feature_name = unique("Checkout");
    let (project, feature) = new_feature(f, &feature_name);
    let story = add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": "customer-buys-product" }),
    );
    let situation = add(
        f,
        &project,
        &feature,
        &json!({
            "title": "User presses the buy button",
            "summary": "Product goes to the cart, or the user sees why it can't.",
            "parent_id": id_of(&story),
        }),
    );

    let read = must(
        read(f, &project, &feature, &id_of(&situation)),
        200,
        "read the situation",
    );

    let node = &read["node"];
    assert_eq!(
        node["path"],
        "customer-buys-product/user-presses-the-buy-button"
    );
    assert_eq!(node["title"], "User presses the buy button");
    assert_eq!(
        node["summary"],
        "Product goes to the cart, or the user sees why it can't."
    );
    assert_eq!(node["status"], "open");
    assert_eq!(node["level"], 2);
    assert_eq!(node["version"], 1);
    assert_eq!(node["created_by"]["kind"], "agent");
    assert_eq!(node["created_by"]["run"], "t1");
    assert_eq!(node["created_by"]["role"], "grill");
    let chain = read["chain"].as_array().expect("`chain` is a list");
    assert_eq!(chain.len(), 2, "{chain:?}");
    assert_eq!(chain[0]["level"], 0);
    assert_eq!(chain[0]["path"], "");
    assert_eq!(chain[0]["title"], json!(feature_name));
    assert_eq!(chain[0]["parent_id"], Value::Null);
    assert_eq!(chain[1]["id"], story["id"]);
    assert_eq!(chain[1]["level"], 1);
    assert_eq!(chain[1]["path"], "customer-buys-product");
}

#[test]
fn a_node_a_person_adds_with_a_session_names_only_the_user_as_its_author() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));

    let created = must(
        post_node(
            f,
            &project,
            &feature,
            Auth::Session,
            &json!({ "title": STORY }),
        ),
        201,
        "add a node with a session",
    );

    assert_eq!(
        created["node"]["created_by"],
        json!({ "kind": "human", "user_id": f.alice })
    );
}

#[test]
fn a_missing_slug_comes_from_the_title_and_a_clash_among_siblings_gets_2_then_3() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": "buy" }),
    );
    let other = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Return", "slug": "return" }),
    );
    let title = json!({ "title": "User presses -- the BUY button!", "parent_id": id_of(&story) });

    let first = add(f, &project, &feature, &title);
    let second = add(f, &project, &feature, &title);
    let third = add(f, &project, &feature, &title);
    let elsewhere = add(
        f,
        &project,
        &feature,
        &json!({ "title": "User presses -- the BUY button!", "parent_id": id_of(&other) }),
    );

    assert_eq!(first["slug"], "user-presses-the-buy-button");
    assert_eq!(second["slug"], "user-presses-the-buy-button-2");
    assert_eq!(third["slug"], "user-presses-the-buy-button-3");
    assert_eq!(elsewhere["slug"], "user-presses-the-buy-button");
}

#[test]
fn a_slug_derived_from_a_long_title_is_cut_to_60_characters() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));

    let node = add(
        f,
        &project,
        &feature,
        &json!({ "title": "abcdefghij".repeat(7) }),
    );

    assert_eq!(node["slug"], json!("abcdefghij".repeat(6)));
}

#[test]
fn the_list_gives_every_node_depth_first_with_siblings_in_the_order_they_were_added() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let buy = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Buy", "slug": "buy" }),
    );
    add(
        f,
        &project,
        &feature,
        &json!({ "title": "Return", "slug": "return" }),
    );
    let press = json!({ "title": "Press", "slug": "press", "parent_id": id_of(&buy) });
    add(f, &project, &feature, &press);
    let pay = json!({ "title": "Pay", "slug": "pay", "parent_id": id_of(&buy) });
    add(f, &project, &feature, &pay);

    let nodes = listed(f, &project, &feature);

    assert_eq!(
        paths_of(&nodes),
        vec![
            json!(""),
            json!("buy"),
            json!("buy/press"),
            json!("buy/pay"),
            json!("return")
        ]
    );
    let levels: Vec<Value> = nodes.iter().map(|node| node["level"].clone()).collect();
    assert_eq!(
        levels,
        vec![json!(0), json!(1), json!(2), json!(2), json!(1)]
    );
    for node in &nodes {
        assert_eq!(node.get("situation"), None, "{node}");
        assert_eq!(node.get("behaviour"), None, "{node}");
    }
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn a_blank_title_is_422_invalid_title_and_the_feature_gets_no_tree() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));

    let response = post_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &json!({ "title": "  \t " }),
    );

    assert_error(&response, 422, "invalid_title");
    assert_eq!(listed(f, &project, &feature), Vec::<Value>::new());
}

#[test]
fn a_slug_with_capitals_and_a_space_is_422_invalid_slug() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));

    let response = post_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &json!({ "title": STORY, "slug": "Buy Now" }),
    );

    assert_error(&response, 422, "invalid_slug");
    assert_eq!(listed(f, &project, &feature), Vec::<Value>::new());
}

#[test]
fn a_slug_of_80_characters_is_taken_and_one_of_81_is_422_invalid_slug() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let longest = "abcdefghij".repeat(8);

    let refused = post_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &json!({ "title": STORY, "slug": format!("{longest}k") }),
    );
    let taken = add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": longest }),
    );

    assert_error(&refused, 422, "invalid_slug");
    assert_eq!(taken["slug"], json!(longest));
}

#[test]
fn a_slug_a_sibling_has_is_409_slug_taken_and_adds_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": "customer-buys-product" }),
    );

    let response = post_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &json!({ "title": "Another story", "slug": "customer-buys-product" }),
    );

    assert_error(&response, 409, "slug_taken");
    assert_eq!(
        paths_of(&listed(f, &project, &feature)),
        vec![json!(""), json!("customer-buys-product")]
    );
}

#[test]
fn an_api_key_without_a_run_is_400_identity_required_and_the_feature_gets_no_tree() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));

    let response = post_node(
        f,
        &project,
        &feature,
        Auth::AgentWithoutRun,
        &json!({ "title": STORY }),
    );

    assert_error(&response, 400, "identity_required");
    assert_eq!(listed(f, &project, &feature), Vec::<Value>::new());
}

#[test]
fn a_node_of_another_feature_is_404_to_read_and_as_a_parent() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (other_project, other_feature) = new_feature(f, &unique("Returns"));
    add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": "buy" }),
    );
    let theirs = add(
        f,
        &other_project,
        &other_feature,
        &json!({ "title": "Return" }),
    );

    let read = read(f, &project, &feature, &id_of(&theirs));
    let added = post_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &json!({ "title": "Press", "parent_id": id_of(&theirs) }),
    );

    assert_error(&read, 404, "not_found");
    assert_error(&added, 404, "not_found");
    assert_eq!(
        paths_of(&listed(f, &project, &feature)),
        vec![json!(""), json!("buy")]
    );
}

// ---------------------------------------------------------------------------
// Live events
// ---------------------------------------------------------------------------

#[test]
fn the_first_add_announces_the_root_and_the_node_and_a_later_add_announces_its_path() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": "customer-buys-product" }),
    );
    let root = story["parent_id"].clone();
    let situation = add(
        f,
        &project,
        &feature,
        &json!({ "title": "User presses the buy button", "parent_id": id_of(&story) }),
    );

    assert_eq!(
        events_of(&project, "behaviour.node_created"),
        vec![
            json!({ "feature_id": feature, "node_id": root, "parent_id": null, "path": "" }),
            json!({
                "feature_id": feature,
                "node_id": story["id"],
                "parent_id": root,
                "path": "customer-buys-product",
            }),
            json!({
                "feature_id": feature,
                "node_id": situation["id"],
                "parent_id": story["id"],
                "path": "customer-buys-product/user-presses-the-buy-button",
            }),
        ]
    );
}
