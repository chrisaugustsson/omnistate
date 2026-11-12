//! US-201 to US-212: the behaviour tree of a feature, through the API. Run with
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
//! - `POST /nodes/{id}/questions` with `{question}` answers 201 `{question}`:
//!   `open`, no `answered_by`, `asked_by` the author, `version` 1, and emits
//!   `behaviour.question_changed` with `{feature_id, node_id, question_id,
//!   state}`. A `done` node becomes `open` (version + 1, `node_changed` with
//!   `fields: ["status"]`); any other status stays. `done` on a node with an
//!   open question is 409 `open_questions`. The read gives the node's
//!   `questions`, the list each node's `open_questions` count, and `GET
//!   /questions?state=&under=` the questions in tree order of their nodes,
//!   then oldest first. Refused with nothing asked: a blank question (422
//!   `invalid_question`), a node of another feature (404), an API key without
//!   a run (400 `identity_required`); an unknown `state` is 422.
//! - `POST /questions/{id}/answers` with `{expected_version, answer, by?}`
//!   answers 201 `{question}`: `answered`, `answered_by` and `answer:
//!   {answer, given_by, author, at}`, `version` + 1, and emits
//!   `behaviour.question_changed` with `state: "answered"`. A session gives
//!   `human`; an API key gives `agent` (an answer by default) unless `by` is
//!   `human`, kept with the agent as `author`. The body is checked first (422
//!   for a blank answer, a `by` other than `human` and `agent`, `agent` from a
//!   session), then `already_answered` (409), then the version (409
//!   `conflict`). `POST /nodes/{id}/questions` with `answer` creates the
//!   question answered at `version` 1 and leaves a `done` node `done`; `by`
//!   without `answer` is 422. The list gives each node's `default_answers`,
//!   and `GET /questions?by=` filters on who gave the current answer.
//! - With `change: true` the answer route gives an answered question a new
//!   answer and keeps the earlier ones: `answers` lists every answer oldest
//!   first, `answer` is the newest, `version` + 1. `change` on an open
//!   question is 409 `not_answered`. After the version check, an agent's own
//!   answer on a question a person answered is 409 `answered_by_human`; an
//!   agent may change an agent's answer, and with `by: "human"` any answer.
//!   `GET /questions/{id}` gives the question with its `answers`.
//! - A write made with a session (an answer, a changed answer, a question or
//!   remark) marks its node; a changed answer, by anyone, marks every node
//!   below, whatever its status. A mark sets `needs_pass_since` where it is
//!   null, keeps `version`, and emits one `behaviour.marked` with
//!   `{feature_id, node_id, below}` per write. An agent's question and first
//!   answer mark nothing. `GET /?needs_pass=true` lists the marked nodes.
//!   `PATCH` with `needs_pass: false` clears that node alone as an edit
//!   (version + 1, `node_changed` with `fields: ["needs_pass"]`); with a
//!   session it is 403 `forbidden`, and `needs_pass: true` is 422.
//! - `POST /nodes/{id}/rules` with `{text, scope?}` answers 201 `{rule}`:
//!   `scope` `node` when left out, `withdrawn` false, `version` 1, the author
//!   stored; `PATCH /rules/{id}` with `{expected_version, text?, scope?,
//!   withdrawn?}` adds 1 to `version`, and `withdrawn: false` puts a rule back
//!   in force. Each emits `behaviour.rule_changed` with `{feature_id, node_id,
//!   rule_id}`. `GET /rules?scope=&under=` lists the rules in tree order of
//!   their nodes, withdrawn ones flagged. Refused with nothing written: `story`
//!   on a rule of the root, added or edited (422 `no_story`), a blank text or
//!   an unknown scope (422 `{"error": <code>}`), a stale version (409
//!   `conflict`), an API key without a run (400), a node or rule of another
//!   feature (404). There is no DELETE (405).
//! - `GET /nodes/{id}` gives `rules`: every rule in force that reaches the
//!   node, each `{id, text, scope, node_id, node_path}`. A `feature` rule
//!   reaches every node, a `story` rule every node of its node's story, a
//!   `node` rule its node and those below; a withdrawn rule reaches nothing,
//!   and reach follows the tree as it is now. `feature` rules come first, then
//!   `story`, then `node` rules from the root down, oldest first within each.
//! - Each `chain` entry of the read carries its own `questions`, and the read
//!   gives `siblings`: the other children of the parent by position, as
//!   `children` entries; none for the root.

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

/// Who sends a request: the grill agent `t1/grill` with the API key, a second
/// agent `t2/review` with the same key, the key without a run, or alice with
/// her session.
#[derive(Clone, Copy)]
enum Auth {
    Agent,
    OtherAgent,
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
        Auth::OtherAgent => vec![
            ("Authorization", bearer),
            ("X-Omni-Run", "t2".to_string()),
            ("X-Omni-Role", "review".to_string()),
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

// ---------------------------------------------------------------------------
// Asking a question on a node (US-206)
// ---------------------------------------------------------------------------

const SOLD_OUT: &str = "What if the product sold out between page load and the click?";

fn post_question(
    f: &Fixture,
    project: &str,
    feature: &str,
    auth: Auth,
    node: &str,
    question: &str,
) -> Response {
    call(
        f,
        "POST",
        &format!("{}/nodes/{node}/questions", tree_path(f, project, feature)),
        auth,
        Some(&json!({ "question": question })),
    )
}

/// Asks a question as the agent; returns the `question` of the answer.
fn ask(f: &Fixture, project: &str, feature: &str, node: &str, question: &str) -> Value {
    must(
        post_question(f, project, feature, Auth::Agent, node, question),
        201,
        "ask a question",
    )["question"]
        .clone()
}

/// The text of the questions that `GET /questions` gives with `query`, in
/// its order.
fn questions_listed(f: &Fixture, project: &str, feature: &str, query: &str) -> Vec<Value> {
    let listed = must(
        call(
            f,
            "GET",
            &format!("{}/questions{query}", tree_path(f, project, feature)),
            Auth::Agent,
            None,
        ),
        200,
        "list the questions",
    );
    listed["questions"]
        .as_array()
        .expect("`questions` is a list")
        .iter()
        .map(|question| question["question"].clone())
        .collect()
}

/// A story `customer-buys-product` with the situation `user-presses-buy`
/// below it: `(story, situation)`.
fn buy_tree(f: &Fixture, project: &str, feature: &str) -> (Value, Value) {
    let story = add(
        f,
        project,
        feature,
        &json!({ "title": STORY, "slug": "customer-buys-product" }),
    );
    let press = add(
        f,
        project,
        feature,
        &json!({ "title": "User presses buy", "slug": "user-presses-buy", "parent_id": id_of(&story) }),
    );
    (story, press)
}

#[test]
fn an_agents_question_is_open_without_an_answerer_reads_back_on_its_node_and_is_announced() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);

    let question = ask(f, &project, &feature, &id_of(&press), SOLD_OUT);

    assert_eq!(question["node_id"], press["id"]);
    assert_eq!(
        question["node_path"],
        "customer-buys-product/user-presses-buy"
    );
    assert_eq!(question["question"], SOLD_OUT);
    assert_eq!(question["state"], "open");
    assert_eq!(question["answered_by"], Value::Null);
    assert_eq!(question["asked_by"]["kind"], "agent");
    assert_eq!(question["asked_by"]["run"], "t1");
    assert_eq!(question["asked_by"]["role"], "grill");
    assert_eq!(question["version"], 1);
    let node = read_node(f, &project, &feature, &id_of(&press));
    assert_eq!(node["status"], "open");
    assert_eq!(node["version"], 1);
    assert_eq!(
        events_of(&project, "behaviour.question_changed"),
        vec![json!({
            "feature_id": feature,
            "node_id": press["id"],
            "question_id": question["id"],
            "state": "open",
        })]
    );
    assert_eq!(
        events_of(&project, "behaviour.node_changed"),
        Vec::<Value>::new()
    );
}

#[test]
fn a_read_gives_the_nodes_own_questions_oldest_first_as_the_ask_answered_them() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (story, press) = buy_tree(f, &project, &feature);
    ask(f, &project, &feature, &id_of(&story), "Who may buy?");
    let first = ask(f, &project, &feature, &id_of(&press), SOLD_OUT);
    let second = ask(
        f,
        &project,
        &feature,
        &id_of(&press),
        "And if the cart is full?",
    );

    let read = must(read(f, &project, &feature, &id_of(&press)), 200, "read");

    assert_eq!(read["questions"], json!([first, second]));
}

#[test]
fn asking_on_a_done_node_sets_it_open_adds_1_to_its_version_and_announces_the_status() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    edit(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "expected_version": 1, "status": "done" }),
    );

    ask(f, &project, &feature, &id_of(&press), SOLD_OUT);

    let node = read_node(f, &project, &feature, &id_of(&press));
    assert_eq!(node["status"], "open");
    assert_eq!(node["version"], 3);
    let changed = events_of(&project, "behaviour.node_changed");
    assert_eq!(changed.len(), 2, "{changed:?}");
    assert_eq!(
        changed[1],
        json!({
            "feature_id": feature,
            "node_id": press["id"],
            "path": "customer-buys-product/user-presses-buy",
            "fields": ["status"],
        })
    );
}

#[test]
fn asking_on_an_out_of_scope_node_leaves_it_out_of_scope_at_its_version() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    edit(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "expected_version": 1, "status": "out_of_scope" }),
    );

    ask(f, &project, &feature, &id_of(&press), SOLD_OUT);

    let node = read_node(f, &project, &feature, &id_of(&press));
    assert_eq!(node["status"], "out_of_scope");
    assert_eq!(node["version"], 2);
    assert_eq!(events_of(&project, "behaviour.node_changed").len(), 1);
}

#[test]
fn done_on_a_node_with_an_open_question_is_409_open_questions_and_a_node_without_one_can_be_done() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (story, press) = buy_tree(f, &project, &feature);
    ask(f, &project, &feature, &id_of(&press), SOLD_OUT);

    let refused = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&press),
        &json!({ "expected_version": 1, "status": "done", "title": "Pressed" }),
    );
    let story_done = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&story),
        &json!({ "expected_version": 1, "status": "done" }),
    );

    assert_error(&refused, 409, "open_questions");
    let node = read_node(f, &project, &feature, &id_of(&press));
    assert_eq!(node["status"], "open");
    assert_eq!(node["title"], "User presses buy");
    assert_eq!(node["version"], 1);
    assert_eq!(story_done.status, 200, "body: {}", story_done.text());
    assert_eq!(events_of(&project, "behaviour.node_changed").len(), 1);
}

#[test]
fn the_list_counts_the_open_questions_of_each_node() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (story, press) = buy_tree(f, &project, &feature);
    add(
        f,
        &project,
        &feature,
        &json!({ "title": "Returns", "slug": "returns" }),
    );
    ask(f, &project, &feature, &id_of(&press), SOLD_OUT);
    ask(
        f,
        &project,
        &feature,
        &id_of(&press),
        "And if the cart is full?",
    );
    ask(f, &project, &feature, &id_of(&story), "Who may buy?");

    let counts: Vec<(Value, Value)> = listed(f, &project, &feature)
        .iter()
        .map(|node| (node["path"].clone(), node["open_questions"].clone()))
        .collect();

    assert_eq!(
        counts,
        vec![
            (json!(""), json!(0)),
            (json!("customer-buys-product"), json!(1)),
            (json!("customer-buys-product/user-presses-buy"), json!(2)),
            (json!("returns"), json!(0)),
        ]
    );
}

#[test]
fn the_questions_come_in_tree_order_of_their_nodes_then_oldest_first() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (story, press) = buy_tree(f, &project, &feature);
    let returns = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Returns", "slug": "returns" }),
    );
    ask(f, &project, &feature, &id_of(&returns), "first on returns");
    ask(f, &project, &feature, &id_of(&press), "on press");
    ask(f, &project, &feature, &id_of(&story), "on the story");
    ask(f, &project, &feature, &id_of(&returns), "second on returns");

    let all = questions_listed(f, &project, &feature, "");
    let open = questions_listed(f, &project, &feature, "?state=open");

    let expected = vec![
        json!("on the story"),
        json!("on press"),
        json!("first on returns"),
        json!("second on returns"),
    ];
    assert_eq!(all, expected);
    assert_eq!(open, expected);
}

#[test]
fn under_a_node_lists_the_questions_on_it_and_below_it_only() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (story, press) = buy_tree(f, &project, &feature);
    let returns = add(
        f,
        &project,
        &feature,
        &json!({ "title": "Returns", "slug": "returns" }),
    );
    let root = must(read(f, &project, &feature, &id_of(&story)), 200, "read")["chain"][0]["id"]
        .as_str()
        .expect("the story has the root above it")
        .to_string();
    ask(f, &project, &feature, &root, "on the root");
    ask(f, &project, &feature, &id_of(&returns), "on returns");
    ask(f, &project, &feature, &id_of(&story), "on the story");
    ask(f, &project, &feature, &id_of(&press), "on press");

    let under_story = questions_listed(
        f,
        &project,
        &feature,
        &format!("?state=open&under={}", id_of(&story)),
    );
    let under_press = questions_listed(f, &project, &feature, &format!("?under={}", id_of(&press)));

    assert_eq!(under_story, vec![json!("on the story"), json!("on press")]);
    assert_eq!(under_press, vec![json!("on press")]);
}

#[test]
fn a_state_other_than_open_or_answered_is_422_invalid_state() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    ask(f, &project, &feature, &id_of(&press), SOLD_OUT);

    let response = call(
        f,
        "GET",
        &format!(
            "{}/questions?state=closed",
            tree_path(f, &project, &feature)
        ),
        Auth::Agent,
        None,
    );

    assert_error(&response, 422, "invalid_state");
}

#[test]
fn a_blank_question_is_422_invalid_question_and_nothing_is_asked() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);

    let response = post_question(f, &project, &feature, Auth::Agent, &id_of(&press), "   ");

    assert_error(&response, 422, "invalid_question");
    assert_eq!(
        questions_listed(f, &project, &feature, ""),
        Vec::<Value>::new()
    );
    assert_eq!(
        events_of(&project, "behaviour.question_changed"),
        Vec::<Value>::new()
    );
}

#[test]
fn a_node_of_another_feature_is_404_to_ask_on_and_nothing_is_asked() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    buy_tree(f, &project, &feature);
    // A second feature of the same project, so only the feature tells the
    // nodes apart.
    let created = must(
        call(
            f,
            "POST",
            &format!("/api/v1/w/{}/projects/{project}/features", f.workspace),
            Auth::Agent,
            Some(&json!({ "name": unique("Returns") })),
        ),
        201,
        "create a second feature",
    );
    let other_feature = created["feature"]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    let theirs = add(f, &project, &other_feature, &json!({ "title": "Returns" }));

    let response = post_question(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&theirs),
        SOLD_OUT,
    );

    assert_error(&response, 404, "not_found");
    assert_eq!(
        questions_listed(f, &project, &feature, ""),
        Vec::<Value>::new()
    );
    assert_eq!(
        questions_listed(f, &project, &other_feature, ""),
        Vec::<Value>::new()
    );
    assert_eq!(
        events_of(&project, "behaviour.question_changed"),
        Vec::<Value>::new()
    );
}

#[test]
fn a_question_with_an_api_key_without_a_run_is_400_identity_required_and_nothing_is_asked() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);

    let response = post_question(
        f,
        &project,
        &feature,
        Auth::AgentWithoutRun,
        &id_of(&press),
        SOLD_OUT,
    );

    assert_error(&response, 400, "identity_required");
    assert_eq!(
        questions_listed(f, &project, &feature, ""),
        Vec::<Value>::new()
    );
}

// ---------------------------------------------------------------------------
// Answering a question (US-207)
// ---------------------------------------------------------------------------

const STOCK: &str = "Same message, and the stock indicator updates.";
const GUEST: &str = "Guest cart, identical behaviour.";

fn post_answer(
    f: &Fixture,
    project: &str,
    feature: &str,
    auth: Auth,
    question: &Value,
    body: &Value,
) -> Response {
    call(
        f,
        "POST",
        &format!(
            "{}/questions/{}/answers",
            tree_path(f, project, feature),
            question["id"].as_str().expect("the question has an id")
        ),
        auth,
        Some(body),
    )
}

/// Answers a question with `auth`; returns the `question` of the answer.
fn answer(
    f: &Fixture,
    project: &str,
    feature: &str,
    auth: Auth,
    question: &Value,
    body: &Value,
) -> Value {
    must(
        post_answer(f, project, feature, auth, question, body),
        201,
        "answer a question",
    )["question"]
        .clone()
}

/// The questions that `GET /questions` gives with `query`, whole.
fn questions_whole(f: &Fixture, project: &str, feature: &str, query: &str) -> Vec<Value> {
    must(
        call(
            f,
            "GET",
            &format!("{}/questions{query}", tree_path(f, project, feature)),
            Auth::Agent,
            None,
        ),
        200,
        "list the questions",
    )["questions"]
        .as_array()
        .expect("`questions` is a list")
        .clone()
}

/// The `state` of the live `behaviour.question_changed` events, oldest first.
fn question_states(project: &str) -> Vec<Value> {
    events_of(project, "behaviour.question_changed")
        .iter()
        .map(|event| event["state"].clone())
        .collect()
}

#[test]
fn an_agents_own_answer_is_given_by_the_agent_and_answers_the_question_and_is_announced() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let asked = ask(f, &project, &feature, &id_of(&press), SOLD_OUT);

    let answered = answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": STOCK }),
    );

    assert_eq!(answered["state"], "answered");
    assert_eq!(answered["answered_by"], "agent");
    assert_eq!(answered["version"], 2);
    assert_eq!(answered["answer"]["answer"], STOCK);
    assert_eq!(answered["answer"]["given_by"], "agent");
    assert_eq!(answered["answer"]["author"]["kind"], "agent");
    assert_eq!(answered["answer"]["author"]["run"], "t1");
    assert_eq!(answered["answer"]["author"]["role"], "grill");
    assert!(
        answered["answer"]["at"]
            .as_str()
            .is_some_and(|at| !at.is_empty()),
        "{answered}"
    );
    assert_eq!(
        questions_whole(f, &project, &feature, ""),
        vec![answered.clone()]
    );
    assert_eq!(
        events_of(&project, "behaviour.question_changed")[1],
        json!({
            "feature_id": feature,
            "node_id": press["id"],
            "question_id": asked["id"],
            "state": "answered",
        })
    );
    assert_eq!(question_states(&project).len(), 2);
}

#[test]
fn a_persons_answer_is_given_by_human_from_a_session_and_when_an_agent_writes_it_down() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let relayed = ask(
        f,
        &project,
        &feature,
        &id_of(&press),
        "Guest or account cart?",
    );
    let direct = ask(f, &project, &feature, &id_of(&press), "And for a guest?");

    let relayed = answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &relayed,
        &json!({ "expected_version": 1, "answer": GUEST, "by": "human" }),
    );
    let direct = answer(
        f,
        &project,
        &feature,
        Auth::Session,
        &direct,
        &json!({ "expected_version": 1, "answer": GUEST }),
    );

    assert_eq!(relayed["answered_by"], "human");
    assert_eq!(relayed["answer"]["given_by"], "human");
    assert_eq!(relayed["answer"]["author"]["kind"], "agent");
    assert_eq!(relayed["answer"]["author"]["run"], "t1");
    assert_eq!(direct["answered_by"], "human");
    assert_eq!(direct["answer"]["given_by"], "human");
    assert_eq!(
        direct["answer"]["author"],
        json!({ "kind": "human", "user_id": f.alice })
    );
}

#[test]
fn a_body_the_answer_cannot_take_is_422_before_the_answered_and_version_checks() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let asked = ask(f, &project, &feature, &id_of(&press), SOLD_OUT);
    let answered = answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": STOCK }),
    );

    let blank = post_answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": "" }),
    );
    let spaces = post_answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": "   " }),
    );
    let unknown_by = post_answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": GUEST, "by": "robot" }),
    );
    let agent_from_session = post_answer(
        f,
        &project,
        &feature,
        Auth::Session,
        &asked,
        &json!({ "expected_version": 1, "answer": GUEST, "by": "agent" }),
    );

    assert_eq!(blank.status, 422, "body: {}", blank.text());
    assert_eq!(spaces.status, 422, "body: {}", spaces.text());
    assert_eq!(unknown_by.status, 422, "body: {}", unknown_by.text());
    assert_eq!(
        agent_from_session.status,
        422,
        "body: {}",
        agent_from_session.text()
    );
    assert_eq!(questions_whole(f, &project, &feature, ""), vec![answered]);
    assert_eq!(
        question_states(&project),
        vec![json!("open"), json!("answered")]
    );
}

#[test]
fn an_answered_question_is_409_already_answered_even_with_a_stale_version_and_keeps_its_answer() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let asked = ask(f, &project, &feature, &id_of(&press), SOLD_OUT);
    let answered = answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": STOCK }),
    );

    let current = post_answer(
        f,
        &project,
        &feature,
        Auth::Session,
        &asked,
        &json!({ "expected_version": 2, "answer": GUEST }),
    );
    let stale = post_answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": GUEST }),
    );

    assert_error(&current, 409, "already_answered");
    assert_error(&stale, 409, "already_answered");
    assert_eq!(questions_whole(f, &project, &feature, ""), vec![answered]);
    assert_eq!(
        question_states(&project),
        vec![json!("open"), json!("answered")]
    );
}

#[test]
fn a_stale_version_on_an_open_question_is_409_with_the_current_version_and_answers_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let asked = ask(f, &project, &feature, &id_of(&press), SOLD_OUT);

    let response = post_answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 2, "answer": STOCK }),
    );

    assert_eq!(response.status, 409, "body: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "conflict", "current_version": 1 })
    );
    assert_eq!(questions_whole(f, &project, &feature, ""), vec![asked]);
    assert_eq!(question_states(&project), vec![json!("open")]);
}

#[test]
fn asking_with_an_answer_on_a_done_node_creates_it_answered_and_leaves_the_node_done() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    edit(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "expected_version": 1, "status": "done" }),
    );

    let asked = must(
        call(
            f,
            "POST",
            &format!(
                "{}/nodes/{}/questions",
                tree_path(f, &project, &feature),
                id_of(&press)
            ),
            Auth::Agent,
            Some(&json!({
                "question": "Region delivery check here?",
                "answer": "No, handled at checkout.",
            })),
        ),
        201,
        "ask with an answer",
    )["question"]
        .clone();

    assert_eq!(asked["state"], "answered");
    assert_eq!(asked["answered_by"], "agent");
    assert_eq!(asked["version"], 1);
    assert_eq!(asked["answer"]["answer"], "No, handled at checkout.");
    assert_eq!(asked["answer"]["given_by"], "agent");
    assert_eq!(asked["answer"]["author"]["run"], "t1");
    let node = read_node(f, &project, &feature, &id_of(&press));
    assert_eq!(node["status"], "done");
    assert_eq!(node["version"], 2);
    assert_eq!(events_of(&project, "behaviour.node_changed").len(), 1);
    assert_eq!(
        events_of(&project, "behaviour.question_changed"),
        vec![json!({
            "feature_id": feature,
            "node_id": press["id"],
            "question_id": asked["id"],
            "state": "answered",
        })]
    );
    let listed_press = listed(f, &project, &feature)
        .into_iter()
        .find(|node| node["id"] == press["id"])
        .expect("the node is listed");
    assert_eq!(listed_press["open_questions"], 0);
    assert_eq!(listed_press["default_answers"], 1);
}

#[test]
fn by_without_an_answer_on_ask_is_422_and_nothing_is_asked() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);

    let response = call(
        f,
        "POST",
        &format!(
            "{}/nodes/{}/questions",
            tree_path(f, &project, &feature),
            id_of(&press)
        ),
        Auth::Agent,
        Some(&json!({ "question": SOLD_OUT, "by": "human" })),
    );

    assert_eq!(response.status, 422, "body: {}", response.text());
    assert_eq!(
        questions_whole(f, &project, &feature, ""),
        Vec::<Value>::new()
    );
    assert_eq!(question_states(&project), Vec::<Value>::new());
}

/// On the situation of `buy_tree`: an open question, one the agent answered
/// on its own and one a person answered, which the agent wrote down.
fn three_questions(f: &Fixture, project: &str, feature: &str, press: &Value) {
    ask(f, project, feature, &id_of(press), "still open");
    let by_default = ask(f, project, feature, &id_of(press), "answered by default");
    let by_person = ask(f, project, feature, &id_of(press), "answered by a person");
    answer(
        f,
        project,
        feature,
        Auth::Agent,
        &by_default,
        &json!({ "expected_version": 1, "answer": STOCK }),
    );
    answer(
        f,
        project,
        feature,
        Auth::Agent,
        &by_person,
        &json!({ "expected_version": 1, "answer": GUEST, "by": "human" }),
    );
}

#[test]
fn the_list_counts_the_default_answers_apart_from_a_persons_and_the_open_questions() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    three_questions(f, &project, &feature, &press);

    let counts: Vec<(Value, Value, Value)> = listed(f, &project, &feature)
        .iter()
        .map(|node| {
            (
                node["path"].clone(),
                node["open_questions"].clone(),
                node["default_answers"].clone(),
            )
        })
        .collect();

    assert_eq!(
        counts,
        vec![
            (json!(""), json!(0), json!(0)),
            (json!("customer-buys-product"), json!(0), json!(0)),
            (
                json!("customer-buys-product/user-presses-buy"),
                json!(1),
                json!(1)
            ),
        ]
    );
}

#[test]
fn the_questions_filter_by_state_and_by_who_gave_the_answer() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    three_questions(f, &project, &feature, &press);

    let open = questions_listed(f, &project, &feature, "?state=open");
    let answered = questions_listed(f, &project, &feature, "?state=answered");
    let by_default = questions_listed(f, &project, &feature, "?state=answered&by=agent");
    let by_person = questions_listed(f, &project, &feature, "?by=human");

    assert_eq!(open, vec![json!("still open")]);
    assert_eq!(
        answered,
        vec![json!("answered by default"), json!("answered by a person")]
    );
    assert_eq!(by_default, vec![json!("answered by default")]);
    assert_eq!(by_person, vec![json!("answered by a person")]);
}

#[test]
fn an_answer_with_an_api_key_without_a_run_is_400_identity_required_and_answers_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let asked = ask(f, &project, &feature, &id_of(&press), SOLD_OUT);

    let response = post_answer(
        f,
        &project,
        &feature,
        Auth::AgentWithoutRun,
        &asked,
        &json!({ "expected_version": 1, "answer": STOCK }),
    );

    assert_error(&response, 400, "identity_required");
    assert_eq!(questions_whole(f, &project, &feature, ""), vec![asked]);
}

#[test]
fn a_question_of_another_feature_is_404_to_answer_and_answers_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    buy_tree(f, &project, &feature);
    // A second feature of the same project, so only the feature tells the
    // questions apart.
    let created = must(
        call(
            f,
            "POST",
            &format!("/api/v1/w/{}/projects/{project}/features", f.workspace),
            Auth::Agent,
            Some(&json!({ "name": unique("Returns") })),
        ),
        201,
        "create a second feature",
    );
    let other_feature = created["feature"]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    let theirs = add(f, &project, &other_feature, &json!({ "title": "Returns" }));
    let asked = ask(f, &project, &other_feature, &id_of(&theirs), SOLD_OUT);

    let response = post_answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": STOCK }),
    );

    assert_error(&response, 404, "not_found");
    assert_eq!(
        questions_whole(f, &project, &other_feature, ""),
        vec![asked]
    );
}

// ---------------------------------------------------------------------------
// Changing an answer (US-208)
// ---------------------------------------------------------------------------

const GUEST_CART: &str = "Guest cart";
const LOGIN: &str = "Login required before buying.";

/// `GET /questions/{id}`, as the agent: the `question`.
fn question_read(f: &Fixture, project: &str, feature: &str, question: &Value) -> Value {
    must(
        call(
            f,
            "GET",
            &format!(
                "{}/questions/{}",
                tree_path(f, project, feature),
                question["id"].as_str().expect("the question has an id")
            ),
            Auth::Agent,
            None,
        ),
        200,
        "read the question",
    )["question"]
        .clone()
}

/// A question on the situation of `buy_tree` that the agent answered on its
/// own with "Guest cart", then a person changed to "Login required before
/// buying.": `(press, question)`, the question at version 3.
fn answered_by_a_person(f: &Fixture, project: &str, feature: &str) -> (Value, Value) {
    let (_, press) = buy_tree(f, project, feature);
    let asked = ask(
        f,
        project,
        feature,
        &id_of(&press),
        "Guest or account cart?",
    );
    answer(
        f,
        project,
        feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": GUEST_CART }),
    );
    let changed = answer(
        f,
        project,
        feature,
        Auth::Session,
        &asked,
        &json!({ "expected_version": 2, "answer": LOGIN, "change": true }),
    );
    (press, changed)
}

#[test]
fn a_person_changes_a_default_answer_and_the_old_one_stays_first_in_the_answers() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let asked = ask(
        f,
        &project,
        &feature,
        &id_of(&press),
        "Guest or account cart?",
    );
    let by_default = answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": GUEST_CART }),
    );

    let changed = answer(
        f,
        &project,
        &feature,
        Auth::Session,
        &asked,
        &json!({ "expected_version": 2, "answer": LOGIN, "change": true }),
    );

    assert_eq!(changed["state"], "answered");
    assert_eq!(changed["answered_by"], "human");
    assert_eq!(changed["version"], 3);
    assert_eq!(changed["answer"]["answer"], LOGIN);
    assert_eq!(changed["answer"]["given_by"], "human");
    assert_eq!(
        changed["answer"]["author"],
        json!({ "kind": "human", "user_id": f.alice })
    );
    assert_eq!(
        changed["answers"],
        json!([by_default["answer"], changed["answer"]])
    );
    assert_ne!(changed["answers"][0]["at"], changed["answers"][1]["at"]);
    assert_eq!(question_read(f, &project, &feature, &asked), changed);
    let node = must(
        read(f, &project, &feature, &id_of(&press)),
        200,
        "read the node",
    );
    assert_eq!(node["questions"], json!([changed]));
    assert_eq!(
        question_states(&project),
        vec![json!("open"), json!("answered"), json!("answered")]
    );
}

#[test]
fn an_agents_own_change_on_a_persons_answer_is_409_answered_by_human_after_the_version_check() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, changed) = answered_by_a_person(f, &project, &feature);

    let current = post_answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &changed,
        &json!({ "expected_version": 3, "answer": "Guest cart after all.", "change": true }),
    );
    let stale = post_answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &changed,
        &json!({ "expected_version": 2, "answer": "Guest cart after all.", "change": true }),
    );

    assert_error(&current, 409, "answered_by_human");
    assert_eq!(stale.status, 409, "body: {}", stale.text());
    assert_eq!(
        stale.json(),
        json!({ "error": "conflict", "current_version": 3 })
    );
    assert_eq!(question_read(f, &project, &feature, &changed), changed);
    assert_eq!(question_states(&project).len(), 3);
}

#[test]
fn an_agent_changes_another_agents_answer_and_a_relayed_change_replaces_a_persons() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (press, by_person) = answered_by_a_person(f, &project, &feature);
    let asked = ask(f, &project, &feature, &id_of(&press), SOLD_OUT);
    answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": STOCK }),
    );

    let by_reviewer = answer(
        f,
        &project,
        &feature,
        Auth::OtherAgent,
        &asked,
        &json!({ "expected_version": 2, "answer": GUEST, "change": true }),
    );
    let relayed = answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &by_person,
        &json!({ "expected_version": 3, "answer": GUEST_CART, "change": true, "by": "human" }),
    );

    assert_eq!(by_reviewer["answered_by"], "agent");
    assert_eq!(by_reviewer["answer"]["answer"], GUEST);
    assert_eq!(by_reviewer["answer"]["author"]["run"], "t2");
    assert_eq!(by_reviewer["answer"]["author"]["role"], "review");
    assert_eq!(relayed["answered_by"], "human");
    assert_eq!(relayed["answer"]["answer"], GUEST_CART);
    assert_eq!(relayed["answer"]["given_by"], "human");
    assert_eq!(relayed["answer"]["author"]["kind"], "agent");
}

#[test]
fn change_on_an_open_question_is_409_not_answered_and_answers_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let asked = ask(f, &project, &feature, &id_of(&press), SOLD_OUT);

    let response = post_answer(
        f,
        &project,
        &feature,
        Auth::Session,
        &asked,
        &json!({ "expected_version": 1, "answer": LOGIN, "change": true }),
    );

    assert_error(&response, 409, "not_answered");
    assert_eq!(
        question_read(f, &project, &feature, &asked)["state"],
        "open"
    );
    assert_eq!(
        question_read(f, &project, &feature, &asked)["answers"],
        json!([])
    );
    assert_eq!(question_states(&project), vec![json!("open")]);
}

// ---------------------------------------------------------------------------
// Marking what needs another grill pass
// ---------------------------------------------------------------------------

const N: &str = "customer-buys-product";
const C: &str = "customer-buys-product/user-presses-buy";
const G: &str = "customer-buys-product/user-presses-buy/product-out-of-stock";

/// The story N of `buy_tree`, its child C and a grandchild G below C:
/// `(n, c, g)`.
fn n_c_g(f: &Fixture, project: &str, feature: &str) -> (Value, Value, Value) {
    let (n, c) = buy_tree(f, project, feature);
    let g = add(
        f,
        project,
        feature,
        &json!({ "title": "Product is out of stock", "slug": "product-out-of-stock", "parent_id": id_of(&c) }),
    );
    (n, c, g)
}

/// Every node of the feature as `[path, needs_pass]`, in tree order.
fn marks(f: &Fixture, project: &str, feature: &str) -> Vec<Value> {
    listed(f, project, feature)
        .iter()
        .map(|node| json!([node["path"], node["needs_pass"]]))
        .collect()
}

/// The paths of the nodes `GET /?needs_pass=true` gives.
fn marked_paths(f: &Fixture, project: &str, feature: &str) -> Vec<Value> {
    let listed = must(
        call(
            f,
            "GET",
            &format!("{}?needs_pass=true", tree_path(f, project, feature)),
            Auth::Agent,
            None,
        ),
        200,
        "list the marked nodes",
    );
    paths_of(listed["nodes"].as_array().expect("`nodes` is a list"))
}

/// A question on N that the agent answered on its own: the question at
/// version 2.
fn answered_by_default_on(f: &Fixture, project: &str, feature: &str, n: &Value) -> Value {
    let asked = ask(f, project, feature, &id_of(n), "Guest or account cart?");
    answer(
        f,
        project,
        feature,
        Auth::Agent,
        &asked,
        &json!({ "expected_version": 1, "answer": GUEST_CART }),
    )
}

#[test]
fn a_persons_first_answer_marks_its_node_only_and_keeps_the_nodes_version() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (n, _, _) = n_c_g(f, &project, &feature);
    let asked = ask(f, &project, &feature, &id_of(&n), "Guest or account cart?");

    answer(
        f,
        &project,
        &feature,
        Auth::Session,
        &asked,
        &json!({ "expected_version": 1, "answer": LOGIN }),
    );

    assert_eq!(
        marks(f, &project, &feature),
        vec![
            json!(["", false]),
            json!([N, true]),
            json!([C, false]),
            json!([G, false]),
        ]
    );
    assert_eq!(marked_paths(f, &project, &feature), vec![json!(N)]);
    let node = read_node(f, &project, &feature, &id_of(&n));
    assert_eq!(node["needs_pass"], true);
    assert!(node["needs_pass_since"].is_string(), "{node}");
    assert_eq!(node["version"], 1);
    assert_eq!(
        events_of(&project, "behaviour.marked"),
        vec![json!({ "feature_id": feature, "node_id": n["id"], "below": false })]
    );
}

#[test]
fn a_changed_answer_marks_every_node_below_and_a_persons_change_marks_its_node_too() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (n, c, g) = n_c_g(f, &project, &feature);
    edit(
        f,
        &project,
        &feature,
        &id_of(&g),
        &json!({ "expected_version": 1, "status": "out_of_scope" }),
    );
    let by_default = answered_by_default_on(f, &project, &feature, &n);

    answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &by_default,
        &json!({ "expected_version": 2, "answer": GUEST, "change": true }),
    );
    let after_agent = marks(f, &project, &feature);
    let c_since = read_node(f, &project, &feature, &id_of(&c))["needs_pass_since"].clone();
    answer(
        f,
        &project,
        &feature,
        Auth::Session,
        &by_default,
        &json!({ "expected_version": 3, "answer": LOGIN, "change": true }),
    );

    assert_eq!(
        after_agent,
        vec![
            json!(["", false]),
            json!([N, false]),
            json!([C, true]),
            json!([G, true]),
        ]
    );
    assert_eq!(
        marks(f, &project, &feature),
        vec![
            json!(["", false]),
            json!([N, true]),
            json!([C, true]),
            json!([G, true]),
        ]
    );
    let c_now = read_node(f, &project, &feature, &id_of(&c));
    assert_eq!(c_now["needs_pass_since"], c_since);
    assert_eq!(c_now["version"], 1);
    let g_now = read_node(f, &project, &feature, &id_of(&g));
    assert_eq!(g_now["status"], "out_of_scope");
    assert_eq!(g_now["version"], 2);
    assert_eq!(
        events_of(&project, "behaviour.marked"),
        vec![
            json!({ "feature_id": feature, "node_id": n["id"], "below": true }),
            json!({ "feature_id": feature, "node_id": n["id"], "below": true }),
        ]
    );
}

#[test]
fn a_persons_remark_on_a_done_node_sets_it_open_and_marks_it() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (n, _, _) = n_c_g(f, &project, &feature);
    edit(
        f,
        &project,
        &feature,
        &id_of(&n),
        &json!({ "expected_version": 1, "status": "done" }),
    );

    must(
        post_question(
            f,
            &project,
            &feature,
            Auth::Session,
            &id_of(&n),
            "The cart should keep its items after login.",
        ),
        201,
        "post a remark",
    );

    let node = read_node(f, &project, &feature, &id_of(&n));
    assert_eq!(node["status"], "open");
    assert_eq!(node["needs_pass"], true);
    assert_eq!(marked_paths(f, &project, &feature), vec![json!(N)]);
    assert_eq!(
        events_of(&project, "behaviour.marked"),
        vec![json!({ "feature_id": feature, "node_id": n["id"], "below": false })]
    );
}

/// N, C and G of `n_c_g`, all marked by a person changing a default answer
/// on N: `(n, c)`.
fn all_marked(f: &Fixture, project: &str, feature: &str) -> (Value, Value) {
    let (n, c, _) = n_c_g(f, project, feature);
    let by_default = answered_by_default_on(f, project, feature, &n);
    answer(
        f,
        project,
        feature,
        Auth::Session,
        &by_default,
        &json!({ "expected_version": 2, "answer": LOGIN, "change": true }),
    );
    (n, c)
}

#[test]
fn an_agent_clears_one_nodes_mark_as_an_edit_that_adds_1_to_its_version() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (n, _) = all_marked(f, &project, &feature);

    let cleared = edit(
        f,
        &project,
        &feature,
        &id_of(&n),
        &json!({ "expected_version": 1, "needs_pass": false }),
    );

    assert_eq!(cleared["needs_pass"], false);
    assert_eq!(cleared["needs_pass_since"], Value::Null);
    assert_eq!(cleared["version"], 2);
    assert_eq!(
        marked_paths(f, &project, &feature),
        vec![json!(C), json!(G)]
    );
    assert_eq!(
        events_of(&project, "behaviour.node_changed"),
        vec![json!({
            "feature_id": feature,
            "node_id": n["id"],
            "path": N,
            "fields": ["needs_pass"],
        })]
    );
}

#[test]
fn clearing_a_mark_with_a_session_is_403_and_setting_one_is_422_and_both_leave_it() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (n, _) = all_marked(f, &project, &feature);

    let by_person = patch_node(
        f,
        &project,
        &feature,
        Auth::Session,
        &id_of(&n),
        &json!({ "expected_version": 1, "needs_pass": false }),
    );
    let set = patch_node(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&n),
        &json!({ "expected_version": 1, "needs_pass": true }),
    );

    assert_error(&by_person, 403, "forbidden");
    assert_eq!(set.status, 422, "body: {}", set.text());
    let node = read_node(f, &project, &feature, &id_of(&n));
    assert_eq!(node["needs_pass"], true);
    assert_eq!(node["version"], 1);
    assert_eq!(
        events_of(&project, "behaviour.node_changed"),
        Vec::<Value>::new()
    );
}

// ---------------------------------------------------------------------------
// Rules on a node (US-210)
// ---------------------------------------------------------------------------

const BUY_DISABLED: &str = "Buy button is disabled whenever the product is out of stock.";

fn post_rule(
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
        &format!("{}/nodes/{node}/rules", tree_path(f, project, feature)),
        auth,
        Some(body),
    )
}

/// Records a rule as the agent; returns the `rule` of the answer.
fn add_rule(f: &Fixture, project: &str, feature: &str, node: &str, body: &Value) -> Value {
    must(
        post_rule(f, project, feature, Auth::Agent, node, body),
        201,
        "record a rule",
    )["rule"]
        .clone()
}

fn patch_rule(
    f: &Fixture,
    project: &str,
    feature: &str,
    auth: Auth,
    rule: &Value,
    body: &Value,
) -> Response {
    call(
        f,
        "PATCH",
        &format!(
            "{}/rules/{}",
            tree_path(f, project, feature),
            rule["id"].as_str().expect("the rule has an id")
        ),
        auth,
        Some(body),
    )
}

/// The rules `GET /rules` gives with `query`, in its order.
fn rules_listed(f: &Fixture, project: &str, feature: &str, query: &str) -> Vec<Value> {
    must(
        call(
            f,
            "GET",
            &format!("{}/rules{query}", tree_path(f, project, feature)),
            Auth::Agent,
            None,
        ),
        200,
        "list the rules",
    )["rules"]
        .as_array()
        .expect("`rules` is a list")
        .clone()
}

/// The rules of the feature as `[node_path, scope, text, withdrawn, version]`,
/// in the list's order.
fn rule_rows(f: &Fixture, project: &str, feature: &str, query: &str) -> Vec<Value> {
    rules_listed(f, project, feature, query)
        .iter()
        .map(|rule| {
            json!([
                rule["node_path"],
                rule["scope"],
                rule["text"],
                rule["withdrawn"],
                rule["version"]
            ])
        })
        .collect()
}

/// The root of the feature's tree, as the list gives it.
fn root_of(f: &Fixture, project: &str, feature: &str) -> Value {
    listed(f, project, feature)[0].clone()
}

#[test]
fn a_story_scope_on_the_root_is_422_no_story_when_added_or_edited_and_nothing_is_written() {
    // Settled decision: both routes refuse `story` on a rule of the root.
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    buy_tree(f, &project, &feature);
    let root = root_of(f, &project, &feature);
    let on_root = add_rule(
        f,
        &project,
        &feature,
        &id_of(&root),
        &json!({ "text": "Prices include VAT.", "scope": "feature" }),
    );

    let added = post_rule(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&root),
        &json!({ "text": BUY_DISABLED, "scope": "story" }),
    );
    let edited = patch_rule(
        f,
        &project,
        &feature,
        Auth::Agent,
        &on_root,
        &json!({ "expected_version": 1, "scope": "story" }),
    );

    assert_error(&added, 422, "no_story");
    assert_error(&edited, 422, "no_story");
    assert_eq!(
        rule_rows(f, &project, &feature, ""),
        vec![json!(["", "feature", "Prices include VAT.", false, 1])]
    );
    assert_eq!(events_of(&project, "behaviour.rule_changed").len(), 1);
}

#[test]
fn a_rule_is_recorded_on_its_node_with_the_scope_given_or_node_and_its_author_and_is_announced() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);

    let story_rule = add_rule(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "text": BUY_DISABLED, "scope": "story" }),
    );
    let node_rule = add_rule(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "text": BUY_DISABLED }),
    );

    assert_eq!(story_rule["node_id"], press["id"]);
    assert_eq!(story_rule["node_path"], C);
    assert_eq!(story_rule["text"], BUY_DISABLED);
    assert_eq!(story_rule["scope"], "story");
    assert_eq!(story_rule["withdrawn"], false);
    assert_eq!(story_rule["version"], 1);
    assert_eq!(story_rule["author"]["kind"], "agent");
    assert_eq!(story_rule["author"]["run"], "t1");
    assert_eq!(story_rule["author"]["role"], "grill");
    assert_eq!(node_rule["scope"], "node");
    assert_eq!(
        rule_rows(f, &project, &feature, ""),
        vec![
            json!([C, "story", BUY_DISABLED, false, 1]),
            json!([C, "node", BUY_DISABLED, false, 1])
        ]
    );
    assert_eq!(
        events_of(&project, "behaviour.rule_changed"),
        vec![
            json!({ "feature_id": feature, "node_id": press["id"], "rule_id": story_rule["id"] }),
            json!({ "feature_id": feature, "node_id": press["id"], "rule_id": node_rule["id"] })
        ]
    );
}

#[test]
fn an_edit_changes_the_scope_adds_1_to_the_version_keeps_the_text_and_is_announced() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let rule = add_rule(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "text": BUY_DISABLED, "scope": "story" }),
    );

    let edited = must(
        patch_rule(
            f,
            &project,
            &feature,
            Auth::Agent,
            &rule,
            &json!({ "expected_version": 1, "scope": "feature" }),
        ),
        200,
        "edit the rule",
    )["rule"]
        .clone();

    assert_eq!(edited["id"], rule["id"]);
    assert_eq!(edited["scope"], "feature");
    assert_eq!(edited["text"], BUY_DISABLED);
    assert_eq!(edited["version"], 2);
    assert_eq!(
        rule_rows(f, &project, &feature, ""),
        vec![json!([C, "feature", BUY_DISABLED, false, 2])]
    );
    assert_eq!(
        events_of(&project, "behaviour.rule_changed"),
        vec![
            json!({ "feature_id": feature, "node_id": press["id"], "rule_id": rule["id"] }),
            json!({ "feature_id": feature, "node_id": press["id"], "rule_id": rule["id"] })
        ]
    );
}

#[test]
fn a_withdrawn_rule_stays_listed_flagged_and_false_puts_it_back_in_force() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let rule = add_rule(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "text": BUY_DISABLED }),
    );

    must(
        patch_rule(
            f,
            &project,
            &feature,
            Auth::Agent,
            &rule,
            &json!({ "expected_version": 1, "withdrawn": true }),
        ),
        200,
        "withdraw the rule",
    );
    let withdrawn = rule_rows(f, &project, &feature, "");
    must(
        patch_rule(
            f,
            &project,
            &feature,
            Auth::Agent,
            &rule,
            &json!({ "expected_version": 2, "withdrawn": false }),
        ),
        200,
        "put the rule back in force",
    );

    assert_eq!(withdrawn, vec![json!([C, "node", BUY_DISABLED, true, 2])]);
    assert_eq!(
        rule_rows(f, &project, &feature, ""),
        vec![json!([C, "node", BUY_DISABLED, false, 3])]
    );
}

/// A 422 in the validation error shape of the story handlers, `{"error":
/// <code>}` (settled decision; the PRD names no code).
fn assert_validation_error(response: &Response) {
    assert_eq!(response.status, 422, "body: {}", response.text());
    let body = response.json();
    assert_eq!(body.as_object().map(|o| o.len()), Some(1), "{body}");
    assert!(body["error"].is_string(), "{body}");
}

#[test]
fn a_blank_text_or_an_unknown_scope_is_422_and_nothing_is_written() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let rule = add_rule(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "text": BUY_DISABLED }),
    );

    let refused = [
        post_rule(
            f,
            &project,
            &feature,
            Auth::Agent,
            &id_of(&press),
            &json!({ "text": "  " }),
        ),
        post_rule(
            f,
            &project,
            &feature,
            Auth::Agent,
            &id_of(&press),
            &json!({ "text": BUY_DISABLED, "scope": "app" }),
        ),
        patch_rule(
            f,
            &project,
            &feature,
            Auth::Agent,
            &rule,
            &json!({ "expected_version": 1, "text": "  " }),
        ),
        patch_rule(
            f,
            &project,
            &feature,
            Auth::Agent,
            &rule,
            &json!({ "expected_version": 1, "scope": "app" }),
        ),
    ];

    assert_validation_error(&refused[0]);
    assert_validation_error(&refused[1]);
    assert_validation_error(&refused[2]);
    assert_validation_error(&refused[3]);
    assert_eq!(
        rule_rows(f, &project, &feature, ""),
        vec![json!([C, "node", BUY_DISABLED, false, 1])]
    );
    assert_eq!(events_of(&project, "behaviour.rule_changed").len(), 1);
}

#[test]
fn a_rule_edit_with_a_stale_version_is_409_with_the_current_version_and_changes_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let rule = add_rule(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "text": BUY_DISABLED }),
    );
    must(
        patch_rule(
            f,
            &project,
            &feature,
            Auth::Agent,
            &rule,
            &json!({ "expected_version": 1, "scope": "story" }),
        ),
        200,
        "edit the rule",
    );

    let response = patch_rule(
        f,
        &project,
        &feature,
        Auth::Agent,
        &rule,
        &json!({ "expected_version": 1, "scope": "feature" }),
    );

    assert_eq!(response.status, 409, "body: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "conflict", "current_version": 2 })
    );
    assert_eq!(
        rule_rows(f, &project, &feature, ""),
        vec![json!([C, "story", BUY_DISABLED, false, 2])]
    );
    assert_eq!(events_of(&project, "behaviour.rule_changed").len(), 2);
}

#[test]
fn delete_on_a_rule_is_405_and_the_rule_stays() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let rule = add_rule(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "text": BUY_DISABLED }),
    );

    let response = call(
        f,
        "DELETE",
        &format!(
            "{}/rules/{}",
            tree_path(f, &project, &feature),
            rule["id"].as_str().expect("an id")
        ),
        Auth::Agent,
        None,
    );

    assert_eq!(response.status, 405, "body: {}", response.text());
    assert_eq!(
        rule_rows(f, &project, &feature, ""),
        vec![json!([C, "node", BUY_DISABLED, false, 1])]
    );
}

/// Rules on G, the root and N, recorded in that order, with the scopes
/// node, feature and story.
fn rules_on_g_root_n(f: &Fixture, project: &str, feature: &str) {
    let (n, _, g) = n_c_g(f, project, feature);
    let root = root_of(f, project, feature);
    add_rule(
        f,
        project,
        feature,
        &id_of(&g),
        &json!({ "text": "Show the restock date." }),
    );
    add_rule(
        f,
        project,
        feature,
        &id_of(&root),
        &json!({ "text": "Prices include VAT.", "scope": "feature" }),
    );
    add_rule(
        f,
        project,
        feature,
        &id_of(&n),
        &json!({ "text": "A guest can buy.", "scope": "story" }),
    );
}

#[test]
fn the_rules_come_in_tree_order_of_their_nodes_and_under_a_node_only_those_on_it_and_below() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    rules_on_g_root_n(f, &project, &feature);
    let (n, c) = {
        let nodes = listed(f, &project, &feature);
        (nodes[1].clone(), nodes[2].clone())
    };

    let whole = rule_rows(f, &project, &feature, "");
    let under_n = rule_rows(f, &project, &feature, &format!("?under={}", id_of(&n)));
    let under_c = rule_rows(f, &project, &feature, &format!("?under={}", id_of(&c)));

    assert_eq!(
        whole,
        vec![
            json!(["", "feature", "Prices include VAT.", false, 1]),
            json!([N, "story", "A guest can buy.", false, 1]),
            json!([G, "node", "Show the restock date.", false, 1])
        ]
    );
    assert_eq!(
        under_n,
        vec![
            json!([N, "story", "A guest can buy.", false, 1]),
            json!([G, "node", "Show the restock date.", false, 1])
        ]
    );
    assert_eq!(
        under_c,
        vec![json!([G, "node", "Show the restock date.", false, 1])]
    );
}

#[test]
fn the_scope_filter_lists_only_the_rules_of_that_scope() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    rules_on_g_root_n(f, &project, &feature);

    let feature_rules = rule_rows(f, &project, &feature, "?scope=feature");
    let node_rules = rule_rows(f, &project, &feature, "?scope=node");

    assert_eq!(
        feature_rules,
        vec![json!(["", "feature", "Prices include VAT.", false, 1])]
    );
    assert_eq!(
        node_rules,
        vec![json!([G, "node", "Show the restock date.", false, 1])]
    );
}

#[test]
fn a_rule_with_an_api_key_without_a_run_is_400_identity_required_and_nothing_is_written() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let (_, press) = buy_tree(f, &project, &feature);
    let rule = add_rule(
        f,
        &project,
        &feature,
        &id_of(&press),
        &json!({ "text": BUY_DISABLED }),
    );

    let added = post_rule(
        f,
        &project,
        &feature,
        Auth::AgentWithoutRun,
        &id_of(&press),
        &json!({ "text": "Show the restock date." }),
    );
    let edited = patch_rule(
        f,
        &project,
        &feature,
        Auth::AgentWithoutRun,
        &rule,
        &json!({ "expected_version": 1, "scope": "feature" }),
    );

    assert_error(&added, 400, "identity_required");
    assert_error(&edited, 400, "identity_required");
    assert_eq!(
        rule_rows(f, &project, &feature, ""),
        vec![json!([C, "node", BUY_DISABLED, false, 1])]
    );
}

#[test]
fn a_node_or_rule_of_another_feature_is_404_and_nothing_is_written() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    buy_tree(f, &project, &feature);
    // A second feature of the same project, so only the feature tells the
    // records apart.
    let created = must(
        call(
            f,
            "POST",
            &format!("/api/v1/w/{}/projects/{project}/features", f.workspace),
            Auth::Agent,
            Some(&json!({ "name": unique("Returns") })),
        ),
        201,
        "create a second feature",
    );
    let other_feature = created["feature"]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    let theirs = add(f, &project, &other_feature, &json!({ "title": "Returns" }));
    let their_rule = add_rule(
        f,
        &project,
        &other_feature,
        &id_of(&theirs),
        &json!({ "text": "A return needs a receipt." }),
    );

    let added = post_rule(
        f,
        &project,
        &feature,
        Auth::Agent,
        &id_of(&theirs),
        &json!({ "text": BUY_DISABLED }),
    );
    let edited = patch_rule(
        f,
        &project,
        &feature,
        Auth::Agent,
        &their_rule,
        &json!({ "expected_version": 1, "text": BUY_DISABLED }),
    );

    assert_error(&added, 404, "not_found");
    assert_error(&edited, 404, "not_found");
    assert_eq!(rule_rows(f, &project, &feature, ""), Vec::<Value>::new());
    assert_eq!(
        rule_rows(f, &project, &other_feature, ""),
        vec![json!([
            "returns",
            "node",
            "A return needs a receipt.",
            false,
            1
        ])]
    );
}

// ---------------------------------------------------------------------------
// The rules that reach a node (US-211)
// ---------------------------------------------------------------------------

const F_TEXT: &str = "Prices include VAT.";
const S_TEXT: &str = "A sold-out product cannot be bought.";
const N_TEXT: &str = "The button names the price.";
const N2_TEXT: &str = "The list shows at most ten products.";

/// The criteria's tree: story `a` with `a/x`, `a/x/z` and `a/y`, story `b`
/// with `b/q`. Returns `[a, x, z, y, b, q]`.
fn two_stories(f: &Fixture, project: &str, feature: &str) -> [Value; 6] {
    let a = add(f, project, feature, &json!({ "title": "A", "slug": "a" }));
    let x = add(
        f,
        project,
        feature,
        &json!({ "title": "X", "slug": "x", "parent_id": a["id"] }),
    );
    let z = add(
        f,
        project,
        feature,
        &json!({ "title": "Z", "slug": "z", "parent_id": x["id"] }),
    );
    let y = add(
        f,
        project,
        feature,
        &json!({ "title": "Y", "slug": "y", "parent_id": a["id"] }),
    );
    let b = add(f, project, feature, &json!({ "title": "B", "slug": "b" }));
    let q = add(
        f,
        project,
        feature,
        &json!({ "title": "Q", "slug": "q", "parent_id": b["id"] }),
    );
    [a, x, z, y, b, q]
}

/// The criteria's rules: F (`feature`) on `b/q`, S (`story`) on `a/x/z`, N
/// (`node`) on `a/x`, N2 (`node`) on `a/y`. Returns `[f, s, n, n2]`.
fn four_rules(f: &Fixture, project: &str, feature: &str, tree: &[Value; 6]) -> [Value; 4] {
    let [_, x, z, y, _, q] = tree;
    [
        add_rule(
            f,
            project,
            feature,
            &id_of(q),
            &json!({ "text": F_TEXT, "scope": "feature" }),
        ),
        add_rule(
            f,
            project,
            feature,
            &id_of(z),
            &json!({ "text": S_TEXT, "scope": "story" }),
        ),
        add_rule(
            f,
            project,
            feature,
            &id_of(x),
            &json!({ "text": N_TEXT, "scope": "node" }),
        ),
        add_rule(
            f,
            project,
            feature,
            &id_of(y),
            &json!({ "text": N2_TEXT, "scope": "node" }),
        ),
    ]
}

/// The `rules` of a node's read as `[node_path, scope, text]`, in its order.
fn reaching(f: &Fixture, project: &str, feature: &str, node: &Value) -> Vec<Value> {
    must(
        read(f, project, feature, &id_of(node)),
        200,
        "read the node",
    )["rules"]
        .as_array()
        .expect("`rules` is a list")
        .iter()
        .map(|rule| json!([rule["node_path"], rule["scope"], rule["text"]]))
        .collect()
}

#[test]
fn a_read_carries_feature_rules_everywhere_story_rules_across_their_story_and_node_rules_down_from_their_node()
 {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let tree = two_stories(f, &project, &feature);
    let [rule_f, ..] = four_rules(f, &project, &feature, &tree);
    let [a, _, z, y, _, q] = &tree;
    let root = root_of(f, &project, &feature);

    let on_z = must(read(f, &project, &feature, &id_of(z)), 200, "read z")["rules"].clone();

    assert_eq!(
        on_z[0],
        json!({
            "id": rule_f["id"],
            "text": F_TEXT,
            "scope": "feature",
            "node_id": q["id"],
            "node_path": "b/q"
        })
    );
    assert_eq!(
        reaching(f, &project, &feature, z),
        vec![
            json!(["b/q", "feature", F_TEXT]),
            json!(["a/x/z", "story", S_TEXT]),
            json!(["a/x", "node", N_TEXT])
        ]
    );
    assert_eq!(
        reaching(f, &project, &feature, y),
        vec![
            json!(["b/q", "feature", F_TEXT]),
            json!(["a/x/z", "story", S_TEXT]),
            json!(["a/y", "node", N2_TEXT])
        ]
    );
    assert_eq!(
        reaching(f, &project, &feature, a),
        vec![
            json!(["b/q", "feature", F_TEXT]),
            json!(["a/x/z", "story", S_TEXT])
        ]
    );
    assert_eq!(
        reaching(f, &project, &feature, q),
        vec![json!(["b/q", "feature", F_TEXT])]
    );
    assert_eq!(
        reaching(f, &project, &feature, &root),
        vec![json!(["b/q", "feature", F_TEXT])]
    );
}

#[test]
fn a_withdrawn_rule_reaches_nothing() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let tree = two_stories(f, &project, &feature);
    let [_, rule_s, ..] = four_rules(f, &project, &feature, &tree);
    let [a, _, z, ..] = &tree;

    must(
        patch_rule(
            f,
            &project,
            &feature,
            Auth::Agent,
            &rule_s,
            &json!({ "expected_version": 1, "withdrawn": true }),
        ),
        200,
        "withdraw S",
    );

    assert_eq!(
        reaching(f, &project, &feature, z),
        vec![
            json!(["b/q", "feature", F_TEXT]),
            json!(["a/x", "node", N_TEXT])
        ]
    );
    assert_eq!(
        reaching(f, &project, &feature, a),
        vec![json!(["b/q", "feature", F_TEXT])]
    );
}

#[test]
fn a_moved_node_gets_the_rules_of_its_new_place_and_its_story_rule_reaches_its_new_story() {
    // Settled decision: reach follows the tree as it is now, so S, still in
    // force, moves to story b with its node.
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let tree = two_stories(f, &project, &feature);
    four_rules(f, &project, &feature, &tree);
    let [a, _, z, y, b, q] = &tree;

    must(
        move_node(
            f,
            &project,
            &feature,
            Auth::Agent,
            &id_of(z),
            &json!({ "expected_version": 1, "parent_id": b["id"] }),
        ),
        200,
        "move a/x/z under b",
    );

    assert_eq!(
        reaching(f, &project, &feature, z),
        vec![
            json!(["b/q", "feature", F_TEXT]),
            json!(["b/z", "story", S_TEXT])
        ]
    );
    assert_eq!(
        reaching(f, &project, &feature, q),
        vec![
            json!(["b/q", "feature", F_TEXT]),
            json!(["b/z", "story", S_TEXT])
        ]
    );
    assert_eq!(
        reaching(f, &project, &feature, a),
        vec![json!(["b/q", "feature", F_TEXT])]
    );
    assert_eq!(
        reaching(f, &project, &feature, y),
        vec![
            json!(["b/q", "feature", F_TEXT]),
            json!(["a/y", "node", N2_TEXT])
        ]
    );
}

#[test]
fn reaching_rules_come_feature_then_story_then_node_from_the_root_down_and_oldest_first_within_each()
 {
    // Settled decision: oldest first among rules of one scope, and among node
    // rules on one node. The rules are recorded in an order unlike the read's.
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let [a, x, z, _, _, q] = two_stories(f, &project, &feature);
    let root = root_of(f, &project, &feature);
    let rule = |node: &Value, text: &str, scope: &str| {
        add_rule(
            f,
            &project,
            &feature,
            &id_of(node),
            &json!({ "text": text, "scope": scope }),
        );
    };
    rule(&z, "Node rule on z.", "node");
    rule(&x, "First node rule on x.", "node");
    rule(&root, "Node rule on the root.", "node");
    rule(&x, "Second node rule on x.", "node");
    rule(&z, "Story rule on z.", "story");
    rule(&a, "Story rule on a.", "story");
    rule(&q, "Feature rule on q.", "feature");
    rule(&root, "Feature rule on the root.", "feature");

    assert_eq!(
        reaching(f, &project, &feature, &z),
        vec![
            json!(["b/q", "feature", "Feature rule on q."]),
            json!(["", "feature", "Feature rule on the root."]),
            json!(["a/x/z", "story", "Story rule on z."]),
            json!(["a", "story", "Story rule on a."]),
            json!(["", "node", "Node rule on the root."]),
            json!(["a/x", "node", "First node rule on x."]),
            json!(["a/x", "node", "Second node rule on x."]),
            json!(["a/x/z", "node", "Node rule on z."])
        ]
    );
}

// ---------------------------------------------------------------------------
// The read of one branch (US-212)
// ---------------------------------------------------------------------------

/// The `[path, questions' text]` of each `chain` entry of a node's read.
fn chain_questions(f: &Fixture, project: &str, feature: &str, node: &Value) -> Vec<Value> {
    must(
        read(f, project, feature, &id_of(node)),
        200,
        "read the node",
    )["chain"]
        .as_array()
        .expect("`chain` is a list")
        .iter()
        .map(|entry| {
            let texts: Vec<Value> = entry["questions"]
                .as_array()
                .expect("a chain entry has `questions`")
                .iter()
                .map(|question| question["question"].clone())
                .collect();
            json!([entry["path"], texts])
        })
        .collect()
}

#[test]
fn each_node_above_the_one_read_carries_its_own_questions() {
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let [a, x, z, _, _, _] = two_stories(f, &project, &feature);
    let root = root_of(f, &project, &feature);
    ask(f, &project, &feature, &id_of(&root), "Who may buy?");
    let on_story = ask(f, &project, &feature, &id_of(&a), "Guests too?");
    answer(
        f,
        &project,
        &feature,
        Auth::Agent,
        &on_story,
        &json!({ "expected_version": 1, "answer": GUEST }),
    );
    ask(f, &project, &feature, &id_of(&x), "Which button?");
    ask(f, &project, &feature, &id_of(&x), "Which colour?");
    ask(
        f,
        &project,
        &feature,
        &id_of(&z),
        "What if quantity is zero?",
    );

    assert_eq!(
        chain_questions(f, &project, &feature, &z),
        vec![
            json!(["", ["Who may buy?"]]),
            json!(["a", ["Guests too?"]]),
            json!(["a/x", ["Which button?", "Which colour?"]])
        ]
    );
}

/// The paths of the `siblings` of a node's read, in its order.
fn siblings_of(f: &Fixture, project: &str, feature: &str, node: &Value) -> Vec<Value> {
    must(
        read(f, project, feature, &id_of(node)),
        200,
        "read the node",
    )["siblings"]
        .as_array()
        .expect("`siblings` is a list")
        .iter()
        .map(|sibling| sibling["path"].clone())
        .collect()
}

#[test]
fn the_siblings_are_the_other_children_of_the_parent_by_position_and_none_for_the_root() {
    // Settled decision: siblings come by position, as children do, without
    // the node itself.
    let (f, _) = ready();
    let (project, feature) = new_feature(f, &unique("Checkout"));
    let [a, x, _, y, _, _] = two_stories(f, &project, &feature);
    let w = add(
        f,
        &project,
        &feature,
        &json!({ "title": "W", "slug": "w", "summary": "Last one.", "parent_id": a["id"] }),
    );
    // x moves to its own parent, so it is last there though added first.
    must(
        move_node(
            f,
            &project,
            &feature,
            Auth::Agent,
            &id_of(&x),
            &json!({ "expected_version": 1, "parent_id": a["id"] }),
        ),
        200,
        "move x last",
    );
    let root = root_of(f, &project, &feature);

    assert_eq!(
        siblings_of(f, &project, &feature, &y),
        vec![json!("a/w"), json!("a/x")]
    );
    assert_eq!(
        siblings_of(f, &project, &feature, &x),
        vec![json!("a/y"), json!("a/w")]
    );
    assert_eq!(siblings_of(f, &project, &feature, &a), vec![json!("b")]);
    assert_eq!(
        siblings_of(f, &project, &feature, &root),
        Vec::<Value>::new()
    );
    assert_eq!(
        must(read(f, &project, &feature, &id_of(&y)), 200, "read y")["siblings"][0],
        json!({
            "id": id_of(&w),
            "path": "a/w",
            "slug": "w",
            "title": "W",
            "summary": "Last one.",
            "status": "open",
            "same_as": null,
            "handoff": null
        })
    );
}
