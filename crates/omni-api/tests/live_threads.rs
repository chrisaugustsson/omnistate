//! US-108: threads. Run with `--features stack-tests` in
//! the Compose test container, which reaches Postgres at `postgres:5432` and
//! RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. Several
//! agents share the key and tell themselves apart by their identity headers
//! (`X-Omni-Run` and `X-Omni-Role`). The tests speak WebSocket to the API with
//! the small client of tests/live_events.rs.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; the rest is the green
//! step's choice):
//!
//! - `POST /threads` (agent) starts a thread at the app level and `POST
//!   /features/{feature_id}/threads` (agent) one in a feature of the project,
//!   both with `{"question", "sealed"}` (`sealed` defaults to `false`). They answer 201 `{"thread": Thread}`. A missing or blank
//!   question is 400 `invalid_request`, a feature the project does not have is
//!   404, and a browser session is 403. Starting a thread emits
//!   `thread.created` with the thread's ID in `data.thread_id`.
//! - A Thread has `id`, `project_id`, `scope` (`"app"` or `"feature"`),
//!   `feature_id` (`null` at the app level), `question`, `sealed`,
//!   `conclusion` (`null` until it is written) and `run_tag` (the run of the
//!   agent that started it). It has none of what US-108 removed: no `criteria`,
//!   no `phases` and no `decision`.
//! - `GET /threads` (both) answers `{"threads": [Thread, ...]}`: the threads of
//!   the project, of every scope.
//! - `GET /threads/{thread_id}` (both) answers `{"thread": Thread}` with
//!   `posts`: the posts the caller may read, oldest first.
//! - `POST /threads/{thread_id}/posts` (both: an agent with its API key, the
//!   human with a browser session) takes `{"summary", "body"}` and answers 201
//!   `{"post": Post}`. A Post has `id`, `thread_id`, `summary`, `body` and
//!   `author` (`{"kind": "agent", "run", "role", ...}` or `{"kind": "human",
//!   ...}`), and no `type` and no `phase`.
//! - In a sealed thread, an agent identity that has not posted in it yet reads
//!   no post of another agent, on any read path: the thread read, the live
//!   events of an open socket and the events a resume (`after`) replays. Once
//!   it has posted, it reads all of them: the next thread read lists them, the
//!   withheld `post.created` events reach its open socket, and a resume
//!   replays them. The human always reads every post. In a thread that is not
//!   sealed every agent reads every post.
//! - `PUT /threads/{thread_id}/conclusion` (agent) takes `{"conclusion"}`, a
//!   text that is not blank (else 400 `invalid_request`), and answers 200
//!   `{"thread": Thread}` with the conclusion set; a session is 403 and an
//!   unknown thread 404. It emits `thread.concluded` with the thread's ID in
//!   `data.thread_id`. Agents and the human can still post afterwards.

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

const TEST_DB: &str = "omni_us108_live_threads";
const TEST_BUCKET: &str = "omni-us108-live-threads";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
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
    format!("us108-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &moderator(f),
                Some(&json!({ "name": unique("forum-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

fn new_feature(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/features", project_base(f, project)),
                &moderator(f),
                Some(&json!({ "name": unique("Checkout"), "description": "Pay in one step" })),
            ),
            201,
            "create a feature",
        )["feature"],
    )
}

/// The route that starts a thread at the app level, or in `feature`.
fn threads_route(f: &Fixture, project: &str, feature: Option<&str>) -> String {
    match feature {
        None => format!("{}/threads", project_base(f, project)),
        Some(feature) => format!("{}/features/{feature}/threads", project_base(f, project)),
    }
}

fn start_thread(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    feature: Option<&str>,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &threads_route(f, project, feature),
        auth,
        Some(body),
    )
}

/// A new thread of the moderator's, at the app level or in `feature`: the
/// thread as the API answered it.
fn new_thread(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: Option<&str>,
    sealed: bool,
) -> Value {
    must(
        start_thread(
            f,
            client,
            &moderator(f),
            project,
            feature,
            &json!({ "question": unique("Which hero layout?"), "sealed": sealed }),
        ),
        201,
        "start a thread",
    )["thread"]
        .clone()
}

/// A new project with an app-level thread, sealed or not: (project, thread).
fn thread_in_new_project(f: &Fixture, client: &Client, sealed: bool) -> (String, String) {
    let project = new_project(f, client);
    let thread = id_of(&new_thread(f, client, &project, None, sealed));
    (project, thread)
}

/// What a test knows of a post it made: its ID and the texts that must not
/// show to an agent that may not read it.
#[derive(Clone)]
struct Made {
    id: String,
    summary: String,
    body: String,
}

/// Posts `{"summary", "body"}` as `auth`, with a unique summary and body.
fn post(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    label: &str,
) -> Made {
    let summary = unique(&format!("{label} summary"));
    let body = unique(&format!("{label} body"));
    let posted = must(
        call(
            client,
            "POST",
            &format!("{}/posts", thread_path(f, project, thread)),
            auth,
            Some(&json!({ "summary": summary, "body": body })),
        ),
        201,
        &format!("{label} posts"),
    );
    Made {
        id: id_of(&posted["post"]),
        summary,
        body,
    }
}

fn read_thread(f: &Fixture, client: &Client, auth: &Auth, project: &str, thread: &str) -> Value {
    must(
        call(client, "GET", &thread_path(f, project, thread), auth, None),
        200,
        "read the thread",
    )
}

/// The posts of a thread read, as the caller sees them.
fn thread_posts(answer: &Value) -> Vec<Value> {
    answer["thread"]["posts"]
        .as_array()
        .unwrap_or_else(|| panic!("the thread has a list of `posts`: {answer}"))
        .clone()
}

/// The IDs of the posts the caller reads in the thread, oldest first.
fn seen_in_thread(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
) -> Vec<String> {
    thread_posts(&read_thread(f, client, auth, project, thread))
        .iter()
        .map(id_of)
        .collect()
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

fn conclude(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "PUT",
        &format!("{}/conclusion", thread_path(f, project, thread)),
        auth,
        Some(body),
    )
}

/// How many threads the project has, as the admin role sees them.
fn threads_in(project: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from threads where project_id = $1::uuid")
            .bind(project)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the project's threads: {e}"))
    })
}

/// A thread of US-108: its scope and nothing of what US-108 removed.
fn assert_live_thread(thread: &Value, feature: Option<&str>) {
    match feature {
        None => {
            assert_eq!(thread["scope"], "app", "{thread}");
            assert_eq!(thread["feature_id"], Value::Null, "{thread}");
        }
        Some(feature) => {
            assert_eq!(thread["scope"], "feature", "{thread}");
            assert_eq!(thread["feature_id"], feature, "{thread}");
        }
    }
    for removed in ["criteria", "phases", "decision"] {
        assert!(
            thread.get(removed).is_none(),
            "a thread has no `{removed}` any more: {thread}"
        );
    }
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

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_thread_routes_are_in_the_registry_with_who_may_call_them() {
    let routes = registry();
    let base = "/api/v1/w/{workspace_id}/projects/{project_id}";
    for (method, path, access) in [
        ("POST", format!("{base}/threads"), Access::Agent),
        (
            "POST",
            format!("{base}/features/{{feature_id}}/threads"),
            Access::Agent,
        ),
        ("GET", format!("{base}/threads"), Access::Both),
        ("GET", format!("{base}/threads/{{thread_id}}"), Access::Both),
        (
            "POST",
            format!("{base}/threads/{{thread_id}}/posts"),
            Access::Both,
        ),
        (
            "PUT",
            format!("{base}/threads/{{thread_id}}/conclusion"),
            Access::Agent,
        ),
    ] {
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
    }
}

// ---------------------------------------------------------------------------
// Starting threads at the app level and in a feature
// ---------------------------------------------------------------------------

#[test]
fn an_agent_starts_a_thread_at_the_app_level() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let question = unique("Which hero layout?");
    let created = must(
        start_thread(
            f,
            client,
            &moderator(f),
            &project,
            None,
            &json!({ "question": question }),
        ),
        201,
        "start a thread at the app level",
    );
    let thread = &created["thread"];
    let id = id_of(thread);
    assert_eq!(thread["project_id"], project.as_str(), "{thread}");
    assert_eq!(thread["question"], question.as_str(), "{thread}");
    assert_eq!(
        thread["sealed"], false,
        "sealed defaults to false: {thread}"
    );
    assert_eq!(thread["conclusion"], Value::Null, "{thread}");
    assert_eq!(thread["run_tag"], "run-1", "the starter's run: {thread}");
    assert_live_thread(thread, None);

    for auth in [moderator(f), scout(f), human(f)] {
        let read = read_thread(f, client, &auth, &project, &id);
        assert_eq!(read["thread"]["question"], question.as_str(), "{read}");
        assert_live_thread(&read["thread"], None);
        assert_eq!(
            read["thread"]["posts"],
            json!([]),
            "a new thread has no posts: {read}"
        );
    }

    let replayed = follow(f, client, &human(f), &project, Some(0)).drain();
    assert!(
        replayed
            .iter()
            .any(|e| e["event"] == "thread.created" && e["data"]["thread_id"] == id.as_str()),
        "starting the thread emitted thread.created: {replayed:?}"
    );
}

#[test]
fn an_agent_starts_a_thread_in_a_feature_of_the_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let thread = new_thread(f, client, &project, Some(&feature), true);
    let id = id_of(&thread);
    assert_eq!(thread["sealed"], true, "{thread}");
    assert_live_thread(&thread, Some(&feature));
    let read = read_thread(f, client, &human(f), &project, &id);
    assert_live_thread(&read["thread"], Some(&feature));

    let other_project = new_project(f, client);
    let elsewhere = new_feature(f, client, &other_project);
    for (missing, what) in [
        (UNKNOWN_ID.to_string(), "a feature that does not exist"),
        (elsewhere, "a feature of another project"),
    ] {
        let response = start_thread(
            f,
            client,
            &moderator(f),
            &project,
            Some(&missing),
            &json!({ "question": "Lost?" }),
        );
        assert_eq!(response.status, 404, "{what}: {}", response.text());
    }
    assert_eq!(threads_in(&project), 1, "no thread was started in vain");
}

#[test]
fn starting_a_thread_needs_a_question_and_an_agent() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    for scope in [None, Some(feature.as_str())] {
        for body in [
            json!({}),
            json!({ "question": "" }),
            json!({ "question": "   " }),
            json!({ "question": 7 }),
            json!({ "question": "Which?", "sealed": "yes" }),
        ] {
            let response = start_thread(f, client, &moderator(f), &project, scope, &body);
            assert_eq!(response.status, 400, "{body}: {}", response.text());
            assert_eq!(response.json()["error"], "invalid_request", "{body}");
        }
        let response = start_thread(
            f,
            client,
            &human(f),
            &project,
            scope,
            &json!({ "question": "Which hero layout?" }),
        );
        assert_eq!(
            response.status,
            403,
            "only agents start threads: {}",
            response.text()
        );
    }
    assert_eq!(threads_in(&project), 0, "nothing was started");
}

#[test]
fn the_thread_list_has_the_threads_of_every_scope() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let app = id_of(&new_thread(f, client, &project, None, false));
    let in_feature = id_of(&new_thread(f, client, &project, Some(&feature), true));
    for auth in [scout(f), human(f)] {
        let listed = must(
            call(
                client,
                "GET",
                &format!("{}/threads", project_base(f, &project)),
                &auth,
                None,
            ),
            200,
            "list the threads",
        );
        let threads = listed["threads"]
            .as_array()
            .unwrap_or_else(|| panic!("a list of `threads`: {listed}"));
        assert_eq!(threads.len(), 2, "{listed}");
        let of = |id: &str| {
            threads
                .iter()
                .find(|t| t["id"] == id)
                .unwrap_or_else(|| panic!("thread {id} is listed: {listed}"))
        };
        assert_live_thread(of(&app), None);
        assert_live_thread(of(&in_feature), Some(&feature));
        assert_eq!(of(&in_feature)["sealed"], true, "{listed}");
    }
}

// ---------------------------------------------------------------------------
// Posts by agents and by the human
// ---------------------------------------------------------------------------

#[test]
fn agents_and_the_human_post_in_a_thread() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    for scope in [None, Some(feature.as_str())] {
        let thread = id_of(&new_thread(f, client, &project, scope, false));
        let posts = format!("{}/posts", thread_path(f, &project, &thread));

        let by_agent = must(
            call(
                client,
                "POST",
                &posts,
                &critic(f),
                Some(&json!({ "summary": "Pick the left layout", "body": "It reads **better**." })),
            ),
            201,
            "an agent posts with its API key",
        )["post"]
            .clone();
        assert_eq!(by_agent["thread_id"], thread.as_str(), "{by_agent}");
        assert_eq!(by_agent["summary"], "Pick the left layout", "{by_agent}");
        assert_eq!(by_agent["body"], "It reads **better**.", "{by_agent}");
        assert_eq!(by_agent["author"]["kind"], "agent", "{by_agent}");
        assert_eq!(by_agent["author"]["run"], "run-1", "{by_agent}");
        assert_eq!(by_agent["author"]["role"], "critic", "{by_agent}");

        let by_human = must(
            call(
                client,
                "POST",
                &posts,
                &human(f),
                Some(&json!({ "summary": "Left it is", "body": "Agreed." })),
            ),
            201,
            "the human posts with a browser session",
        )["post"]
            .clone();
        assert_eq!(by_human["author"]["kind"], "human", "{by_human}");
        assert_eq!(by_human["summary"], "Left it is", "{by_human}");
        assert_eq!(by_human["body"], "Agreed.", "{by_human}");

        for post in [&by_agent, &by_human] {
            for removed in ["type", "phase"] {
                assert!(
                    post.get(removed).is_none(),
                    "a post has no `{removed}` any more: {post}"
                );
            }
        }

        for auth in [scout(f), critic(f), human(f)] {
            assert_eq!(
                seen_in_thread(f, client, &auth, &project, &thread),
                vec![id_of(&by_agent), id_of(&by_human)],
                "both posts, oldest first"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Sealed threads
// ---------------------------------------------------------------------------

#[test]
fn in_a_sealed_thread_an_agent_that_has_not_posted_reads_no_other_agents_posts() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let by_critic = post(f, client, &critic(f), &project, &thread, "critic");
    let by_scout = post(f, client, &scout(f), &project, &thread, "scout");

    let answer = read_thread(f, client, &moderator(f), &project, &thread);
    assert_eq!(
        thread_posts(&answer),
        Vec::<Value>::new(),
        "the moderator has not posted, so it reads no other agent's post"
    );
    let text = answer.to_string();
    assert_nothing_of(&text, &by_critic, "the moderator's thread read");
    assert_nothing_of(&text, &by_scout, "the moderator's thread read");

    assert_eq!(
        seen_in_thread(f, client, &critic(f), &project, &thread),
        ids_of(&[&by_critic, &by_scout]),
        "the critic has posted, so it reads its own post and the scout's later one"
    );
    let scouts = read_thread(f, client, &scout(f), &project, &thread).to_string();
    assert!(
        scouts.contains(&by_critic.id),
        "the scout has posted, so its thread read shows the critic's post: {scouts}"
    );
    assert_eq!(
        seen_in_thread(f, client, &human(f), &project, &thread),
        ids_of(&[&by_critic, &by_scout]),
        "the human reads every post"
    );

    let by_moderator = post(f, client, &moderator(f), &project, &thread, "moderator");
    assert_eq!(
        seen_in_thread(f, client, &moderator(f), &project, &thread),
        ids_of(&[&by_critic, &by_scout, &by_moderator]),
        "once it has posted, the moderator reads every post on its next read"
    );
    assert_eq!(
        seen_in_thread(f, client, &critic(f), &project, &thread),
        ids_of(&[&by_critic, &by_scout, &by_moderator]),
        "the critic has posted, so it reads the later posts too"
    );
    assert_eq!(
        seen_in_thread(
            f,
            client,
            &agent_as(f, "run-2", "critic"),
            &project,
            &thread
        ),
        Vec::<String>::new(),
        "another run of the same role is another agent, which has not posted"
    );
    assert_eq!(
        seen_in_thread(f, client, &human(f), &project, &thread),
        ids_of(&[&by_critic, &by_scout, &by_moderator]),
        "the human reads every post"
    );
}

#[test]
fn the_live_events_of_a_sealed_thread_reach_an_agent_only_once_it_has_posted() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let mut scouts = follow(f, client, &scout(f), &project, None);
    let mut critics = follow(f, client, &critic(f), &project, None);
    let mut humans = follow(f, client, &human(f), &project, None);

    let hidden = post(f, client, &critic(f), &project, &thread, "critic");
    assert_eq!(
        created_posts(&humans.drain()),
        ids_of(&[&hidden]),
        "the human's socket gets every post.created"
    );
    assert_eq!(
        created_posts(&critics.drain()),
        ids_of(&[&hidden]),
        "the critic's socket gets its own post"
    );
    let to_scout = scouts.drain();
    assert!(
        !names_post(&to_scout, &hidden.id),
        "no event names the critic's post before the scout has posted: {to_scout:?}"
    );

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
    let resume_from = newest_seq(&to_scout);

    let later = post(f, client, &critic(f), &project, &thread, "critic again");
    let to_scout = scouts.drain();
    assert_eq!(
        created_posts(&to_scout),
        ids_of(&[&later]),
        "once the scout has posted, the critic's next post reaches it at once"
    );
    let replayed = follow(f, client, &scout(f), &project, Some(resume_from)).drain();
    assert_eq!(
        created_posts(&replayed),
        ids_of(&[&later]),
        "a resume from the scout's newest sequence number replays only what came after"
    );
}

#[test]
fn a_resume_replays_a_sealed_threads_posts_only_once_the_agent_has_posted() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let hidden = post(f, client, &critic(f), &project, &thread, "critic");

    let replayed = follow(f, client, &scout(f), &project, Some(0)).drain();
    assert!(
        replayed.iter().any(|e| e["event"] == "thread.created"),
        "the replay holds the project's other events: {replayed:?}"
    );
    assert!(
        !names_post(&replayed, &hidden.id),
        "resuming before posting replays no event of the critic's post: {replayed:?}"
    );
    let replayed = follow(f, client, &human(f), &project, Some(0)).drain();
    assert_eq!(
        created_posts(&replayed),
        ids_of(&[&hidden]),
        "the human's replay holds every post"
    );

    let own = post(f, client, &scout(f), &project, &thread, "scout");
    let replayed = follow(f, client, &scout(f), &project, Some(0)).drain();
    assert_eq!(
        created_posts(&replayed),
        ids_of(&[&hidden, &own]),
        "after posting, resuming from before replays the critic's post too"
    );
    let replayed = follow(f, client, &agent_as(f, "run-2", "scout"), &project, Some(0)).drain();
    assert!(
        !names_post(&replayed, &hidden.id),
        "an identity that has not posted still gets no event of it: {replayed:?}"
    );
}

// ---------------------------------------------------------------------------
// The conclusion
// ---------------------------------------------------------------------------

#[test]
fn an_agent_concludes_a_thread_and_posts_can_still_be_added() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, false);
    let before = post(f, client, &critic(f), &project, &thread, "critic");
    let mut humans = follow(f, client, &human(f), &project, None);
    let mut scouts = follow(f, client, &scout(f), &project, None);

    let conclusion = "We take the left layout: it reads better at 320 px.";
    let concluded = must(
        conclude(
            f,
            client,
            &moderator(f),
            &project,
            &thread,
            &json!({ "conclusion": conclusion }),
        ),
        200,
        "conclude the thread",
    );
    assert_eq!(concluded["thread"]["id"], thread.as_str(), "{concluded}");
    assert_eq!(concluded["thread"]["conclusion"], conclusion, "{concluded}");
    for auth in [scout(f), human(f)] {
        let read = read_thread(f, client, &auth, &project, &thread);
        assert_eq!(read["thread"]["conclusion"], conclusion, "{read}");
    }
    for (events, whose) in [(humans.drain(), "human"), (scouts.drain(), "scout")] {
        assert!(
            events
                .iter()
                .any(|e| e["event"] == "thread.concluded"
                    && e["data"]["thread_id"] == thread.as_str()),
            "the {whose}'s socket gets thread.concluded: {events:?}"
        );
    }

    let after = post(f, client, &scout(f), &project, &thread, "scout");
    let by_human = post(f, client, &human(f), &project, &thread, "human");
    let read = read_thread(f, client, &moderator(f), &project, &thread);
    assert_eq!(
        thread_posts(&read).iter().map(id_of).collect::<Vec<_>>(),
        ids_of(&[&before, &after, &by_human]),
        "posts are still added after the conclusion"
    );
    assert_eq!(
        read["thread"]["conclusion"], conclusion,
        "posting leaves the conclusion: {read}"
    );
}

#[test]
fn a_conclusion_needs_a_text_an_agent_and_a_thread_of_the_project() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, false);
    for body in [
        json!({}),
        json!({ "conclusion": "" }),
        json!({ "conclusion": "  " }),
        json!({ "conclusion": 7 }),
    ] {
        let response = conclude(f, client, &moderator(f), &project, &thread, &body);
        assert_eq!(response.status, 400, "{body}: {}", response.text());
        assert_eq!(response.json()["error"], "invalid_request", "{body}");
    }
    let response = conclude(
        f,
        client,
        &human(f),
        &project,
        &thread,
        &json!({ "conclusion": "The human does not conclude." }),
    );
    assert_eq!(
        response.status,
        403,
        "agents write the conclusion: {}",
        response.text()
    );
    let other_project = new_project(f, client);
    for (project, thread, what) in [
        (project.as_str(), UNKNOWN_ID, "a thread that does not exist"),
        (
            other_project.as_str(),
            thread.as_str(),
            "a thread of another project",
        ),
    ] {
        let response = conclude(
            f,
            client,
            &moderator(f),
            project,
            thread,
            &json!({ "conclusion": "Nowhere." }),
        );
        assert_eq!(response.status, 404, "{what}: {}", response.text());
    }
    let read = read_thread(f, client, &human(f), &project, &thread);
    assert_eq!(
        read["thread"]["conclusion"],
        Value::Null,
        "nothing was concluded: {read}"
    );
}
