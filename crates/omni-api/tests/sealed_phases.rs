//! US-035: sealed phases, through every read path of the API. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. Several
//! agents share the key and tell themselves apart by their identity headers.
//! The tests speak WebSocket to the API with the small client of
//! tests/live_events.rs.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; threads, phases and posts
//! are those of US-033 and US-034, tests/threads.rs and tests/posts.rs, and
//! live events those of US-017, tests/live_events.rs):
//!
//! - An agent identity is the run and role of a request (`X-Omni-Run` and
//!   `X-Omni-Role`), as for claims and `replaces`; its model and harness do
//!   not count. A WebSocket opened with an API key takes the identity from the
//!   same headers of its handshake.
//! - A post is visible to an agent identity when the human wrote it, when the
//!   identity wrote it, when its phase is not the thread's current phase (an
//!   earlier phase, whatever phase followed it), when its phase is open, or
//!   when the identity has posted in that phase itself. Every other post (one
//!   by another agent in the current, sealed phase) is hidden from it. Nothing
//!   is cached: the next read after the agent posted, or after the next phase
//!   started, shows what was hidden. The human sees every post.
//! - Every read path keeps to this, for an agent:
//!   - `GET /threads/{thread_id}` (both) answers `{"thread": Thread}`, where
//!     the Thread of US-033 also has `posts`: the posts the caller may see,
//!     oldest first, each a Post of US-034.
//!   - `GET /threads/{thread_id}/digest` (both) answers `{"posts": [...]}`: one
//!     entry per post the caller may see, oldest first, each with at least
//!     `{"id", "phase", "type", "author", "summary"}` and without the body
//!     (`body` and `body_html` are left out). An unknown thread is 404.
//!   - `GET /posts/{post_id}` (both) answers 200 `{"post": Post}`. A post the
//!     caller may not see is 404 `{"error": "not_found"}`, the very answer for
//!     a post that does not exist or is of another project.
//!   - Links: a post that links to a post its author may not see is refused
//!     like a link to a post that does not exist (400 `invalid_request`,
//!     nothing recorded). In what an agent reads (the thread and single post
//!     reads), a `{"kind": "post"}` link to a post it may not see is left out
//!     of `links`; its other links stay, in order.
//!   - Live events: no event that names a post the agent may not see (its
//!     `post_id`) reaches the agent's socket, neither live nor when it resumes
//!     with `after`. Once the post is visible to the agent, resuming from
//!     before it replays its `post.created`. The human's socket gets every
//!     event.
//!   - `omni wait --thread` (US-036) reads through these routes, so its
//!     results keep to the same rule.
//!
//!   A hidden post's ID, summary and body appear nowhere in an answer to the
//!   agent. Which identities have posted may still be told (US-036 waits for
//!   roles); these tests do not pin that either way.
//! - The story adds no tenant table: what an agent may see follows from the
//!   posts and phases of US-033 and US-034.
//!
//! The follow-ups of US-055:
//!
//! - A live event that was withheld from an agent's open socket is delivered
//!   to it, once, as soon as its post becomes visible to the agent (it posted
//!   in the phase, or a later phase started), even though the socket already
//!   got events with higher sequence numbers. A resume from the newest
//!   sequence number the agent had got before then includes it too. Neither
//!   pins where in the stream it arrives.
//! - Agent requests that read or write posts (the thread read, the digest,
//!   the single post read and posting) need both `X-Omni-Run` and
//!   `X-Omni-Role`; an API key without both is answered 400, so agents
//!   without an identity never share one view of a sealed phase. The error
//!   code is not pinned. The human needs no identity. The workspace socket,
//!   whose events name posts, is such a read too: a handshake with an API key
//!   and without both headers is answered 400 instead of switching protocols.
//!   Decisions are left to US-056.
//! - A directive of the human that links to a post hidden from the agent
//!   gives away neither the link nor the post's ID in any answer to the
//!   agent: the thread read, the `directives` of a change set request, the
//!   single post read, the digest and the live events.
//! - The live-event filter compares the post IDs as UUIDs within the
//!   workspace, so it can use the index of posts: no `::text` cast on a post
//!   ID in `src/live.rs`.
//!
//! Sealed decisions (US-056, Q50). Decisions are those of US-037
//! (tests/decisions.rs):
//!
//! - A decision proposed while its thread's current phase is sealed belongs
//!   to that phase. It is hidden from an agent identity that has not posted in
//!   that phase (posting in an earlier phase does not count) until the
//!   identity posts there or a later phase starts, sealed or open; then its
//!   next read shows it. The human sees every decision. A decision proposed in
//!   an open phase is seen by every agent, also once a sealed phase follows.
//! - Hidden looks exactly like no decision, on every read path:
//!   `GET /decisions/{decision_id}` (what `omni decision show` reads) is 404
//!   `{"error": "not_found"}`, the answer for an unknown decision; the
//!   Thread's `decision` is `null`; a Changeset's `decisions` (show and list)
//!   leaves it out and keeps the other linked decisions in order; linking a
//!   change set to it is 404 `not_found`, as for an unknown decision and
//!   linking nothing; no live event that names it (`data.decision_id`)
//!   reaches the agent's socket, neither live nor on a resume. The hidden
//!   decision's ID and direction appear nowhere in an answer to the agent:
//!   not in the thread read, the digest or a change set read either.
//! - Once the decision is visible, the events of it that were withheld from
//!   an open socket (`decision.proposed`, and `decision.approved` when the
//!   human approved it meanwhile) are delivered to it, once each, and a
//!   resume from the newest sequence number the agent had got includes them.
//! - Not pinned: what an agent sees when a hidden decision supersedes one it
//!   could see, and whether a proposer that has not posted in the phase sees
//!   its own decision (the proposers of these tests have posted).

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
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const TEST_DB: &str = "omni_us035_sealed_phases";
const TEST_BUCKET: &str = "omni-us035-sealed-phases";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const THREAD_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}";
const DIGEST_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/digest";
const POST_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/posts/{post_id}";
/// The handshake key of RFC 6455's example, whose accept value is fixed.
const HANDSHAKE_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
const HANDSHAKE_ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";
/// How long a test waits for something that must happen.
const PATIENCE: Duration = Duration::from_secs(5);
/// How long a test waits to be sure that something does not happen.
const QUIET: Duration = Duration::from_millis(1500);

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
    format!("us035-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

/// Who sends a request: an agent (the API key with a run and role) or the
/// human (a session cookie).
#[derive(Clone, Copy)]
enum Auth<'a> {
    Agent {
        key: &'a str,
        run: &'a str,
        role: &'a str,
    },
    Session(&'a str),
    /// The API key with a run, a role, both or neither: an identity is only
    /// complete with both.
    Partial {
        key: &'a str,
        run: Option<&'a str>,
        role: Option<&'a str>,
    },
}

impl Auth<'_> {
    fn headers(&self) -> Vec<(&'static str, String)> {
        match self {
            Auth::Agent { key, run, role } => vec![
                ("Authorization", format!("Bearer {key}")),
                ("X-Omni-Run", run.to_string()),
                ("X-Omni-Role", role.to_string()),
                ("X-Omni-Model", "test-model".to_string()),
                ("X-Omni-Harness", "test-harness".to_string()),
            ],
            Auth::Session(token) => vec![("Cookie", cookie(token))],
            Auth::Partial { key, run, role } => {
                let mut headers = vec![("Authorization", format!("Bearer {key}"))];
                if let Some(run) = run {
                    headers.push(("X-Omni-Run", run.to_string()));
                }
                if let Some(role) = role {
                    headers.push(("X-Omni-Role", role.to_string()));
                }
                headers.push(("X-Omni-Model", "test-model".to_string()));
                headers.push(("X-Omni-Harness", "test-harness".to_string()));
                headers
            }
        }
    }
}

fn agent_as<'a>(f: &'a Fixture, run: &'a str, role: &'a str) -> Auth<'a> {
    Auth::Agent {
        key: &f.key,
        run,
        role,
    }
}

/// The agent that starts threads and phases: `run-1/moderator`.
fn moderator(f: &Fixture) -> Auth<'_> {
    agent_as(f, "run-1", "moderator")
}

/// The agent whose posts the others must not see too early: `run-1/critic`.
fn critic(f: &Fixture) -> Auth<'_> {
    agent_as(f, "run-1", "critic")
}

/// The agent that reads: `run-1/scout`.
fn scout(f: &Fixture) -> Auth<'_> {
    agent_as(f, "run-1", "scout")
}

fn human(f: &Fixture) -> Auth<'_> {
    Auth::Session(&f.alice_token)
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let owned = auth.headers();
    let headers: Vec<(&str, &str)> = owned.iter().map(|(n, v)| (*n, v.as_str())).collect();
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

fn id_of(value: &Value) -> String {
    value["id"]
        .as_str()
        .unwrap_or_else(|| panic!("there is an `id`: {value}"))
        .to_string()
}

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
}

fn thread_path(f: &Fixture, project: &str, thread: &str) -> String {
    format!("{}/threads/{thread}", project_base(f, project))
}

fn post_path(f: &Fixture, project: &str, post: &str) -> String {
    format!("{}/posts/{post}", project_base(f, project))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &moderator(f),
                Some(&json!({ "name": unique("sealed-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

/// A project with a change set and a thread whose first phase is sealed or
/// open: (project, thread).
fn thread_in_new_project(f: &Fixture, client: &Client, sealed: bool) -> (String, String) {
    let project = new_project(f, client);
    let changeset = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/changesets", project_base(f, &project)),
                &moderator(f),
                Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
            ),
            201,
            "create a change set",
        )["changeset"],
    );
    let thread = id_of(
        &must(
            call(
                client,
                "POST",
                &format!(
                    "{}/changesets/{changeset}/threads",
                    project_base(f, &project)
                ),
                &moderator(f),
                Some(&json!({
                    "question": unique("Which hero layout?"),
                    "criteria": ["Readable at 320 px"],
                    "sealed": sealed
                })),
            ),
            201,
            "start a thread",
        )["thread"],
    );
    (project, thread)
}

fn start_phase(f: &Fixture, client: &Client, project: &str, thread: &str, sealed: bool) {
    must(
        call(
            client,
            "POST",
            &format!("{}/phases", thread_path(f, project, thread)),
            &moderator(f),
            Some(&json!({ "sealed": sealed })),
        ),
        201,
        "start the next phase",
    );
}

/// What a test knows of a post it made: its ID and the texts that must not
/// show to an agent that may not see it.
#[derive(Clone)]
struct Made {
    id: String,
    summary: String,
    body: String,
}

/// Posts a note as `auth` with a unique summary and body; `extra` is merged
/// into the request (e.g. `links`).
fn post_with(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    label: &str,
    extra: Value,
) -> Made {
    let summary = unique(&format!("{label} summary"));
    let body = unique(&format!("{label} body"));
    let mut request = json!({ "type": "position", "summary": summary, "body": body });
    if let Value::Object(extra) = extra {
        for (name, value) in extra {
            request[name] = value;
        }
    }
    let posted = must(
        call(
            client,
            "POST",
            &format!("{}/posts", thread_path(f, project, thread)),
            auth,
            Some(&request),
        ),
        201,
        &format!("post {request}"),
    );
    Made {
        id: id_of(&posted["post"]),
        summary,
        body,
    }
}

fn post(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    label: &str,
) -> Made {
    post_with(f, client, auth, project, thread, label, json!({}))
}

fn read_thread(f: &Fixture, client: &Client, auth: &Auth, project: &str, thread: &str) -> Response {
    call(client, "GET", &thread_path(f, project, thread), auth, None)
}

fn read_digest(f: &Fixture, client: &Client, auth: &Auth, project: &str, thread: &str) -> Response {
    call(
        client,
        "GET",
        &format!("{}/digest", thread_path(f, project, thread)),
        auth,
        None,
    )
}

fn read_post(f: &Fixture, client: &Client, auth: &Auth, project: &str, post: &str) -> Response {
    call(client, "GET", &post_path(f, project, post), auth, None)
}

/// The posts of a thread read, as the caller sees them.
fn thread_posts(answer: &Value) -> Vec<Value> {
    answer["thread"]["posts"]
        .as_array()
        .unwrap_or_else(|| panic!("the thread has a list of `posts`: {answer}"))
        .clone()
}

/// The IDs of the posts of a thread read, oldest first.
fn seen_in_thread(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
) -> Vec<String> {
    let answer = must(
        read_thread(f, client, auth, project, thread),
        200,
        "read the thread",
    );
    thread_posts(&answer).iter().map(id_of).collect()
}

/// The entries of a digest.
fn digest_posts(answer: &Value) -> Vec<Value> {
    answer["posts"]
        .as_array()
        .unwrap_or_else(|| panic!("the digest has a list of `posts`: {answer}"))
        .clone()
}

fn ids_of(made: &[&Made]) -> Vec<String> {
    made.iter().map(|m| m.id.clone()).collect()
}

/// Neither the ID, the summary nor the body of `hidden` is anywhere in `text`.
fn assert_nothing_of(text: &str, hidden: &Made, what: &str) {
    for (name, marker) in [
        ("ID", &hidden.id),
        ("summary", &hidden.summary),
        ("body", &hidden.body),
    ] {
        assert!(
            !text.contains(marker.as_str()),
            "{what}: the {name} of a hidden post shows: {text}"
        );
    }
}

fn assert_not_found(response: &Response, what: &str) {
    assert_eq!(response.status, 404, "{what}: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "not_found" }),
        "{what}: the answer carries the code and nothing else"
    );
}

/// How many posts the thread has, as the admin role sees them.
fn posts_in(thread: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from posts where thread_id = $1::uuid")
            .bind(thread)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the thread's posts: {e}"))
    })
}

// ---------------------------------------------------------------------------
// A small WebSocket client (as in tests/live_events.rs)
// ---------------------------------------------------------------------------

/// What a read of the stream found.
enum Arrived {
    Data(Vec<u8>),
    Timeout,
    Eof,
}

/// What the next look at the socket found.
enum Frame {
    Message(Value),
    Silence,
    Closed,
}

struct Ws {
    stream: TcpStream,
    buffer: Vec<u8>,
}

impl Ws {
    /// Opens the socket of the workspace as `auth`, which must work.
    fn open(client: &Client, workspace: &str, auth: &Auth) -> Ws {
        match Ws::try_open(client, workspace, auth) {
            Ok(ws) => ws,
            Err(head) => panic!("the socket should open: {head}"),
        }
    }

    /// Opens the socket of the workspace as `auth`, or gives the head and
    /// body of the answer when the server does not switch protocols.
    fn try_open(client: &Client, workspace: &str, auth: &Auth) -> Result<Ws, String> {
        let path = format!("/ws/w/{workspace}");
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
        if !head.starts_with("HTTP/1.1 101") {
            // Whatever of the body arrives soon is enough to tell the error.
            let deadline = Instant::now() + Duration::from_millis(500);
            while let Arrived::Data(bytes) = ws.read_some(deadline) {
                ws.buffer.extend(bytes);
            }
            return Err(String::from_utf8_lossy(&ws.buffer).into_owned());
        }
        assert!(
            head.contains(HANDSHAKE_ACCEPT),
            "the accept value of the handshake: {head}"
        );
        ws.buffer.drain(..split + 4);
        Ok(ws)
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
        let masked = b[1] & 0x80 != 0;
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
        let mask = if masked {
            if b.len() < at + 4 {
                return None;
            }
            let mask = [b[at], b[at + 1], b[at + 2], b[at + 3]];
            at += 4;
            Some(mask)
        } else {
            None
        };
        if b.len() < at + length {
            return None;
        }
        let mut payload = b[at..at + length].to_vec();
        if let Some(mask) = mask {
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[i % 4];
            }
        }
        self.buffer.drain(..at + length);
        Some((opcode, payload))
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) {
        let mask = [0x12u8, 0x34, 0x56, 0x78];
        let mut frame = vec![0x80 | opcode];
        match payload.len() {
            n if n < 126 => frame.push(0x80 | n as u8),
            n if n <= u16::MAX as usize => {
                frame.push(0x80 | 126);
                frame.extend((n as u16).to_be_bytes());
            }
            n => {
                frame.push(0x80 | 127);
                frame.extend((n as u64).to_be_bytes());
            }
        }
        frame.extend(mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.stream.write_all(&frame).expect("send a frame");
    }

    fn send(&mut self, message: &Value) {
        self.send_frame(1, message.to_string().as_bytes());
    }

    /// The next message from the server, waiting at most `wait`.
    fn next(&mut self, wait: Duration) -> Frame {
        let deadline = Instant::now() + wait;
        loop {
            while let Some((opcode, payload)) = self.take_frame() {
                match opcode {
                    1 => {
                        let text = String::from_utf8_lossy(&payload).into_owned();
                        let message = serde_json::from_str(&text)
                            .unwrap_or_else(|e| panic!("a text frame is JSON ({e}): {text}"));
                        return Frame::Message(message);
                    }
                    8 => return Frame::Closed,
                    9 => self.send_frame(10, &payload),
                    _ => {}
                }
            }
            match self.read_some(deadline) {
                Arrived::Data(bytes) => self.buffer.extend(bytes),
                Arrived::Timeout => {
                    if Instant::now() >= deadline {
                        return Frame::Silence;
                    }
                }
                Arrived::Eof => return Frame::Closed,
            }
        }
    }

    /// The next `event` message within `wait`; other messages are skipped.
    fn next_event(&mut self, wait: Duration) -> Option<Value> {
        let deadline = Instant::now() + wait;
        loop {
            match self.next(deadline.saturating_duration_since(Instant::now())) {
                Frame::Message(m) if m["type"] == "event" => return Some(m),
                Frame::Message(_) => {}
                Frame::Silence | Frame::Closed => return None,
            }
        }
    }

    /// Every event that arrives until the socket has been quiet for `QUIET`.
    fn drain(&mut self) -> Vec<Value> {
        let mut found = Vec::new();
        while let Some(event) = self.next_event(QUIET) {
            found.push(event);
        }
        found
    }

    /// Subscribes and waits for the acknowledgement, which comes before any event.
    fn subscribe(&mut self, project: &str, after: Option<i64>) {
        let mut message = json!({ "type": "subscribe", "project_id": project });
        if let Some(after) = after {
            message["after"] = json!(after);
        }
        self.send(&message);
        match self.next(PATIENCE) {
            Frame::Message(ack) => assert_eq!(ack["type"], "subscribed", "{ack}"),
            Frame::Silence => panic!("the server did not acknowledge the subscription"),
            Frame::Closed => panic!("the server closed the socket"),
        }
    }
}

/// Opens a socket as `auth` and subscribes to the project, after `after` when
/// given.
fn follow(f: &Fixture, client: &Client, auth: &Auth, project: &str, after: Option<i64>) -> Ws {
    let mut ws = Ws::open(client, &f.workspace, auth);
    ws.subscribe(project, after);
    ws
}

/// Whether one of `events` names the post.
fn names_post(events: &[Value], post: &str) -> bool {
    events.iter().any(|e| e.to_string().contains(post))
}

/// The `post.created` events among `events`, as their post IDs.
fn created_posts(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["event"] == "post.created")
        .filter_map(|e| e["data"]["post_id"].as_str().map(str::to_string))
        .collect()
}

/// How many of the `post.created` events among `events` are of the post.
fn times_created(events: &[Value], post: &str) -> usize {
    created_posts(events)
        .iter()
        .filter(|id| *id == post)
        .count()
}

/// The newest sequence number among `events`, the number a client resumes
/// from.
fn newest_seq(events: &[Value]) -> i64 {
    events
        .iter()
        .filter_map(|e| e["seq"].as_i64())
        .max()
        .unwrap_or_else(|| panic!("there are events with a `seq`: {events:?}"))
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_read_routes_are_in_the_registry_for_agents_and_the_human() {
    for (method, path) in [
        ("GET", THREAD_ROUTE),
        ("GET", DIGEST_ROUTE),
        ("GET", POST_ROUTE),
    ] {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, Access::Both, "{method} {path}");
    }
}

// ---------------------------------------------------------------------------
// What an agent sees in a sealed phase
// ---------------------------------------------------------------------------

#[test]
fn in_a_sealed_phase_an_agent_sees_earlier_phases_its_own_and_the_humans_posts_only() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, false);
    let earlier = post(f, client, &critic(f), &project, &thread, "critic phase 1");
    start_phase(f, client, &project, &thread, true);
    let hidden = post(f, client, &critic(f), &project, &thread, "critic phase 2");
    let humans = post(f, client, &human(f), &project, &thread, "human phase 2");

    let answer = read_thread(f, client, &scout(f), &project, &thread);
    let text = answer.text();
    let answer = must(answer, 200, "the scout reads the thread");
    let seen: Vec<String> = thread_posts(&answer).iter().map(id_of).collect();
    assert_eq!(
        seen,
        ids_of(&[&earlier, &humans]),
        "the scout sees the earlier phase's post and the human's, oldest first"
    );
    assert_nothing_of(&text, &hidden, "the scout's thread read");

    assert_eq!(
        seen_in_thread(f, client, &critic(f), &project, &thread),
        ids_of(&[&earlier, &hidden, &humans]),
        "the critic sees its own post of the sealed phase"
    );
}

#[test]
fn an_agent_identity_is_its_run_and_role() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let architect = agent_as(f, "run-1", "architect");
    let own = post(f, client, &architect, &project, &thread, "architect");

    for (auth, who) in [
        (
            agent_as(f, "run-1", "critic"),
            "the same run with another role",
        ),
        (
            agent_as(f, "run-2", "architect"),
            "the same role in another run",
        ),
    ] {
        let response = read_thread(f, client, &auth, &project, &thread);
        let text = response.text();
        let answer = must(response, 200, who);
        assert!(
            thread_posts(&answer).is_empty(),
            "{who} sees nothing: {answer}"
        );
        assert_nothing_of(&text, &own, who);
    }
    // Another model or harness is still the same identity.
    let bearer = format!("Bearer {}", f.key);
    let response = client.request(
        "GET",
        &thread_path(f, &project, &thread),
        &[
            ("Authorization", bearer.as_str()),
            ("X-Omni-Run", "run-1"),
            ("X-Omni-Role", "architect"),
            ("X-Omni-Model", "another-model"),
            ("X-Omni-Harness", "another-harness"),
        ],
        None,
    );
    let answer = must(
        response,
        200,
        "the architect reads with another model and harness",
    );
    assert_eq!(
        thread_posts(&answer).iter().map(id_of).collect::<Vec<_>>(),
        ids_of(&[&own]),
        "the model and harness do not count: {answer}"
    );
    assert_eq!(
        seen_in_thread(f, client, &architect, &project, &thread),
        ids_of(&[&own]),
        "the architect sees its own post"
    );
}

// ---------------------------------------------------------------------------
// When hidden posts show
// ---------------------------------------------------------------------------

#[test]
fn once_the_agent_has_posted_in_the_phase_it_sees_the_hidden_posts_on_its_next_read() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let first = post(f, client, &critic(f), &project, &thread, "critic");
    let second = post(f, client, &moderator(f), &project, &thread, "moderator");
    assert!(
        seen_in_thread(f, client, &scout(f), &project, &thread).is_empty(),
        "before it posts, the scout sees nothing of the sealed phase"
    );

    let own = post(f, client, &scout(f), &project, &thread, "scout");
    assert_eq!(
        seen_in_thread(f, client, &scout(f), &project, &thread),
        ids_of(&[&first, &second, &own]),
        "after it posted, the scout sees every post of the phase"
    );
    // A later post of another agent in the same phase shows at once.
    let later = post(f, client, &critic(f), &project, &thread, "critic again");
    assert_eq!(
        seen_in_thread(f, client, &scout(f), &project, &thread),
        ids_of(&[&first, &second, &own, &later]),
    );
    // An agent that has not posted still sees nothing of the phase.
    assert!(
        seen_in_thread(
            f,
            client,
            &agent_as(f, "run-1", "builder"),
            &project,
            &thread
        )
        .is_empty(),
        "another agent that has not posted"
    );
}

#[test]
fn a_sealed_phase_followed_by_an_open_phase_shows_its_posts_to_every_agent() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let sealed = post(f, client, &critic(f), &project, &thread, "critic sealed");
    assert!(seen_in_thread(f, client, &scout(f), &project, &thread).is_empty());

    start_phase(f, client, &project, &thread, false);
    assert_eq!(
        seen_in_thread(f, client, &scout(f), &project, &thread),
        ids_of(&[&sealed]),
        "the sealed phase's post shows once an open phase follows it"
    );
    let open = post(f, client, &critic(f), &project, &thread, "critic open");
    assert_eq!(
        seen_in_thread(f, client, &scout(f), &project, &thread),
        ids_of(&[&sealed, &open]),
        "a post of an open phase shows to an agent that has not posted in it"
    );
}

#[test]
fn a_sealed_phase_followed_by_another_sealed_phase_shows_its_posts_as_an_earlier_phase() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let first = post(f, client, &critic(f), &project, &thread, "critic phase 1");
    start_phase(f, client, &project, &thread, true);
    let second = post(f, client, &critic(f), &project, &thread, "critic phase 2");

    let response = read_thread(f, client, &scout(f), &project, &thread);
    let text = response.text();
    assert_eq!(
        thread_posts(&must(response, 200, "the scout reads"))
            .iter()
            .map(id_of)
            .collect::<Vec<_>>(),
        ids_of(&[&first]),
        "the earlier sealed phase is open to read, the current one is not"
    );
    assert_nothing_of(&text, &second, "the scout's thread read");
}

// ---------------------------------------------------------------------------
// The human
// ---------------------------------------------------------------------------

#[test]
fn the_human_always_sees_every_post() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let critics = post(f, client, &critic(f), &project, &thread, "critic");
    let moderators = post(f, client, &moderator(f), &project, &thread, "moderator");
    let all = ids_of(&[&critics, &moderators]);

    assert_eq!(
        seen_in_thread(f, client, &human(f), &project, &thread),
        all,
        "the thread read"
    );
    let digest = must(
        read_digest(f, client, &human(f), &project, &thread),
        200,
        "the human reads the digest",
    );
    assert_eq!(
        digest_posts(&digest).iter().map(id_of).collect::<Vec<_>>(),
        all,
        "the digest"
    );
    for made in [&critics, &moderators] {
        let read = must(
            read_post(f, client, &human(f), &project, &made.id),
            200,
            "the human reads one post",
        );
        assert_eq!(read["post"]["id"], made.id.as_str(), "{read}");
        assert_eq!(read["post"]["summary"], made.summary.as_str(), "{read}");
    }
}

// ---------------------------------------------------------------------------
// Every read path
// ---------------------------------------------------------------------------

#[test]
fn the_digest_lists_only_the_posts_the_agent_may_see_without_their_bodies() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let hidden = post(f, client, &critic(f), &project, &thread, "critic");
    let humans = post(f, client, &human(f), &project, &thread, "human");

    let response = read_digest(f, client, &scout(f), &project, &thread);
    let text = response.text();
    let digest = must(response, 200, "the scout reads the digest");
    let entries = digest_posts(&digest);
    assert_eq!(
        entries.iter().map(id_of).collect::<Vec<_>>(),
        ids_of(&[&humans]),
        "the digest holds the human's post only: {digest}"
    );
    let entry = &entries[0];
    assert_eq!(entry["summary"], humans.summary.as_str(), "{entry}");
    assert_eq!(entry["type"], "position", "{entry}");
    assert_eq!(entry["phase"], 1, "{entry}");
    assert_eq!(entry["author"]["kind"], "human", "{entry}");
    assert!(
        entry.get("body").is_none() && entry.get("body_html").is_none(),
        "a digest carries summaries, not bodies: {entry}"
    );
    assert!(
        !text.contains(&humans.body),
        "no body in the digest: {text}"
    );
    assert_nothing_of(&text, &hidden, "the scout's digest");

    let own = post(f, client, &scout(f), &project, &thread, "scout");
    let digest = must(
        read_digest(f, client, &scout(f), &project, &thread),
        200,
        "the scout reads the digest after posting",
    );
    assert_eq!(
        digest_posts(&digest).iter().map(id_of).collect::<Vec<_>>(),
        ids_of(&[&hidden, &humans, &own]),
        "after posting, the digest holds every post"
    );
    assert_eq!(
        digest_posts(&digest)[0]["summary"],
        hidden.summary.as_str(),
        "{digest}"
    );

    let unknown = read_digest(f, client, &scout(f), &project, UNKNOWN_ID);
    assert_not_found(&unknown, "the digest of an unknown thread");
}

#[test]
fn a_single_post_read_of_a_hidden_post_is_answered_like_one_that_does_not_exist() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let hidden = post(f, client, &critic(f), &project, &thread, "critic");
    let humans = post(f, client, &human(f), &project, &thread, "human");

    let refused = read_post(f, client, &scout(f), &project, &hidden.id);
    assert_not_found(&refused, "the scout reads the critic's hidden post");
    assert_nothing_of(&refused.text(), &hidden, "the refusal");
    let unknown = read_post(f, client, &scout(f), &project, UNKNOWN_ID);
    assert_eq!(
        (refused.status, refused.body.clone()),
        (unknown.status, unknown.body.clone()),
        "a hidden post is answered like a post that does not exist"
    );

    // The author and the human read it, and the human's post shows to all.
    for (auth, who) in [(critic(f), "the critic"), (human(f), "the human")] {
        let read = must(read_post(f, client, &auth, &project, &hidden.id), 200, who);
        assert_eq!(read["post"]["id"], hidden.id.as_str(), "{who}: {read}");
        assert_eq!(read["post"]["body"], hidden.body.as_str(), "{who}: {read}");
    }
    let read = must(
        read_post(f, client, &scout(f), &project, &humans.id),
        200,
        "the scout reads the human's post",
    );
    assert_eq!(read["post"]["summary"], humans.summary.as_str(), "{read}");

    post(f, client, &scout(f), &project, &thread, "scout");
    let read = must(
        read_post(f, client, &scout(f), &project, &hidden.id),
        200,
        "the scout reads the critic's post after posting",
    );
    assert_eq!(read["post"]["summary"], hidden.summary.as_str(), "{read}");

    // A post of another project is not found under this one.
    let (other_project, _) = thread_in_new_project(f, client, false);
    assert_not_found(
        &read_post(f, client, &human(f), &other_project, &hidden.id),
        "a post read under another project",
    );
}

#[test]
fn a_post_that_links_to_a_post_its_author_may_not_see_is_refused_like_a_link_to_no_post() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let hidden = post(f, client, &critic(f), &project, &thread, "critic");
    let before = posts_in(&thread);

    let link = |post: &str| {
        json!({
            "type": "objection",
            "summary": unique("A reply"),
            "body": "I disagree.",
            "links": [{ "kind": "post", "post_id": post }]
        })
    };
    let path = format!("{}/posts", thread_path(f, &project, &thread));
    let refused = call(client, "POST", &path, &scout(f), Some(&link(&hidden.id)));
    assert_eq!(refused.status, 400, "{}", refused.text());
    assert_eq!(refused.json(), json!({ "error": "invalid_request" }));
    let unknown = call(client, "POST", &path, &scout(f), Some(&link(UNKNOWN_ID)));
    assert_eq!(
        (refused.status, refused.body.clone()),
        (unknown.status, unknown.body.clone()),
        "a link to a hidden post is refused like a link to no post"
    );
    assert_eq!(posts_in(&thread), before, "nothing was recorded");

    // Once the scout has posted in the phase, it may link to the post.
    post(f, client, &scout(f), &project, &thread, "scout");
    let linked = must(
        call(client, "POST", &path, &scout(f), Some(&link(&hidden.id))),
        201,
        "the scout links to the critic's post after posting",
    );
    assert_eq!(linked["post"]["links"][0]["post_id"], hidden.id.as_str());
}

#[test]
fn a_link_to_a_hidden_post_is_left_out_of_what_the_agent_reads() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let hidden = post(f, client, &critic(f), &project, &thread, "critic");
    let source = json!({ "kind": "url", "url": "https://example.test/hero" });
    let pointer = json!({ "kind": "post", "post_id": hidden.id });
    let humans = post_with(
        f,
        client,
        &human(f),
        &project,
        &thread,
        "human links",
        json!({ "links": [pointer, source] }),
    );

    let links_in_thread = |auth: &Auth| -> (Value, String) {
        let response = read_thread(f, client, auth, &project, &thread);
        let text = response.text();
        let answer = must(response, 200, "read the thread");
        let post = thread_posts(&answer)
            .into_iter()
            .find(|p| id_of(p) == humans.id)
            .unwrap_or_else(|| panic!("the human's post shows: {answer}"));
        (post["links"].clone(), text)
    };
    let links_in_post = |auth: &Auth| -> (Value, String) {
        let response = read_post(f, client, auth, &project, &humans.id);
        let text = response.text();
        (
            must(response, 200, "read the human's post")["post"]["links"].clone(),
            text,
        )
    };

    for read in [
        &links_in_thread as &dyn Fn(&Auth) -> (Value, String),
        &links_in_post as &dyn Fn(&Auth) -> (Value, String),
    ] {
        let (links, text) = read(&scout(f));
        assert_eq!(
            links,
            json!([source]),
            "the link to the hidden post is left out, the web source stays"
        );
        assert_nothing_of(&text, &hidden, "the scout's read of the human's links");
        let (links, _) = read(&human(f));
        assert_eq!(links, json!([pointer, source]), "the human sees both links");
    }

    post(f, client, &scout(f), &project, &thread, "scout");
    for read in [
        &links_in_thread as &dyn Fn(&Auth) -> (Value, String),
        &links_in_post as &dyn Fn(&Auth) -> (Value, String),
    ] {
        let (links, _) = read(&scout(f));
        assert_eq!(
            links,
            json!([pointer, source]),
            "after posting, the scout sees the link too"
        );
    }
}

#[test]
fn live_events_of_hidden_posts_do_not_reach_the_agent() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let mut scouts = follow(f, client, &scout(f), &project, None);
    let mut critics = follow(f, client, &critic(f), &project, None);
    let mut humans = follow(f, client, &human(f), &project, None);

    let hidden = post(f, client, &critic(f), &project, &thread, "critic");
    let human_post = post(f, client, &human(f), &project, &thread, "human");

    let to_human = humans.drain();
    assert_eq!(
        created_posts(&to_human),
        ids_of(&[&hidden, &human_post]),
        "the human's socket gets every post.created"
    );
    let to_critic = critics.drain();
    assert_eq!(
        created_posts(&to_critic),
        ids_of(&[&hidden, &human_post]),
        "the critic's socket gets its own post and the human's"
    );
    let to_scout = scouts.drain();
    assert_eq!(
        created_posts(&to_scout),
        ids_of(&[&human_post]),
        "the scout's socket gets the human's post only"
    );
    assert!(
        !names_post(&to_scout, &hidden.id),
        "no event names the hidden post: {to_scout:?}"
    );
    // The socket already has the human's post, whose sequence number is above
    // the hidden post's.
    let resume_from = newest_seq(&to_scout);

    let own = post(f, client, &scout(f), &project, &thread, "scout");
    let to_scout = scouts.drain();
    assert!(
        created_posts(&to_scout).contains(&own.id),
        "the scout's socket gets its own post: {to_scout:?}"
    );
    assert_eq!(
        times_created(&to_scout, &hidden.id),
        1,
        "once the scout has posted, the withheld event of the critic's post \
         reaches its open socket, once: {to_scout:?}"
    );
    let replayed = follow(f, client, &scout(f), &project, Some(resume_from)).drain();
    assert_eq!(
        times_created(&replayed, &hidden.id),
        1,
        "a resume from the scout's last sequence number includes the critic's \
         post: {replayed:?}"
    );
    assert!(
        created_posts(&replayed).contains(&own.id),
        "the resume includes the scout's own post: {replayed:?}"
    );
    let later = post(f, client, &critic(f), &project, &thread, "critic again");
    let to_scout = scouts.drain();
    assert_eq!(
        created_posts(&to_scout),
        ids_of(&[&later]),
        "once the scout has posted, the critic's next post reaches it"
    );
}

#[test]
fn resuming_replays_a_hidden_posts_event_only_once_the_agent_may_see_it() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let hidden = post(f, client, &critic(f), &project, &thread, "critic");
    let humans = post(f, client, &human(f), &project, &thread, "human");

    let replayed = follow(f, client, &scout(f), &project, Some(0)).drain();
    assert!(
        replayed.iter().any(|e| e["event"] == "thread.created"),
        "the replay holds the project's other events: {replayed:?}"
    );
    assert_eq!(
        created_posts(&replayed),
        ids_of(&[&humans]),
        "resuming before posting replays the human's post only"
    );
    assert!(
        !names_post(&replayed, &hidden.id),
        "no replayed event names the hidden post: {replayed:?}"
    );
    let replayed = follow(f, client, &human(f), &project, Some(0)).drain();
    assert_eq!(
        created_posts(&replayed),
        ids_of(&[&hidden, &humans]),
        "the human's replay holds every post"
    );

    let own = post(f, client, &scout(f), &project, &thread, "scout");
    let replayed = follow(f, client, &scout(f), &project, Some(0)).drain();
    assert_eq!(
        created_posts(&replayed),
        ids_of(&[&hidden, &humans, &own]),
        "after posting, resuming from before replays the critic's post too"
    );
    let replayed = follow(f, client, &agent_as(f, "run-2", "scout"), &project, Some(0)).drain();
    assert!(
        !names_post(&replayed, &hidden.id),
        "an identity that has not posted still gets no event of it: {replayed:?}"
    );

    start_phase(f, client, &project, &thread, false);
    let replayed = follow(f, client, &agent_as(f, "run-2", "scout"), &project, Some(0)).drain();
    assert!(
        created_posts(&replayed).contains(&hidden.id),
        "once an open phase follows, every agent's replay holds it: {replayed:?}"
    );
}

// ---------------------------------------------------------------------------
// US-055: the follow-ups
// ---------------------------------------------------------------------------

#[test]
fn a_withheld_live_event_reaches_the_agent_once_a_later_phase_starts() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let mut scouts = follow(f, client, &scout(f), &project, None);

    let hidden = post(f, client, &critic(f), &project, &thread, "critic");
    let humans = post(f, client, &human(f), &project, &thread, "human");
    let to_scout = scouts.drain();
    assert_eq!(
        created_posts(&to_scout),
        ids_of(&[&humans]),
        "before the next phase, the scout's socket gets the human's post only"
    );
    let resume_from = newest_seq(&to_scout);

    // Whatever phase follows, the earlier one's posts are visible.
    start_phase(f, client, &project, &thread, true);
    let to_scout = scouts.drain();
    assert!(
        to_scout.iter().any(|e| e["event"] == "phase.started"),
        "the scout's socket gets the new phase: {to_scout:?}"
    );
    assert_eq!(
        times_created(&to_scout, &hidden.id),
        1,
        "once a later phase started, the withheld event of the critic's post \
         reaches the scout's open socket, once: {to_scout:?}"
    );

    let replayed = follow(f, client, &scout(f), &project, Some(resume_from)).drain();
    assert_eq!(
        times_created(&replayed, &hidden.id),
        1,
        "a resume from the scout's last sequence number includes the critic's \
         post: {replayed:?}"
    );

    // Nothing is delivered twice: the next event is the next one only.
    let later = post(f, client, &critic(f), &project, &thread, "critic phase 2");
    let to_scout = scouts.drain();
    assert!(
        !names_post(&to_scout, &hidden.id),
        "the critic's first post is not delivered again: {to_scout:?}"
    );
    assert!(
        !names_post(&to_scout, &later.id),
        "the critic's post of the new sealed phase is hidden again: {to_scout:?}"
    );
}

#[test]
fn agent_requests_that_read_or_write_posts_need_a_run_and_a_role() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let hidden = post(f, client, &critic(f), &project, &thread, "critic");
    let humans = post(f, client, &human(f), &project, &thread, "human");

    // Two agents without an identity would share one view of the phase: once
    // one of them posted, the other would read what the critic wrote.
    let first = Auth::Partial {
        key: &f.key,
        run: None,
        role: None,
    };
    let second = Auth::Partial {
        key: &f.key,
        run: None,
        role: None,
    };
    let run_only = Auth::Partial {
        key: &f.key,
        run: Some("run-1"),
        role: None,
    };
    let role_only = Auth::Partial {
        key: &f.key,
        run: None,
        role: Some("scout"),
    };
    let path = format!("{}/posts", thread_path(f, &project, &thread));
    let before = posts_in(&thread);
    for (auth, who) in [
        (&first, "the first agent without an identity"),
        (&second, "the second agent without an identity"),
        (&run_only, "an agent with a run and no role"),
        (&role_only, "an agent with a role and no run"),
    ] {
        let request = json!({
            "type": "position",
            "summary": unique("An unidentified position"),
            "body": "Mine."
        });
        let posted = call(client, "POST", &path, auth, Some(&request));
        assert_eq!(posted.status, 400, "{who} posts: {}", posted.text());
        assert_eq!(posts_in(&thread), before, "{who}: nothing was recorded");

        for (response, what) in [
            (
                read_thread(f, client, auth, &project, &thread),
                "reads the thread",
            ),
            (
                read_digest(f, client, auth, &project, &thread),
                "reads the digest",
            ),
            (
                read_post(f, client, auth, &project, &humans.id),
                "reads the human's post",
            ),
            (
                read_post(f, client, auth, &project, &hidden.id),
                "reads the critic's post",
            ),
        ] {
            let text = response.text();
            assert_eq!(response.status, 400, "{who} {what}: {text}");
            assert!(
                response.json()["error"].is_string(),
                "{who} {what}: the answer is an error: {text}"
            );
            assert_nothing_of(&text, &hidden, &format!("{who} {what}"));
        }
    }

    // With both, an agent reads and posts as before, and the human needs none.
    assert_eq!(
        seen_in_thread(f, client, &scout(f), &project, &thread),
        ids_of(&[&humans]),
        "the scout, with a run and a role"
    );
    post(f, client, &scout(f), &project, &thread, "scout");
    assert_eq!(
        seen_in_thread(f, client, &human(f), &project, &thread).len(),
        3,
        "the human reads every post without an identity"
    );
}

#[test]
fn an_agent_socket_needs_a_run_and_a_role() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);

    // Two agents without an identity would share one view of the phase on
    // their sockets as well: once one of them posted, the other would get the
    // events of what the critic wrote.
    let first = Auth::Partial {
        key: &f.key,
        run: None,
        role: None,
    };
    let second = Auth::Partial {
        key: &f.key,
        run: None,
        role: None,
    };
    let run_only = Auth::Partial {
        key: &f.key,
        run: Some("run-1"),
        role: None,
    };
    let role_only = Auth::Partial {
        key: &f.key,
        run: None,
        role: Some("scout"),
    };
    for (auth, who) in [
        (&first, "the first agent without an identity"),
        (&second, "the second agent without an identity"),
        (&run_only, "an agent with a run and no role"),
        (&role_only, "an agent with a role and no run"),
    ] {
        match Ws::try_open(client, &f.workspace, auth) {
            Ok(mut ws) => {
                ws.subscribe(&project, None);
                let hidden = post(f, client, &critic(f), &project, &thread, "critic");
                let events = ws.drain();
                panic!(
                    "{who} opened the socket (the critic's post {} reached it: {}): {events:?}",
                    hidden.id,
                    names_post(&events, &hidden.id)
                );
            }
            Err(answer) => {
                assert!(
                    answer.starts_with("HTTP/1.1 400"),
                    "{who} is refused before the upgrade with a 400: {answer}"
                );
            }
        }
    }

    // With both, an agent opens its socket as before, and the human needs none.
    let mut scouts = follow(f, client, &scout(f), &project, None);
    let mut humans = follow(f, client, &human(f), &project, None);
    let hidden = post(f, client, &critic(f), &project, &thread, "critic");
    let to_human = humans.drain();
    assert_eq!(times_created(&to_human, &hidden.id), 1, "{to_human:?}");
    let to_scout = scouts.drain();
    assert!(
        !names_post(&to_scout, &hidden.id),
        "the scout does not get the critic's post: {to_scout:?}"
    );
}

#[test]
fn a_directive_link_to_a_post_hidden_from_the_agent_shows_nowhere_in_its_answers() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let changeset = must(
        read_thread(f, client, &human(f), &project, &thread),
        200,
        "the human reads the thread",
    )["thread"]["changeset_id"]
        .as_str()
        .expect("the thread has a change set")
        .to_string();
    let mut scouts = follow(f, client, &scout(f), &project, None);
    let hidden = post(f, client, &critic(f), &project, &thread, "critic");

    let pointer = json!({ "kind": "post", "post_id": hidden.id });
    let source = json!({ "kind": "url", "url": "https://example.test/brief" });
    let directive = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/directives", thread_path(f, &project, &thread)),
                &human(f),
                Some(&json!({
                    "summary": unique("Weigh the critic's point"),
                    "body": "Answer the critic before you decide.",
                    "links": [pointer, source]
                })),
            ),
            201,
            "the human posts a directive that links to the critic's post",
        )["post"],
    );
    let given_to = |answer: &Value, who: &str| -> Value {
        let given = answer["directives"]
            .as_array()
            .unwrap_or_else(|| panic!("{who} is given the directive: {answer}"));
        let entry = given
            .iter()
            .find(|d| id_of(d) == directive)
            .unwrap_or_else(|| panic!("{who} is given the directive: {answer}"));
        entry["links"].clone()
    };

    // The thread read, which also gives the scout the directive.
    let response = read_thread(f, client, &scout(f), &project, &thread);
    let text = response.text();
    let answer = must(response, 200, "the scout reads the thread");
    assert_eq!(
        given_to(&answer, "the scout"),
        json!([source]),
        "the given directive keeps the web source only"
    );
    let shown = thread_posts(&answer)
        .into_iter()
        .find(|p| id_of(p) == directive)
        .unwrap_or_else(|| panic!("the directive shows in the thread: {answer}"));
    assert_eq!(shown["links"], json!([source]), "{shown}");
    assert_nothing_of(&text, &hidden, "the scout's thread read");

    // A change set request of another agent, which is given it there.
    let builder = agent_as(f, "run-1", "builder");
    let response = call(
        client,
        "GET",
        &format!("{}/changesets/{changeset}", project_base(f, &project)),
        &builder,
        None,
    );
    let text = response.text();
    let answer = must(response, 200, "the builder reads the change set");
    assert_eq!(given_to(&answer, "the builder"), json!([source]));
    assert_nothing_of(&text, &hidden, "the builder's change set read");

    // The single post read and the digest.
    let response = read_post(f, client, &builder, &project, &directive);
    let text = response.text();
    let answer = must(response, 200, "the builder reads the directive");
    assert_eq!(answer["post"]["links"], json!([source]), "{answer}");
    assert_nothing_of(&text, &hidden, "the builder's read of the directive");
    let response = read_digest(f, client, &builder, &project, &thread);
    let text = response.text();
    must(response, 200, "the builder reads the digest");
    assert!(
        text.contains(&directive),
        "the digest lists the directive: {text}"
    );
    assert_nothing_of(&text, &hidden, "the builder's digest");

    // The live events.
    let to_scout = scouts.drain();
    assert!(
        created_posts(&to_scout).contains(&directive),
        "the scout's socket gets the directive: {to_scout:?}"
    );
    assert_nothing_of(
        &serde_json::to_string(&to_scout).expect("events as JSON"),
        &hidden,
        "the scout's live events",
    );

    // The human sees the link.
    let read = must(
        read_post(f, client, &human(f), &project, &directive),
        200,
        "the human reads the directive",
    );
    assert_eq!(read["post"]["links"], json!([pointer, source]), "{read}");
}

/// `src/live.rs` without its comments.
fn live_source() -> String {
    include_str!("../src/live.rs")
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_live_event_filter_compares_post_ids_as_uuids_within_the_workspace() {
    let source = live_source();
    let start = source
        .find("async fn read_events")
        .expect("src/live.rs reads the events with `read_events`");
    let body = &source[start..];
    let body = &body[..body.find("\n}\n").map_or(body.len(), |end| end)];
    assert!(
        !body.contains("id::text"),
        "the filter casts no post ID to text, so it can use the index of \
         posts:\n{body}"
    );
    assert!(
        body.contains("'post_id')::uuid") || body.contains("'post_id' AS uuid"),
        "the filter reads the event's post ID as a UUID:\n{body}"
    );
    let posts = body
        .find("FROM posts")
        .map(|at| &body[at..])
        .unwrap_or_else(|| panic!("the filter looks the post up in posts:\n{body}"));
    assert!(
        posts.contains("workspace_id"),
        "the filter looks the post up within the event's workspace:\n{body}"
    );
}

// ---------------------------------------------------------------------------
// US-056: sealed decisions
// ---------------------------------------------------------------------------

/// What a test knows of a decision it proposed: its ID and its direction,
/// which must not show to an agent that may not see it.
struct Proposed {
    id: String,
    direction: String,
}

fn decision_path(f: &Fixture, project: &str, decision: &str) -> String {
    format!("{}/decisions/{decision}", project_base(f, project))
}

fn changeset_path(f: &Fixture, project: &str, changeset: &str) -> String {
    format!("{}/changesets/{changeset}", project_base(f, project))
}

/// The change set of the thread, as the human reads it.
fn changeset_of_thread(f: &Fixture, client: &Client, project: &str, thread: &str) -> String {
    must(
        read_thread(f, client, &human(f), project, thread),
        200,
        "the human reads the thread",
    )["thread"]["changeset_id"]
        .as_str()
        .expect("the thread has a change set")
        .to_string()
}

/// Another change set of the project, made by the moderator.
fn new_changeset(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/changesets", project_base(f, project)),
                &moderator(f),
                Some(&json!({ "title": unique("carry out"), "run_tag": "run-1" })),
            ),
            201,
            "create a change set",
        )["changeset"],
    )
}

/// `auth` proposes a decision for the thread that relies on `posts`, with a
/// unique direction.
fn propose(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    posts: &[&str],
) -> Proposed {
    let direction = unique("Use the split hero");
    let body = json!({
        "direction": direction,
        "chosen": "Split hero",
        "criteria": ["Readable at 320 px"],
        "options": [
            {
                "name": "Split hero",
                "scores": { "Readable at 320 px": 8 },
                "lost_because": null
            },
            {
                "name": "Carousel",
                "scores": { "Readable at 320 px": 4 },
                "lost_because": "It hides two of the three messages."
            }
        ],
        "assumptions": [],
        "open_questions": [],
        "dissent": [],
        "posts": posts
    });
    let proposed = must(
        call(
            client,
            "POST",
            &format!("{}/decisions", thread_path(f, project, thread)),
            auth,
            Some(&body),
        ),
        201,
        "propose a decision",
    );
    Proposed {
        id: id_of(&proposed["decision"]),
        direction,
    }
}

fn read_decision(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    decision: &str,
) -> Response {
    call(
        client,
        "GET",
        &decision_path(f, project, decision),
        auth,
        None,
    )
}

/// The decision as `auth` reads it, which must work.
fn decision_as(f: &Fixture, client: &Client, auth: &Auth, project: &str, decision: &str) -> Value {
    must(
        read_decision(f, client, auth, project, decision),
        200,
        "read the decision",
    )["decision"]
        .clone()
}

/// The Thread's `decision` as `auth` reads it, and the whole answer's text.
fn thread_decision(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
) -> (Value, String) {
    let response = read_thread(f, client, auth, project, thread);
    let text = response.text();
    let answer = must(response, 200, "read the thread");
    let decision = answer["thread"]
        .get("decision")
        .unwrap_or_else(|| panic!("the thread has `decision`: {answer}"))
        .clone();
    (decision, text)
}

fn approve_decision(f: &Fixture, client: &Client, project: &str, decision: &str) {
    must(
        call(
            client,
            "POST",
            &format!("{}/approve", decision_path(f, project, decision)),
            &human(f),
            Some(&json!({})),
        ),
        200,
        "the human approves the decision",
    );
}

fn link_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    decision: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!(
            "{}/decisions/{decision}",
            changeset_path(f, project, changeset)
        ),
        auth,
        None,
    )
}

/// `auth` links the change set to the decision, which must work.
fn link(f: &Fixture, client: &Client, auth: &Auth, project: &str, changeset: &str, decision: &str) {
    must(
        link_response(f, client, auth, project, changeset, decision),
        200,
        "link the change set to the decision",
    );
}

/// The IDs of a Changeset's linked decisions, in order.
fn linked_ids(changeset: &Value) -> Vec<String> {
    changeset["decisions"]
        .as_array()
        .unwrap_or_else(|| panic!("a change set carries `decisions`: {changeset}"))
        .iter()
        .map(id_of)
        .collect()
}

/// The change set as `auth` reads it (show), and the whole answer's text.
fn changeset_as(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
) -> (Value, String) {
    let response = call(
        client,
        "GET",
        &changeset_path(f, project, changeset),
        auth,
        None,
    );
    let text = response.text();
    let answer = must(response, 200, "read the change set");
    (answer["changeset"].clone(), text)
}

/// The change set as the list of the project's change sets shows it to
/// `auth`, and the whole answer's text.
fn listed_changeset(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
) -> (Value, String) {
    let response = call(
        client,
        "GET",
        &format!("{}/changesets", project_base(f, project)),
        auth,
        None,
    );
    let text = response.text();
    let answer = must(response, 200, "list the change sets");
    let listed = answer["changesets"]
        .as_array()
        .unwrap_or_else(|| panic!("the list has `changesets`: {answer}"))
        .iter()
        .find(|c| id_of(c) == changeset)
        .unwrap_or_else(|| panic!("the list has the change set {changeset}: {answer}"))
        .clone();
    (listed, text)
}

/// Neither the ID nor the direction of `hidden` is anywhere in `text`.
fn assert_no_decision(text: &str, hidden: &Proposed, what: &str) {
    for (name, marker) in [("ID", &hidden.id), ("direction", &hidden.direction)] {
        assert!(
            !text.contains(marker.as_str()),
            "{what}: the {name} of a hidden decision shows: {text}"
        );
    }
}

/// How many of `events` are `event` events of the decision.
fn times_of(events: &[Value], event: &str, decision: &str) -> usize {
    events
        .iter()
        .filter(|e| e["event"] == event && e["data"]["decision_id"] == decision)
        .count()
}

#[test]
fn a_decision_proposed_in_a_sealed_phase_reads_like_no_decision_to_an_agent_that_has_not_posted() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let changeset = changeset_of_thread(f, client, &project, &thread);
    let relied_on = post(f, client, &critic(f), &project, &thread, "critic");
    let decision = propose(f, client, &critic(f), &project, &thread, &[&relied_on.id]);
    link(f, client, &critic(f), &project, &changeset, &decision.id);

    // The critic has posted in the phase, so it sees its decision.
    assert_eq!(
        decision_as(f, client, &critic(f), &project, &decision.id)["id"],
        decision.id.as_str()
    );

    // Neither the scout nor the moderator, who started the thread, has
    // posted in the sealed phase.
    for (who, agent) in [("the scout", scout(f)), ("the moderator", moderator(f))] {
        let response = read_decision(f, client, &agent, &project, &decision.id);
        assert_not_found(&response, &format!("{who} reads the hidden decision"));
        let unknown = read_decision(f, client, &agent, &project, UNKNOWN_ID);
        assert_not_found(
            &unknown,
            &format!("{who} reads a decision that does not exist"),
        );

        let (current, text) = thread_decision(f, client, &agent, &project, &thread);
        assert_eq!(
            current,
            Value::Null,
            "{who}: the thread reads as one without a decision"
        );
        assert_no_decision(&text, &decision, &format!("{who}'s thread read"));

        let response = read_digest(f, client, &agent, &project, &thread);
        let text = response.text();
        must(response, 200, "read the digest");
        assert_no_decision(&text, &decision, &format!("{who}'s digest"));

        let (shown, text) = changeset_as(f, client, &agent, &project, &changeset);
        assert_eq!(
            linked_ids(&shown),
            Vec::<String>::new(),
            "{who}: the change set reads as one without a linked decision"
        );
        assert_no_decision(&text, &decision, &format!("{who}'s change set read"));
    }
}

#[test]
fn a_hidden_decision_shows_in_no_live_event_neither_live_nor_on_a_resume() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let mut scouts = follow(f, client, &scout(f), &project, None);
    let mut humans = follow(f, client, &human(f), &project, None);

    let relied_on = post(f, client, &critic(f), &project, &thread, "critic");
    let decision = propose(f, client, &critic(f), &project, &thread, &[&relied_on.id]);
    approve_decision(f, client, &project, &decision.id);
    // A visible event after the hidden ones, so the socket gets past them.
    let human_post = post(f, client, &human(f), &project, &thread, "human");

    let to_human = humans.drain();
    assert_eq!(
        times_of(&to_human, "decision.proposed", &decision.id),
        1,
        "the human's socket gets the proposal: {to_human:?}"
    );
    assert_eq!(
        times_of(&to_human, "decision.approved", &decision.id),
        1,
        "the human's socket gets the approval: {to_human:?}"
    );

    let to_scout = scouts.drain();
    assert!(
        created_posts(&to_scout).contains(&human_post.id),
        "the scout's socket gets the human's later post: {to_scout:?}"
    );
    assert_no_decision(
        &serde_json::to_string(&to_scout).expect("events as JSON"),
        &decision,
        "the scout's live events",
    );

    let replayed = follow(f, client, &scout(f), &project, Some(0)).drain();
    assert!(
        created_posts(&replayed).contains(&human_post.id),
        "the replay holds the human's post: {replayed:?}"
    );
    assert_no_decision(
        &serde_json::to_string(&replayed).expect("events as JSON"),
        &decision,
        "the scout's replay from the start",
    );
    let replayed = follow(f, client, &human(f), &project, Some(0)).drain();
    assert_eq!(
        times_of(&replayed, "decision.proposed", &decision.id),
        1,
        "the human's replay holds the proposal: {replayed:?}"
    );
}

#[test]
fn once_the_agent_posts_in_the_phase_the_decision_shows_on_its_next_read_and_reaches_its_socket() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let mut scouts = follow(f, client, &scout(f), &project, None);

    let relied_on = post(f, client, &critic(f), &project, &thread, "critic");
    let decision = propose(f, client, &critic(f), &project, &thread, &[&relied_on.id]);
    approve_decision(f, client, &project, &decision.id);
    let human_post = post(f, client, &human(f), &project, &thread, "human");
    let to_scout = scouts.drain();
    assert!(
        created_posts(&to_scout).contains(&human_post.id),
        "the scout's socket gets the human's post: {to_scout:?}"
    );
    assert_no_decision(
        &serde_json::to_string(&to_scout).expect("events as JSON"),
        &decision,
        "the scout's live events before it posted",
    );
    let resume_from = newest_seq(&to_scout);
    assert_not_found(
        &read_decision(f, client, &scout(f), &project, &decision.id),
        "the scout reads the decision before it posted",
    );

    let own = post(f, client, &scout(f), &project, &thread, "scout");

    let read = decision_as(f, client, &scout(f), &project, &decision.id);
    assert_eq!(read["id"], decision.id.as_str(), "{read}");
    assert_eq!(read["status"], "approved", "{read}");
    assert_eq!(
        read["posts"],
        json!([relied_on.id]),
        "the posts of the phase are the scout's to see now: {read}"
    );
    let (current, _) = thread_decision(f, client, &scout(f), &project, &thread);
    assert_eq!(current["id"], decision.id.as_str(), "{current}");
    assert_eq!(current, read, "the thread's decision is the decision read");

    let to_scout = scouts.drain();
    assert!(
        created_posts(&to_scout).contains(&own.id),
        "the scout's socket gets its own post: {to_scout:?}"
    );
    assert_eq!(
        times_of(&to_scout, "decision.proposed", &decision.id),
        1,
        "the withheld proposal reaches the scout's open socket, once: {to_scout:?}"
    );
    assert_eq!(
        times_of(&to_scout, "decision.approved", &decision.id),
        1,
        "the withheld approval reaches the scout's open socket, once: {to_scout:?}"
    );

    let replayed = follow(f, client, &scout(f), &project, Some(resume_from)).drain();
    assert_eq!(
        times_of(&replayed, "decision.proposed", &decision.id),
        1,
        "a resume from the scout's last sequence number includes the proposal: \
         {replayed:?}"
    );
    let replayed = follow(f, client, &scout(f), &project, Some(0)).drain();
    assert_eq!(
        times_of(&replayed, "decision.proposed", &decision.id),
        1,
        "a resume from the start includes the proposal: {replayed:?}"
    );

    // Another identity that has not posted still sees nothing of it.
    let other = agent_as(f, "run-2", "scout");
    assert_not_found(
        &read_decision(f, client, &other, &project, &decision.id),
        "an identity that has not posted reads the decision",
    );
    let replayed = follow(f, client, &other, &project, Some(0)).drain();
    assert_no_decision(
        &serde_json::to_string(&replayed).expect("events as JSON"),
        &decision,
        "the replay of an identity that has not posted",
    );
}

#[test]
fn once_a_later_phase_starts_a_sealed_decision_shows_to_every_agent() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    // The scout posts in the first sealed phase only.
    post(f, client, &scout(f), &project, &thread, "scout phase 1");
    start_phase(f, client, &project, &thread, true);
    let mut scouts = follow(f, client, &scout(f), &project, None);

    let relied_on = post(f, client, &critic(f), &project, &thread, "critic phase 2");
    let decision = propose(f, client, &critic(f), &project, &thread, &[&relied_on.id]);
    let human_post = post(f, client, &human(f), &project, &thread, "human");
    assert_not_found(
        &read_decision(f, client, &scout(f), &project, &decision.id),
        "posting in an earlier phase does not show the decision of this one",
    );
    let (current, _) = thread_decision(f, client, &scout(f), &project, &thread);
    assert_eq!(current, Value::Null, "the scout's thread read");
    let to_scout = scouts.drain();
    assert!(
        created_posts(&to_scout).contains(&human_post.id),
        "the scout's socket gets the human's post: {to_scout:?}"
    );
    assert_no_decision(
        &serde_json::to_string(&to_scout).expect("events as JSON"),
        &decision,
        "the scout's live events in the sealed phase",
    );

    // Whatever phase follows, the decision of the earlier one is visible.
    start_phase(f, client, &project, &thread, true);
    for (who, agent) in [
        ("the scout", scout(f)),
        (
            "an identity that never posted",
            agent_as(f, "run-2", "scout"),
        ),
    ] {
        let read = decision_as(f, client, &agent, &project, &decision.id);
        assert_eq!(read["id"], decision.id.as_str(), "{who}: {read}");
        let (current, _) = thread_decision(f, client, &agent, &project, &thread);
        assert_eq!(current["id"], decision.id.as_str(), "{who}: {current}");
    }
    let to_scout = scouts.drain();
    assert!(
        to_scout.iter().any(|e| e["event"] == "phase.started"),
        "the scout's socket gets the new phase: {to_scout:?}"
    );
    assert_eq!(
        times_of(&to_scout, "decision.proposed", &decision.id),
        1,
        "once a later phase started, the withheld proposal reaches the scout's \
         open socket, once: {to_scout:?}"
    );
}

#[test]
fn the_human_always_sees_every_decision() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let changeset = changeset_of_thread(f, client, &project, &thread);
    let mut humans = follow(f, client, &human(f), &project, None);

    let relied_on = post(f, client, &critic(f), &project, &thread, "critic");
    let decision = propose(f, client, &critic(f), &project, &thread, &[&relied_on.id]);
    link(f, client, &critic(f), &project, &changeset, &decision.id);

    // The human has not posted in the sealed phase.
    let read = decision_as(f, client, &human(f), &project, &decision.id);
    assert_eq!(read["id"], decision.id.as_str(), "{read}");
    assert_eq!(read["posts"], json!([relied_on.id]), "{read}");
    let (current, _) = thread_decision(f, client, &human(f), &project, &thread);
    assert_eq!(current, read, "the thread's decision is the decision read");
    let (shown, _) = changeset_as(f, client, &human(f), &project, &changeset);
    assert_eq!(linked_ids(&shown), vec![decision.id.clone()], "{shown}");
    let (listed, _) = listed_changeset(f, client, &human(f), &project, &changeset);
    assert_eq!(linked_ids(&listed), vec![decision.id.clone()], "{listed}");
    let home = must(
        call(
            client,
            "GET",
            &format!("{}/home", project_base(f, &project)),
            &human(f),
            None,
        ),
        200,
        "the human reads the project's home",
    );
    let waiting: Vec<String> = home["waiting"]["decisions"]
        .as_array()
        .unwrap_or_else(|| panic!("the home lists the decisions waiting: {home}"))
        .iter()
        .map(id_of)
        .collect();
    assert_eq!(waiting, vec![decision.id.clone()], "{home}");

    let to_human = humans.drain();
    assert_eq!(
        times_of(&to_human, "decision.proposed", &decision.id),
        1,
        "the human's socket gets the proposal: {to_human:?}"
    );
    let replayed = follow(f, client, &human(f), &project, Some(0)).drain();
    assert_eq!(
        times_of(&replayed, "decision.proposed", &decision.id),
        1,
        "the human's replay holds the proposal: {replayed:?}"
    );
    approve_decision(f, client, &project, &decision.id);
    assert_eq!(
        decision_as(f, client, &human(f), &project, &decision.id)["status"],
        "approved"
    );
}

#[test]
fn a_decision_proposed_in_an_open_phase_is_seen_by_every_agent_also_once_a_sealed_phase_follows() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, false);
    let changeset = changeset_of_thread(f, client, &project, &thread);
    let mut scouts = follow(f, client, &scout(f), &project, None);

    let relied_on = post(f, client, &critic(f), &project, &thread, "critic");
    let decision = propose(f, client, &critic(f), &project, &thread, &[&relied_on.id]);
    link(f, client, &critic(f), &project, &changeset, &decision.id);

    let check = |what: &str| {
        let read = decision_as(f, client, &scout(f), &project, &decision.id);
        assert_eq!(read["id"], decision.id.as_str(), "{what}: {read}");
        assert_eq!(read["posts"], json!([relied_on.id]), "{what}: {read}");
        let (current, _) = thread_decision(f, client, &scout(f), &project, &thread);
        assert_eq!(current["id"], decision.id.as_str(), "{what}: {current}");
        let (shown, _) = changeset_as(f, client, &scout(f), &project, &changeset);
        assert_eq!(
            linked_ids(&shown),
            vec![decision.id.clone()],
            "{what}: {shown}"
        );
        let (listed, _) = listed_changeset(f, client, &scout(f), &project, &changeset);
        assert_eq!(
            linked_ids(&listed),
            vec![decision.id.clone()],
            "{what}: {listed}"
        );
    };
    check("in the open phase");
    let to_scout = scouts.drain();
    assert_eq!(
        times_of(&to_scout, "decision.proposed", &decision.id),
        1,
        "the scout's socket gets the proposal: {to_scout:?}"
    );

    // A sealed phase follows; the decision was proposed in the open one.
    start_phase(f, client, &project, &thread, true);
    check("once a sealed phase follows");
    approve_decision(f, client, &project, &decision.id);
    let to_scout = scouts.drain();
    assert_eq!(
        times_of(&to_scout, "decision.approved", &decision.id),
        1,
        "the scout's socket gets the approval in the sealed phase: {to_scout:?}"
    );
    let replayed = follow(f, client, &scout(f), &project, Some(0)).drain();
    assert_eq!(
        times_of(&replayed, "decision.proposed", &decision.id),
        1,
        "the scout's replay holds the proposal: {replayed:?}"
    );
}

#[test]
fn a_change_sets_linked_decisions_shown_to_an_agent_keep_to_the_sealed_phase() {
    let (f, client) = ready();
    let (project, sealed_thread) = thread_in_new_project(f, client, true);
    let changeset = changeset_of_thread(f, client, &project, &sealed_thread);
    let relied_on = post(f, client, &critic(f), &project, &sealed_thread, "critic");
    let hidden = propose(
        f,
        client,
        &critic(f),
        &project,
        &sealed_thread,
        &[&relied_on.id],
    );
    // A decision of an open thread of the same change set, which every agent
    // sees, linked after the hidden one.
    let open_thread = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/threads", changeset_path(f, &project, &changeset)),
                &moderator(f),
                Some(&json!({
                    "question": unique("Which footer?"),
                    "criteria": ["Readable at 320 px"],
                    "sealed": false
                })),
            ),
            201,
            "start an open thread",
        )["thread"],
    );
    let seen = propose(f, client, &critic(f), &project, &open_thread, &[]);
    // Both are carried out by another change set as well.
    let carrier = new_changeset(f, client, &project);
    for target in [&changeset, &carrier] {
        link(f, client, &critic(f), &project, target, &hidden.id);
        link(f, client, &critic(f), &project, target, &seen.id);
    }

    for target in [&changeset, &carrier] {
        let (shown, text) = changeset_as(f, client, &scout(f), &project, target);
        assert_eq!(
            linked_ids(&shown),
            vec![seen.id.clone()],
            "the scout's change set read leaves the hidden decision out: {shown}"
        );
        assert_no_decision(&text, &hidden, "the scout's change set read");
        let (listed, text) = listed_changeset(f, client, &scout(f), &project, target);
        assert_eq!(
            linked_ids(&listed),
            vec![seen.id.clone()],
            "the scout's change set list leaves the hidden decision out: {listed}"
        );
        assert_no_decision(&text, &hidden, "the scout's change set list");
        let (shown, _) = changeset_as(f, client, &human(f), &project, target);
        assert_eq!(
            linked_ids(&shown),
            vec![hidden.id.clone(), seen.id.clone()],
            "the human sees both: {shown}"
        );
    }

    // Linking to the hidden decision is linking to no decision.
    let third = new_changeset(f, client, &project);
    let response = link_response(f, client, &scout(f), &project, &third, &hidden.id);
    assert_not_found(&response, "the scout links the hidden decision");
    let (shown, _) = changeset_as(f, client, &human(f), &project, &third);
    assert_eq!(
        linked_ids(&shown),
        Vec::<String>::new(),
        "the refused link linked nothing: {shown}"
    );

    // Once the scout has posted in the phase, it sees both, in order.
    post(f, client, &scout(f), &project, &sealed_thread, "scout");
    for target in [&changeset, &carrier] {
        let (shown, _) = changeset_as(f, client, &scout(f), &project, target);
        assert_eq!(
            linked_ids(&shown),
            vec![hidden.id.clone(), seen.id.clone()],
            "{shown}"
        );
        let (listed, _) = listed_changeset(f, client, &scout(f), &project, target);
        assert_eq!(
            linked_ids(&listed),
            vec![hidden.id.clone(), seen.id.clone()],
            "{listed}"
        );
    }
    link(f, client, &scout(f), &project, &third, &hidden.id);
}
