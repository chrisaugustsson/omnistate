//! US-201 to US-205: the behaviour tree of a feature, through the API. Run with
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
//! - `PATCH /nodes/{id}` with `{expected_version, title?, slug?, summary?,
//!   situation?, behaviour?, status?}` overwrites the fields given, adds 1 to
//!   `version` and emits `behaviour.node_changed` with `{feature_id, node_id,
//!   path, fields}`, the fields in schema order. The body's shape is checked
//!   first (422 `no_changes`, `invalid_status`), then the version (409
//!   `{"error": "conflict", "current_version": n}`), then what depends on the
//!   stored node (422 `invalid_slug` on the root, 409 `slug_taken`). Children
//!   of an `out_of_scope` node keep their status. A new slug moves the paths
//!   below and keeps the ids. There is no DELETE (405).
//! - `GET /?under=<id>` lists that node and every node below it, in tree
//!   order; a node of another feature is 404. `GET /nodes/{id}` also gives
//!   `children`: the direct children by position, each `{id, path, slug,
//!   title, summary, status, same_as, handoff}`.
//! - `POST /nodes/{id}/move` with `{expected_version, parent_id}` makes the
//!   node the last child of the parent, with everything below it; ids keep,
//!   only the moved node's `version` goes up, and it emits one
//!   `behaviour.node_changed` with `fields: ["parent_id", "position"]` and
//!   the new `path`. Its own parent is accepted like any other. Refused with
//!   nothing changed: the node itself or a node below it (409 `cycle`), the
//!   root (422), a parent with a child of that slug (409 `slug_taken`), a
//!   parent of another feature (404), a stale version (409 `conflict`).
//! - `POST /nodes` and `PATCH /nodes/{id}` take `same_as` or `handoff`, a node
//!   id of the feature (`null` in a PATCH clears it; left out keeps it). The
//!   read's `node` gives a pointer as `{id, path, title, summary}`, the list
//!   and `children` as `{id, path}`, else `null`. Refused with nothing
//!   written: both pointers on one node (422 `two_pointers`), a node of
//!   another feature (422 `pointer_target_unknown`), the node itself (422
//!   `pointer_to_itself`), a child added or moved under a node with a pointer
//!   (409 `points_elsewhere`), a pointer set on a node with a child (409
//!   `has_children`). The database refuses a pointer out of the feature and
//!   two pointers on one row.

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

// ---------------------------------------------------------------------------
// Rewriting a node and setting its status (US-202)
// ---------------------------------------------------------------------------

fn patch_node(
    f: &Fixture,
    project: &str,
    feature: &str,
    auth: Auth,
    node: &str,
    body: &Value,
) -> Response {
    call(
        f,
        "PATCH",
        &format!("{}/nodes/{node}", tree_path(f, project, feature)),
        auth,
        Some(body),
    )
}

/// Edits a node as the agent; returns the `node` of the answer.
fn edit(f: &Fixture, project: &str, feature: &str, node: &str, body: &Value) -> Value {
    must(
        patch_node(f, project, feature, Auth::Agent, node, body),
        200,
        "edit a node",
    )["node"]
        .clone()
}

fn read_node(f: &Fixture, project: &str, feature: &str, node: &str) -> Value {
    must(read(f, project, feature, node), 200, "read the node")["node"].clone()
}

#[test]
fn an_edit_overwrites_the_text_adds_1_to_the_version_and_announces_the_fields_in_schema_order() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": "customer-buys-product", "summary": "Buying." }),
    );

    edit(
        f,
        &project,
        &feature,
        &id_of(&story),
        &json!({
            "status": "done",
            "behaviour": "The product is added to the cart and a confirmation appears.",
            "situation": "The user is on a product page and presses buy.",
            "expected_version": 1,
        }),
    );

    let node = read_node(f, &project, &feature, &id_of(&story));
    assert_eq!(
        node["situation"],
        "The user is on a product page and presses buy."
    );
    assert_eq!(
        node["behaviour"],
        "The product is added to the cart and a confirmation appears."
    );
    assert_eq!(node["status"], "done");
    assert_eq!(node["version"], 2);
    assert_eq!(node["title"], STORY);
    assert_eq!(node["summary"], "Buying.");
    assert_eq!(
        events_of(&project, "behaviour.node_changed"),
        vec![json!({
            "feature_id": feature,
            "node_id": story["id"],
            "path": "customer-buys-product",
            "fields": ["situation", "behaviour", "status"],
        })]
    );
}

#[test]
fn a_stale_version_is_409_with_the_current_version_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(f, &project, &feature, &json!({ "title": STORY }));
    edit(
        f,
        &project,
        &feature,
        &id_of(&story),
        &json!({ "expected_version": 1, "status": "done" }),
    );

    let response = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&story),
        &json!({ "expected_version": 1, "title": "Another title", "status": "open" }),
    );

    assert_eq!(response.status, 409, "body: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "conflict", "current_version": 2 })
    );
    let node = read_node(f, &project, &feature, &id_of(&story));
    assert_eq!(node["title"], STORY);
    assert_eq!(node["status"], "done");
    assert_eq!(node["version"], 2);
    assert_eq!(events_of(&project, "behaviour.node_changed").len(), 1);
}

#[test]
fn no_change_or_the_status_stale_is_422_even_with_a_stale_version() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(f, &project, &feature, &json!({ "title": STORY }));
    edit(
        f,
        &project,
        &feature,
        &id_of(&story),
        &json!({ "expected_version": 1, "status": "done" }),
    );

    let empty = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&story),
        &json!({ "expected_version": 1 }),
    );
    let stale = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&story),
        &json!({ "expected_version": 1, "status": "stale" }),
    );
    let blank_title = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&story),
        &json!({ "expected_version": 1, "title": "  " }),
    );
    let bad_slug = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&story),
        &json!({ "expected_version": 1, "slug": "Buy Now" }),
    );

    assert_error(&empty, 422, "no_changes");
    assert_error(&stale, 422, "invalid_status");
    assert_error(&blank_title, 422, "invalid_title");
    assert_error(&bad_slug, 422, "invalid_slug");
    let node = read_node(f, &project, &feature, &id_of(&story));
    assert_eq!(node["status"], "done");
    assert_eq!(node["version"], 2);
}

#[test]
fn a_slug_on_the_root_is_422_and_with_a_stale_version_409() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(f, &project, &feature, &json!({ "title": STORY }));
    let root = story["parent_id"].as_str().expect("a root").to_string();

    let current = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &root,
        &json!({ "expected_version": 1, "slug": "checkout" }),
    );
    let stale = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &root,
        &json!({ "expected_version": 2, "slug": "checkout" }),
    );

    assert_error(&current, 422, "invalid_slug");
    assert_eq!(stale.status, 409, "body: {}", stale.text());
    assert_eq!(
        stale.json(),
        json!({ "error": "conflict", "current_version": 1 })
    );
    let node = read_node(f, &project, &feature, &root);
    assert_eq!(node["path"], "");
    assert_eq!(node["version"], 1);
}

#[test]
fn out_of_scope_leaves_the_node_and_its_children_in_the_tree_with_their_own_status() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": "buy" }),
    );
    let pressed = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Press", "parent_id": id_of(&story) }),
    );
    add(
        f,
        &project,
        &feature,
        &json!({ "title": "Sold out", "parent_id": id_of(&story) }),
    );
    edit(
        f,
        &project,
        &feature,
        &id_of(&pressed),
        &json!({ "expected_version": 1, "status": "done" }),
    );

    edit(
        f,
        &project,
        &feature,
        &id_of(&story),
        &json!({ "expected_version": 1, "status": "out_of_scope" }),
    );

    let statuses: Vec<(Value, Value)> = listed(f, &project, &feature)
        .iter()
        .map(|node| (node["path"].clone(), node["status"].clone()))
        .collect();
    assert_eq!(
        statuses,
        vec![
            (json!(""), json!("open")),
            (json!("buy"), json!("out_of_scope")),
            (json!("buy/press"), json!("done")),
            (json!("buy/sold-out"), json!("open")),
        ]
    );
    assert_eq!(
        read_node(f, &project, &feature, &id_of(&pressed))["status"],
        "done"
    );
}

#[test]
fn a_new_slug_gives_the_node_and_those_below_a_new_path_and_keeps_the_ids() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": "buy" }),
    );
    let pressed = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Press", "parent_id": id_of(&story) }),
    );

    let renamed = edit(
        f,
        &project,
        &feature,
        &id_of(&story),
        &json!({ "expected_version": 1, "slug": "customer-buys-product" }),
    );

    assert_eq!(renamed["id"], story["id"]);
    assert_eq!(renamed["path"], "customer-buys-product");
    let below = read_node(f, &project, &feature, &id_of(&pressed));
    assert_eq!(below["path"], "customer-buys-product/press");
}

#[test]
fn a_new_slug_a_sibling_has_is_409_slug_taken_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": "buy" }),
    );
    let returns = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Return a product", "slug": "return" }),
    );

    let response = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&returns),
        &json!({ "expected_version": 1, "slug": "buy" }),
    );

    assert_error(&response, 409, "slug_taken");
    let node = read_node(f, &project, &feature, &id_of(&returns));
    assert_eq!(node["path"], "return");
    assert_eq!(node["version"], 1);
}

#[test]
fn an_edit_with_an_api_key_without_a_run_is_400_identity_required_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(f, &project, &feature, &json!({ "title": STORY }));

    let response = patch_node(
        f,
        &project,
        &feature,
        Auth::AgentWithoutRun,
        &id_of(&story),
        &json!({ "expected_version": 1, "status": "done" }),
    );

    assert_error(&response, 400, "identity_required");
    let node = read_node(f, &project, &feature, &id_of(&story));
    assert_eq!(node["status"], "open");
    assert_eq!(node["version"], 1);
}

#[test]
fn delete_on_a_node_is_405_and_the_node_stays() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(
        f,
        &project,
        &feature,
        &json!({ "title": STORY, "slug": "buy" }),
    );

    let response = call(
        f,
        "DELETE",
        &format!(
            "{}/nodes/{}",
            tree_path(f, &project, &feature),
            id_of(&story)
        ),
        Auth::Agent,
        None,
    );

    assert_eq!(response.status, 405, "body: {}", response.text());
    assert_eq!(
        paths_of(&listed(f, &project, &feature)),
        vec![json!(""), json!("buy")]
    );
}

// ---------------------------------------------------------------------------
// Listing below a node, and the children of a read (US-203)
// ---------------------------------------------------------------------------

fn listed_under(f: &Fixture, project: &str, feature: &str, under: &str) -> Response {
    call(
        f,
        "GET",
        &format!("{}?under={under}", tree_path(f, project, feature)),
        Auth::Agent,
        None,
    )
}

#[test]
fn under_a_node_lists_it_and_everything_below_and_not_its_next_sibling_or_the_next_story() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let buy = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Buy", "slug": "buy" }),
    );
    let press = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Press", "slug": "press", "parent_id": id_of(&buy) }),
    );
    add(
        f,
        &project,
        &feature,
        &json!({ "title": "Out of stock", "slug": "out-of-stock", "parent_id": id_of(&press) }),
    );
    add(
        f,
        &project,
        &feature,
        &json!({ "title": "Pay", "slug": "pay", "parent_id": id_of(&buy) }),
    );
    let ret = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Return", "slug": "return" }),
    );
    add(
        f,
        &project,
        &feature,
        &json!({ "title": "Refund", "slug": "refund", "parent_id": id_of(&ret) }),
    );

    let story = must(
        listed_under(f, &project, &feature, &id_of(&buy)),
        200,
        "list under the story",
    );
    let situation = must(
        listed_under(f, &project, &feature, &id_of(&press)),
        200,
        "list under the situation",
    );

    assert_eq!(
        paths_of(story["nodes"].as_array().expect("`nodes` is a list")),
        vec![
            json!("buy"),
            json!("buy/press"),
            json!("buy/press/out-of-stock"),
            json!("buy/pay")
        ]
    );
    assert_eq!(
        paths_of(situation["nodes"].as_array().expect("`nodes` is a list")),
        vec![json!("buy/press"), json!("buy/press/out-of-stock")]
    );
}

#[test]
fn under_a_node_of_another_feature_is_404() {
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

    let response = listed_under(f, &project, &feature, &id_of(&theirs));

    assert_error(&response, 404, "not_found");
}

#[test]
fn a_read_gives_the_direct_children_in_the_order_they_were_added_without_the_grandchildren() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let story = add(
        f,
        &project,
        &feature,
        &json!({
            "title": STORY,
            "slug": "customer-buys-product",
            "summary": "Customer buys a product."
        }),
    );
    let press = add(
        f,
        &project,
        &feature,
        &json!({
            "title": "User presses buy",
            "slug": "user-presses-buy",
            "summary": "Product goes to the cart, or the user sees why it can't.",
            "parent_id": id_of(&story)
        }),
    );
    add(
        f,
        &project,
        &feature,
        &json!({
            "title": "Product is out of stock",
            "slug": "product-out-of-stock",
            "parent_id": id_of(&press)
        }),
    );
    let pay = add(
        f,
        &project,
        &feature,
        &json!({ "title": "User pays", "slug": "user-pays", "parent_id": id_of(&story) }),
    );
    edit(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "expected_version": 1, "status": "done" }),
    );

    let read = must(
        read(f, &project, &feature, &id_of(&story)),
        200,
        "read the story",
    );

    assert_eq!(
        read["children"],
        json!([
            {
                "id": id_of(&press),
                "path": "customer-buys-product/user-presses-buy",
                "slug": "user-presses-buy",
                "title": "User presses buy",
                "summary": "Product goes to the cart, or the user sees why it can't.",
                "status": "done",
                "same_as": null,
                "handoff": null
            },
            {
                "id": id_of(&pay),
                "path": "customer-buys-product/user-pays",
                "slug": "user-pays",
                "title": "User pays",
                "summary": "",
                "status": "open",
                "same_as": null,
                "handoff": null
            }
        ])
    );
}

// ---------------------------------------------------------------------------
// Moving a node (US-204)
// ---------------------------------------------------------------------------

fn move_node(
    f: &Fixture,
    project: &str,
    feature: &str,
    auth: Auth,
    node: &str,
    body: &Value,
) -> Response {
    call(
        f,
        "POST",
        &format!("{}/nodes/{node}/move", tree_path(f, project, feature)),
        auth,
        Some(body),
    )
}

/// The tree of the move criteria: `customer-buys-product/user-presses-buy/
/// product-out-of-stock` and the story `customer-checks-out` with one
/// situation, `user-pays`. Returns `(press, out_of_stock, checks_out)`.
fn checkout_tree(f: &Fixture, project: &str, feature: &str) -> (Value, Value, Value) {
    let buys = add(
        f,
        project,
        feature,
        &json!({ "title": STORY, "slug": "customer-buys-product" }),
    );
    let press = add(
        f,
        project,
        feature,
        &json!({ "title": "User presses buy", "slug": "user-presses-buy", "parent_id": id_of(&buys) }),
    );
    let out_of_stock = add(
        f,
        project,
        feature,
        &json!({
            "title": "Product is out of stock",
            "slug": "product-out-of-stock",
            "parent_id": id_of(&press)
        }),
    );
    let checks_out = add(
        f,
        project,
        feature,
        &json!({ "title": "Customer checks out", "slug": "customer-checks-out" }),
    );
    add(
        f,
        project,
        feature,
        &json!({ "title": "User pays", "slug": "user-pays", "parent_id": id_of(&checks_out) }),
    );
    (press, out_of_stock, checks_out)
}

/// The tree as listed: each node's path, id and version, in tree order.
fn shape(f: &Fixture, project: &str, feature: &str) -> Vec<Value> {
    listed(f, project, feature)
        .iter()
        .map(|node| json!([node["path"], node["id"], node["version"]]))
        .collect()
}

#[test]
fn a_move_puts_the_node_last_under_the_new_parent_with_its_subtree_and_bumps_only_its_version() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, out_of_stock, checks_out) = checkout_tree(f, &project, &feature);

    let moved = must(
        move_node(
            f,
            &project,
            &feature,
            Auth::Agent,
            &id_of(&press),
            &json!({ "expected_version": 1, "parent_id": id_of(&checks_out) }),
        ),
        200,
        "move the node",
    )["node"]
        .clone();

    assert_eq!(moved["id"], press["id"]);
    assert_eq!(moved["path"], "customer-checks-out/user-presses-buy");
    assert_eq!(moved["version"], 2);
    let below = read_node(f, &project, &feature, &id_of(&out_of_stock));
    assert_eq!(
        below["path"],
        "customer-checks-out/user-presses-buy/product-out-of-stock"
    );
    assert_eq!(below["version"], 1);
    assert_eq!(
        paths_of(&listed(f, &project, &feature)),
        vec![
            json!(""),
            json!("customer-buys-product"),
            json!("customer-checks-out"),
            json!("customer-checks-out/user-pays"),
            json!("customer-checks-out/user-presses-buy"),
            json!("customer-checks-out/user-presses-buy/product-out-of-stock"),
        ]
    );
    assert_eq!(
        events_of(&project, "behaviour.node_changed"),
        vec![json!({
            "feature_id": feature,
            "node_id": press["id"],
            "path": "customer-checks-out/user-presses-buy",
            "fields": ["parent_id", "position"],
        })]
    );
}

#[test]
fn a_move_to_the_parent_it_already_has_puts_it_last_and_is_announced() {
    // Settled decision: not a slug clash with itself, but a move like any other.
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, _, _) = checkout_tree(f, &project, &feature);
    let buys = read_node(f, &project, &feature, &id_of(&press))["parent_id"].clone();
    add(
        f,
        &project,
        &feature,
        &json!({ "title": "User adds a coupon", "slug": "user-adds-a-coupon", "parent_id": buys }),
    );

    let moved = must(
        move_node(
            f,
            &project,
            &feature,
            Auth::Agent,
            &id_of(&press),
            &json!({ "expected_version": 1, "parent_id": buys }),
        ),
        200,
        "move the node to its own parent",
    )["node"]
        .clone();

    assert_eq!(moved["path"], "customer-buys-product/user-presses-buy");
    assert_eq!(moved["version"], 2);
    assert_eq!(
        paths_of(&listed(f, &project, &feature))[1..4].to_vec(),
        vec![
            json!("customer-buys-product"),
            json!("customer-buys-product/user-adds-a-coupon"),
            json!("customer-buys-product/user-presses-buy"),
        ]
    );
    assert_eq!(events_of(&project, "behaviour.node_changed").len(), 1);
}

#[test]
fn a_move_to_the_node_itself_or_below_it_is_409_cycle_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, out_of_stock, _) = checkout_tree(f, &project, &feature);
    let before = shape(f, &project, &feature);

    let to_itself = move_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&press),
        &json!({ "expected_version": 1, "parent_id": id_of(&press) }),
    );
    let below_itself = move_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&press),
        &json!({ "expected_version": 1, "parent_id": id_of(&out_of_stock) }),
    );

    assert_error(&to_itself, 409, "cycle");
    assert_error(&below_itself, 409, "cycle");
    assert_eq!(shape(f, &project, &feature), before);
    assert_eq!(
        events_of(&project, "behaviour.node_changed"),
        Vec::<Value>::new()
    );
}

#[test]
fn moving_the_root_is_422_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, _, _) = checkout_tree(f, &project, &feature);
    let chain = must(
        read(f, &project, &feature, &id_of(&press)),
        200,
        "read the node",
    )["chain"]
        .clone();
    let root = chain[0]["id"]
        .as_str()
        .expect("the root has an id")
        .to_string();
    let before = shape(f, &project, &feature);

    let response = move_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &root,
        &json!({ "expected_version": 1, "parent_id": id_of(&press) }),
    );

    assert_eq!(response.status, 422, "body: {}", response.text());
    assert_eq!(shape(f, &project, &feature), before);
    assert_eq!(
        events_of(&project, "behaviour.node_changed"),
        Vec::<Value>::new()
    );
}

#[test]
fn a_move_where_the_new_parent_has_a_child_with_the_slug_is_409_slug_taken_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, _, checks_out) = checkout_tree(f, &project, &feature);
    add(
        f,
        &project,
        &feature,
        &json!({ "title": "User presses buy", "slug": "user-presses-buy", "parent_id": id_of(&checks_out) }),
    );
    let before = shape(f, &project, &feature);

    let response = move_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&press),
        &json!({ "expected_version": 1, "parent_id": id_of(&checks_out) }),
    );

    assert_error(&response, 409, "slug_taken");
    assert_eq!(shape(f, &project, &feature), before);
    assert_eq!(
        events_of(&project, "behaviour.node_changed"),
        Vec::<Value>::new()
    );
}

#[test]
fn a_move_with_a_stale_version_is_409_with_the_current_version_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, _, checks_out) = checkout_tree(f, &project, &feature);
    edit(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "expected_version": 1, "status": "done" }),
    );
    let before = shape(f, &project, &feature);

    let response = move_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&press),
        &json!({ "expected_version": 1, "parent_id": id_of(&checks_out) }),
    );

    assert_eq!(response.status, 409, "body: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "conflict", "current_version": 2 })
    );
    assert_eq!(shape(f, &project, &feature), before);
    assert_eq!(events_of(&project, "behaviour.node_changed").len(), 1);
}

#[test]
fn a_move_under_a_node_of_another_feature_is_404_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (other_project, other_feature) = new_feature(f, &unique("Returns"));
    let (press, _, _) = checkout_tree(f, &project, &feature);
    let theirs = add(
        f,
        &other_project,
        &other_feature,
        &json!({ "title": "Return" }),
    );
    let before = shape(f, &project, &feature);

    let response = move_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&press),
        &json!({ "expected_version": 1, "parent_id": id_of(&theirs) }),
    );

    assert_error(&response, 404, "not_found");
    assert_eq!(shape(f, &project, &feature), before);
}

#[test]
fn a_move_with_an_api_key_without_a_run_is_400_identity_required_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, _, checks_out) = checkout_tree(f, &project, &feature);
    let before = shape(f, &project, &feature);

    let response = move_node(
        f,
        &project,
        &feature,
        Auth::AgentWithoutRun,
        &id_of(&press),
        &json!({ "expected_version": 1, "parent_id": id_of(&checks_out) }),
    );

    assert_error(&response, 400, "identity_required");
    assert_eq!(shape(f, &project, &feature), before);
}

// ---------------------------------------------------------------------------
// Same-as and handoff pointers (US-205)
// ---------------------------------------------------------------------------

/// The tree as listed: each node's path, version and pointers, in tree order.
fn pointers(f: &Fixture, project: &str, feature: &str) -> Vec<Value> {
    listed(f, project, feature)
        .iter()
        .map(|node| {
            json!([
                node["path"],
                node["version"],
                node["same_as"],
                node["handoff"]
            ])
        })
        .collect()
}

#[test]
fn a_handoff_reads_back_with_the_targets_path_title_and_summary_and_lists_with_its_path() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, out_of_stock, checks_out) = checkout_tree(f, &project, &feature);
    edit(
        f,
        &project,
        &feature,
        &id_of(&checks_out),
        &json!({ "expected_version": 1, "summary": "The customer pays and gets a receipt." }),
    );

    let goes = add(
        f,
        &project,
        &feature,
        &json!({
            "title": "User goes to checkout",
            "slug": "user-goes-to-checkout",
            "parent_id": id_of(&press),
            "handoff": id_of(&checks_out)
        }),
    );
    let deep = add(
        f,
        &project,
        &feature,
        &json!({ "title": "User pays later", "handoff": id_of(&out_of_stock) }),
    );

    let node = read_node(f, &project, &feature, &id_of(&goes));
    assert_eq!(
        node["handoff"],
        json!({
            "id": id_of(&checks_out),
            "path": "customer-checks-out",
            "title": "Customer checks out",
            "summary": "The customer pays and gets a receipt."
        })
    );
    assert_eq!(node["same_as"], Value::Null);
    assert_eq!(
        read_node(f, &project, &feature, &id_of(&deep))["handoff"]["path"],
        "customer-buys-product/user-presses-buy/product-out-of-stock"
    );
    let entry = json!({ "id": id_of(&checks_out), "path": "customer-checks-out" });
    let listed_goes = listed(f, &project, &feature)
        .into_iter()
        .find(|node| node["id"] == goes["id"])
        .expect("the new node is listed");
    assert_eq!(listed_goes["handoff"], entry);
    assert_eq!(listed_goes["same_as"], Value::Null);
    let children = must(
        read(f, &project, &feature, &id_of(&press)),
        200,
        "read the parent",
    )["children"]
        .clone();
    assert_eq!(
        children[1]["path"],
        "customer-buys-product/user-presses-buy/user-goes-to-checkout"
    );
    assert_eq!(children[1]["handoff"], entry);
    assert_eq!(children[1]["same_as"], Value::Null);
}

#[test]
fn a_same_as_set_by_an_edit_stays_through_other_edits_and_null_clears_it() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, out_of_stock, checks_out) = checkout_tree(f, &project, &feature);
    let sold_out = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Product is sold out", "parent_id": id_of(&checks_out) }),
    );
    let target = json!({
        "id": id_of(&out_of_stock),
        "path": "customer-buys-product/user-presses-buy/product-out-of-stock",
        "title": "Product is out of stock",
        "summary": ""
    });

    edit(
        f,
        &project,
        &feature,
        &id_of(&sold_out),
        &json!({ "expected_version": 1, "same_as": id_of(&out_of_stock) }),
    );
    assert_eq!(
        read_node(f, &project, &feature, &id_of(&sold_out))["same_as"],
        target
    );

    edit(
        f,
        &project,
        &feature,
        &id_of(&sold_out),
        &json!({ "expected_version": 2, "title": "Product sold out" }),
    );
    assert_eq!(
        read_node(f, &project, &feature, &id_of(&sold_out))["same_as"],
        target
    );

    edit(
        f,
        &project,
        &feature,
        &id_of(&sold_out),
        &json!({ "expected_version": 3, "same_as": null }),
    );
    let node = read_node(f, &project, &feature, &id_of(&sold_out));
    assert_eq!(node["same_as"], Value::Null);
    assert_eq!(node["handoff"], Value::Null);
    assert_eq!(node["version"], 4);
}

#[test]
fn both_pointers_on_one_node_are_422_two_pointers_and_nothing_is_written() {
    // Settled decision: its own code, so the three pointer refusals differ.
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, out_of_stock, checks_out) = checkout_tree(f, &project, &feature);
    let sold_out = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Product is sold out", "parent_id": id_of(&checks_out), "same_as": id_of(&out_of_stock) }),
    );
    let before = pointers(f, &project, &feature);

    let added = post_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &json!({
            "title": "User goes to checkout",
            "parent_id": id_of(&press),
            "same_as": id_of(&out_of_stock),
            "handoff": id_of(&checks_out)
        }),
    );
    let both = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&out_of_stock),
        &json!({ "expected_version": 1, "same_as": id_of(&sold_out), "handoff": id_of(&checks_out) }),
    );
    let beside_stored = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&sold_out),
        &json!({ "expected_version": 1, "handoff": id_of(&press) }),
    );

    assert_error(&added, 422, "two_pointers");
    assert_error(&both, 422, "two_pointers");
    assert_error(&beside_stored, 422, "two_pointers");
    assert_eq!(pointers(f, &project, &feature), before);
}

#[test]
fn a_pointer_to_a_node_of_another_feature_is_422_pointer_target_unknown_and_nothing_is_written() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (other_project, other_feature) = new_feature(f, &unique("Returns"));
    let (press, out_of_stock, _) = checkout_tree(f, &project, &feature);
    let theirs = add(
        f,
        &other_project,
        &other_feature,
        &json!({ "title": "Customer checks out" }),
    );
    let before = pointers(f, &project, &feature);

    let added = post_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &json!({ "title": "User goes to checkout", "parent_id": id_of(&press), "handoff": id_of(&theirs) }),
    );
    let edited = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&out_of_stock),
        &json!({ "expected_version": 1, "same_as": id_of(&theirs) }),
    );

    assert_error(&added, 422, "pointer_target_unknown");
    assert_error(&edited, 422, "pointer_target_unknown");
    assert_eq!(pointers(f, &project, &feature), before);
    assert_eq!(
        events_of(&project, "behaviour.node_changed"),
        Vec::<Value>::new()
    );
}

#[test]
fn a_node_pointing_at_itself_is_422_pointer_to_itself_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, out_of_stock, _) = checkout_tree(f, &project, &feature);
    let before = pointers(f, &project, &feature);

    let response = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&out_of_stock),
        &json!({ "expected_version": 1, "same_as": id_of(&out_of_stock) }),
    );

    assert_error(&response, 422, "pointer_to_itself");
    assert_eq!(pointers(f, &project, &feature), before);
}

#[test]
fn adding_or_moving_a_node_under_one_with_a_pointer_is_409_points_elsewhere_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, out_of_stock, checks_out) = checkout_tree(f, &project, &feature);
    let goes = add(
        f,
        &project,
        &feature,
        &json!({
            "title": "User goes to checkout",
            "slug": "user-goes-to-checkout",
            "parent_id": id_of(&press),
            "handoff": id_of(&checks_out)
        }),
    );
    let before = pointers(f, &project, &feature);

    let added = post_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &json!({ "title": "x", "parent_id": id_of(&goes) }),
    );
    let moved = move_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&out_of_stock),
        &json!({ "expected_version": 1, "parent_id": id_of(&goes) }),
    );

    assert_error(&added, 409, "points_elsewhere");
    assert_error(&moved, 409, "points_elsewhere");
    assert_eq!(pointers(f, &project, &feature), before);
}

#[test]
fn a_pointer_on_a_node_that_has_a_child_is_409_has_children_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, _, checks_out) = checkout_tree(f, &project, &feature);
    let before = pointers(f, &project, &feature);

    let response = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&press),
        &json!({ "expected_version": 1, "same_as": id_of(&checks_out) }),
    );

    assert_error(&response, 409, "has_children");
    assert_eq!(pointers(f, &project, &feature), before);
}

/// The SQLSTATE of setting `assignments` on the node `id` as the admin role.
fn sqlstate_of_update(id: &str, assignments: &str) -> String {
    block(async {
        let mut conn = admin_connection().await;
        let error = sqlx::query(&format!(
            "UPDATE behaviour_nodes SET {assignments} WHERE id = $1::uuid"
        ))
        .bind(id)
        .execute(&mut conn)
        .await
        .expect_err("the database refuses the pointer");
        error
            .as_database_error()
            .and_then(|e| e.code())
            .map(|code| code.to_string())
            .unwrap_or_default()
    })
}

#[test]
fn the_database_refuses_a_pointer_out_of_the_feature_and_two_pointers_on_one_node() {
    // FR-10: the database, not only the API, keeps a pointer in its feature.
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (other_project, other_feature) = new_feature(f, &unique("Returns"));
    let (press, out_of_stock, checks_out) = checkout_tree(f, &project, &feature);
    let theirs = add(
        f,
        &other_project,
        &other_feature,
        &json!({ "title": "Customer checks out" }),
    );

    let out_of_feature = sqlstate_of_update(
        &id_of(&out_of_stock),
        &format!("handoff_id = '{}'::uuid", id_of(&theirs)),
    );
    let same_as_out_of_feature = sqlstate_of_update(
        &id_of(&out_of_stock),
        &format!("same_as_id = '{}'::uuid", id_of(&theirs)),
    );
    let two = sqlstate_of_update(
        &id_of(&out_of_stock),
        &format!(
            "same_as_id = '{}'::uuid, handoff_id = '{}'::uuid",
            id_of(&press),
            id_of(&checks_out)
        ),
    );

    assert_eq!(out_of_feature, "23503", "a foreign key violation");
    assert_eq!(same_as_out_of_feature, "23503", "a foreign key violation");
    assert_eq!(two, "23514", "a check violation");
}
