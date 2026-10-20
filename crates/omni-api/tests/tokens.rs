//! US-028: the design system doc and its tokens through the API. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; docs and drafts are those
//! of US-022, versions of US-024 and approvals of US-025; the token format is
//! that of docs/formats/tokens.md, checked by `omni_core::design::tokens`):
//!
//! - A project has at most one design system doc. `POST
//!   /changesets/{changeset_id}/docs` with `"kind": "design-system"` in a
//!   project that already has one (started in any change set, merged or not)
//!   is 409 `{"error": "design_system_exists"}` and stores nothing. Another
//!   project may have its own, and a project may have any number of `design`
//!   docs.
//! - `PUT /changesets/{changeset_id}/tokens` (agent) takes `{"tokens":
//!   <DTCG tree>}` and replaces the whole token tree of the change set's draft
//!   of the project's design system doc, in one all-or-nothing batch. A change
//!   set without a draft of the doc gets one, based on the doc's mainline
//!   version, as `POST /docs/{doc_id}/open` would make it. The answer is 200
//!   `{"doc_id", "changeset_id", "draft_id", "modes", "tokens"}` with the tree
//!   as stored. A tree that the rules refuse (an unknown reference, a loop, a
//!   typography token without a line height, ...) or that names a font file
//!   which is not a font attached to the workspace is 400 `{"error":
//!   "invalid_tokens", "message"}`, where the message names the token or the
//!   attachment, and nothing changes. A body without `tokens` is 400
//!   `{"error": "invalid_request"}`. A project without a design system doc is
//!   404 `{"error": "no_design_system"}`. A session is 403 `forbidden`. An
//!   import is logged in the draft's log (`draft_ops`, with the agent) and
//!   emits one `tokens.imported` event with `data` `{"doc_id",
//!   "changeset_id", "draft_id", "modes"}`; a refused one logs and emits
//!   nothing.
//! - `GET /changesets/{changeset_id}/tokens` (both) answers the change set's
//!   draft: `{"doc_id", "changeset_id", "draft_id", "modes", "tokens"}`.
//! - `GET /tokens` (both) answers the mainline version of the design system:
//!   `{"doc_id", "version_id", "modes", "tokens"}`, or `"version_id": null`,
//!   `"modes": []` and `"tokens": null` before the doc has one; `?version=<id>`
//!   answers that version instead. Saving a version of the design system's
//!   draft (`POST /changesets/{changeset_id}/docs/{doc_id}/versions`) keeps
//!   its token tree, and merging it makes the tree the mainline one.
//! - Both reads take `?mode=<mode>`: `"tokens"` is then the tree of that one
//!   mode (each `$value` is the mode's value, no token has
//!   `$extensions.omnistate.modes`) and the answer adds `"mode"`. A mode that
//!   the design system does not list is 400 `{"error": "unknown_mode"}`.
//! - The three routes are in the route registry: the import as `agent`, the
//!   reads as `both`.

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

const TEST_DB: &str = "omni_us028_tokens";
const TEST_BUCKET: &str = "omni-us028-tokens";
/// An attachment ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const IMPORT_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/tokens";
const MAINLINE_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/tokens";

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
    format!("us028-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
            ("X-Omni-Run", "run-28".to_string()),
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

/// Asserts a 400 `invalid_tokens` whose message has every needle in it.
fn assert_invalid_tokens(response: &Response, needles: &[&str]) {
    assert_eq!(response.status, 400, "body: {}", response.text());
    let body = response.json();
    assert_eq!(body["error"], "invalid_tokens", "{body}");
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
                Some(&json!({ "name": unique("tokens-project") })),
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
                Some(&json!({ "title": unique("change"), "run_tag": "run-28" })),
            ),
            201,
            "create a change set",
        )["changeset"],
    )
}

fn create_doc_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    kind: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/docs", changeset_path(f, project, changeset)),
        &agent(f),
        Some(&json!({ "kind": kind, "module": "ui", "title": unique(kind) })),
    )
}

/// A project with an open change set that started its design system doc;
/// returns (project, change set, doc).
fn design_system(f: &Fixture, client: &Client) -> (String, String, String) {
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let doc = id_of(
        &must(
            create_doc_response(f, client, &project, &changeset, "design-system"),
            201,
            "start the design system doc",
        )["doc"],
    );
    (project, changeset, doc)
}

fn import_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    tokens: &Value,
) -> Response {
    call(
        client,
        "PUT",
        &format!("{}/tokens", changeset_path(f, project, changeset)),
        auth,
        Some(&json!({ "tokens": tokens })),
    )
}

fn import(f: &Fixture, client: &Client, project: &str, changeset: &str, tokens: &Value) -> Value {
    must(
        import_response(f, client, &agent(f), project, changeset, tokens),
        200,
        "import the tokens",
    )
}

fn draft_tokens(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/tokens", changeset_path(f, project, changeset)),
            &agent(f),
            None,
        ),
        200,
        "read the draft's tokens",
    )
}

fn mainline_tokens(f: &Fixture, client: &Client, project: &str, query: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/tokens{query}", project_base(f, project)),
            &agent(f),
            None,
        ),
        200,
        "read the mainline tokens",
    )
}

fn save(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> String {
    id_of(
        &must(
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
        )["version"],
    )
}

/// Submits the change set and has the human approve it, which merges it.
fn merge(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let path = changeset_path(f, project, changeset);
    must(
        call(client, "POST", &format!("{path}/submit"), &agent(f), None),
        200,
        "submit the change set",
    );
    let approved = must(
        call(
            client,
            "POST",
            &format!("{path}/approve"),
            &human(f),
            Some(&json!({})),
        ),
        200,
        "the human approves",
    );
    assert_eq!(approved["changeset"]["status"], "merged", "{approved}");
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

fn upload_font(f: &Fixture, client: &Client) -> String {
    let mut bytes = b"wOF2".to_vec();
    bytes.extend((0..200u32).map(|n| (n % 251) as u8));
    upload(f, client, &bytes, "inter.woff2")
}

fn upload(f: &Fixture, client: &Client, bytes: &[u8], filename: &str) -> String {
    let headers = auth_headers(&agent(f));
    let mut headers: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
    headers.push(("Content-Type", "application/octet-stream"));
    headers.push(("X-Omni-Filename", filename));
    let response = client.request_bytes(
        "POST",
        &format!("/api/v1/w/{}/attachments", f.workspace),
        &headers,
        Some(bytes),
    );
    id_of(&must(response, 201, "upload a file")["attachment"])
}

/// A design system with a light and a dark mode.
fn light_and_dark() -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "$type": "color",
            "ink": {"$value": "#0f172a"},
            "paper": {"$value": "#ffffff"},
            "surface": {
                "$value": "#ffffff",
                "$extensions": {"omnistate": {"modes": {"light": "#ffffff", "dark": "#0f172a"}}}
            },
            "text": {
                "$value": "{color.ink}",
                "$extensions": {"omnistate": {"modes": {"light": "{color.ink}", "dark": "{color.paper}"}}}
            }
        },
        "spacing": {"$type": "dimension", "md": {"$value": "16px"}},
        "radius": {"$type": "dimension", "md": {"$value": "8px"}},
        "typography": {
            "$type": "typography",
            "body": {"$value": {
                "fontFamily": "Inter",
                "fontSize": "16px",
                "fontWeight": 400,
                "lineHeight": 1.5,
                "letterSpacing": "0px"
            }}
        },
        "shadow": {
            "$type": "shadow",
            "card": {"$value": {
                "color": "{color.ink}",
                "offsetX": "0px",
                "offsetY": "4px",
                "blur": "12px",
                "spread": "-2px"
            }}
        }
    })
}

/// Another tree: two brand modes and no spacing or radius tokens.
fn two_brands() -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": ["acme", "globex"]}},
        "color": {
            "$type": "color",
            "brand": {
                "$value": "#f97316",
                "$extensions": {"omnistate": {"modes": {"acme": "#f97316", "globex": "#2563eb"}}}
            }
        }
    })
}

fn with_typography_font(file: &str) -> Value {
    let mut tree = light_and_dark();
    tree["typography"]["body"]["$extensions"] = json!({"omnistate": {"fontFiles": [file]}});
    tree
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_token_routes_are_in_the_registry() {
    let routes = registry();
    for (method, path, access) in [
        ("PUT", IMPORT_ROUTE, Access::Agent),
        ("GET", IMPORT_ROUTE, Access::Both),
        ("GET", MAINLINE_ROUTE, Access::Both),
    ] {
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
    }
}

// ---------------------------------------------------------------------------
// One design system doc per project, created through a change set
// ---------------------------------------------------------------------------

#[test]
fn a_project_has_at_most_one_design_system_doc() {
    let (f, client) = ready();
    let (project, changeset, _doc) = design_system(f, client);

    let again = create_doc_response(f, client, &project, &changeset, "design-system");
    assert_error(&again, 409, "design_system_exists");
    let other_changeset = new_changeset(f, client, &project);
    let elsewhere = create_doc_response(f, client, &project, &other_changeset, "design-system");
    assert_error(&elsewhere, 409, "design_system_exists");
    assert_eq!(
        count(
            "select count(*) from docs where project_id = $1::uuid and kind = 'design-system'",
            &[&project],
        ),
        1,
        "a refused design system doc is not stored"
    );

    for _ in 0..2 {
        must(
            create_doc_response(f, client, &project, &changeset, "design"),
            201,
            "a project has any number of design docs",
        );
    }
    let other_project = new_project(f, client);
    let other_project_changeset = new_changeset(f, client, &other_project);
    must(
        create_doc_response(
            f,
            client,
            &other_project,
            &other_project_changeset,
            "design-system",
        ),
        201,
        "another project has a design system of its own",
    );
}

#[test]
fn a_merged_design_system_still_stops_a_second_one() {
    let (f, client) = ready();
    let (project, changeset, doc) = design_system(f, client);
    import(f, client, &project, &changeset, &light_and_dark());
    save(f, client, &project, &changeset, &doc);
    merge(f, client, &project, &changeset);

    let next = new_changeset(f, client, &project);
    let again = create_doc_response(f, client, &project, &next, "design-system");
    assert_error(&again, 409, "design_system_exists");
}

// ---------------------------------------------------------------------------
// Importing the token tree
// ---------------------------------------------------------------------------

#[test]
fn an_import_replaces_the_token_tree_of_the_change_sets_draft() {
    let (f, client) = ready();
    let (project, changeset, doc) = design_system(f, client);

    let answer = import(f, client, &project, &changeset, &light_and_dark());
    assert_eq!(answer["doc_id"], doc.as_str(), "{answer}");
    assert_eq!(answer["changeset_id"], changeset.as_str(), "{answer}");
    assert!(answer["draft_id"].is_string(), "{answer}");
    assert_eq!(answer["modes"], json!(["light", "dark"]), "{answer}");
    assert_eq!(answer["tokens"], light_and_dark(), "{answer}");

    let read = draft_tokens(f, client, &project, &changeset);
    assert_eq!(read["doc_id"], doc.as_str(), "{read}");
    assert_eq!(read["draft_id"], answer["draft_id"], "{read}");
    assert_eq!(read["modes"], json!(["light", "dark"]), "{read}");
    assert_eq!(read["tokens"], light_and_dark(), "{read}");

    let replaced = import(f, client, &project, &changeset, &two_brands());
    assert_eq!(replaced["modes"], json!(["acme", "globex"]), "{replaced}");
    let read = draft_tokens(f, client, &project, &changeset);
    assert_eq!(
        read["tokens"],
        two_brands(),
        "the whole tree is replaced: nothing of the old tree is left"
    );
    assert_eq!(read["modes"], json!(["acme", "globex"]), "{read}");
}

#[test]
fn a_refused_import_changes_nothing() {
    let (f, client) = ready();
    let (project, changeset, _doc) = design_system(f, client);
    let answer = import(f, client, &project, &changeset, &light_and_dark());
    let draft = answer["draft_id"].as_str().expect("a draft").to_string();
    let logged = logged_ops(&draft);
    let events = events_of(&project, "tokens.imported").len();

    let mut unknown = two_brands();
    unknown["color"]["accent"] = json!({"$value": "{color.missing}"});
    let mut looped = two_brands();
    looped["color"]["a"] = json!({"$value": "{color.b}"});
    looped["color"]["b"] = json!({"$value": "{color.a}"});
    let mut no_line_height = light_and_dark();
    no_line_height["typography"]["body"]["$value"]
        .as_object_mut()
        .unwrap()
        .remove("lineHeight");
    for (tree, needles) in [
        (unknown, vec!["color.missing"]),
        (looped, vec!["color."]),
        (no_line_height, vec!["typography.body", "lineHeight"]),
    ] {
        let response = import_response(f, client, &agent(f), &project, &changeset, &tree);
        assert_invalid_tokens(&response, &needles);
    }
    let response = call(
        client,
        "PUT",
        &format!("{}/tokens", changeset_path(f, &project, &changeset)),
        &agent(f),
        Some(&json!({ "tree": light_and_dark() })),
    );
    assert_error(&response, 400, "invalid_request");

    let read = draft_tokens(f, client, &project, &changeset);
    assert_eq!(
        read["tokens"],
        light_and_dark(),
        "a refused import leaves the tree as it was"
    );
    assert_eq!(logged_ops(&draft), logged, "a refused import logs nothing");
    assert_eq!(
        events_of(&project, "tokens.imported").len(),
        events,
        "a refused import emits nothing"
    );
}

#[test]
fn an_import_is_logged_with_its_agent_and_emits_an_event() {
    let (f, client) = ready();
    let (project, changeset, doc) = design_system(f, client);
    let answer = import(f, client, &project, &changeset, &light_and_dark());
    let draft = answer["draft_id"].as_str().expect("a draft");
    assert!(logged_ops(draft) >= 1, "the import is in the draft's log");
    assert_eq!(
        count(
            "select count(*) from draft_ops where draft_id = $1::uuid \
             and agent->>'run' = 'run-28' and agent->>'role' = 'designer'",
            &[draft],
        ),
        logged_ops(draft),
        "the log names the agent that imported"
    );
    let events = events_of(&project, "tokens.imported");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["doc_id"], doc.as_str(), "{}", events[0]);
    assert_eq!(
        events[0]["changeset_id"],
        changeset.as_str(),
        "{}",
        events[0]
    );
    assert_eq!(events[0]["draft_id"], draft, "{}", events[0]);
    assert_eq!(
        events[0]["modes"],
        json!(["light", "dark"]),
        "{}",
        events[0]
    );
}

#[test]
fn importing_is_for_agents_and_needs_a_design_system() {
    let (f, client) = ready();
    let (project, changeset, _doc) = design_system(f, client);
    let response = import_response(
        f,
        client,
        &human(f),
        &project,
        &changeset,
        &light_and_dark(),
    );
    assert_error(&response, 403, "forbidden");

    let bare = new_project(f, client);
    let bare_changeset = new_changeset(f, client, &bare);
    let response = import_response(
        f,
        client,
        &agent(f),
        &bare,
        &bare_changeset,
        &light_and_dark(),
    );
    assert_error(&response, 404, "no_design_system");
}

#[test]
fn an_import_opens_the_draft_of_a_change_set_that_has_none() {
    let (f, client) = ready();
    let (project, changeset, doc) = design_system(f, client);
    import(f, client, &project, &changeset, &light_and_dark());
    let version = save(f, client, &project, &changeset, &doc);
    merge(f, client, &project, &changeset);

    let next = new_changeset(f, client, &project);
    let answer = import(f, client, &project, &next, &two_brands());
    let shown = must(
        call(
            client,
            "GET",
            &format!("{}/docs/{doc}", project_base(f, &project)),
            &agent(f),
            None,
        ),
        200,
        "show the design system doc",
    );
    let drafts = shown["doc"]["drafts"].as_array().expect("drafts");
    let draft = drafts
        .iter()
        .find(|d| d["changeset_id"] == next.as_str())
        .unwrap_or_else(|| panic!("the import made a draft in the change set: {shown}"));
    assert_eq!(draft["id"], answer["draft_id"], "{shown}");
    assert_eq!(
        draft["base_version_id"],
        version.as_str(),
        "the new draft is based on the mainline version"
    );
    assert_eq!(
        mainline_tokens(f, client, &project, "")["tokens"],
        light_and_dark(),
        "the import changes only the change set's draft, not the mainline"
    );
}

// ---------------------------------------------------------------------------
// Font files
// ---------------------------------------------------------------------------

#[test]
fn typography_tokens_use_fonts_attached_to_the_workspace() {
    let (f, client) = ready();
    let (project, changeset, _doc) = design_system(f, client);
    let font = upload_font(f, client);
    let tree = with_typography_font(&format!("attachment:{font}"));
    let answer = import(f, client, &project, &changeset, &tree);
    assert_eq!(answer["tokens"], tree, "{answer}");

    let unknown = with_typography_font(&format!("attachment:{UNKNOWN_ID}"));
    let response = import_response(f, client, &agent(f), &project, &changeset, &unknown);
    assert_invalid_tokens(&response, &[UNKNOWN_ID]);

    let notes = upload(f, client, b"plain notes, not a font", "notes.txt");
    let not_a_font = with_typography_font(&format!("attachment:{notes}"));
    let response = import_response(f, client, &agent(f), &project, &changeset, &not_a_font);
    assert_invalid_tokens(&response, &[notes.as_str()]);

    assert_eq!(
        draft_tokens(f, client, &project, &changeset)["tokens"],
        tree,
        "the refused imports changed nothing"
    );
}

// ---------------------------------------------------------------------------
// Reading the tokens: drafts, mainline, versions and modes
// ---------------------------------------------------------------------------

#[test]
fn the_mainline_tokens_are_those_of_the_merged_version() {
    let (f, client) = ready();
    let (project, changeset, doc) = design_system(f, client);
    let before = mainline_tokens(f, client, &project, "");
    assert_eq!(before["doc_id"], doc.as_str(), "{before}");
    assert_eq!(before["version_id"], Value::Null, "{before}");
    assert_eq!(before["modes"], json!([]), "{before}");
    assert_eq!(before["tokens"], Value::Null, "{before}");

    import(f, client, &project, &changeset, &light_and_dark());
    let first = save(f, client, &project, &changeset, &doc);
    merge(f, client, &project, &changeset);
    let mainline = mainline_tokens(f, client, &project, "");
    assert_eq!(mainline["version_id"], first.as_str(), "{mainline}");
    assert_eq!(mainline["modes"], json!(["light", "dark"]), "{mainline}");
    assert_eq!(mainline["tokens"], light_and_dark(), "{mainline}");

    let next = new_changeset(f, client, &project);
    import(f, client, &project, &next, &two_brands());
    let second = save(f, client, &project, &next, &doc);
    merge(f, client, &project, &next);
    let mainline = mainline_tokens(f, client, &project, "");
    assert_eq!(mainline["version_id"], second.as_str(), "{mainline}");
    assert_eq!(mainline["tokens"], two_brands(), "{mainline}");

    let old = mainline_tokens(f, client, &project, &format!("?version={first}"));
    assert_eq!(old["version_id"], first.as_str(), "{old}");
    assert_eq!(old["tokens"], light_and_dark(), "{old}");

    for auth in [agent(f), human(f)] {
        must(
            call(
                client,
                "GET",
                &format!("{}/tokens", project_base(f, &project)),
                &auth,
                None,
            ),
            200,
            "the agent and the human read the tokens",
        );
    }
}

#[test]
fn a_read_can_give_the_tree_of_one_mode() {
    let (f, client) = ready();
    let (project, changeset, doc) = design_system(f, client);
    import(f, client, &project, &changeset, &light_and_dark());

    let dark = must(
        call(
            client,
            "GET",
            &format!(
                "{}/tokens?mode=dark",
                changeset_path(f, &project, &changeset)
            ),
            &agent(f),
            None,
        ),
        200,
        "read the draft's dark mode",
    );
    assert_eq!(dark["mode"], "dark", "{dark}");
    assert_eq!(
        dark["tokens"]["color"]["surface"]["$value"], "#0f172a",
        "{dark}"
    );
    assert_eq!(
        dark["tokens"]["color"]["text"]["$value"], "{color.paper}",
        "{dark}"
    );
    assert!(
        dark["tokens"]["color"]["surface"]
            .pointer("/$extensions/omnistate/modes")
            .is_none(),
        "the tree of one mode has no mode values: {dark}"
    );

    save(f, client, &project, &changeset, &doc);
    merge(f, client, &project, &changeset);
    let light = mainline_tokens(f, client, &project, "?mode=light");
    assert_eq!(light["mode"], "light", "{light}");
    assert_eq!(
        light["tokens"]["color"]["surface"]["$value"], "#ffffff",
        "{light}"
    );
    assert_eq!(
        light["tokens"]["color"]["text"]["$value"], "{color.ink}",
        "{light}"
    );

    let response = call(
        client,
        "GET",
        &format!("{}/tokens?mode=sepia", project_base(f, &project)),
        &agent(f),
        None,
    );
    assert_error(&response, 400, "unknown_mode");
}
