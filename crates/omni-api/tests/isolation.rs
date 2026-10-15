//! US-018: the cross-workspace and authority suite. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432`. It runs in `make test` and on every build from now on, and
//! it attacks every route that the API has, so a story that adds a route (or a
//! tenant table) extends this file in its red step.
//!
//! The suite migrates a scratch database and seeds two workspaces, Alpha (A)
//! and Beta (B), each with a member (alice, bob), a session, an API key, a spare
//! key, two projects and two change sets (one of them rejected by the human),
//! all made through the API itself so that every tenant table has rows of both
//! workspaces. The API's router is served on a local port.
//!
//! The contract these tests pin down:
//!
//! - `CASES` lists every route of `omni_api::routes::registry()`, each either
//!   `Workspace` (its path holds `{workspace_id}`, and the suite attacks it) or
//!   `Global` (no workspace in the path, with the reason). The path may hold
//!   `{project_id}`, `{changeset_id}`, `{key_id}` and `{attachment_id}`, which
//!   the suite fills with the IDs of the seeded records; a path with another
//!   placeholder, or a registry route without a case, fails the suite.
//! - Workspace A's credentials against workspace B's workspace and IDs, and
//!   against B's IDs under A's own workspace, are always 404
//!   `{"error": "not_found"}` with nothing of B in the body, and the answer is
//!   the one for an ID that does not exist. The caller's authority is checked
//!   first (FR-5): an API key on a `human` route is 403 whatever the path says,
//!   so the attack on a `human` route uses alice's session.
//! - Every `human` route (the login, which takes a password and not a key, is
//!   the one exemption) answers an API key with 403 `{"error": "forbidden"}`
//!   and changes nothing.
//! - `omni_api::routes::websocket_registry()` lists the WebSocket routes, as
//!   `registry()` lists the HTTP ones; the suite covers each of them, and every
//!   path that the router source serves is in one of the two registries.
//! - At the database, the API role (`omni_api`) sees none of the other
//!   workspace's rows in any tenant table, and cannot insert, update, move or
//!   delete them.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, RouteEntry, registry, websocket_registry};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::BTreeSet;
use std::future::Future;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

const TEST_DB: &str = "omni_us018_isolation";
/// The scratch bucket of the attachments that the suite seeds (US-019).
const TEST_BUCKET: &str = "omni-us018-isolation";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
/// The handshake key of RFC 6455's example.
const HANDSHAKE_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
/// How long a test waits for something that must happen.
const PATIENCE: Duration = Duration::from_secs(5);
/// How long a test waits to be sure that something does not happen.
const QUIET: Duration = Duration::from_millis(600);

// ---------------------------------------------------------------------------
// The coverage table
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Kind {
    /// The path holds `{workspace_id}`: the suite attacks it.
    Workspace,
    /// No workspace in the path, so there is no other workspace to reach.
    Global(&'static str),
}

struct Case {
    method: &'static str,
    path: &'static str,
    /// The JSON body of a call that would succeed if the caller were allowed.
    body: Option<&'static str>,
    kind: Kind,
}

const fn case(
    method: &'static str,
    path: &'static str,
    body: Option<&'static str>,
    kind: Kind,
) -> Case {
    Case {
        method,
        path,
        body,
        kind,
    }
}

const CASES: &[Case] = &[
    case(
        "GET",
        "/api/health",
        None,
        Kind::Global("reports the stack; no workspace"),
    ),
    case(
        "GET",
        "/api/openapi.json",
        None,
        Kind::Global("the public API description; no workspace"),
    ),
    case(
        "POST",
        "/api/v1/auth/login",
        Some(r#"{"email":"alice@example.test","password":"wrong"}"#),
        Kind::Global("takes an email and a password; no workspace"),
    ),
    case(
        "POST",
        "/api/v1/auth/logout",
        None,
        Kind::Global("ends the caller's own session; no workspace"),
    ),
    case(
        "GET",
        "/api/v1/auth/me",
        None,
        Kind::Global("the caller's own user; no workspace"),
    ),
    case(
        "GET",
        "/api/v1/auth/workspaces",
        None,
        Kind::Global("the caller's own memberships; no workspace"),
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/whoami",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/api-keys",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/api-keys",
        Some("{}"),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/api-keys/{key_id}/revoke",
        None,
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects",
        Some(r#"{"name":"intruder project"}"#),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
        Some(r#"{"title":"intruder change","run_tag":"run-1"}"#),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/submit",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reject",
        Some(r#"{"reason":"not good enough yet"}"#),
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reopen",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/abandon",
        None,
        Kind::Workspace,
    ),
    case(
        "POST",
        "/api/v1/w/{workspace_id}/attachments",
        Some("the bytes of a file"),
        Kind::Workspace,
    ),
    case(
        "GET",
        "/api/v1/w/{workspace_id}/attachments/{attachment_id}",
        None,
        Kind::Workspace,
    ),
];

/// The WebSocket routes that the suite attacks, as the paths of
/// `websocket_registry()`.
const WEBSOCKET_PATHS: &[&str] = &["/ws/w/{workspace_id}"];

/// The human routes that an API key may still reach: the login checks a
/// password, so a key next to it is not an attempt at the human's authority.
const KEY_EXEMPT: &[(&str, &str)] = &[("POST", "/api/v1/auth/login")];

fn route_name(method: &str, path: &str) -> String {
    format!("{method} {path}")
}

/// The routes of `routes` that no case covers.
fn uncovered(routes: &[RouteEntry], cases: &[Case]) -> Vec<String> {
    routes
        .iter()
        .filter(|r| {
            !cases
                .iter()
                .any(|c| c.method == r.method && c.path == r.path)
        })
        .map(|r| route_name(&r.method, &r.path))
        .collect()
}

/// The cases that match no route of `routes`.
fn stale(routes: &[RouteEntry], cases: &[Case]) -> Vec<String> {
    cases
        .iter()
        .filter(|c| {
            !routes
                .iter()
                .any(|r| c.method == r.method && c.path == r.path)
        })
        .map(|c| route_name(c.method, c.path))
        .collect()
}

fn case_for(entry: &RouteEntry) -> &'static Case {
    CASES
        .iter()
        .find(|c| c.method == entry.method && c.path == entry.path)
        .unwrap_or_else(|| {
            panic!(
                "no suite coverage for {}: add a case to CASES in tests/isolation.rs",
                route_name(&entry.method, &entry.path)
            )
        })
}

fn case_named(method: &str, path: &str) -> &'static Case {
    CASES
        .iter()
        .find(|c| c.method == method && c.path == path)
        .unwrap_or_else(|| panic!("no case for {}", route_name(method, path)))
}

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
        // The attachments (US-019) keep their bytes in RustFS.
        s3_endpoint: s3_endpoint(),
        s3_access_key: s3_access_key(),
        s3_secret_key: s3_secret_key(),
        s3_bucket: TEST_BUCKET.to_string(),
    }
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

/// The keys in the scratch bucket under `prefix`, sorted.
fn keys_under(prefix: &str) -> Vec<String> {
    let pages = block(bucket().list(prefix.to_string(), None))
        .unwrap_or_else(|e| panic!("cannot list {prefix}: {e}"));
    let mut keys: Vec<String> = pages
        .into_iter()
        .flat_map(|page| page.contents)
        .map(|object| object.key)
        .collect();
    keys.sort();
    keys
}

async fn admin_connection() -> PgConnection {
    PgConnection::connect_with(&admin_options().database(TEST_DB))
        .await
        .expect("connect to the scratch database as the admin role")
}

/// A connection that logs in as the API role itself, as the API's pool does.
async fn api_connection() -> PgConnection {
    PgConnection::connect(&config().database_url)
        .await
        .expect("connect to the scratch database as the API role")
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Tests that change workspace B on purpose, and tests that check B did not
/// change, take this lock so they do not see each other's work.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// How a request authenticates: an API key (an agent), a session cookie (the
/// human), both at once, or nothing.
#[derive(Clone, Copy)]
enum Auth<'a> {
    Key(&'a str),
    Session(&'a str),
    KeyAndSession(&'a str, &'a str),
    Nobody,
}

impl Auth<'_> {
    fn headers(&self) -> Vec<(&'static str, String)> {
        let key = |secret: &str| ("Authorization", format!("Bearer {secret}"));
        match self {
            Auth::Key(secret) => vec![key(secret)],
            Auth::Session(token) => vec![("Cookie", cookie(token))],
            Auth::KeyAndSession(secret, token) => vec![key(secret), ("Cookie", cookie(token))],
            Auth::Nobody => vec![],
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Auth::Key(_) => "an API key",
            Auth::Session(_) => "a session",
            Auth::KeyAndSession(..) => "an API key and a session",
            Auth::Nobody => "no credentials",
        }
    }
}

fn call_with(
    client: &Client,
    method: &str,
    path: &str,
    auth: &Auth,
    extra: &[(&str, &str)],
    body: Option<&str>,
) -> Response {
    let owned = auth.headers();
    let mut headers: Vec<(&str, &str)> = owned.iter().map(|(n, v)| (*n, v.as_str())).collect();
    headers.extend_from_slice(extra);
    client.request(method, path, &headers, body)
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&str>) -> Response {
    call_with(client, method, path, auth, &[], body)
}

/// A response that must have `status`; returns its JSON body.
fn must(response: Response, status: u16, what: &str) -> Value {
    assert_eq!(response.status, status, "{what}: {}", response.text());
    response.json()
}

/// Whether the router has the attachment routes (US-019) yet.
fn has_attachment_routes() -> bool {
    registry()
        .iter()
        .any(|r| r.method == "POST" && r.path == "/api/v1/w/{workspace_id}/attachments")
}

/// Uploads `bytes` as `name` with an API key; the raw bytes are the body.
fn upload_attachment(
    client: &Client,
    workspace: &str,
    key: &str,
    bytes: &[u8],
    name: &str,
) -> Response {
    let bearer = format!("Bearer {key}");
    client.request_bytes(
        "POST",
        &format!("/api/v1/w/{workspace}/attachments"),
        &[
            ("Authorization", bearer.as_str()),
            ("Content-Type", "text/plain"),
            ("X-Omni-Filename", name),
        ],
        Some(bytes),
    )
}

/// One workspace as the suite sees it: who belongs to it and which records it
/// has. The fields of a stranger hold one made-up ID.
struct Side {
    id: String,
    email: String,
    token: String,
    /// The key that the workspace's tests use; never revoked.
    key: String,
    /// A second key, which is the target of the revoke attacks.
    key_id: String,
    key_prefix: String,
    project_id: String,
    project_name: String,
    other_project_id: String,
    changeset_id: String,
    rejected_changeset_id: String,
    /// A file that the workspace's agent uploaded (US-019): empty until the
    /// attachment routes exist.
    attachment_id: String,
    attachment_name: String,
    /// The file's content, which is text that nothing else holds.
    attachment_text: String,
}

impl Side {
    /// A "workspace" whose every ID is `id`: one that does not exist.
    fn stranger(id: &str) -> Side {
        let same = || id.to_string();
        Side {
            id: same(),
            email: String::new(),
            token: String::new(),
            key: String::new(),
            key_id: same(),
            key_prefix: String::new(),
            project_id: same(),
            project_name: String::new(),
            other_project_id: same(),
            changeset_id: same(),
            rejected_changeset_id: same(),
            attachment_id: same(),
            attachment_name: String::new(),
            attachment_text: String::new(),
        }
    }

    /// Everything about the workspace that must never appear in an answer to
    /// another workspace.
    fn markers(&self) -> Vec<&str> {
        [
            &self.id,
            &self.email,
            &self.key_id,
            &self.key_prefix,
            &self.project_id,
            &self.project_name,
            &self.other_project_id,
            &self.changeset_id,
            &self.rejected_changeset_id,
            &self.attachment_id,
            &self.attachment_name,
            &self.attachment_text,
        ]
        .into_iter()
        .map(String::as_str)
        .filter(|m| !m.is_empty())
        .collect()
    }
}

struct Fixture {
    a: Side,
    b: Side,
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
            if !bucket().exists().await.unwrap_or(false) {
                Bucket::create_with_path_style(
                    TEST_BUCKET,
                    s3_region(),
                    s3_credentials(),
                    BucketConfiguration::default(),
                )
                .await
                .unwrap_or_else(|e| panic!("cannot create the scratch bucket: {e}"));
            }
        });
        let workspaces = block(async {
            let mut conn = admin_connection().await;
            let mut text = async |sql: &str, binds: &[&str]| -> String {
                let mut query = sqlx::query_scalar::<_, String>(sql);
                for bind in binds {
                    query = query.bind(*bind);
                }
                query.fetch_one(&mut conn).await.expect("seed")
            };
            let mut found = Vec::new();
            for (name, workspace_name) in [("alice", "Alpha"), ("bob", "Beta")] {
                let workspace = text(
                    "insert into workspaces (name) values ($1) returning id::text",
                    &[workspace_name],
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
                    &[&user, &session_token(name)],
                )
                .await;
                text(
                    "insert into memberships (workspace_id, user_id) \
                     values ($1::uuid, $2::uuid) returning id::text",
                    &[&workspace, &user],
                )
                .await;
                found.push((name, workspace));
            }
            found
        });
        let server = TestServer::start(&config());
        let client = &server.client;
        let mut sides = workspaces.into_iter().map(|(name, workspace)| {
            let token = session_token(name);
            let human = Auth::Session(&token);
            let keys_path = format!("/api/v1/w/{workspace}/api-keys");
            let minted = must(
                call(client, "POST", &keys_path, &human, Some("{}")),
                201,
                "mint a key",
            );
            let key = minted["secret"].as_str().expect("a secret").to_string();
            let spare = must(
                call(client, "POST", &keys_path, &human, Some("{}")),
                201,
                "mint a spare key",
            );
            let agent = Auth::Key(&key);
            let project = |label: &str| -> (String, String) {
                let project_name = unique(&format!("{name}-{label}"));
                let body = json!({ "name": project_name }).to_string();
                let created = must(
                    call(
                        client,
                        "POST",
                        &format!("/api/v1/w/{workspace}/projects"),
                        &agent,
                        Some(&body),
                    ),
                    201,
                    "create a project",
                );
                let id = created["project"]["id"].as_str().expect("an id");
                (id.to_string(), project_name)
            };
            let (project_id, project_name) = project("roadmap");
            let (other_project_id, _) = project("backlog");
            let changesets = format!("/api/v1/w/{workspace}/projects/{project_id}/changesets");
            let changeset = || -> String {
                let body = json!({ "title": unique(&format!("{name}-change")) }).to_string();
                let created = must(
                    call(client, "POST", &changesets, &agent, Some(&body)),
                    201,
                    "create a change set",
                );
                created["changeset"]["id"]
                    .as_str()
                    .expect("an id")
                    .to_string()
            };
            let changeset_id = changeset();
            let rejected_changeset_id = changeset();
            let path = format!("{changesets}/{rejected_changeset_id}");
            must(
                call(client, "POST", &format!("{path}/submit"), &agent, None),
                200,
                "submit",
            );
            must(
                call(
                    client,
                    "POST",
                    &format!("{path}/reject"),
                    &human,
                    Some(r#"{"reason":"seeded"}"#),
                ),
                200,
                "reject",
            );
            // US-019: a file of the workspace's own. The attachment routes are
            // seeded as soon as the registry has them; the table behind them
            // would otherwise make the checks below fail on a tenant table
            // without rows.
            let (attachment_id, attachment_name, attachment_text) = if has_attachment_routes() {
                let file_name = format!("{name}-notes.txt");
                let text = unique(&format!("{name}-file-content"));
                let uploaded =
                    upload_attachment(client, &workspace, &key, text.as_bytes(), &file_name);
                let uploaded = must(uploaded, 201, "upload a file");
                let id = uploaded["attachment"]["id"].as_str().expect("an id");
                (id.to_string(), file_name, text)
            } else {
                (String::new(), String::new(), String::new())
            };
            Side {
                id: workspace,
                email: format!("{name}@example.test"),
                token: token.clone(),
                key: key.clone(),
                key_id: spare["key"]["id"].as_str().expect("an id").to_string(),
                key_prefix: spare["key"]["prefix"]
                    .as_str()
                    .expect("a prefix")
                    .to_string(),
                project_id,
                project_name,
                other_project_id,
                changeset_id,
                rejected_changeset_id,
                attachment_id,
                attachment_name,
                attachment_text,
            }
        });
        let a = sides.next().expect("workspace A");
        let b = sides.next().expect("workspace B");
        Fixture { a, b, server }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

/// `template` with its placeholders filled: `{workspace_id}` from `workspace`,
/// the IDs of records from `record`.
fn fill(template: &str, workspace: &Side, record: &Side) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let end = start
            + rest[start..]
                .find('}')
                .unwrap_or_else(|| panic!("unclosed placeholder in {template}"));
        out.push_str(match &rest[start + 1..end] {
            "workspace_id" => &workspace.id,
            "project_id" => &record.project_id,
            "changeset_id" => &record.changeset_id,
            "key_id" => &record.key_id,
            "attachment_id" => &record.attachment_id,
            other => panic!(
                "{template} has the placeholder {{{other}}}, for which the suite has no seeded \
                 ID: add it to Side and fill() in tests/isolation.rs"
            ),
        });
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Whether the path names a record besides the workspace.
fn names_a_record(template: &str) -> bool {
    template.matches('{').count() > 1
}

/// The credentials of `side` that the route accepts, as the route's access
/// says: an API key for an agent route, a session for a human one, either for both.
fn credentials(access: Access, side: &Side) -> Vec<Auth<'_>> {
    match access {
        Access::Agent => vec![Auth::Key(&side.key)],
        Access::Human => vec![Auth::Session(&side.token)],
        Access::Both => vec![Auth::Key(&side.key), Auth::Session(&side.token)],
    }
}

fn assert_not_found_without_data(response: &Response, victim: &Side, what: &str) {
    assert_eq!(response.status, 404, "{what}: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "not_found" }),
        "{what}: the answer carries the code and nothing else"
    );
    for marker in victim.markers() {
        assert!(
            !response.text().contains(marker),
            "{what}: the answer leaks {marker}: {}",
            response.text()
        );
    }
}

// ---------------------------------------------------------------------------
// Database snapshots
// ---------------------------------------------------------------------------

fn scope_column(table: &str) -> &'static str {
    if table == "workspaces" {
        "id"
    } else {
        "workspace_id"
    }
}

/// Every tenant table: `workspaces` and each table with a `workspace_id` column.
async fn tenant_tables(conn: &mut PgConnection) -> Vec<String> {
    let mut tables: Vec<String> = sqlx::query_scalar(
        "select distinct table_name::text from information_schema.columns \
         where table_schema = 'public' and column_name = 'workspace_id' \
         and table_name <> '_sqlx_migrations'",
    )
    .fetch_all(&mut *conn)
    .await
    .expect("list the tenant tables");
    tables.push("workspaces".to_string());
    tables.sort();
    tables.dedup();
    tables
}

async fn rows_of(conn: &mut PgConnection, table: &str, workspace: &str) -> i64 {
    let column = scope_column(table);
    sqlx::query_scalar(&format!(
        "select count(*) from {table} where {column} = '{workspace}'::uuid"
    ))
    .fetch_one(&mut *conn)
    .await
    .unwrap_or_else(|e| panic!("count the rows of {table}: {e}"))
}

/// A digest of every row of the workspace in every tenant table, so a test can
/// tell that an attack changed nothing.
fn snapshot(workspace: &str) -> Vec<(String, String)> {
    block(async {
        let mut conn = admin_connection().await;
        let mut found = Vec::new();
        for table in tenant_tables(&mut conn).await {
            let column = scope_column(&table);
            let digest: String = sqlx::query_scalar(&format!(
                "select md5(coalesce(string_agg(t::text, '|' order by t::text), '')) \
                 from {table} t where t.{column} = '{workspace}'::uuid"
            ))
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("digest {table}: {e}"));
            found.push((table, digest));
        }
        found
    })
}

// ---------------------------------------------------------------------------
// Coverage of the routes
// ---------------------------------------------------------------------------

#[test]
fn every_route_in_the_registry_has_suite_coverage() {
    let routes = registry();
    assert!(!routes.is_empty());
    let missing = uncovered(&routes, CASES);
    assert!(
        missing.is_empty(),
        "routes without suite coverage, add a case to CASES in tests/isolation.rs: {missing:?}"
    );
    let gone = stale(&routes, CASES);
    assert!(
        gone.is_empty(),
        "cases for routes that the registry no longer has: {gone:?}"
    );
    for entry in &routes {
        let name = route_name(&entry.method, &entry.path);
        match case_for(entry).kind {
            Kind::Workspace => assert!(
                entry.path.contains("{workspace_id}"),
                "{name} is a Workspace case without {{workspace_id}} in its path"
            ),
            Kind::Global(reason) => {
                assert!(!reason.is_empty(), "{name} is Global without a reason");
                assert!(
                    !entry.path.contains("{workspace_id}"),
                    "{name} has a workspace in its path, so the suite must attack it"
                );
            }
        }
    }
}

#[test]
fn a_route_without_suite_coverage_is_reported() {
    let mut routes = registry();
    assert!(
        uncovered(&routes, CASES).is_empty(),
        "the real registry is covered"
    );
    routes.push(RouteEntry {
        method: "GET".to_string(),
        path: "/api/v1/w/{workspace_id}/projects/{project_id}/documents".to_string(),
        access: Access::Both,
    });
    routes.push(RouteEntry {
        method: "DELETE".to_string(),
        path: "/api/v1/w/{workspace_id}/projects".to_string(),
        access: Access::Agent,
    });
    assert_eq!(
        uncovered(&routes, CASES),
        [
            "GET /api/v1/w/{workspace_id}/projects/{project_id}/documents",
            "DELETE /api/v1/w/{workspace_id}/projects"
        ],
        "a new route, and a new method on a covered path, are both uncovered"
    );
    let without_one = &CASES[1..];
    assert_eq!(
        uncovered(&registry(), without_one),
        ["GET /api/health"],
        "a case taken away uncovers its route"
    );
}

#[test]
fn every_websocket_route_has_suite_coverage() {
    let routes = websocket_registry();
    let paths: BTreeSet<&str> = routes.iter().map(|r| r.path.as_str()).collect();
    let covered: BTreeSet<&str> = WEBSOCKET_PATHS.iter().copied().collect();
    assert_eq!(
        paths, covered,
        "the WebSocket routes of websocket_registry() against the paths this suite attacks \
         (WEBSOCKET_PATHS in tests/isolation.rs)"
    );
    for route in &routes {
        assert_eq!(route.method, "GET", "{}", route.path);
        assert!(
            route.path.starts_with("/ws/") && route.path.contains("{workspace_id}"),
            "a WebSocket route lives under /ws/ and names its workspace: {}",
            route.path
        );
    }
}

/// The path literals of every `.route("...")` call in the API's source.
fn served_paths() -> BTreeSet<String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir(&source).expect("read src/") {
        let path = entry.expect("a directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read a source file");
        let mut rest = text.as_str();
        while let Some(at) = rest.find(".route(") {
            rest = rest[at + ".route(".len()..].trim_start();
            if let Some(literal) = rest.strip_prefix('"')
                && let Some(end) = literal.find('"')
            {
                found.insert(literal[..end].to_string());
            }
        }
    }
    found
}

#[test]
fn every_path_that_the_router_serves_is_in_a_route_registry() {
    let served = served_paths();
    assert!(
        served.contains("/api/health") && served.contains("/api/v1/auth/login"),
        "the scan of src/ found no routes, so it proves nothing: {served:?}"
    );
    let registered: BTreeSet<String> = registry()
        .into_iter()
        .chain(websocket_registry())
        .map(|r| r.path)
        .collect();
    let hidden: Vec<&String> = served.difference(&registered).collect();
    assert!(
        hidden.is_empty(),
        "routes that the router serves but no registry lists, so no suite attacks them \
         (add them to routes::registry() or routes::websocket_registry()): {hidden:?}"
    );
    let phantom: Vec<&String> = registered.difference(&served).collect();
    assert!(
        phantom.is_empty(),
        "registry entries that the router does not serve: {phantom:?}"
    );
}

#[test]
fn the_suite_runs_in_make_test() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |file: &str| -> String {
        std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("read {file}: {e}"))
    };

    // `make test` runs the test container.
    let makefile = read("Makefile");
    let target = makefile
        .split("\ntest:")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("the test target of the Makefile");
    assert!(
        target.contains("docker compose run --rm test"),
        "make test must run the test container: {target}"
    );

    // The container runs all of omni-api's tests, not a chosen few.
    let dockerfile = read("docker/test.Dockerfile");
    let run = dockerfile
        .split("&&")
        .find(|part| part.contains("-p omni-api"))
        .expect("the test container runs omni-api's tests");
    assert!(
        run.contains("--features stack-tests") && !run.contains("--test "),
        "the container must run every omni-api test target: {run}"
    );

    // The suite needs Postgres, so its target is built with the stack-tests
    // feature only, which keeps `cargo test --workspace` on the host green.
    let manifest = read("crates/omni-api/Cargo.toml");
    let target = manifest
        .split("[[test]]")
        .find(|section| section.contains("name = \"isolation\""))
        .expect("Cargo.toml registers the isolation test target");
    assert!(
        target
            .lines()
            .any(|l| l.trim() == "required-features = [\"stack-tests\"]"),
        "the isolation target needs required-features = [\"stack-tests\"]: {target}"
    );
}

// ---------------------------------------------------------------------------
// Workspace A against workspace B, over HTTP
// ---------------------------------------------------------------------------

#[test]
fn credentials_of_workspace_a_get_404_and_no_data_for_the_workspace_and_ids_of_b() {
    let (f, client) = ready();
    let _guard = serial();
    let before = snapshot(&f.b.id);
    let mut attacks = 0;
    for entry in registry() {
        let case = case_for(&entry);
        if !matches!(case.kind, Kind::Workspace) {
            continue;
        }
        let path = fill(&entry.path, &f.b, &f.b);
        let mut callers = credentials(entry.access, &f.a);
        if entry.access == Access::Agent {
            // The workspace is checked before the caller's authority (FR-5): a
            // session of A on an agent route of B is a stranger first.
            callers.push(Auth::Session(&f.a.token));
        }
        for auth in callers {
            let what = format!(
                "{} with {} of workspace A against B",
                route_name(&entry.method, &path),
                auth.label()
            );
            let response = call(client, &entry.method, &path, &auth, case.body);
            assert_not_found_without_data(&response, &f.b, &what);
            attacks += 1;
        }
    }
    assert!(attacks >= 20, "only {attacks} attacks ran");
    assert_eq!(
        snapshot(&f.b.id),
        before,
        "an attack on workspace B changed its rows"
    );
}

#[test]
fn ids_of_workspace_b_under_the_workspace_of_a_get_the_answer_for_an_id_that_does_not_exist() {
    let (f, client) = ready();
    let _guard = serial();
    let before = snapshot(&f.b.id);
    let nobody = Side::stranger(UNKNOWN_ID);
    let mut attacks = 0;
    for entry in registry() {
        let case = case_for(&entry);
        if !matches!(case.kind, Kind::Workspace) || !names_a_record(&entry.path) {
            continue;
        }
        let stolen = fill(&entry.path, &f.a, &f.b);
        let missing = fill(&entry.path, &f.a, &nobody);
        for auth in credentials(entry.access, &f.a) {
            let what = format!(
                "{} with {} of workspace A and B's IDs",
                route_name(&entry.method, &stolen),
                auth.label()
            );
            let response = call(client, &entry.method, &stolen, &auth, case.body);
            assert_not_found_without_data(&response, &f.b, &what);
            let unknown = call(client, &entry.method, &missing, &auth, case.body);
            assert_eq!(
                (response.status, response.body.clone()),
                (unknown.status, unknown.body.clone()),
                "{what}: an ID of another workspace is answered like one that does not exist"
            );
            attacks += 1;
        }
    }
    assert!(attacks >= 10, "only {attacks} attacks ran");
    assert_eq!(
        snapshot(&f.b.id),
        before,
        "an attack on workspace B changed its rows"
    );
}

#[test]
fn the_same_attacks_work_the_other_way_round() {
    let (f, client) = ready();
    let _guard = serial();
    let before = snapshot(&f.a.id);
    for entry in registry() {
        let case = case_for(&entry);
        if !matches!(case.kind, Kind::Workspace) {
            continue;
        }
        let abroad = fill(&entry.path, &f.a, &f.a);
        let stolen = fill(&entry.path, &f.b, &f.a);
        for auth in credentials(entry.access, &f.b) {
            let what = format!(
                "{} with {} of workspace B against A",
                route_name(&entry.method, &abroad),
                auth.label()
            );
            let response = call(client, &entry.method, &abroad, &auth, case.body);
            assert_not_found_without_data(&response, &f.a, &what);
            if names_a_record(&entry.path) {
                let response = call(client, &entry.method, &stolen, &auth, case.body);
                assert_not_found_without_data(&response, &f.a, &what);
            }
        }
    }
    assert_eq!(
        snapshot(&f.a.id),
        before,
        "an attack on workspace A changed it"
    );
}

#[test]
fn the_lists_of_a_workspace_hold_nothing_of_the_other() {
    let (f, client) = ready();
    for (own, other) in [(&f.a, &f.b), (&f.b, &f.a)] {
        for entry in registry() {
            let case = case_for(&entry);
            if !matches!(case.kind, Kind::Workspace) || entry.method != "GET" {
                continue;
            }
            let path = fill(&entry.path, own, own);
            for auth in credentials(entry.access, own) {
                let what = format!("{} with {}", route_name(&entry.method, &path), auth.label());
                let response = call(client, &entry.method, &path, &auth, None);
                assert_eq!(response.status, 200, "{what}: {}", response.text());
                for marker in other.markers() {
                    assert!(
                        !response.text().contains(marker),
                        "{what} shows {marker} of the other workspace: {}",
                        response.text()
                    );
                }
                if entry.path.contains("/attachments/") {
                    assert_eq!(
                        response.text(),
                        own.attachment_text,
                        "{what}: the control call returns the workspace's own file"
                    );
                } else if names_a_record(&entry.path)
                    || entry.path.ends_with("/projects")
                    || entry.path.ends_with("/api-keys")
                {
                    let text = response.text();
                    assert!(
                        text.contains(&own.project_id)
                            || text.contains(&own.changeset_id)
                            || text.contains(&own.key_id),
                        "{what}: the control call shows none of its own records: {text}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_calls_of_the_attacks_work_for_the_workspace_that_owns_the_records() {
    // The control: the same requests, with the same bodies, succeed for the
    // owner, so the 404s above come from isolation and not from a bad request.
    let (f, client) = ready();
    let _guard = serial();
    let key = Auth::Key(&f.a.key);
    let human = Auth::Session(&f.a.token);
    let path = |template: &str| fill(template, &f.a, &f.a);

    let projects = case_named("POST", "/api/v1/w/{workspace_id}/projects");
    let body = projects
        .body
        .map(|b| b.replace("intruder", &unique("owner")));
    let response = call(client, "POST", &path(projects.path), &key, body.as_deref());
    must(response, 201, "create a project");

    let changesets = case_named(
        "POST",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets",
    );
    let created = must(
        call(
            client,
            "POST",
            &path(changesets.path),
            &key,
            changesets.body,
        ),
        201,
        "create a change set",
    );
    let id = created["changeset"]["id"].as_str().expect("an id");
    let base = format!(
        "/api/v1/w/{}/projects/{}/changesets/{id}",
        f.a.id, f.a.project_id
    );
    let action = |name: &str, auth: &Auth| {
        let template = format!(
            "/api/v1/w/{{workspace_id}}/projects/{{project_id}}/changesets/{{changeset_id}}/{name}"
        );
        let case = case_named("POST", &template);
        call(client, "POST", &format!("{base}/{name}"), auth, case.body)
    };
    assert_eq!(action("submit", &key).status, 200);
    assert_eq!(action("reject", &human).status, 200);
    assert_eq!(action("reopen", &key).status, 200);
    assert_eq!(action("abandon", &key).status, 200);

    let keys = case_named("POST", "/api/v1/w/{workspace_id}/api-keys");
    must(
        call(client, "POST", &path(keys.path), &human, keys.body),
        201,
        "mint a key",
    );
    let spare = must(
        call(client, "POST", &path(keys.path), &human, keys.body),
        201,
        "mint a key to revoke",
    );
    let spare_id = spare["key"]["id"].as_str().expect("an id");
    let revoked = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/api-keys/{spare_id}/revoke", f.a.id),
            &human,
            None,
        ),
        200,
        "revoke a key",
    );
    assert!(revoked["key"]["revoked_at"].is_string(), "{revoked}");
}

#[test]
fn every_workspace_route_needs_credentials() {
    let (f, client) = ready();
    for entry in registry() {
        let case = case_for(&entry);
        if !matches!(case.kind, Kind::Workspace) {
            continue;
        }
        for (workspace, record) in [(&f.a, &f.a), (&f.b, &f.b)] {
            let path = fill(&entry.path, workspace, record);
            let response = call(client, &entry.method, &path, &Auth::Nobody, case.body);
            assert_eq!(
                response.status,
                401,
                "{} without credentials: {}",
                route_name(&entry.method, &path),
                response.text()
            );
            assert_eq!(response.json(), json!({ "error": "unauthorized" }));
        }
    }
}

#[test]
fn the_workspace_of_a_request_is_the_one_in_the_path_never_one_in_the_body_query_or_header() {
    let (f, client) = ready();
    let _guard = serial();
    let name = unique("smuggled");
    let body = json!({ "name": name, "workspace_id": f.b.id }).to_string();
    let path = format!("/api/v1/w/{}/projects?workspace_id={}", f.a.id, f.b.id);
    let response = call_with(
        client,
        "POST",
        &path,
        &Auth::Key(&f.a.key),
        &[
            ("X-Workspace-Id", f.b.id.as_str()),
            ("X-Omni-Workspace", f.b.id.as_str()),
        ],
        Some(&body),
    );
    must(response, 201, "create a project");
    let owner: Vec<String> = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select workspace_id::text from projects where name = $1")
            .bind(&name)
            .fetch_all(&mut conn)
            .await
            .expect("look the project up")
    });
    assert_eq!(
        owner,
        [f.a.id.clone()],
        "the project belongs to the path's workspace"
    );

    // The other way: a request to B's path, with A named elsewhere, stays B's
    // business, so A's key still gets 404.
    let path = format!("/api/v1/w/{}/projects?workspace_id={}", f.b.id, f.a.id);
    let response = call_with(
        client,
        "GET",
        &path,
        &Auth::Key(&f.a.key),
        &[("X-Workspace-Id", f.a.id.as_str())],
        None,
    );
    assert_not_found_without_data(&response, &f.b, "A's key on B's path naming A elsewhere");
}

// ---------------------------------------------------------------------------
// Attachments (US-019)
// ---------------------------------------------------------------------------

#[test]
fn the_attachment_routes_are_in_the_registry_and_the_suite_attacks_them() {
    let wanted = [
        (
            "POST",
            "/api/v1/w/{workspace_id}/attachments",
            Access::Agent,
        ),
        (
            "GET",
            "/api/v1/w/{workspace_id}/attachments/{attachment_id}",
            Access::Both,
        ),
    ];
    let routes = registry();
    for (method, path, access) in wanted {
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no {}", route_name(method, path)));
        assert_eq!(entry.access, access, "{}", route_name(method, path));
        assert!(
            matches!(case_for(entry).kind, Kind::Workspace),
            "{} is a workspace route that the suite attacks",
            route_name(method, path)
        );
    }

    let (f, client) = ready();
    let _guard = serial();
    for side in [&f.a, &f.b] {
        assert!(
            !side.attachment_id.is_empty(),
            "the suite seeded no attachment for {}",
            side.id
        );
    }
    let b_folder = format!("ws/{}/attachments/", f.b.id);
    let a_folder = format!("ws/{}/attachments/", f.a.id);
    assert!(
        keys_under(&b_folder).contains(&format!("{b_folder}{}", f.b.attachment_id)),
        "B's file is stored in B's folder"
    );
    assert!(
        keys_under(&a_folder).contains(&format!("{a_folder}{}", f.a.attachment_id)),
        "A's file is stored in A's folder"
    );

    let before = snapshot(&f.b.id);
    let objects = keys_under(&format!("ws/{}/", f.b.id));
    let read_b_file = |workspace: &Side, auth: &Auth| {
        let path = format!(
            "/api/v1/w/{}/attachments/{}",
            workspace.id, f.b.attachment_id
        );
        call(client, "GET", &path, auth, None)
    };
    // A's credentials read B's file neither under B's workspace nor under A's.
    for auth in [
        Auth::Key(&f.a.key),
        Auth::Session(&f.a.token),
        Auth::KeyAndSession(&f.a.key, &f.a.token),
    ] {
        for workspace in [&f.b, &f.a] {
            let what = format!(
                "B's file under workspace {} with {}",
                workspace.id,
                auth.label()
            );
            let response = read_b_file(workspace, &auth);
            assert_not_found_without_data(&response, &f.b, &what);
        }
    }
    // A's credentials cannot put a file into B's workspace.
    let intruder = upload_attachment(client, &f.b.id, &f.a.key, b"intruder", "intruder.txt");
    assert_not_found_without_data(&intruder, &f.b, "A's key uploading into B");
    let intruder = call(
        client,
        "POST",
        &format!("/api/v1/w/{}/attachments", f.b.id),
        &Auth::Session(&f.a.token),
        Some("intruder"),
    );
    assert_not_found_without_data(&intruder, &f.b, "A's session uploading into B");
    assert_eq!(snapshot(&f.b.id), before, "an attack changed B's rows");
    assert_eq!(
        keys_under(&format!("ws/{}/", f.b.id)),
        objects,
        "an attack changed B's files"
    );

    // The control: the owner uploads and reads, and the file lands in its folder.
    let name = unique("owner") + ".txt";
    let uploaded = must(
        upload_attachment(client, &f.a.id, &f.a.key, name.as_bytes(), &name),
        201,
        "upload a file",
    );
    let id = uploaded["attachment"]["id"].as_str().expect("an id");
    assert!(keys_under(&a_folder).contains(&format!("{a_folder}{id}")));
    let path = format!("/api/v1/w/{}/attachments/{id}", f.a.id);
    for auth in [Auth::Key(&f.a.key), Auth::Session(&f.a.token)] {
        let response = call(client, "GET", &path, &auth, None);
        assert_eq!(response.status, 200, "read the file: {}", response.text());
        assert_eq!(response.text(), name, "{}", auth.label());
    }
}

// ---------------------------------------------------------------------------
// Human authority
// ---------------------------------------------------------------------------

#[test]
fn every_human_route_answers_an_api_key_with_403_and_changes_nothing() {
    let (f, client) = ready();
    let _guard = serial();
    let before = (snapshot(&f.a.id), snapshot(&f.b.id));
    let nobody = Side::stranger(UNKNOWN_ID);
    let garbage = Side::stranger("not-a-uuid");
    let mut attacks = 0;
    let mut routes = 0;
    for entry in registry() {
        if entry.access != Access::Human {
            continue;
        }
        let exempt = KEY_EXEMPT
            .iter()
            .any(|(m, p)| *m == entry.method && *p == entry.path);
        if exempt {
            continue;
        }
        let case = case_for(&entry);
        routes += 1;
        // The paths: the key's own workspace and records, the other
        // workspace's, ones that do not exist, and ones that are no UUIDs. The
        // authority check comes first, so all of them are 403.
        let paths = [
            fill(&entry.path, &f.a, &f.a),
            fill(&entry.path, &f.b, &f.b),
            fill(&entry.path, &f.a, &f.b),
            fill(&entry.path, &nobody, &nobody),
            fill(&entry.path, &garbage, &garbage),
        ];
        for path in paths {
            let callers = [
                Auth::Key(&f.a.key),
                Auth::Key(&f.b.key),
                // Next to a valid cookie, the key still decides: this is an agent.
                Auth::KeyAndSession(&f.a.key, &f.a.token),
            ];
            for auth in callers {
                let response = call(client, &entry.method, &path, &auth, case.body);
                let what = format!("{} with {}", route_name(&entry.method, &path), auth.label());
                assert_eq!(response.status, 403, "{what}: {}", response.text());
                assert_eq!(response.json(), json!({ "error": "forbidden" }), "{what}");
                attacks += 1;
            }
        }
    }
    assert!(routes >= 5, "only {routes} human routes were attacked");
    assert!(attacks >= 60, "only {attacks} attacks ran");

    // Nothing happened: no row changed, the human's session lives on and the
    // key the attacks used was not revoked.
    assert_eq!(
        (snapshot(&f.a.id), snapshot(&f.b.id)),
        before,
        "a refused call changed rows"
    );
    let me = call(
        client,
        "GET",
        "/api/v1/auth/me",
        &Auth::Session(&f.a.token),
        None,
    );
    assert_eq!(me.status, 200, "alice's session survived: {}", me.text());
    let whoami = format!("/api/v1/w/{}/whoami", f.a.id);
    let response = call(client, "GET", &whoami, &Auth::Key(&f.a.key), None);
    assert_eq!(
        response.status,
        200,
        "A's key survived: {}",
        response.text()
    );
}

#[test]
fn the_login_is_the_only_human_route_that_an_api_key_does_not_turn_down() {
    // The exemption list is closed: a new human route that a key could use
    // would need a decision here.
    assert_eq!(KEY_EXEMPT, [("POST", "/api/v1/auth/login")]);
    let human_routes: Vec<String> = registry()
        .into_iter()
        .filter(|r| r.access == Access::Human)
        .map(|r| route_name(&r.method, &r.path))
        .collect();
    assert!(
        human_routes.contains(&"POST /api/v1/auth/login".to_string()),
        "{human_routes:?}"
    );
    let (f, client) = ready();
    let body = json!({ "email": f.a.email, "password": "not the password" }).to_string();
    let response = call(
        client,
        "POST",
        "/api/v1/auth/login",
        &Auth::Key(&f.a.key),
        Some(&body),
    );
    assert_ne!(
        response.status,
        403,
        "the login checks the password: {}",
        response.text()
    );
}

// ---------------------------------------------------------------------------
// WebSockets
// ---------------------------------------------------------------------------

/// The answer to a handshake that did not open the socket.
#[derive(Debug)]
struct Refused {
    status: u16,
    body: String,
}

struct Ws {
    stream: TcpStream,
    buffer: Vec<u8>,
}

enum Arrived {
    Data(Vec<u8>),
    Timeout,
    Eof,
}

impl Ws {
    fn open(client: &Client, path: &str, auth: &Auth) -> Result<Ws, Refused> {
        let stream = TcpStream::connect(("127.0.0.1", client.port())).expect("connect");
        let mut ws = Ws {
            stream,
            buffer: Vec::new(),
        };
        let mut head = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: {HANDSHAKE_KEY}\r\n\
             Sec-WebSocket-Version: 13\r\n"
        );
        for (name, value) in auth.headers() {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        ws.stream
            .write_all(head.as_bytes())
            .expect("send handshake");

        let deadline = Instant::now() + PATIENCE;
        let split = loop {
            if let Some(at) = ws.buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            match ws.read_some(deadline) {
                Arrived::Data(bytes) => ws.buffer.extend(bytes),
                _ => panic!(
                    "no complete handshake answer for {path}: {:?}",
                    String::from_utf8_lossy(&ws.buffer)
                ),
            }
        };
        let head = String::from_utf8_lossy(&ws.buffer[..split]).into_owned();
        let status: u16 = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .expect("status line");
        ws.buffer.drain(..split + 4);
        if status == 101 {
            return Ok(ws);
        }
        let length: usize = head
            .lines()
            .filter_map(|l| l.split_once(':'))
            .find(|(n, _)| n.trim().eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.trim().parse().ok())
            .unwrap_or(0);
        while ws.buffer.len() < length {
            match ws.read_some(deadline) {
                Arrived::Data(bytes) => ws.buffer.extend(bytes),
                _ => break,
            }
        }
        Err(Refused {
            status,
            body: String::from_utf8_lossy(&ws.buffer).into_owned(),
        })
    }

    /// Reads whatever arrives before `deadline`.
    fn read_some(&mut self, deadline: Instant) -> Arrived {
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        self.stream
            .set_read_timeout(Some(wait))
            .expect("set the read timeout");
        let mut chunk = [0u8; 8192];
        match self.stream.read(&mut chunk) {
            Ok(0) => Arrived::Eof,
            Ok(n) => Arrived::Data(chunk[..n].to_vec()),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Arrived::Timeout
            }
            Err(_) => Arrived::Eof,
        }
    }

    /// One complete frame off the front of the buffer, as (opcode, payload).
    fn take_frame(&mut self) -> Option<(u8, Vec<u8>)> {
        let b = &self.buffer;
        if b.len() < 2 {
            return None;
        }
        let opcode = b[0] & 0x0f;
        let mut length = (b[1] & 0x7f) as usize;
        let mut at = 2;
        if length == 126 {
            if b.len() < 4 {
                return None;
            }
            length = u16::from_be_bytes([b[2], b[3]]) as usize;
            at = 4;
        } else if length == 127 {
            if b.len() < 10 {
                return None;
            }
            length = u64::from_be_bytes(b[2..10].try_into().unwrap()) as usize;
            at = 10;
        }
        // The server's frames are not masked.
        if b.len() < at + length {
            return None;
        }
        let payload = b[at..at + length].to_vec();
        self.buffer.drain(..at + length);
        Some((opcode, payload))
    }

    fn send(&mut self, message: &Value) {
        let payload = message.to_string().into_bytes();
        let mask = [0x12u8, 0x34, 0x56, 0x78];
        let mut frame = vec![0x81];
        match payload.len() {
            n if n < 126 => frame.push(0x80 | n as u8),
            n => {
                frame.push(0x80 | 126);
                frame.extend((n as u16).to_be_bytes());
            }
        }
        frame.extend(mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.stream.write_all(&frame).expect("send a frame");
    }

    /// The next JSON message from the server, `None` after `wait` of silence or
    /// when the socket closes.
    fn next(&mut self, wait: Duration) -> Option<Value> {
        let deadline = Instant::now() + wait;
        loop {
            while let Some((opcode, payload)) = self.take_frame() {
                match opcode {
                    1 => {
                        let text = String::from_utf8_lossy(&payload).into_owned();
                        return Some(
                            serde_json::from_str(&text)
                                .unwrap_or_else(|e| panic!("a text frame is JSON ({e}): {text}")),
                        );
                    }
                    8 => return None,
                    _ => {}
                }
            }
            match self.read_some(deadline) {
                Arrived::Data(bytes) => self.buffer.extend(bytes),
                Arrived::Timeout if Instant::now() < deadline => {}
                Arrived::Timeout | Arrived::Eof => return None,
            }
        }
    }

    /// Every message that arrives until the socket has been quiet for `QUIET`.
    fn drain(&mut self) -> Vec<Value> {
        let mut found = Vec::new();
        while let Some(message) = self.next(QUIET) {
            found.push(message);
        }
        found
    }
}

fn assert_refused(result: Result<Ws, Refused>, status: u16, code: &str, what: &str) {
    let refused = match result {
        Ok(_) => panic!("{what}: the socket opened, but {status} {code} was expected"),
        Err(refused) => refused,
    };
    assert_eq!(refused.status, status, "{what}: {refused:?}");
    let body: Value = serde_json::from_str(&refused.body)
        .unwrap_or_else(|e| panic!("{what}: the refusal is JSON ({e}): {refused:?}"));
    assert_eq!(body, json!({ "error": code }), "{what}");
}

#[test]
fn the_channel_of_workspace_b_is_refused_to_the_credentials_of_workspace_a() {
    let (f, client) = ready();
    let routes = websocket_registry();
    assert!(!routes.is_empty(), "the router has WebSocket routes");
    for route in routes {
        let theirs = fill(&route.path, &f.b, &f.b);
        let own = fill(&route.path, &f.a, &f.a);
        for auth in [
            Auth::Key(&f.a.key),
            Auth::Session(&f.a.token),
            Auth::KeyAndSession(&f.a.key, &f.a.token),
        ] {
            let what = format!("{theirs} with {} of workspace A", auth.label());
            assert_refused(Ws::open(client, &theirs, &auth), 404, "not_found", &what);
            // The control: the same credentials open the channel of A.
            assert!(
                Ws::open(client, &own, &auth).is_ok(),
                "{own} with {} should open",
                auth.label()
            );
        }
        // The channel is the one in the path, whatever the query says.
        let sneaky = format!("{own}?workspace_id={}", f.b.id);
        assert!(Ws::open(client, &sneaky, &Auth::Key(&f.a.key)).is_ok());
        let sneaky = format!("{theirs}?workspace_id={}", f.a.id);
        assert_refused(
            Ws::open(client, &sneaky, &Auth::Key(&f.a.key)),
            404,
            "not_found",
            "B's channel with A named in the query",
        );
        assert_refused(
            Ws::open(client, &theirs, &Auth::Nobody),
            401,
            "unauthorized",
            "B's channel without credentials",
        );
    }
}

#[test]
fn a_socket_of_workspace_a_cannot_subscribe_to_a_project_of_workspace_b() {
    let (f, client) = ready();
    let _guard = serial();
    let channel = format!("/ws/w/{}", f.a.id);
    let mut ws = Ws::open(client, &channel, &Auth::Key(&f.a.key)).expect("A's channel opens");

    for after in [None, Some(0)] {
        let mut message = json!({ "type": "subscribe", "project_id": f.b.project_id });
        if let Some(after) = after {
            message["after"] = json!(after);
        }
        ws.send(&message);
        let reply = ws.next(PATIENCE).expect("the server answers");
        assert_eq!(
            reply,
            json!({ "type": "error", "error": "not_found" }),
            "subscribing to B's project (after {after:?})"
        );
    }

    // Workspace B goes on working, and none of it reaches the socket of A,
    // which also holds a subscription of its own to prove it is listening.
    ws.send(&json!({ "type": "subscribe", "project_id": f.a.project_id }));
    let ack = ws.next(PATIENCE).expect("the server answers");
    assert_eq!(ack["type"], "subscribed", "A's own project: {ack}");
    let created = call(
        client,
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{}/changesets",
            f.b.id, f.b.project_id
        ),
        &Auth::Key(&f.b.key),
        Some(&json!({ "title": unique("b-only") }).to_string()),
    );
    must(created, 201, "B's agent creates a change set");
    let heard = ws.drain();
    assert!(
        heard.is_empty(),
        "the socket of workspace A heard of workspace B: {heard:?}"
    );

    // The control: a change in A is heard.
    let created = call(
        client,
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{}/changesets",
            f.a.id, f.a.project_id
        ),
        &Auth::Key(&f.a.key),
        Some(&json!({ "title": unique("a-only") }).to_string()),
    );
    must(created, 201, "A's agent creates a change set");
    let event = ws.next(PATIENCE).expect("A hears its own event");
    assert_eq!(event["type"], "event", "{event}");
    assert_eq!(event["event"], "changeset.open", "{event}");
}

// ---------------------------------------------------------------------------
// Row-level security at the database
// ---------------------------------------------------------------------------

fn sqlstate(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .and_then(|e| e.code().map(|c| c.into_owned()))
        .unwrap_or_default()
}

/// Runs `sql` as the API role with `workspace` set, in a transaction that is
/// rolled back: the rows it changed, or the SQLSTATE of the error.
async fn attempt(conn: &mut PgConnection, workspace: &str, sql: &str) -> Result<u64, String> {
    let mut tx = conn.begin().await.expect("begin");
    sqlx::query("select set_config('app.workspace_id', $1, true)")
        .bind(workspace)
        .execute(&mut *tx)
        .await
        .expect("set app.workspace_id");
    let outcome = sqlx::query(sql)
        .execute(&mut *tx)
        .await
        .map(|done| done.rows_affected())
        .map_err(|e| format!("{} ({e})", sqlstate(&e)));
    drop(tx);
    outcome
}

/// The tenant tables, each with rows of both workspaces: a table whose rows the
/// suite did not seed would make every check below vacuous.
async fn seeded_tenant_tables(a: &str, b: &str) -> Vec<String> {
    let mut admin = admin_connection().await;
    let tables = tenant_tables(&mut admin).await;
    for table in &tables {
        for workspace in [a, b] {
            assert!(
                rows_of(&mut admin, table, workspace).await > 0,
                "the suite seeded no row of {table} for workspace {workspace}: \
                 make fixture() in tests/isolation.rs create one"
            );
        }
    }
    tables
}

#[test]
fn row_level_security_shows_workspace_a_none_of_the_rows_of_b_in_any_tenant_table() {
    let f = fixture();
    let _guard = serial();
    block(async {
        let tables = seeded_tenant_tables(&f.a.id, &f.b.id).await;
        assert!(tables.len() >= 7, "the tenant tables: {tables:?}");
        let mut admin = admin_connection().await;
        let mut conn = api_connection().await;
        for (own, other) in [(&f.a.id, &f.b.id), (&f.b.id, &f.a.id)] {
            for table in &tables {
                let column = scope_column(table);
                let mut tx = conn.begin().await.expect("begin");
                sqlx::query("select set_config('app.workspace_id', $1, true)")
                    .bind(own)
                    .execute(&mut *tx)
                    .await
                    .expect("set app.workspace_id");
                let all: i64 = sqlx::query_scalar(&format!("select count(*) from {table}"))
                    .fetch_one(&mut *tx)
                    .await
                    .unwrap_or_else(|e| panic!("{table}: {e}"));
                let foreign: i64 = sqlx::query_scalar(&format!(
                    "select count(*) from {table} where {column} = '{other}'::uuid"
                ))
                .fetch_one(&mut *tx)
                .await
                .unwrap_or_else(|e| panic!("{table}: {e}"));
                drop(tx);
                assert_eq!(foreign, 0, "{table}: rows of {other} visible to {own}");
                assert_eq!(
                    all,
                    rows_of(&mut admin, table, own).await,
                    "{table}: {own} sees exactly its own rows"
                );
            }
        }
    });
}

#[test]
fn without_a_workspace_set_the_api_role_sees_no_row_of_any_tenant_table() {
    let f = fixture();
    block(async {
        let tables = seeded_tenant_tables(&f.a.id, &f.b.id).await;
        let mut conn = api_connection().await;
        for table in &tables {
            let rows: i64 = sqlx::query_scalar(&format!("select count(*) from {table}"))
                .fetch_one(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("{table}: {e}"));
            assert_eq!(rows, 0, "{table}: rows visible without a workspace");
        }
    });
}

#[test]
fn row_level_security_stops_workspace_a_from_changing_inserting_moving_or_deleting_rows_of_b() {
    let f = fixture();
    let _guard = serial();
    let before = (snapshot(&f.a.id), snapshot(&f.b.id));
    block(async {
        let tables = seeded_tenant_tables(&f.a.id, &f.b.id).await;
        let mut admin = admin_connection().await;
        let mut conn = api_connection().await;
        let (a, b) = (&f.a.id, &f.b.id);
        for table in &tables {
            let column = scope_column(table);

            // Updates of B's rows touch nothing: the rows are not there for A,
            // or A may not update the table at all.
            let sql =
                format!("update {table} set {column} = {column} where {column} = '{b}'::uuid");
            match attempt(&mut conn, a, &sql).await {
                Ok(rows) => assert_eq!(rows, 0, "{table}: A updated {rows} rows of B"),
                Err(code) => assert!(code.starts_with("42501"), "{table}: {code}"),
            }

            // The control for the line above: A's own rows can be updated
            // wherever the API role has the privilege.
            let may_update: bool =
                sqlx::query_scalar("select has_table_privilege('omni_api', $1, 'UPDATE')")
                    .bind(format!("public.{table}"))
                    .fetch_one(&mut admin)
                    .await
                    .expect("look up the privilege");
            if may_update {
                let sql =
                    format!("update {table} set {column} = {column} where {column} = '{a}'::uuid");
                let own = rows_of(&mut admin, table, a).await;
                match attempt(&mut conn, a, &sql).await {
                    // Append-only tables refuse every update with a trigger.
                    Err(message) => assert!(
                        message.contains("append-only"),
                        "{table}: the control update was refused: {message}"
                    ),
                    Ok(rows) => assert_eq!(rows as i64, own, "{table}: A's own rows"),
                }
            }

            // A row of A cannot be moved to B.
            let sql =
                format!("update {table} set {column} = '{b}'::uuid where {column} = '{a}'::uuid");
            let moved = attempt(&mut conn, a, &sql).await;
            assert!(moved.is_err(), "{table}: A moved rows to B: {moved:?}");

            // A copy of a row of A cannot be inserted as a row of B.
            let (workspace_key, id_key) = ("{workspace_id}", "{id}");
            let sql = if table == "workspaces" {
                format!(
                    "insert into workspaces select (jsonb_populate_record(null::workspaces, \
                     jsonb_set(to_jsonb(r), '{id_key}', to_jsonb('{b}'::text)))).* \
                     from workspaces r where r.id = '{a}'::uuid"
                )
            } else {
                format!(
                    "insert into {table} select (jsonb_populate_record(null::{table}, \
                     jsonb_set(jsonb_set(to_jsonb(r), '{workspace_key}', to_jsonb('{b}'::text)), \
                     '{id_key}', to_jsonb(uuid_v7()::text)))).* \
                     from {table} r where r.workspace_id = '{a}'::uuid limit 1"
                )
            };
            let inserted = attempt(&mut conn, a, &sql).await;
            match &inserted {
                Err(code) => assert!(code.starts_with("42501"), "{table}: {code}"),
                Ok(rows) => panic!("{table}: A inserted {rows} rows into B"),
            }

            // And no row can be deleted, whose it is.
            for owner in [a, b] {
                let sql = format!("delete from {table} where {column} = '{owner}'::uuid");
                let deleted = attempt(&mut conn, a, &sql).await;
                assert!(
                    deleted.is_err(),
                    "{table}: A deleted rows of {owner}: {deleted:?}"
                );
            }
        }
    });
    assert_eq!(
        (snapshot(&f.a.id), snapshot(&f.b.id)),
        before,
        "the attempts changed rows"
    );
}

#[test]
fn a_connection_that_set_workspace_a_for_one_transaction_has_forgotten_it_in_the_next() {
    let f = fixture();
    block(async {
        let tables = seeded_tenant_tables(&f.a.id, &f.b.id).await;
        let mut conn = api_connection().await;
        let mut tx = conn.begin().await.expect("begin");
        sqlx::query("select set_config('app.workspace_id', $1, true)")
            .bind(&f.a.id)
            .execute(&mut *tx)
            .await
            .expect("set app.workspace_id");
        let seen: i64 = sqlx::query_scalar("select count(*) from projects")
            .fetch_one(&mut *tx)
            .await
            .expect("count");
        assert!(seen > 0, "the transaction sees its workspace");
        tx.commit().await.expect("commit");
        for table in &tables {
            let rows: i64 = sqlx::query_scalar(&format!("select count(*) from {table}"))
                .fetch_one(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("{table}: {e}"));
            assert_eq!(
                rows, 0,
                "{table}: the finished transaction's workspace lingers"
            );
        }
    });
}
