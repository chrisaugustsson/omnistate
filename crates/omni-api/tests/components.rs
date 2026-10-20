//! US-029: components and instances through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; docs and drafts are those
//! of US-022, versions of US-024, approvals of US-025 and the design system
//! doc and its tokens of US-028; the tags are those of
//! docs/formats/design-tags.md, checked by `omni_core::design::tags`):
//!
//! - `PUT /changesets/{changeset_id}/components` (agent) takes `{"tags":
//!   "<Component ...>...</Component> ..."}`, zero or more `<Component>` tags,
//!   and replaces every component of the change set's draft of the project's
//!   design system doc in one all-or-nothing batch; the token tree stays as
//!   it is. A tag with an `id` names a node that the draft already has, which
//!   keeps its ID; a tag without one is new and gets its ID from the server.
//!   A change set without a draft of the design system gets one, as for a
//!   token import. The answer is 200 `{"doc_id", "changeset_id", "draft_id",
//!   "tags", "components"}`: the components as tags with every ID, and as
//!   JSON nodes of type `component`, in the order written. Tags that do not
//!   parse, or a nested instance or slot default that names no component of
//!   the import, are 400 `{"error": "invalid_components", "message"}` (the
//!   message names the problem) and change nothing. A body without `tags` is
//!   400 `invalid_request`, a project without a design system 404
//!   `no_design_system`, a session 403 `forbidden`. An import is logged in
//!   the draft's log and emits one `components.imported` event with `data`
//!   `{"doc_id", "changeset_id", "draft_id"}`.
//! - `PUT /changesets/{changeset_id}/docs/{doc_id}/design` (agent) takes
//!   `{"tags": "<Page ...>...</Page> ..."}`, one or more `<Page>` tags, and
//!   replaces the content of the change set's draft of the design doc,
//!   assigning IDs to new nodes. The answer is 200 `{"doc_id",
//!   "changeset_id", "draft_id", "tags", "pages"}`. Every instance is checked
//!   against the components that the draft resolves to (below): a component
//!   that is not there, a property that the component does not define, a
//!   value outside a variant's values, or an override of a node that is not
//!   in the component (or of `content` on a node that is no text) is 400
//!   `{"error": "invalid_design", "message"}` naming the component, the
//!   property, the value or the node, and so is a `<Component>` tag. The doc
//!   of another kind is 400 `not_a_design`. A refused import changes
//!   nothing. An import emits one `design.imported` event with `data`
//!   `{"doc_id", "changeset_id", "draft_id"}`.
//! - A draft resolves instances against the change set's draft of the design
//!   system, unsaved changes included; a change set without one resolves
//!   against the mainline design system. `GET
//!   /changesets/{changeset_id}/docs/{doc_id}` of a design doc's draft adds
//!   to the draft `"tags"`, `"pages"`, `"design_system": {"doc_id",
//!   "source": "draft" | "mainline", "draft_id", "version_id"}` (the draft's
//!   ID for `draft`, the mainline version's for `mainline`) and
//!   `"components"`, the JSON of the components its instances use, as
//!   resolved.
//! - Saving a version of a design doc (`POST
//!   /changesets/{changeset_id}/docs/{doc_id}/versions`) records the design
//!   system version it uses, as `"design_system_version_id"` of the version:
//!   the change set's design system draft's, which is saved first when it has
//!   changes that no version holds yet (so the change set's approval holds
//!   that version too), or the mainline one without a draft. `GET
//!   /versions/{version_id}` answers `design_system_version_id` (`null` for
//!   other docs) and, for a design version, `"tags"`, `"pages"` and
//!   `"components"` resolved from the recorded version, whatever the
//!   mainline is now. A comparison (`GET /versions/{a}/diff/{b}`) names the
//!   design system version of each side, `from_design_system_version_id` and
//!   `to_design_system_version_id`.
//! - `GET /components/usage?component=<id>` (both) lists every board of the
//!   mainline versions of the project's design docs that uses the component:
//!   through an instance anywhere in the board, a slot value of an instance,
//!   or an instance of another component whose tree uses it. The answer is
//!   200 `{"project_id", "component", "boards": [{"doc_id", "doc_title",
//!   "version_id", "page_id", "page_name", "board_id", "board_name",
//!   "components"}]}`, where `components` lists every component the board
//!   uses. Without `component`, every board that uses a component is listed
//!   and `component` is `null`. A component that the mainline design system
//!   does not define is 404 `{"error": "unknown_component"}`, and text that
//!   is no node ID 400 `invalid_request`. Drafts are not listed.
//! - The three routes are in the route registry: the imports as `agent`, the
//!   usage as `both`.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
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
const COMPONENTS_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/components";
const DESIGN_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}/design";
const USAGE_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/components/usage";
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
        // Versions and attachments keep their content in RustFS, so the API
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

fn changeset_path(f: &Fixture, project: &str, changeset: &str) -> String {
    format!("{}/changesets/{changeset}", project_base(f, project))
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

fn new_changeset(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/changesets", project_base(f, project)),
                &agent(f),
                Some(&json!({ "title": unique("change"), "run_tag": "run-29" })),
            ),
            201,
            "create a change set",
        )["changeset"],
    )
}

fn create_doc(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    kind: &str,
    module: &str,
    title: &str,
) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", changeset_path(f, project, changeset)),
                &agent(f),
                Some(&json!({ "kind": kind, "module": module, "title": title })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

fn open_draft(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) {
    let response = call(
        client,
        "POST",
        &format!("{}/docs/{doc}/open", changeset_path(f, project, changeset)),
        &agent(f),
        None,
    );
    assert!(
        response.status == 200 || response.status == 201,
        "open a draft of the doc: {}",
        response.text()
    );
}

/// A small token tree, which a design system version needs.
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

fn import_tokens(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    must(
        call(
            client,
            "PUT",
            &format!("{}/tokens", changeset_path(f, project, changeset)),
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
    changeset: &str,
    tags: &str,
) -> Response {
    call(
        client,
        "PUT",
        &format!("{}/components", changeset_path(f, project, changeset)),
        auth,
        Some(&json!({ "tags": tags })),
    )
}

fn import_components(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    tags: &str,
) -> Value {
    must(
        components_response(f, client, &agent(f), project, changeset, tags),
        200,
        "import the components",
    )
}

fn design_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    doc: &str,
    tags: &str,
) -> Response {
    call(
        client,
        "PUT",
        &format!(
            "{}/docs/{doc}/design",
            changeset_path(f, project, changeset)
        ),
        auth,
        Some(&json!({ "tags": tags })),
    )
}

fn import_design(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    tags: &str,
) -> Value {
    must(
        design_response(f, client, &agent(f), project, changeset, doc, tags),
        200,
        "import the design",
    )
}

fn read_draft(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/docs/{doc}", changeset_path(f, project, changeset)),
            &agent(f),
            None,
        ),
        200,
        "read the draft",
    )["draft"]
        .clone()
}

/// Saves a version of the change set's draft of `doc`; returns the version.
fn save(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "POST",
            &format!(
                "{}/docs/{doc}/versions",
                changeset_path(f, project, changeset)
            ),
            &agent(f),
            Some(&json!({})),
        ),
        201,
        "save a version",
    )["version"]
        .clone()
}

fn read_version(f: &Fixture, client: &Client, project: &str, version: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/versions/{version}", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "read a version",
    )["version"]
        .clone()
}

/// Submits the change set and has the human approve it; returns the change
/// set as the approval left it.
fn submit_and_approve(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    let path = changeset_path(f, project, changeset);
    must(
        call(client, "POST", &format!("{path}/submit"), &agent(f), None),
        200,
        "submit the change set",
    );
    must(
        call(
            client,
            "POST",
            &format!("{path}/approve"),
            &human(f),
            Some(&json!({})),
        ),
        200,
        "the human approves",
    )["changeset"]
        .clone()
}

/// Submits the change set and has the human approve it, which merges it.
fn merge(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    let approved = submit_and_approve(f, client, project, changeset);
    assert_eq!(approved["status"], "merged", "{approved}");
    approved
}

fn mainline_of(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    must(
        call(
            client,
            "GET",
            &format!("{}/docs/{doc}", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "read the doc",
    )["doc"]["mainline_version_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the doc {doc} has a mainline version"))
        .to_string()
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

fn logged_ops(draft: &str) -> i64 {
    count(
        "select count(*) from draft_ops where draft_id = $1::uuid",
        &[draft],
    )
}

fn versions_of(doc: &str, changeset: &str) -> i64 {
    count(
        "select count(*) from versions where doc_id = $1::uuid and changeset_id = $2::uuid",
        &[doc, changeset],
    )
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

/// ` id="<id>"` for a node the draft already has, nothing for a new one.
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

/// The shop: a checkout board with the pay button, a product board with a
/// card, and a board without instances.
fn shop_tags(ids: &Ids, instance: &str) -> String {
    format!(
        r##"<Page name="Home">
  <Board name="Checkout" x={{0}} y={{0}} width={{375}} height={{812}} dir="col" gap={{16}}>
    <Frame name="Footer" dir="row" gap={{8}}>
      {instance}
    </Frame>
  </Board>
  <Board name="Product" x={{400}} y={{0}} width={{375}} height={{812}} dir="col">
    <Instance name="Product card" component="{}"/>
  </Board>
  <Board name="Empty" x={{800}} y={{0}} width={{375}} height={{812}} dir="col">
    <Text name="Nothing" content="Tomt" fontFamily="Inter" fontSize={{14}}/>
  </Board>
</Page>
"##,
        id(ids, "Card"),
    )
}

/// The admin: one board with a plain button.
fn admin_tags(ids: &Ids) -> String {
    format!(
        r#"<Page name="Admin">
  <Board name="Settings" x={{0}} y={{0}} width={{1280}} height={{800}} dir="col">
    <Instance name="Save" component="{}"/>
  </Board>
</Page>
"#,
        id(ids, "Button"),
    )
}

const STYLES: [&str; 2] = ["primary", "secondary"];
const WITH_DANGER: [&str; 3] = ["primary", "secondary", "danger"];

struct Library {
    project: String,
    design_system: String,
    shop: String,
    shop_title: String,
    admin: String,
    /// Component and layer IDs by name.
    ids: Ids,
    /// Board, page and instance IDs by name.
    nodes: Ids,
    /// The answer of the shop's design import.
    shop_import: Value,
    shop_version: String,
    admin_version: String,
    /// The design system version that both design versions record.
    design_system_version: String,
}

/// A project whose merged change set made the design system (tokens and the
/// five components) and two design docs that use them.
fn library(f: &Fixture, client: &Client) -> Library {
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let design_system = create_doc(
        f,
        client,
        &project,
        &changeset,
        "design-system",
        "ui",
        "Design system",
    );
    import_tokens(f, client, &project, &changeset);
    // New components get their IDs from the server, so a component that
    // places another is written once the other has its ID.
    let icons = import_components(f, client, &project, &changeset, &icon_tags(&Ids::new()));
    let mut ids = ids_in(&icons["components"]);
    let with_button = import_components(
        f,
        client,
        &project,
        &changeset,
        &format!("{}{}", icon_tags(&ids), button_tags(&ids, &STYLES, "Buy")),
    );
    ids = ids_in(&with_button["components"]);
    let all = import_components(
        f,
        client,
        &project,
        &changeset,
        &library_tags(&ids, &STYLES, "Buy"),
    );
    ids = ids_in(&all["components"]);
    for name in ["Cart", "Heart", "Button", "Card", "Tag"] {
        assert!(ids.contains_key(name), "the import holds {name}: {all}");
    }

    let shop_title = unique("Shop");
    let shop = create_doc(f, client, &project, &changeset, "design", "ui", &shop_title);
    let shop_import = import_design(
        f,
        client,
        &project,
        &changeset,
        &shop,
        &shop_tags(&ids, &pay_button(&ids, "secondary")),
    );
    let admin = create_doc(f, client, &project, &changeset, "design", "ui", "Admin");
    let admin_import = import_design(f, client, &project, &changeset, &admin, &admin_tags(&ids));
    let mut nodes = ids_in(&shop_import["pages"]);
    nodes.extend(ids_in(&admin_import["pages"]));

    let shop_saved = save(f, client, &project, &changeset, &shop);
    let admin_saved = save(f, client, &project, &changeset, &admin);
    let design_system_version = shop_saved["design_system_version_id"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("a design version records its design system version: {shop_saved}")
        })
        .to_string();
    merge(f, client, &project, &changeset);
    Library {
        project,
        design_system,
        shop,
        shop_title,
        admin,
        ids,
        nodes,
        shop_import,
        shop_version: id_of(&shop_saved),
        admin_version: id_of(&admin_saved),
        design_system_version,
    }
}

/// The definition of `name` among the components of an answer.
fn component_in(answer: &Value, name: &str) -> Value {
    answer["components"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer lists components: {answer}"))
        .iter()
        .find(|component| component["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("the answer holds the component {name}: {answer:#}"))
}

fn style_values(component: &Value) -> Value {
    component["props"]["style"]["values"].clone()
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_component_routes_are_in_the_registry() {
    let routes = registry();
    for (method, path, access) in [
        ("PUT", COMPONENTS_ROUTE, Access::Agent),
        ("PUT", DESIGN_ROUTE, Access::Agent),
        ("GET", USAGE_ROUTE, Access::Both),
    ] {
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
    }
}

// ---------------------------------------------------------------------------
// <Component> tags in the design system doc
// ---------------------------------------------------------------------------

#[test]
fn an_import_defines_the_components_of_the_change_sets_design_system() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let design_system = create_doc(
        f,
        client,
        &project,
        &changeset,
        "design-system",
        "ui",
        "Design system",
    );
    import_tokens(f, client, &project, &changeset);
    let icons = import_components(f, client, &project, &changeset, &icon_tags(&Ids::new()));
    assert_eq!(icons["doc_id"], design_system.as_str(), "{icons}");
    assert_eq!(icons["changeset_id"], changeset.as_str(), "{icons}");
    let ids = ids_in(&icons["components"]);
    for name in ["Cart", "Cart glyph", "Heart", "Heart glyph"] {
        let given = id(&ids, name);
        assert!(
            given.len() == 8 && given.chars().all(|c| c.is_ascii_alphanumeric()),
            "the server gives {name} a node ID: {given}"
        );
    }
    let names: Vec<&str> = icons["components"]
        .as_array()
        .expect("the components")
        .iter()
        .map(|component| {
            assert_eq!(component["type"], "component", "{component}");
            component["name"].as_str().unwrap()
        })
        .collect();
    assert_eq!(
        names,
        ["Cart", "Heart"],
        "the components in the order written"
    );
    assert!(
        icons["tags"]
            .as_str()
            .unwrap_or_default()
            .contains(&id(&ids, "Cart")),
        "the tags carry the new IDs: {icons}"
    );

    // A second import replaces the components: those written with an ID keep
    // it, and the button's properties are stored as written.
    let with_button = import_components(
        f,
        client,
        &project,
        &changeset,
        &format!("{}{}", icon_tags(&ids), button_tags(&ids, &STYLES, "Buy")),
    );
    let again = ids_in(&with_button["components"]);
    for name in ["Cart", "Cart glyph", "Heart", "Heart glyph"] {
        assert_eq!(again[name], ids[name], "{name} keeps its ID");
    }
    let button = component_in(&with_button, "Button");
    assert_eq!(
        button["props"],
        json!({
            "style": {"type": "variant", "values": ["primary", "secondary"], "default": "primary"},
            "showIcon": {"type": "boolean", "default": true},
            "label": {"type": "text", "default": "Buy"},
            "icon": {"type": "slot", "default": ids["Cart"]}
        }),
        "{button:#}"
    );
    assert_eq!(
        find(&button["children"], "Icon")["bind"],
        json!({"visible": "showIcon", "component": "icon"})
    );
    assert_eq!(
        find(&button["children"], "Label")["bind"],
        json!({"content": "label"})
    );

    // Only the import with the button is left: the icons-only one is gone.
    let only_button = import_components(
        f,
        client,
        &project,
        &changeset,
        &format!(
            "{}{}",
            icon_tags(&again),
            button_tags(&again, &STYLES, "Buy")
        ),
    );
    assert_eq!(only_button["components"].as_array().unwrap().len(), 3);

    // The token tree stays as it was.
    let read = must(
        call(
            client,
            "GET",
            &format!("{}/tokens", changeset_path(f, &project, &changeset)),
            &agent(f),
            None,
        ),
        200,
        "read the draft's tokens",
    );
    assert_eq!(read["tokens"], tokens(), "{read}");
}

#[test]
fn a_components_import_is_logged_and_emits_an_event() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    create_doc(f, client, &project, &changeset, "design-system", "ui", "DS");
    let before = events_of(&project, "components.imported").len();
    let imported = import_components(f, client, &project, &changeset, &icon_tags(&Ids::new()));
    let draft = imported["draft_id"]
        .as_str()
        .expect("the draft")
        .to_string();
    let logged = logged_ops(&draft);
    assert!(logged >= 1, "the import is in the draft's log");
    let events = events_of(&project, "components.imported");
    assert_eq!(events.len(), before + 1, "{events:?}");
    let data = events.last().unwrap();
    assert_eq!(data["doc_id"], imported["doc_id"], "{data}");
    assert_eq!(data["changeset_id"], changeset.as_str(), "{data}");
    assert_eq!(data["draft_id"], draft.as_str(), "{data}");
}

#[test]
fn a_refused_components_import_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    create_doc(f, client, &project, &changeset, "design-system", "ui", "DS");
    let first = import_components(f, client, &project, &changeset, &icon_tags(&Ids::new()));
    let draft = first["draft_id"].as_str().unwrap().to_string();
    let ids = ids_in(&first["components"]);
    let ops = logged_ops(&draft);
    let events = events_of(&project, "components.imported").len();

    let other_kind = icon_tags(&ids).replace(
        r#"name="Cart" width"#,
        r#"name="Cart" props={{size: {type: "number", default: 1}}} width"#,
    );
    assert_refused(
        &components_response(f, client, &agent(f), &project, &changeset, &other_kind),
        "invalid_components",
        &["number"],
    );
    let unknown_slot = format!("{}{}", icon_tags(&ids), button_tags(&ids, &STYLES, "Buy")).replace(
        &format!(r#"default: "{}""#, ids["Cart"]),
        &format!(r#"default: "{UNKNOWN_NODE}""#),
    );
    assert_refused(
        &components_response(f, client, &agent(f), &project, &changeset, &unknown_slot),
        "invalid_components",
        &[UNKNOWN_NODE],
    );
    let unknown_nested = format!("{}{}", icon_tags(&ids), button_tags(&ids, &STYLES, "Buy"))
        .replace(
            &format!(r#"name="Icon" component="{}""#, ids["Cart"]),
            &format!(r#"name="Icon" component="{UNKNOWN_NODE}""#),
        );
    assert_refused(
        &components_response(f, client, &agent(f), &project, &changeset, &unknown_nested),
        "invalid_components",
        &[UNKNOWN_NODE],
    );
    assert_refused(
        &components_response(
            f,
            client,
            &agent(f),
            &project,
            &changeset,
            "<Component name=",
        ),
        "invalid_components",
        &["line 1"],
    );
    assert_eq!(logged_ops(&draft), ops, "a refused import logs nothing");
    assert_eq!(
        events_of(&project, "components.imported").len(),
        events,
        "a refused import emits nothing"
    );
    let unchanged = import_components(f, client, &project, &changeset, &icon_tags(&ids));
    assert_eq!(ids_in(&unchanged["components"]), ids);

    // Who may import, and what an import needs.
    let response =
        components_response(f, client, &human(f), &project, &changeset, &icon_tags(&ids));
    assert_error(&response, 403, "forbidden");
    let response = call(
        client,
        "PUT",
        &format!("{}/components", changeset_path(f, &project, &changeset)),
        &agent(f),
        Some(&json!({})),
    );
    assert_error(&response, 400, "invalid_request");
    let bare = new_project(f, client);
    let bare_changeset = new_changeset(f, client, &bare);
    let response = components_response(
        f,
        client,
        &agent(f),
        &bare,
        &bare_changeset,
        &icon_tags(&Ids::new()),
    );
    assert_error(&response, 404, "no_design_system");
}

#[test]
fn a_components_import_opens_the_design_system_draft_of_a_change_set_that_has_none() {
    let (f, client) = ready();
    let lib = library(f, client);
    let changeset = new_changeset(f, client, &lib.project);
    let imported = import_components(
        f,
        client,
        &lib.project,
        &changeset,
        &library_tags(&lib.ids, &WITH_DANGER, "Buy"),
    );
    assert_eq!(imported["doc_id"], lib.design_system.as_str(), "{imported}");
    let draft = imported["draft_id"].as_str().expect("a new draft");
    let read = read_draft(f, client, &lib.project, &changeset, &lib.design_system);
    assert_eq!(read["id"], draft, "{read}");
    assert_eq!(
        read["base_version_id"],
        lib.design_system_version.as_str(),
        "the draft is based on the mainline design system: {read}"
    );
    assert_eq!(
        style_values(&component_in(&imported, "Button")),
        json!(WITH_DANGER)
    );
}

// ---------------------------------------------------------------------------
// <Instance component props overrides> in design docs
// ---------------------------------------------------------------------------

#[test]
fn an_instance_stores_only_its_component_its_property_values_and_its_overrides() {
    let (f, client) = ready();
    let lib = library(f, client);
    let ids = &lib.ids;
    let instance = find(&lib.shop_import["pages"], "Pay button");
    assert_eq!(instance["type"], "instance", "{instance}");
    assert_eq!(instance["component"], ids["Button"].as_str());
    assert_eq!(
        instance["props"],
        json!({"style": "secondary", "showIcon": false, "label": "Pay", "icon": ids["Heart"]}),
        "{instance:#}"
    );
    let mut overrides = serde_json::Map::new();
    overrides.insert(ids["Label"].clone(), json!({"content": "Pay now"}));
    overrides.insert(
        ids["Badge"].clone(),
        json!({"fills": [{"type": "solid", "color": "#16a34a"}]}),
    );
    overrides.insert(ids["Icon"].clone(), json!({"visible": false}));
    assert_eq!(
        instance["overrides"],
        Value::Object(overrides),
        "{instance:#}"
    );
    let keys: BTreeSet<&str> = instance
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "id",
            "type",
            "name",
            "visible",
            "component",
            "props",
            "overrides"
        ]
        .into_iter()
        .collect(),
        "nothing of the component's tree or style is stored in the instance: {instance:#}"
    );
    assert!(
        node_named(&lib.shop_import["pages"], "Label").is_none(),
        "the component's nodes are not copied into the design"
    );
    let tags = lib.shop_import["tags"]
        .as_str()
        .expect("the design as tags");
    for attribute in ["props=", "overrides="] {
        assert!(tags.contains(attribute), "{attribute}: {tags}");
    }

    // The saved version stores the same instance.
    let version = read_version(f, client, &lib.project, &lib.shop_version);
    assert_eq!(
        find(&version["pages"], "Pay button"),
        instance,
        "{version:#}"
    );
}

#[test]
fn an_instance_may_swap_the_contents_of_a_slot_with_an_override() {
    let (f, client) = ready();
    let lib = library(f, client);
    let ids = &lib.ids;
    let changeset = new_changeset(f, client, &lib.project);
    open_draft(f, client, &lib.project, &changeset, &lib.shop);
    let swapped = format!(
        r#"<Instance name="Pay button" component="{}" overrides={{{{"{}": {{component: "{}"}}}}}}/>"#,
        ids["Button"], ids["Icon"], ids["Heart"]
    );
    let imported = import_design(
        f,
        client,
        &lib.project,
        &changeset,
        &lib.shop,
        &shop_tags(ids, &swapped),
    );
    let instance = find(&imported["pages"], "Pay button");
    assert_eq!(
        instance["overrides"][ids["Icon"].as_str()],
        json!({"component": ids["Heart"]}),
        "{instance:#}"
    );
}

#[test]
fn a_design_import_is_logged_and_emits_an_event() {
    let (f, client) = ready();
    let lib = library(f, client);
    let changeset = new_changeset(f, client, &lib.project);
    open_draft(f, client, &lib.project, &changeset, &lib.shop);
    let before = events_of(&lib.project, "design.imported").len();
    let imported = import_design(
        f,
        client,
        &lib.project,
        &changeset,
        &lib.shop,
        &shop_tags(&lib.ids, &pay_button(&lib.ids, "primary")),
    );
    assert_eq!(imported["doc_id"], lib.shop.as_str());
    assert_eq!(imported["changeset_id"], changeset.as_str());
    let events = events_of(&lib.project, "design.imported");
    assert_eq!(events.len(), before + 1, "{events:?}");
    let data = events.last().unwrap();
    assert_eq!(data["doc_id"], lib.shop.as_str(), "{data}");
    assert_eq!(data["changeset_id"], changeset.as_str(), "{data}");
    assert_eq!(data["draft_id"], imported["draft_id"], "{data}");
}

#[test]
fn a_design_import_refuses_instances_that_do_not_fit_their_component() {
    let (f, client) = ready();
    let lib = library(f, client);
    let ids = &lib.ids;
    let changeset = new_changeset(f, client, &lib.project);
    open_draft(f, client, &lib.project, &changeset, &lib.shop);
    let fine = import_design(
        f,
        client,
        &lib.project,
        &changeset,
        &lib.shop,
        &shop_tags(ids, &pay_button(ids, "primary")),
    );
    let draft = fine["draft_id"].as_str().unwrap().to_string();
    let before = read_draft(f, client, &lib.project, &changeset, &lib.shop);
    let ops = logged_ops(&draft);
    let events = events_of(&lib.project, "design.imported").len();
    let refuse = |tags: String, needles: &[&str]| {
        assert_refused(
            &design_response(
                f,
                client,
                &agent(f),
                &lib.project,
                &changeset,
                &lib.shop,
                &tags,
            ),
            "invalid_design",
            needles,
        );
    };

    let unknown_component = format!(r#"<Instance name="Pay button" component="{UNKNOWN_NODE}"/>"#);
    refuse(shop_tags(ids, &unknown_component), &[UNKNOWN_NODE]);
    let unknown_prop = format!(
        r#"<Instance name="Pay button" component="{}" props={{{{colour: "red"}}}}/>"#,
        ids["Button"]
    );
    refuse(shop_tags(ids, &unknown_prop), &["colour"]);
    refuse(
        shop_tags(ids, &pay_button(ids, "ghost")),
        &["ghost", "primary"],
    );
    let boolean_as_text = format!(
        r#"<Instance name="Pay button" component="{}" props={{{{showIcon: "no"}}}}/>"#,
        ids["Button"]
    );
    refuse(shop_tags(ids, &boolean_as_text), &["showIcon"]);
    let slot_of_nothing = format!(
        r#"<Instance name="Pay button" component="{}" props={{{{icon: "{UNKNOWN_NODE}"}}}}/>"#,
        ids["Button"]
    );
    refuse(shop_tags(ids, &slot_of_nothing), &[UNKNOWN_NODE]);
    let override_of_nothing = format!(
        r#"<Instance name="Pay button" component="{}" overrides={{{{"{UNKNOWN_NODE}": {{visible: false}}}}}}/>"#,
        ids["Button"]
    );
    refuse(shop_tags(ids, &override_of_nothing), &[UNKNOWN_NODE]);
    let text_on_a_frame = format!(
        r#"<Instance name="Pay button" component="{}" overrides={{{{"{}": {{content: "Hej"}}}}}}/>"#,
        ids["Button"], ids["Badge"]
    );
    refuse(shop_tags(ids, &text_on_a_frame), &["content"]);
    refuse(
        r##"<Component name="Inline" fill="#ffffff"/>"##.to_string(),
        &["Component"],
    );
    refuse("<Page name=".to_string(), &["line 1"]);

    let after = read_draft(f, client, &lib.project, &changeset, &lib.shop);
    assert_eq!(
        after["tags"], before["tags"],
        "a refused import changes nothing"
    );
    assert_eq!(logged_ops(&draft), ops, "a refused import logs nothing");
    assert_eq!(
        events_of(&lib.project, "design.imported").len(),
        events,
        "a refused import emits nothing"
    );

    // Who may import, and into what.
    let tags = shop_tags(ids, &pay_button(ids, "primary"));
    assert_error(
        &design_response(
            f,
            client,
            &human(f),
            &lib.project,
            &changeset,
            &lib.shop,
            &tags,
        ),
        403,
        "forbidden",
    );
    let html = create_doc(f, client, &lib.project, &changeset, "html", "prd", "PRD");
    assert_error(
        &design_response(f, client, &agent(f), &lib.project, &changeset, &html, &tags),
        400,
        "not_a_design",
    );
}

// ---------------------------------------------------------------------------
// Resolving instances: the change set's design system draft, else the mainline
// ---------------------------------------------------------------------------

#[test]
fn a_draft_resolves_instances_against_the_change_sets_design_system_draft() {
    let (f, client) = ready();
    let lib = library(f, client);
    let ids = &lib.ids;
    let changeset = new_changeset(f, client, &lib.project);
    // The change set's design system draft gives the button a `danger`
    // style, which no version holds yet.
    let imported = import_components(
        f,
        client,
        &lib.project,
        &changeset,
        &library_tags(ids, &WITH_DANGER, "Buy"),
    );
    let design_system_draft = imported["draft_id"].as_str().unwrap().to_string();
    open_draft(f, client, &lib.project, &changeset, &lib.shop);
    import_design(
        f,
        client,
        &lib.project,
        &changeset,
        &lib.shop,
        &shop_tags(ids, &pay_button(ids, "danger")),
    );
    let draft = read_draft(f, client, &lib.project, &changeset, &lib.shop);
    assert_eq!(
        draft["design_system"]["source"], "draft",
        "{:#}",
        draft["design_system"]
    );
    assert_eq!(
        draft["design_system"]["draft_id"],
        design_system_draft.as_str()
    );
    assert_eq!(draft["design_system"]["doc_id"], lib.design_system.as_str());
    assert_eq!(
        style_values(&component_in(&draft, "Button")),
        json!(WITH_DANGER),
        "the draft's instances resolve to the change set's draft of the button"
    );
    assert!(
        draft["tags"]
            .as_str()
            .unwrap_or_default()
            .contains("danger"),
        "{draft:#}"
    );
}

#[test]
fn a_change_set_without_a_design_system_draft_resolves_against_the_mainline() {
    let (f, client) = ready();
    let lib = library(f, client);
    let ids = &lib.ids;
    // Another change set has a `danger` style in its own draft only.
    let other = new_changeset(f, client, &lib.project);
    import_components(
        f,
        client,
        &lib.project,
        &other,
        &library_tags(ids, &WITH_DANGER, "Buy"),
    );

    let changeset = new_changeset(f, client, &lib.project);
    open_draft(f, client, &lib.project, &changeset, &lib.shop);
    assert_refused(
        &design_response(
            f,
            client,
            &agent(f),
            &lib.project,
            &changeset,
            &lib.shop,
            &shop_tags(ids, &pay_button(ids, "danger")),
        ),
        "invalid_design",
        &["danger"],
    );
    import_design(
        f,
        client,
        &lib.project,
        &changeset,
        &lib.shop,
        &shop_tags(ids, &pay_button(ids, "primary")),
    );
    let draft = read_draft(f, client, &lib.project, &changeset, &lib.shop);
    assert_eq!(
        draft["design_system"]["source"], "mainline",
        "{:#}",
        draft["design_system"]
    );
    assert_eq!(
        draft["design_system"]["version_id"],
        lib.design_system_version.as_str()
    );
    assert_eq!(draft["design_system"]["draft_id"], Value::Null);
    assert_eq!(
        style_values(&component_in(&draft, "Button")),
        json!(STYLES),
        "the mainline button has no danger style"
    );
}

// ---------------------------------------------------------------------------
// Saving a design version records the design system version
// ---------------------------------------------------------------------------

#[test]
fn saving_a_design_version_saves_the_design_system_draft_first_and_records_it() {
    let (f, client) = ready();
    let lib = library(f, client);
    let ids = &lib.ids;
    let changeset = new_changeset(f, client, &lib.project);
    import_components(
        f,
        client,
        &lib.project,
        &changeset,
        &library_tags(ids, &WITH_DANGER, "Buy"),
    );
    open_draft(f, client, &lib.project, &changeset, &lib.shop);
    import_design(
        f,
        client,
        &lib.project,
        &changeset,
        &lib.shop,
        &shop_tags(ids, &pay_button(ids, "danger")),
    );
    assert_eq!(versions_of(&lib.design_system, &changeset), 0);

    let first = save(f, client, &lib.project, &changeset, &lib.shop);
    let recorded = first["design_system_version_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the version records its design system version: {first}"))
        .to_string();
    assert_ne!(
        recorded, lib.design_system_version,
        "the draft's unsaved changes are in a new version, not the mainline one"
    );
    let design_system_version = read_version(f, client, &lib.project, &recorded);
    assert_eq!(design_system_version["doc_id"], lib.design_system.as_str());
    assert_eq!(design_system_version["changeset_id"], changeset.as_str());
    assert_eq!(
        versions_of(&lib.design_system, &changeset),
        1,
        "the design system draft was saved first"
    );
    let viewed = read_version(f, client, &lib.project, &id_of(&first));
    assert_eq!(viewed["design_system_version_id"], recorded.as_str());
    assert_eq!(
        style_values(&component_in(&viewed, "Button")),
        json!(WITH_DANGER)
    );

    // Without changes since, the saved design system version is recorded
    // again and nothing more is saved.
    let second = save(f, client, &lib.project, &changeset, &lib.shop);
    assert_eq!(second["design_system_version_id"], recorded.as_str());
    assert_eq!(versions_of(&lib.design_system, &changeset), 1);

    // New changes to the design system are saved by the next design save.
    import_components(
        f,
        client,
        &lib.project,
        &changeset,
        &library_tags(ids, &WITH_DANGER, "Köp"),
    );
    let third = save(f, client, &lib.project, &changeset, &lib.shop);
    let newer = third["design_system_version_id"]
        .as_str()
        .expect("the third version records one too")
        .to_string();
    assert_ne!(newer, recorded, "the changed draft was saved again");
    assert_eq!(versions_of(&lib.design_system, &changeset), 2);

    // The approval holds that design system version, and it merges with the
    // design.
    let approved = merge(f, client, &lib.project, &changeset);
    let approved_versions = approved["approved_versions"]
        .as_array()
        .unwrap_or_else(|| panic!("the approval lists versions: {approved}"));
    assert!(
        approved_versions.iter().any(|v| v["doc_id"] == lib.design_system.as_str()
            && v["version_id"] == newer.as_str()),
        "{approved:#}"
    );
    assert_eq!(
        mainline_of(f, client, &lib.project, &lib.design_system),
        newer
    );
}

#[test]
fn saving_a_design_version_without_a_design_system_draft_records_the_mainline_one() {
    let (f, client) = ready();
    let lib = library(f, client);
    assert_eq!(
        read_version(f, client, &lib.project, &lib.admin_version)["design_system_version_id"],
        lib.design_system_version.as_str(),
        "every design version of a change set records the same saved version"
    );
    let changeset = new_changeset(f, client, &lib.project);
    open_draft(f, client, &lib.project, &changeset, &lib.admin);
    import_design(
        f,
        client,
        &lib.project,
        &changeset,
        &lib.admin,
        &admin_tags(&lib.ids),
    );
    let saved = save(f, client, &lib.project, &changeset, &lib.admin);
    assert_eq!(
        saved["design_system_version_id"],
        lib.design_system_version.as_str(),
        "{saved}"
    );
    assert_eq!(versions_of(&lib.design_system, &changeset), 0);
}

#[test]
fn a_design_system_of_components_without_tokens_is_saved_with_the_design_that_uses_it() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let design_system = create_doc(
        f,
        client,
        &project,
        &changeset,
        "design-system",
        "ui",
        "Design system",
    );
    // Components only: the change set imports no tokens.
    let icons = import_components(f, client, &project, &changeset, &icon_tags(&Ids::new()));
    let ids = ids_in(&icons["components"]);
    let design = create_doc(f, client, &project, &changeset, "design", "ui", "Icons");
    import_design(
        f,
        client,
        &project,
        &changeset,
        &design,
        &format!(
            r#"<Page name="Home">
  <Board name="Toolbar" x={{0}} y={{0}} width={{375}} height={{64}} dir="row" gap={{8}}>
    <Instance name="Cart button" component="{}"/>
  </Board>
</Page>
"#,
            id(&ids, "Cart")
        ),
    );
    assert_eq!(versions_of(&design_system, &changeset), 0);

    // The design save does not dead-end on the missing tokens: it saves the
    // design system draft first and records that version.
    let saved = save(f, client, &project, &changeset, &design);
    let recorded = saved["design_system_version_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the version records its design system version: {saved}"))
        .to_string();
    assert_eq!(
        versions_of(&design_system, &changeset),
        1,
        "the design system draft was saved first"
    );
    let design_system_version = read_version(f, client, &project, &recorded);
    assert_eq!(design_system_version["doc_id"], design_system.as_str());
    assert_eq!(design_system_version["changeset_id"], changeset.as_str());
    let viewed = read_version(f, client, &project, &id_of(&saved));
    assert_eq!(viewed["design_system_version_id"], recorded.as_str());
    assert_eq!(
        component_in(&viewed, "Cart")["id"],
        id(&ids, "Cart").as_str(),
        "the instance resolves against the recorded components: {viewed:#}"
    );

    // Both merge, and the design system's mainline is the recorded version.
    let approved = merge(f, client, &project, &changeset);
    let approved_versions = approved["approved_versions"]
        .as_array()
        .unwrap_or_else(|| panic!("the approval lists versions: {approved}"));
    assert!(
        approved_versions
            .iter()
            .any(|v| v["doc_id"] == design_system.as_str() && v["version_id"] == recorded.as_str()),
        "{approved:#}"
    );
    assert_eq!(mainline_of(f, client, &project, &design_system), recorded);
}

#[test]
fn an_empty_design_system_draft_does_not_stop_a_design_save() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    // The design system doc is new and its draft holds neither tokens nor
    // components; it has no mainline version either.
    let design_system = create_doc(
        f,
        client,
        &project,
        &changeset,
        "design-system",
        "ui",
        "Design system",
    );
    let design = create_doc(f, client, &project, &changeset, "design", "ui", "Blank");
    import_design(
        f,
        client,
        &project,
        &changeset,
        &design,
        r#"<Page name="Home">
  <Board name="Blank" x={0} y={0} width={375} height={64}>
    <Text name="Title" content="Hello" fontFamily="Inter" fontSize={14}/>
  </Board>
</Page>
"#,
    );

    // A design that uses no component saves, and there is no design system
    // version to record.
    let saved = save(f, client, &project, &changeset, &design);
    assert!(
        saved["design_system_version_id"].is_null(),
        "nothing to record: {saved}"
    );
    assert_eq!(
        versions_of(&design_system, &changeset),
        0,
        "the empty design system draft is not saved"
    );
    assert_eq!(
        read_version(f, client, &project, &id_of(&saved))["design_system_version_id"],
        Value::Null
    );
}

#[test]
fn viewers_and_comparisons_use_the_recorded_design_system_version() {
    let (f, client) = ready();
    let lib = library(f, client);
    let ids = &lib.ids;
    assert_eq!(
        mainline_of(f, client, &lib.project, &lib.shop),
        lib.shop_version
    );

    // A later change set gives the button a danger style and merges it.
    let restyle = new_changeset(f, client, &lib.project);
    import_components(
        f,
        client,
        &lib.project,
        &restyle,
        &library_tags(ids, &WITH_DANGER, "Köp"),
    );
    let newer = id_of(&save(f, client, &lib.project, &restyle, &lib.design_system));
    merge(f, client, &lib.project, &restyle);
    assert_eq!(
        mainline_of(f, client, &lib.project, &lib.design_system),
        newer
    );

    // The shop's first version still shows the button as it was.
    let first = read_version(f, client, &lib.project, &lib.shop_version);
    assert_eq!(
        first["design_system_version_id"],
        lib.design_system_version.as_str()
    );
    let button = component_in(&first, "Button");
    assert_eq!(style_values(&button), json!(STYLES), "{button:#}");
    assert_eq!(button["props"]["label"]["default"], "Buy", "{button:#}");
    assert!(
        first["tags"]
            .as_str()
            .unwrap_or_default()
            .contains("Pay button"),
        "{first:#}"
    );

    // A new version uses the danger style of the newer design system.
    let changeset = new_changeset(f, client, &lib.project);
    open_draft(f, client, &lib.project, &changeset, &lib.shop);
    import_design(
        f,
        client,
        &lib.project,
        &changeset,
        &lib.shop,
        &shop_tags(ids, &pay_button(ids, "danger")),
    );
    let second = save(f, client, &lib.project, &changeset, &lib.shop);
    assert_eq!(second["design_system_version_id"], newer.as_str());
    let viewed = read_version(f, client, &lib.project, &id_of(&second));
    assert_eq!(
        style_values(&component_in(&viewed, "Button")),
        json!(WITH_DANGER)
    );

    // Comparing the two names the design system version of each side.
    let diff = must(
        call(
            client,
            "GET",
            &format!(
                "{}/versions/{}/diff/{}",
                project_base(f, &lib.project),
                lib.shop_version,
                id_of(&second)
            ),
            &agent(f),
            None,
        ),
        200,
        "compare the two versions",
    )["diff"]
        .clone();
    assert_eq!(
        diff["from_design_system_version_id"],
        lib.design_system_version.as_str(),
        "{diff:#}"
    );
    assert_eq!(
        diff["to_design_system_version_id"],
        newer.as_str(),
        "{diff:#}"
    );

    // A human viewer sees the same.
    let seen = must(
        call(
            client,
            "GET",
            &format!(
                "{}/versions/{}",
                project_base(f, &lib.project),
                lib.shop_version
            ),
            &human(f),
            None,
        ),
        200,
        "the human reads the version",
    )["version"]
        .clone();
    assert_eq!(style_values(&component_in(&seen, "Button")), json!(STYLES));
}

#[test]
fn a_version_of_another_doc_records_no_design_system_version() {
    let (f, client) = ready();
    let lib = library(f, client);
    let version = read_version(f, client, &lib.project, &lib.design_system_version);
    assert!(
        version
            .as_object()
            .is_some_and(|v| v.contains_key("design_system_version_id")),
        "{version:#}"
    );
    assert_eq!(
        version["design_system_version_id"],
        Value::Null,
        "{version:#}"
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
    assert_eq!(
        checkout["version_id"],
        lib.shop_version.as_str(),
        "{checkout}"
    );
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
fn usage_counts_the_mainline_and_not_drafts() {
    let (f, client) = ready();
    let lib = library(f, client);
    let changeset = new_changeset(f, client, &lib.project);
    let doc = create_doc(
        f,
        client,
        &lib.project,
        &changeset,
        "design",
        "ui",
        "Labels",
    );
    import_design(
        f,
        client,
        &lib.project,
        &changeset,
        &doc,
        &format!(
            r#"<Page name="Labels">
  <Board name="Tags" x={{0}} y={{0}} width={{375}} height={{812}} dir="col">
    <Instance name="New" component="{}"/>
  </Board>
</Page>
"#,
            lib.ids["Tag"]
        ),
    );
    save(f, client, &lib.project, &changeset, &doc);
    let tag = usage(f, client, &lib.project, &lib.ids["Tag"]);
    assert_eq!(tag["boards"], json!([]), "a draft is no use yet: {tag}");
    merge(f, client, &lib.project, &changeset);
    assert_eq!(
        board_names(&usage(f, client, &lib.project, &lib.ids["Tag"])),
        set(&["Tags"]),
        "once merged, it is"
    );
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
