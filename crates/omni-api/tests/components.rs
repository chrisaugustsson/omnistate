//! US-029: components and instances through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; the design system doc and
//! its tokens are those of tests/tokens.rs, the writes to heads those of
//! tests/live_designs.rs; the tags are those of docs/formats/design-tags.md,
//! checked by `omni_core::design::tags`):
//!
//! - `PUT /components` (agent) takes `{"tags": "<Component ...>...</Component>
//!   ..."}`, zero or more `<Component>` tags, and replaces every component of
//!   the design system's head in one all-or-nothing batch; the token tree stays
//!   as it is. A tag with an `id` names a node that the head already has, which
//!   keeps its ID; a tag without one is new and gets its ID from the server.
//!   The answer is 200 `{"project_id", "doc_id", "head", "tags",
//!   "components"}`. Tags that do not parse, or a nested instance or slot
//!   default that names no component of the import, are 400 `{"error":
//!   "invalid_components", "message"}` (the message names the problem) and
//!   change nothing. A body without `tags` is 400 `invalid_request`, a project
//!   without a design system 404 `no_design_system`, a session 403 `forbidden`.
//!   An import adds one row of `doc_changes` and emits `doc.changed`.
//! - `POST /docs/{doc_id}/ops` of a design checks every instance against the
//!   components of the design system's head: a component that is not there, a
//!   property that the component does not define, a value outside a variant's
//!   values, or an override of a node that is not in the component (or of
//!   `content` on a node that is no text) is 400 `{"error":
//!   "invalid_operation", "message"}` naming the component, the property, the
//!   value or the node, and so is a `<Component>` tag. A refused batch changes
//!   nothing.
//! - `GET /components/usage?component=<id>` (both) lists every board of the
//!   heads of the project's design docs that uses the component: through an
//!   instance anywhere in the board, a slot value of an instance, or an
//!   instance of another component whose tree uses it. The answer is 200
//!   `{"project_id", "component", "boards": [{"doc_id", "doc_title", "page_id",
//!   "page_name", "board_id", "board_name", "components"}]}`, where `components`
//!   lists every component the board uses. Without `component`, every board
//!   that uses a component is listed and `component` is `null`. A component
//!   that the design system does not define is 404 `{"error":
//!   "unknown_component"}`, and text that is no node ID 400 `invalid_request`.
//! - The outline of a design (`GET /docs/{doc_id}/html?view=outline`,
//!   tests/boards.rs), which the design viewer reads before the boards it
//!   shows, carries the same `tokens` as the whole read, and in `components`
//!   every component that the whole read lists, defined the same (it may list
//!   more of the design system), so that a board read later resolves its
//!   instances against it.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us029_components";
const TEST_BUCKET: &str = "omni-us029-components";
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
        // Heads and attachments keep their content in RustFS, so the API
        // gets a real bucket.
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
    format!("us029-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

/// Who sends a request: an agent (an API key) or the human (a session cookie).
#[derive(Clone, Copy)]
enum Auth<'a> {
    Agent(&'a str),
    Session(&'a str),
}

fn agent(f: &Fixture) -> Auth<'_> {
    Auth::Agent(&f.key)
}

fn human(f: &Fixture) -> Auth<'_> {
    Auth::Session(&f.alice_token)
}

fn auth_headers(auth: &Auth) -> Vec<(&'static str, String)> {
    match auth {
        Auth::Agent(key) => vec![
            ("Authorization", format!("Bearer {key}")),
            ("X-Omni-Run", "run-29".to_string()),
            ("X-Omni-Role", "designer".to_string()),
            ("X-Omni-Model", "test-model".to_string()),
            ("X-Omni-Harness", "test-harness".to_string()),
        ],
        Auth::Session(token) => vec![("Cookie", cookie(token))],
    }
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let headers = auth_headers(auth);
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

/// Asserts a 400 refusal with `code` whose message has every needle in it.
fn assert_refused(response: &Response, code: &str, needles: &[&str]) {
    assert_eq!(response.status, 400, "body: {}", response.text());
    let body = response.json();
    assert_eq!(body["error"], code, "{body}");
    let message = body["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal has a message: {body}"));
    for needle in needles {
        assert!(
            message.contains(needle),
            "the message should name {needle:?}: {message:?}"
        );
    }
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

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &agent(f),
                Some(&json!({ "name": unique("components-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

/// Creates a doc; returns its ID.
fn create_doc(
    f: &Fixture,
    client: &Client,
    project: &str,
    kind: &str,
    pillar: &str,
    title: &str,
) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", project_base(f, project)),
                &agent(f),
                Some(&json!({ "kind": kind, "pillar": pillar, "title": title })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

fn import_tokens(f: &Fixture, client: &Client, project: &str) {
    must(
        call(
            client,
            "PUT",
            &format!("{}/tokens", project_base(f, project)),
            &agent(f),
            Some(&json!({ "tokens": tokens() })),
        ),
        200,
        "import the tokens",
    );
}

fn components_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    tags: &str,
) -> Response {
    call(
        client,
        "PUT",
        &format!("{}/components", project_base(f, project)),
        auth,
        Some(&json!({ "tags": tags })),
    )
}

fn import_components(f: &Fixture, client: &Client, project: &str, tags: &str) -> Value {
    must(
        components_response(f, client, &agent(f), project, tags),
        200,
        "import the components",
    )
}

fn edit_response(f: &Fixture, client: &Client, project: &str, doc: &str, ops: &Value) -> Response {
    call(
        client,
        "POST",
        &format!("{}/docs/{doc}/ops", project_base(f, project)),
        &agent(f),
        Some(&json!({ "ops": ops })),
    )
}

fn edit(f: &Fixture, client: &Client, project: &str, doc: &str, ops: &Value) {
    must(
        edit_response(f, client, project, doc, ops),
        200,
        "edit the design",
    );
}

/// The whole read of a design's head.
fn read_head(f: &Fixture, client: &Client, project: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/docs/{doc}/html", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "read the design's head",
    )
}

/// The ID of the first page of a design.
fn first_page(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    id_of(&read_head(f, client, project, doc)["pages"][0])
}

fn insert(parent: &str, content: &str) -> Value {
    json!({ "op": "insert", "parent": parent, "position": { "after": null }, "content": content })
}

/// How many writes the doc's log holds.
fn logged_writes(doc: &str) -> i64 {
    count(
        "select count(*) from doc_changes where doc_id = $1::uuid",
        &[doc],
    )
}

/// A small token tree.
fn tokens() -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "$type": "color",
            "surface": {
                "$value": "#ffffff",
                "$extensions": {"omnistate": {"modes": {"light": "#ffffff", "dark": "#0f172a"}}}
            }
        }
    })
}

fn usage_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    query: &str,
) -> Response {
    call(
        client,
        "GET",
        &format!("{}/components/usage{query}", project_base(f, project)),
        auth,
        None,
    )
}

fn usage(f: &Fixture, client: &Client, project: &str, component: &str) -> Value {
    must(
        usage_response(
            f,
            client,
            &agent(f),
            project,
            &format!("?component={component}"),
        ),
        200,
        "list the boards that use a component",
    )
}

fn board_names(answer: &Value) -> BTreeSet<String> {
    answer["boards"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer lists boards: {answer}"))
        .iter()
        .map(|board| {
            board["board_name"]
                .as_str()
                .unwrap_or_else(|| panic!("each board has a name: {board}"))
                .to_string()
        })
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
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
            .unwrap_or_else(|e| panic!("count with {sql}: {e}"))
    })
}

/// The `data` of the project's live events named `event`, oldest first.
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
        rows.iter()
            .map(|text| serde_json::from_str(text).expect("event data is JSON"))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// The library: a design system with five components, and two design docs
// ---------------------------------------------------------------------------

/// Node IDs by node name. Every name in the library is used once.
type Ids = HashMap<String, String>;

/// Collects the ID of every node in `nodes` and their subtrees, by name.
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

/// ` id="<id>"` for a node the head already has, nothing for a new one.
fn id_attr(ids: &Ids, name: &str) -> String {
    ids.get(name)
        .map(|id| format!(r#" id="{id}""#))
        .unwrap_or_default()
}

fn id(ids: &Ids, name: &str) -> String {
    ids.get(name)
        .unwrap_or_else(|| panic!("the node {name} has an ID: {ids:?}"))
        .clone()
}

/// The node named `name` in `nodes` and their subtrees.
fn node_named(nodes: &Value, name: &str) -> Option<Value> {
    for node in nodes.as_array()? {
        if node["name"] == name {
            return Some(node.clone());
        }
        if let Some(found) = node_named(&node["children"], name) {
            return Some(found);
        }
    }
    None
}

fn find(nodes: &Value, name: &str) -> Value {
    node_named(nodes, name).unwrap_or_else(|| panic!("there is a node named {name}: {nodes:#}"))
}

/// Two icons.
fn icon_tags(ids: &Ids) -> String {
    format!(
        r#"<Component{} name="Cart" width={{16}} height={{16}}>
  <Vector{} name="Cart glyph" x={{0}} y={{0}} width={{16}} height={{16}} path="M0 0 L16 16"/>
</Component>
<Component{} name="Heart" width={{16}} height={{16}}>
  <Vector{} name="Heart glyph" x={{0}} y={{0}} width={{16}} height={{16}} path="M8 16 L0 4 L16 4 Z"/>
</Component>
"#,
        id_attr(ids, "Cart"),
        id_attr(ids, "Cart glyph"),
        id_attr(ids, "Heart"),
        id_attr(ids, "Heart glyph"),
    )
}

/// A button with a property of each kind; `values` are the values of its
/// `style` variant and `label` the default of its text property.
fn button_tags(ids: &Ids, values: &[&str], label: &str) -> String {
    let values: Vec<String> = values.iter().map(|value| format!("\"{value}\"")).collect();
    let cart = id(ids, "Cart");
    format!(
        r##"<Component{} name="Button" dir="row" gap={{8}} pad={{12}} fill="#2563eb" radius={{8}} props={{{{style: {{type: "variant", values: [{}], default: "primary"}}, showIcon: {{type: "boolean", default: true}}, label: {{type: "text", default: "{label}"}}, icon: {{type: "slot", default: "{cart}"}}}}}}>
  <Instance{} name="Icon" component="{cart}" bind={{{{visible: "showIcon", component: "icon"}}}}/>
  <Text{} name="Label" content="{label}" fontFamily="Inter" fontSize={{14}} color="#ffffff" bind={{{{content: "label"}}}}/>
  <Frame{} name="Badge" width={{8}} height={{8}} fill="#dc2626"/>
</Component>
"##,
        id_attr(ids, "Button"),
        values.join(", "),
        id_attr(ids, "Icon"),
        id_attr(ids, "Label"),
        id_attr(ids, "Badge"),
    )
}

/// A card that holds a button, and a tag that no board uses.
fn card_and_tag_tags(ids: &Ids) -> String {
    format!(
        r##"<Component{} name="Card" dir="col" gap={{8}} pad={{16}} fill="#ffffff">
  <Text{} name="Card title" content="Däck" fontFamily="Inter" fontSize={{16}}/>
  <Instance{} name="Card button" component="{}"/>
</Component>
<Component{} name="Tag" dir="row" pad={{4}} fill="#e2e8f0">
  <Text{} name="Tag text" content="Ny" fontFamily="Inter" fontSize={{12}}/>
</Component>
"##,
        id_attr(ids, "Card"),
        id_attr(ids, "Card title"),
        id_attr(ids, "Card button"),
        id(ids, "Button"),
        id_attr(ids, "Tag"),
        id_attr(ids, "Tag text"),
    )
}

/// Every component of the library, with the IDs it has.
fn library_tags(ids: &Ids, values: &[&str], label: &str) -> String {
    format!(
        "{}{}{}",
        icon_tags(ids),
        button_tags(ids, values, label),
        card_and_tag_tags(ids)
    )
}

/// An instance of the button with a value for each property and overrides of
/// its label, badge and icon.
fn pay_button(ids: &Ids, style: &str) -> String {
    format!(
        r##"<Instance name="Pay button" component="{}" props={{{{style: "{style}", showIcon: false, label: "Pay", icon: "{}"}}}} overrides={{{{"{}": {{content: "Pay now"}}, "{}": {{fills: [{{type: "solid", color: "#16a34a"}}]}}, "{}": {{visible: false}}}}}}/>"##,
        id(ids, "Button"),
        id(ids, "Heart"),
        id(ids, "Label"),
        id(ids, "Badge"),
        id(ids, "Icon"),
    )
}

/// The three boards of the shop: a checkout board with `instance`, a product
/// board with a card, and a board without instances.
fn shop_boards(ids: &Ids, instance: &str) -> [String; 3] {
    [
        format!(
            r##"<Board name="Checkout" x={{0}} y={{0}} width={{375}} height={{812}} dir="col" gap={{16}}>
  <Frame name="Footer" dir="row" gap={{8}}>
    {instance}
  </Frame>
</Board>"##
        ),
        format!(
            r##"<Board name="Product" x={{400}} y={{0}} width={{375}} height={{812}} dir="col">
  <Instance name="Product card" component="{}"/>
</Board>"##,
            id(ids, "Card"),
        ),
        r##"<Board name="Empty" x={800} y={0} width={375} height={812} dir="col">
  <Text name="Nothing" content="Tomt" fontFamily="Inter" fontSize={14}/>
</Board>"##
            .to_string(),
    ]
}

/// The one board of the admin, with a plain button.
fn admin_board(ids: &Ids) -> String {
    format!(
        r#"<Board name="Settings" x={{0}} y={{0}} width={{1280}} height={{800}} dir="col">
  <Instance name="Save" component="{}"/>
</Board>"#,
        id(ids, "Button"),
    )
}

/// A board with one instance, to try an instance out on.
fn board_with(instance: &str) -> String {
    format!(
        r#"<Board name="Trial" x={{2000}} y={{0}} width={{375}} height={{812}} dir="col">
  {instance}
</Board>"#
    )
}

const STYLES: [&str; 2] = ["primary", "secondary"];

struct Library {
    project: String,
    shop: String,
    shop_title: String,
    /// The first page of the shop, named Home.
    shop_page: String,
    /// Component and layer IDs by name.
    ids: Ids,
    /// Board, page and instance IDs by name.
    nodes: Ids,
}

/// A project with a design system (tokens and the five components) and two
/// design docs that use them.
fn library(f: &Fixture, client: &Client) -> Library {
    let project = new_project(f, client);
    create_doc(f, client, &project, "design-system", "ui", "Design system");
    import_tokens(f, client, &project);
    // New components get their IDs from the server, so a component that
    // places another is written once the other has its ID.
    let icons = import_components(f, client, &project, &icon_tags(&Ids::new()));
    let mut ids = ids_in(&icons["components"]);
    let with_button = import_components(
        f,
        client,
        &project,
        &format!("{}{}", icon_tags(&ids), button_tags(&ids, &STYLES, "Buy")),
    );
    ids = ids_in(&with_button["components"]);
    let all = import_components(f, client, &project, &library_tags(&ids, &STYLES, "Buy"));
    ids = ids_in(&all["components"]);
    for name in ["Cart", "Heart", "Button", "Card", "Tag"] {
        assert!(ids.contains_key(name), "the import holds {name}: {all}");
    }

    let shop_title = unique("Shop");
    let shop = create_doc(f, client, &project, "design", "ui", &shop_title);
    let shop_page = first_page(f, client, &project, &shop);
    let boards = shop_boards(&ids, &pay_button(&ids, "secondary"));
    edit(
        f,
        client,
        &project,
        &shop,
        &json!([
            { "op": "set", "node": shop_page, "rev": 1, "properties": { "name": "Home" } },
            insert(&shop_page, &boards[2]),
            insert(&shop_page, &boards[1]),
            insert(&shop_page, &boards[0])
        ]),
    );
    let admin = create_doc(f, client, &project, "design", "ui", "Admin");
    let admin_page = first_page(f, client, &project, &admin);
    edit(
        f,
        client,
        &project,
        &admin,
        &json!([insert(&admin_page, &admin_board(&ids))]),
    );
    let mut nodes = ids_in(&read_head(f, client, &project, &shop)["pages"]);
    nodes.extend(ids_in(&read_head(f, client, &project, &admin)["pages"]));
    Library {
        project,
        shop,
        shop_title,
        shop_page,
        ids,
        nodes,
    }
}

// ---------------------------------------------------------------------------
// <Component> tags in the design system doc
// ---------------------------------------------------------------------------

#[test]
fn a_components_import_is_logged_and_emits_an_event() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = create_doc(f, client, &project, "design-system", "ui", "DS");
    let before = logged_writes(&doc);
    let imported = import_components(f, client, &project, &icon_tags(&Ids::new()));
    assert_eq!(imported["doc_id"], doc.as_str(), "{imported}");
    assert_eq!(logged_writes(&doc), before + 1, "the import is in the log");
    let events = events_of(&project, "doc.changed");
    let data = events.last().expect("the import emits doc.changed");
    assert_eq!(data["doc_id"], doc.as_str(), "{data}");
    assert_eq!(data["head"], imported["head"], "{data}");
}

#[test]
fn a_refused_components_import_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = create_doc(f, client, &project, "design-system", "ui", "DS");
    let first = import_components(f, client, &project, &icon_tags(&Ids::new()));
    let ids = ids_in(&first["components"]);
    let writes = logged_writes(&doc);
    let events = events_of(&project, "doc.changed").len();

    let other_kind = icon_tags(&ids).replace(
        r#"name="Cart" width"#,
        r#"name="Cart" props={{size: {type: "number", default: 1}}} width"#,
    );
    assert_refused(
        &components_response(f, client, &agent(f), &project, &other_kind),
        "invalid_components",
        &["number"],
    );
    let unknown_slot = format!("{}{}", icon_tags(&ids), button_tags(&ids, &STYLES, "Buy")).replace(
        &format!(r#"default: "{}""#, ids["Cart"]),
        &format!(r#"default: "{UNKNOWN_NODE}""#),
    );
    assert_refused(
        &components_response(f, client, &agent(f), &project, &unknown_slot),
        "invalid_components",
        &[UNKNOWN_NODE],
    );
    let unknown_nested = format!("{}{}", icon_tags(&ids), button_tags(&ids, &STYLES, "Buy"))
        .replace(
            &format!(r#"name="Icon" component="{}""#, ids["Cart"]),
            &format!(r#"name="Icon" component="{UNKNOWN_NODE}""#),
        );
    assert_refused(
        &components_response(f, client, &agent(f), &project, &unknown_nested),
        "invalid_components",
        &[UNKNOWN_NODE],
    );
    assert_refused(
        &components_response(f, client, &agent(f), &project, "<Component name="),
        "invalid_components",
        &["line 1"],
    );
    assert_eq!(logged_writes(&doc), writes, "a refused import logs nothing");
    assert_eq!(
        events_of(&project, "doc.changed").len(),
        events,
        "a refused import emits nothing"
    );
    let unchanged = import_components(f, client, &project, &icon_tags(&ids));
    assert_eq!(ids_in(&unchanged["components"]), ids);

    // Who may import, and what an import needs.
    let response = components_response(f, client, &human(f), &project, &icon_tags(&ids));
    assert_error(&response, 403, "forbidden");
    let response = call(
        client,
        "PUT",
        &format!("{}/components", project_base(f, &project)),
        &agent(f),
        Some(&json!({})),
    );
    assert_error(&response, 400, "invalid_request");
    let bare = new_project(f, client);
    let response = components_response(f, client, &agent(f), &bare, &icon_tags(&Ids::new()));
    assert_error(&response, 404, "no_design_system");
}

// ---------------------------------------------------------------------------
// <Instance component props overrides> in design docs
// ---------------------------------------------------------------------------

#[test]
fn an_instance_may_swap_the_contents_of_a_slot_with_an_override() {
    let (f, client) = ready();
    let lib = library(f, client);
    let ids = &lib.ids;
    let swapped = format!(
        r#"<Instance name="Swapped button" component="{}" overrides={{{{"{}": {{component: "{}"}}}}}}/>"#,
        ids["Button"], ids["Icon"], ids["Heart"]
    );
    edit(
        f,
        client,
        &lib.project,
        &lib.shop,
        &json!([insert(&lib.shop_page, &board_with(&swapped))]),
    );
    let head = read_head(f, client, &lib.project, &lib.shop);
    let instance = find(&head["pages"], "Swapped button");
    assert_eq!(
        instance["overrides"][ids["Icon"].as_str()],
        json!({"component": ids["Heart"]}),
        "{instance:#}"
    );
}

#[test]
fn an_edit_refuses_instances_that_do_not_fit_their_component() {
    let (f, client) = ready();
    let lib = library(f, client);
    let ids = &lib.ids;
    let before = read_head(f, client, &lib.project, &lib.shop);
    let writes = logged_writes(&lib.shop);
    let events = events_of(&lib.project, "doc.changed").len();
    let refuse_content = |content: &str, needles: &[&str]| {
        assert_refused(
            &edit_response(
                f,
                client,
                &lib.project,
                &lib.shop,
                &json!([insert(&lib.shop_page, content)]),
            ),
            "invalid_operation",
            needles,
        );
    };
    let refuse = |instance: &str, needles: &[&str]| refuse_content(&board_with(instance), needles);

    let unknown_component = format!(r#"<Instance name="Pay button" component="{UNKNOWN_NODE}"/>"#);
    refuse(&unknown_component, &[UNKNOWN_NODE]);
    let unknown_prop = format!(
        r#"<Instance name="Pay button" component="{}" props={{{{colour: "red"}}}}/>"#,
        ids["Button"]
    );
    refuse(&unknown_prop, &["colour"]);
    refuse(&pay_button(ids, "ghost"), &["ghost", "primary"]);
    let boolean_as_text = format!(
        r#"<Instance name="Pay button" component="{}" props={{{{showIcon: "no"}}}}/>"#,
        ids["Button"]
    );
    refuse(&boolean_as_text, &["showIcon"]);
    let slot_of_nothing = format!(
        r#"<Instance name="Pay button" component="{}" props={{{{icon: "{UNKNOWN_NODE}"}}}}/>"#,
        ids["Button"]
    );
    refuse(&slot_of_nothing, &[UNKNOWN_NODE]);
    let override_of_nothing = format!(
        r#"<Instance name="Pay button" component="{}" overrides={{{{"{UNKNOWN_NODE}": {{visible: false}}}}}}/>"#,
        ids["Button"]
    );
    refuse(&override_of_nothing, &[UNKNOWN_NODE]);
    let text_on_a_frame = format!(
        r#"<Instance name="Pay button" component="{}" overrides={{{{"{}": {{content: "Hej"}}}}}}/>"#,
        ids["Button"], ids["Badge"]
    );
    refuse(&text_on_a_frame, &["content"]);
    refuse_content(
        r##"<Component name="Inline" fill="#ffffff"/>"##,
        &["Component"],
    );
    refuse_content("<Board name=", &["line 1"]);

    let after = read_head(f, client, &lib.project, &lib.shop);
    assert_eq!(
        after["tags"], before["tags"],
        "a refused edit changes nothing"
    );
    assert_eq!(
        logged_writes(&lib.shop),
        writes,
        "a refused edit logs nothing"
    );
    assert_eq!(
        events_of(&lib.project, "doc.changed").len(),
        events,
        "a refused edit emits nothing"
    );
}

// ---------------------------------------------------------------------------
// Component usage
// ---------------------------------------------------------------------------

#[test]
fn usage_lists_every_board_of_the_project_that_uses_the_component() {
    let (f, client) = ready();
    let lib = library(f, client);
    let ids = &lib.ids;
    let nodes = &lib.nodes;

    let button = usage(f, client, &lib.project, &ids["Button"]);
    assert_eq!(button["project_id"], lib.project.as_str(), "{button}");
    assert_eq!(button["component"], ids["Button"].as_str(), "{button}");
    assert_eq!(
        board_names(&button),
        set(&["Checkout", "Product", "Settings"]),
        "an instance deep in a board, an instance in another component's tree and an instance in another doc: {button:#}"
    );
    let checkout = button["boards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|board| board["board_name"] == "Checkout")
        .unwrap()
        .clone();
    assert_eq!(checkout["doc_id"], lib.shop.as_str(), "{checkout}");
    assert_eq!(checkout["doc_title"], lib.shop_title.as_str(), "{checkout}");
    assert_eq!(checkout["page_id"], nodes["Home"].as_str(), "{checkout}");
    assert_eq!(checkout["page_name"], "Home", "{checkout}");
    assert_eq!(
        checkout["board_id"],
        nodes["Checkout"].as_str(),
        "{checkout}"
    );
    let used: BTreeSet<&str> = checkout["components"]
        .as_array()
        .unwrap_or_else(|| panic!("the board lists its components: {checkout}"))
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        used.contains(ids["Button"].as_str()) && used.contains(ids["Heart"].as_str()),
        "{checkout}"
    );

    assert_eq!(
        board_names(&usage(f, client, &lib.project, &ids["Heart"])),
        set(&["Checkout"]),
        "a slot value is a use"
    );
    assert_eq!(
        board_names(&usage(f, client, &lib.project, &ids["Card"])),
        set(&["Product"])
    );
    assert!(
        board_names(&usage(f, client, &lib.project, &ids["Cart"])).contains("Settings"),
        "the default of a slot is a use"
    );
    let tag = usage(f, client, &lib.project, &ids["Tag"]);
    assert_eq!(tag["boards"], json!([]), "a component no board uses: {tag}");
}

#[test]
fn usage_without_a_component_lists_every_board_that_uses_one() {
    let (f, client) = ready();
    let lib = library(f, client);
    for auth in [agent(f), human(f)] {
        let answer = must(
            usage_response(f, client, &auth, &lib.project, ""),
            200,
            "list the boards that use components",
        );
        assert_eq!(answer["project_id"], lib.project.as_str(), "{answer}");
        assert_eq!(answer["component"], Value::Null, "{answer}");
        assert_eq!(
            board_names(&answer),
            set(&["Checkout", "Product", "Settings"]),
            "{answer:#}"
        );
    }
}

#[test]
fn usage_of_an_unknown_component_is_refused() {
    let (f, client) = ready();
    let lib = library(f, client);
    assert_error(
        &usage_response(
            f,
            client,
            &agent(f),
            &lib.project,
            &format!("?component={UNKNOWN_NODE}"),
        ),
        404,
        "unknown_component",
    );
    assert_error(
        &usage_response(f, client, &agent(f), &lib.project, "?component=Button"),
        400,
        "invalid_request",
    );
}

// ---------------------------------------------------------------------------
// The outline that the design viewer reads first
// ---------------------------------------------------------------------------

#[test]
fn the_outline_of_a_design_carries_its_design_system_as_the_whole_read_does() {
    let (f, client) = ready();
    let lib = library(f, client);
    let whole = read_head(f, client, &lib.project, &lib.shop);
    let response = call(
        client,
        "GET",
        &format!(
            "{}/docs/{}/html?view=outline",
            project_base(f, &lib.project),
            lib.shop
        ),
        &human(f),
        None,
    );
    let text = response.text();
    let outline = must(response, 200, "the viewer reads the design's outline");
    assert!(
        outline["outline"].is_array(),
        "the answer is the outline: {outline:#}"
    );
    assert!(
        outline.get("pages").is_none_or(Value::is_null),
        "the outline has no pages body: {outline:#}"
    );
    assert_eq!(
        outline["tokens"], whole["tokens"],
        "the design system's tokens"
    );
    let listed = outline["components"]
        .as_array()
        .unwrap_or_else(|| panic!("the outline lists components: {outline:#}"));
    let used = whole["components"]
        .as_array()
        .expect("the whole read lists components");
    assert!(
        !used.is_empty(),
        "the shop's instances use components: {whole:#}"
    );
    for component in used {
        assert!(
            listed.contains(component),
            "the outline defines {} as the whole read does: {outline:#}",
            component["name"]
        );
    }
    // The instances themselves are inside boards, so not in the outline.
    for name in ["Pay button", "Product card", "Footer"] {
        let node = lib
            .nodes
            .get(name)
            .unwrap_or_else(|| panic!("the shop has {name}: {:?}", lib.nodes));
        assert!(
            !text.contains(node.as_str()),
            "the outline holds nothing of {name}, which is inside a board: {text}"
        );
    }
}
