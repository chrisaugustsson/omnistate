//! US-18: destinations of interactions are checked on write (D17), through
//! the API. Run with `--features stack-tests` in the Compose test container.
//!
//! The contract these tests pin down (the batch route is that of
//! tests/design_edits.rs, the tags those of docs/formats/design-tags.md):
//!
//! - A `set`, `insert` or `replace` whose `interactions` hold a navigate,
//!   open overlay or swap overlay `dest` that is a board of the doc applies;
//!   one that is another node answers 400 `invalid_operation` with "`<id>` is
//!   not a board: navigate, openOverlay and swapOverlay take a board of this
//!   design", one that is in no node of the doc with "`<id>` is not in the
//!   doc". A refused write leaves the node as it was.
//! - A scroll to takes a layer below the top-level board of the trigger; a
//!   change to takes a component of the variant set of the instance's
//!   component, and only an instance has one.
//! - Deleting a board that interactions point at applies; they stay as they
//!   were, a set that does not write `interactions` applies, and one that
//!   writes the dangling `dest` again is refused.
//! - US-29: the variables and modes that the actions of a `set` or `insert`
//!   name are checked against the project's tokens (the rules themselves are
//!   pinned in omni-core's tests/design_interactions.rs). A project without
//!   tokens has no variables.
//! - US-26: every token reference of a node that a `set` or `insert` writes
//!   is checked against the project's tokens (the rules themselves are pinned
//!   in omni-core's tests/design_system_tokens.rs), including references the
//!   operation leaves as they were. A project without a design system has no
//!   tokens to refer to.

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

const TEST_DB: &str = "omni_us018_design_interactions";
const TEST_BUCKET: &str = "omni-us018-design-interactions";
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
        // Heads keep their content in RustFS, so the API gets a real bucket.
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
    key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us018-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
            key,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
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

const CART: &str = r##"<Board name="Cart" x={0} y={0} width={360} height={640} dir="col" gap={8}>
  <Frame name="Summary" dir="col" gap={4}>
    <Text name="Total" content="Summa 10 kr" fontFamily="Inter" fontSize={16}/>
    <Text name="Shipping" content="Frakt ingår" fontFamily="Inter" fontSize={14}/>
  </Frame>
  <Frame name="Actions" dir="row" gap={8}/>
</Board>"##;

const PAYMENT: &str = r##"<Board name="Payment" x={400} y={0} width={360} height={640} dir="col">
  <Text name="Pay" content="Betala nu" fontFamily="Inter" fontSize={16}/>
</Board>"##;

/// A project with a design doc whose first page, here Checkout, has the
/// boards of `CART` and `PAYMENT`.
struct Design {
    project: String,
    doc: String,
    ids: Ids,
}

impl Design {
    fn base(&self, f: &Fixture) -> String {
        format!("/api/v1/w/{}/projects/{}", f.workspace, self.project)
    }

    fn doc_path(&self, f: &Fixture) -> String {
        format!("{}/docs/{}", self.base(f), self.doc)
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
    let doc = str_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/docs"),
                &holder(f),
                Some(&json!({ "kind": "design", "pillar": "ui", "title": unique("design") })),
            ),
            201,
            "create a design doc",
        )["doc"],
        "id",
    )
    .to_string();
    let mut design = Design {
        project,
        doc,
        ids: Ids::new(),
    };
    let page = str_of(&head_pages(f, client, &design)[0], "id").to_string();
    applied(
        f,
        client,
        &design,
        json!([insert(&page, None, PAYMENT), insert(&page, None, CART)]),
    );
    collect_ids(&head_pages(f, client, &design), &mut design.ids);
    design.ids.insert("Checkout".to_string(), page);
    assert_eq!(
        design.ids.len(),
        9,
        "the first page's own name, then 8 nodes: {:?}",
        design.ids
    );
    design
}

/// Sends a batch of operations as `auth`.
fn edit(f: &Fixture, client: &Client, auth: &Auth, design: &Design, ops: Value) -> Response {
    call(
        client,
        "POST",
        &format!("{}/ops", design.doc_path(f)),
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

/// One board of the head in the tag format.
fn board_tags(f: &Fixture, client: &Client, design: &Design, board: &str) -> String {
    let answer = must(
        call(
            client,
            "GET",
            &format!("{}/boards/{board}", design.doc_path(f)),
            &holder(f),
            None,
        ),
        200,
        "read a board of the head",
    );
    str_of(&answer, "tags").to_string()
}

/// The pages of the head as JSON nodes.
fn head_pages(f: &Fixture, client: &Client, design: &Design) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/html", design.doc_path(f)),
            &holder(f),
            None,
        ),
        200,
        "read the head",
    )["pages"]
        .clone()
}

fn insert(parent: &str, after: Option<&str>, content: &str) -> Value {
    json!({ "op": "insert", "parent": parent, "position": { "after": after }, "content": content })
}

fn set(node: &str, rev: u64, properties: Value) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": properties })
}

fn delete(node: &str, rev: u64) -> Value {
    json!({ "op": "delete", "node": node, "rev": rev })
}

// ---------------------------------------------------------------------------
// Interactions
// ---------------------------------------------------------------------------

fn interactions(action: Value) -> Value {
    json!({ "interactions": [{ "trigger": { "type": "onClick" }, "actions": [action] }] })
}

fn go(kind: &str, dest: &str) -> Value {
    json!({ "type": kind, "dest": dest })
}

/// The message of a refused batch, after the refusal's shape is asserted.
fn refusal(response: &Response) -> String {
    assert_error(response, 400, "invalid_operation");
    str_of(&response.json(), "message").to_string()
}

#[test]
fn a_destination_of_the_wrong_kind_or_outside_the_doc_is_refused_and_leaves_the_node() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let (total, payment, summary) = (
        design.id("Total"),
        design.id("Payment"),
        design.id("Summary"),
    );
    let before = board_tags(f, client, &design, &design.id("Cart"));

    for kind in ["navigate", "openOverlay", "swapOverlay"] {
        let wrong = edit(
            f,
            client,
            &holder(f),
            &design,
            json!([set(&total, 1, interactions(go(kind, &summary)))]),
        );
        assert_eq!(
            refusal(&wrong),
            format!(
                "`{summary}` is not a board: navigate, openOverlay and swapOverlay take a board of this design"
            )
        );
        for missing in [UNKNOWN_NODE, "xyz!"] {
            let unknown = edit(
                f,
                client,
                &holder(f),
                &design,
                json!([set(&total, 1, interactions(go(kind, missing)))]),
            );
            assert_eq!(refusal(&unknown), format!("`{missing}` is not in the doc"));
        }
    }
    assert_eq!(board_tags(f, client, &design, &design.id("Cart")), before);

    // A conditional block is checked too.
    let nested = interactions(json!({
        "type": "conditional",
        "blocks": [{ "actions": [go("navigate", &summary)] }]
    }));
    let response = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([set(&total, 1, nested)]),
    );
    assert!(refusal(&response).contains("is not a board"));

    // An insert is checked like a set.
    let content = |dest: &str| {
        format!(
            r#"<Text name="Link" content="Gå" fontFamily="Inter" fontSize={{14}} interactions={{[{{trigger: {{type: "onClick"}}, actions: [{{type: "navigate", dest: "{dest}"}}]}}]}}/>"#
        )
    };
    let response = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([insert(&design.id("Actions"), None, &content(&summary))]),
    );
    assert!(refusal(&response).contains("is not a board"));
    assert_eq!(board_tags(f, client, &design, &design.id("Cart")), before);

    applied(
        f,
        client,
        &design,
        json!([insert(&design.id("Actions"), None, &content(&payment))]),
    );
    applied(
        f,
        client,
        &design,
        json!([set(&total, 1, interactions(go("openOverlay", &payment)))]),
    );
}

#[test]
fn a_scroll_to_takes_a_layer_of_its_own_board_and_a_change_to_needs_an_instance() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let total = design.id("Total");
    let layer = |id: &str| {
        format!(
            "`{id}` is not a layer of this board: scrollTo takes a layer of the board the trigger is on"
        )
    };

    for outside in [
        design.id("Cart"),
        design.id("Pay"),
        design.id("Payment"),
        design.id("Checkout"),
    ] {
        let response = edit(
            f,
            client,
            &holder(f),
            &design,
            json!([set(&total, 1, interactions(go("scrollTo", &outside)))]),
        );
        assert_eq!(refusal(&response), layer(&outside));
    }
    let response = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([set(&total, 1, interactions(go("scrollTo", UNKNOWN_NODE)))]),
    );
    assert_eq!(
        refusal(&response),
        format!("`{UNKNOWN_NODE}` is not in the doc")
    );

    let response = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([set(
            &total,
            1,
            interactions(go("changeTo", &design.id("Shipping")))
        )]),
    );
    assert_eq!(
        refusal(&response),
        "changeTo is for instances and the layers of a component"
    );

    applied(
        f,
        client,
        &design,
        json!([set(
            &total,
            1,
            interactions(go("scrollTo", &design.id("Shipping")))
        )]),
    );
    // A layer right below the board is one of its layers too.
    applied(
        f,
        client,
        &design,
        json!([set(
            &total,
            2,
            interactions(go("scrollTo", &design.id("Actions")))
        )]),
    );
}

#[test]
fn a_replace_checks_the_layers_below_the_replaced_node() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let actions = design.id("Actions");
    let before = board_tags(f, client, &design, &design.id("Cart"));
    let content = format!(
        r#"<Frame name="Summary" dir="col" gap={{4}}>
  <Text name="Total" content="Summa 10 kr" fontFamily="Inter" fontSize={{16}} interactions={{[{{trigger: {{type: "onClick"}}, actions: [{{type: "navigate", dest: "{actions}"}}]}}]}}/>
</Frame>"#
    );

    let response = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([{ "op": "replace", "node": design.id("Summary"), "rev": 1, "content": content }]),
    );
    assert_eq!(
        refusal(&response),
        format!(
            "`{actions}` is not a board: navigate, openOverlay and swapOverlay take a board of this design"
        )
    );
    assert_eq!(board_tags(f, client, &design, &design.id("Cart")), before);
}

/// Imports the components of `tags` into a design system doc of the
/// design's project; returns their IDs, in order.
fn import_components(f: &Fixture, client: &Client, design: &Design, tags: &str) -> Vec<String> {
    must(
        call(
            client,
            "POST",
            &format!("{}/docs", design.base(f)),
            &holder(f),
            Some(&json!({ "kind": "design-system", "pillar": "ui", "title": unique("ds") })),
        ),
        201,
        "create a design system doc",
    );
    let answer = must(
        call(
            client,
            "PUT",
            &format!("{}/components", design.base(f)),
            &holder(f),
            Some(&json!({ "tags": tags })),
        ),
        200,
        "import the components",
    );
    answer["components"]
        .as_array()
        .expect("the import lists components")
        .iter()
        .map(|component| str_of(component, "id").to_string())
        .collect()
}

#[test]
fn a_change_to_takes_a_component_of_the_variant_set_of_the_instance() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let variant = |default: &str| {
        format!(
            r#"<Component name="Button" width={{40}} height={{20}} props={{{{state: {{type: "variant", values: ["on", "off"], default: "{default}"}}}}}}/>
"#
        )
    };
    let chip = r#"<Component name="Chip" width={20} height={20}/>"#;
    let ids = import_components(
        f,
        client,
        &design,
        &format!("{}{}{chip}", variant("on"), variant("off")),
    );
    let [on, off, chip] = [&ids[0], &ids[1], &ids[2]];
    let instance = format!(r#"<Instance name="Toggle" component="{on}"/>"#);
    let results = applied(
        f,
        client,
        &design,
        json!([insert(&design.id("Actions"), None, &instance)]),
    );
    let toggle = str_of(&results[0], "node").to_string();

    let response = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([set(&toggle, 1, interactions(go("changeTo", chip)))]),
    );
    assert_eq!(
        refusal(&response),
        format!("`{chip}` is not a variant of `Button`")
    );
    applied(
        f,
        client,
        &design,
        json!([set(&toggle, 1, interactions(go("changeTo", off)))]),
    );
    applied(
        f,
        client,
        &design,
        json!([set(&toggle, 2, interactions(go("changeTo", on)))]),
    );
}

#[test]
fn deleting_a_board_leaves_the_interactions_that_point_at_it() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let (total, payment) = (design.id("Total"), design.id("Payment"));
    applied(
        f,
        client,
        &design,
        json!([set(&total, 1, interactions(go("navigate", &payment)))]),
    );
    let before = board_tags(f, client, &design, &design.id("Cart"));
    assert!(before.contains(&payment), "{before}");

    applied(f, client, &design, json!([delete(&payment, 1)]));
    assert_eq!(board_tags(f, client, &design, &design.id("Cart")), before);

    applied(
        f,
        client,
        &design,
        json!([set(&total, 2, json!({ "name": "Summa" }))]),
    );
    let again = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([set(&total, 3, interactions(go("navigate", &payment)))]),
    );
    assert_eq!(refusal(&again), format!("`{payment}` is not in the doc"));
}

// ---------------------------------------------------------------------------
// Variable actions
// ---------------------------------------------------------------------------

/// Gives the design's project a design system and imports `tokens` into it.
fn import_tokens(f: &Fixture, client: &Client, design: &Design, tokens: Value) {
    must(
        call(
            client,
            "POST",
            &format!("{}/docs", design.base(f)),
            &holder(f),
            Some(&json!({ "kind": "design-system", "pillar": "ui", "title": unique("system") })),
        ),
        201,
        "create the design system doc",
    );
    must(
        call(
            client,
            "PUT",
            &format!("{}/tokens", design.base(f)),
            &holder(f),
            Some(&json!({ "tokens": tokens })),
        ),
        200,
        "import the tokens",
    );
}

#[test]
fn a_variable_action_is_checked_against_the_tokens_of_the_project() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let total = design.id("Total");
    let set_count = interactions(json!({
        "type": "setVariable",
        "variable": "state.count",
        "expr": { "op": "add", "args": [{ "variable": "state.count" }, { "value": 1 }] }
    }));
    let before = board_tags(f, client, &design, &design.id("Cart"));

    // Without tokens there is no variable to set.
    let response = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([set(&total, 1, set_count.clone())]),
    );
    assert!(
        refusal(&response).contains("`state.count` is not a token"),
        "{}",
        response.json()
    );
    assert_eq!(board_tags(f, client, &design, &design.id("Cart")), before);

    import_tokens(
        f,
        client,
        &design,
        json!({
            "$extensions": { "omnistate": { "modes": ["light", "dark"] } },
            "color": {
                "$type": "color",
                "surface": {
                    "$value": "#ffffff",
                    "$extensions": { "omnistate": { "modes": { "light": "#ffffff", "dark": "#0f172a" } } }
                }
            },
            "state": { "count": { "$type": "number", "$value": 0 } }
        }),
    );
    applied(f, client, &design, json!([set(&total, 1, set_count)]));

    // An insert is checked like a set.
    let response = edit(
        f,
        client,
        &holder(f),
        &design,
        json!([insert(
            &design.id("Actions"),
            None,
            r#"<Text name="Theme" content="Mörkt" fontFamily="Inter" fontSize={14} interactions={[{actions: [{type: "setVariableMode", collection: "color", mode: "dusk"}]}]}/>"#
        )]),
    );
    assert!(
        refusal(&response).contains("`dusk` is not a mode of `color` (light, dark)"),
        "{}",
        response.json()
    );
}

// ---------------------------------------------------------------------------
// A board's modes (US-27)
// ---------------------------------------------------------------------------

#[test]
fn a_boards_modes_must_name_collections_of_the_design_system_and_their_modes() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let cart = design.id("Cart");
    let before = board_tags(f, client, &design, &cart);
    let write = |modes: Value| {
        edit(
            f,
            client,
            &holder(f),
            &design,
            json!([set(&cart, 1, json!({ "modes": modes }))]),
        )
    };

    // Without a design system there is no collection, and none to list.
    assert_eq!(
        refusal(&write(json!({ "color": "dark" }))),
        "`color` is not a collection of the design system"
    );

    import_tokens(
        f,
        client,
        &design,
        json!({
            "$extensions": { "omnistate": { "modes": ["light", "dark"] } },
            "color": {
                "$type": "color",
                "surface": {
                    "$value": "#ffffff",
                    "$extensions": { "omnistate": { "modes": { "light": "#ffffff", "dark": "#0f172a" } } }
                }
            },
            "density": {
                "$extensions": { "omnistate": { "modes": ["comfortable", "compact"] } },
                "space": {
                    "$type": "dimension",
                    "$value": "16px",
                    "$extensions": { "omnistate": { "modes": { "comfortable": "16px", "compact": "12px" } } }
                }
            }
        }),
    );
    assert_eq!(
        refusal(&write(json!({ "color": "dusk" }))),
        "`dusk` is not a mode of `color` (light, dark)"
    );
    assert_eq!(
        refusal(&write(json!({ "nope": "x" }))),
        "`nope` is not a collection of the design system (color, density)"
    );
    assert_eq!(board_tags(f, client, &design, &cart), before);

    applied(
        f,
        client,
        &design,
        json!([set(&cart, 1, json!({ "modes": { "color": "dark" } }))]),
    );
    assert!(
        board_tags(f, client, &design, &cart).contains(r#"modes={{color: "dark"}}"#),
        "{}",
        board_tags(f, client, &design, &cart)
    );
}

// ---------------------------------------------------------------------------
// Token references (US-26)
// ---------------------------------------------------------------------------

/// A token tree with one light mode, the color group `colors` and the
/// string token `state.cta`.
fn cta_tokens(colors: Value) -> Value {
    json!({
        "$extensions": { "omnistate": { "modes": ["light"] } },
        "color": colors,
        "state": { "cta": { "$type": "string", "$value": "Pay now" } }
    })
}

#[test]
fn every_token_reference_of_a_written_node_is_checked_against_the_design_system() {
    let (f, client) = ready();
    let design = new_design(f, client);
    let (cart, summary, total) = (design.id("Cart"), design.id("Summary"), design.id("Total"));
    let before = board_tags(f, client, &design, &cart);
    let write = |ops: Value| edit(f, client, &holder(f), &design, ops);

    // Without a design system every reference is unknown.
    let response = write(json!([set(&summary, 1, json!({ "fill": "$color.ink" }))]));
    assert_eq!(
        refusal(&response),
        "the project has no design system: import tokens first"
    );
    assert_eq!(response.json()["index"], 0);

    import_tokens(
        f,
        client,
        &design,
        cta_tokens(json!({
            "$type": "color",
            "ink": { "$value": "#0f172a" },
            "gone": { "$value": "#ff0000" }
        })),
    );
    let response = write(json!([
        set(&total, 1, json!({ "content": "$state.cta" })),
        set(&summary, 1, json!({ "fill": "$color.nope" })),
    ]));
    assert_eq!(
        refusal(&response),
        "`$color.nope` is not a token of the design system"
    );
    assert_eq!(response.json()["index"], 1);
    assert_eq!(board_tags(f, client, &design, &cart), before);

    applied(
        f,
        client,
        &design,
        json!([
            set(&total, 1, json!({ "content": "$state.cta" })),
            set(&summary, 1, json!({ "fill": "$color.gone" })),
        ]),
    );
    let tags = board_tags(f, client, &design, &cart);
    assert!(tags.contains(r#"content="$state.cta""#), "{tags}");

    // The design system drops a token the design still refers to.
    must(
        call(
            client,
            "PUT",
            &format!("{}/tokens", design.base(f)),
            &holder(f),
            Some(&json!({
                "tokens": cta_tokens(json!({ "$type": "color", "ink": { "$value": "#0f172a" } }))
            })),
        ),
        200,
        "import the tokens without color.gone",
    );
    assert_eq!(
        refusal(&write(json!([set(&summary, 2, json!({ "name": "Sum" }))]))),
        "`$color.gone` is not a token of the design system"
    );
    applied(
        f,
        client,
        &design,
        json!([set(&summary, 2, json!({ "fill": "$color.ink" }))]),
    );
}
