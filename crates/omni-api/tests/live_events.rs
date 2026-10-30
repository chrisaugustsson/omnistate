//! US-017: live updates over WebSockets. Run with `--features stack-tests` in
//! the Compose test container, which reaches Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace (A) with one member
//! (alice) and a live session, mint an API key for it through the API, and
//! serve the API's router on a local port. They speak WebSocket to it with a
//! small client of their own (a TCP stream, the handshake and the frames of
//! RFC 6455).
//!
//! The contract these tests pin down:
//!
//! - `GET /ws/w/{workspace_id}` with the usual upgrade headers opens the socket
//!   (101). The caller is checked exactly as for the HTTP routes of US-013,
//!   before the upgrade: an API key (`Authorization: Bearer`) is an agent, a
//!   session cookie is the human, any `Authorization` header means an agent
//!   (a bad key is never rescued by a cookie), no credentials or a bad key are
//!   401 `{"error": "unauthorized"}`, and a workspace that the caller cannot
//!   reach, or that is not a workspace, is 404 `{"error": "not_found"}`. The
//!   workspace is the one in the path, never one in the query. Agents and the
//!   human both may connect.
//! - Messages are JSON text frames. A client subscribes to a project with
//!   `{"type": "subscribe", "project_id": "<uuid>", "after": <seq>}`; `after`
//!   is optional. The server answers with the acknowledgement `{"type":
//!   "subscribed", "project_id": ..., "last_seq": <seq>}`, where `last_seq` is
//!   the newest sequence number of the project (0 if it has no event). The
//!   acknowledgement comes first. Without `after`, the client gets only the
//!   events from then on. With `after: N`, the server then sends every event of
//!   the project with a sequence number above N, oldest first, each exactly
//!   once, and continues with the live events without a gap or a duplicate.
//!   An unknown project, or one of another workspace, is answered with `{"type":
//!   "error", "error": "not_found"}`; text that is not a message of this
//!   protocol is `{"type": "error", "error": "invalid_request"}`. Both leave
//!   the socket open. A client only gets events of the projects it subscribed
//!   to.
//! - An event is `{"type": "event", "seq", "event", "project_id", "at",
//!   "data"}`. `event` is `<entity>.<action>`; `seq` is a number that goes up
//!   by one for every event of a project (so it is per project, and gapless).
//! - Events are kept in the tenant table `live_events` (columns `workspace_id`,
//!   `project_id`, `seq`, `event`, `data`), which is how a client resumes. The
//!   deletion tests seed it once it exists (tests/workspace_delete.rs).

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
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

const TEST_DB: &str = "omni_us017_live_events";
/// The handshake key of RFC 6455's example, whose accept value is fixed.
const HANDSHAKE_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
const HANDSHAKE_ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";
/// How long a test waits for something that must happen.
const PATIENCE: Duration = Duration::from_secs(5);
/// How long a test waits to be sure that something does not happen.
const QUIET: Duration = Duration::from_millis(500);

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
        // Nothing here reaches RustFS: the routes under test only use Postgres.
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
    a: String,
    alice_token: String,
    key_a: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us017-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
        let a = block(async {
            let mut conn = admin_connection().await;
            let mut text = async |sql: &str, binds: &[&str]| -> String {
                let mut query = sqlx::query_scalar::<_, String>(sql);
                for bind in binds {
                    query = query.bind(*bind);
                }
                query.fetch_one(&mut conn).await.expect("seed")
            };
            let a = text(
                "insert into workspaces (name) values ('Alpha') returning id::text",
                &[],
            )
            .await;
            let user = text(
                "insert into users (email) values ($1) returning id::text",
                &["alice@example.test"],
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
                &[&a, &user],
            )
            .await;
            a
        });
        let server = TestServer::start(&config());
        let mint = |token: &str, workspace: &str| -> String {
            let response = call(
                &server.client,
                "POST",
                &format!("/api/v1/w/{workspace}/api-keys"),
                &Auth::Session(token),
                Some(&json!({})),
            );
            assert_eq!(response.status, 201, "mint a key: {}", response.text());
            response.json()["secret"]
                .as_str()
                .expect("the new key has a secret")
                .to_string()
        };
        let key_a = mint(&session_token("alice"), &a);
        Fixture {
            a,
            alice_token: session_token("alice"),
            key_a,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

/// How a request authenticates: an API key (an agent) or a session cookie (the
/// human).
#[derive(Clone, Copy)]
enum Auth<'a> {
    Key(&'a str),
    Session(&'a str),
}

impl Auth<'_> {
    fn headers(&self) -> Vec<(&'static str, String)> {
        let key = |secret: &str| ("Authorization", format!("Bearer {secret}"));
        // An agent declares its run and role, which its socket needs, since
        // the events name posts (US-055).
        let identity = || {
            vec![
                ("X-Omni-Run", "run-1".to_string()),
                ("X-Omni-Role", "agent".to_string()),
            ]
        };
        match self {
            Auth::Key(secret) => [vec![key(secret)], identity()].concat(),
            Auth::Session(token) => vec![("Cookie", cookie(token))],
        }
    }
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let owned = auth.headers();
    let headers: Vec<(&str, &str)> = owned.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let body = body.map(Value::to_string);
    client.request(method, path, &headers, body.as_deref())
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

// ---------------------------------------------------------------------------
// Doing things over HTTP: the changes whose events the tests watch
// ---------------------------------------------------------------------------

/// Creates a project in `workspace` as its agent (`key`); returns its ID.
fn new_project_in(client: &Client, workspace: &str, key: &str, label: &str) -> String {
    let response = call(
        client,
        "POST",
        &format!("/api/v1/w/{workspace}/projects"),
        &Auth::Key(key),
        Some(&json!({ "name": unique(label) })),
    );
    assert_eq!(
        response.status,
        201,
        "create a project: {}",
        response.text()
    );
    response.json()["project"]["id"]
        .as_str()
        .expect("the project has an id")
        .to_string()
}

fn new_project(f: &Fixture, client: &Client, label: &str) -> String {
    new_project_in(client, &f.a, &f.key_a, label)
}

fn features_path(workspace: &str, project: &str) -> String {
    format!("/api/v1/w/{workspace}/projects/{project}/features")
}

/// Creates a feature in a project of workspace A as its agent; returns its ID.
/// This emits `feature.created`.
fn new_feature(f: &Fixture, client: &Client, project: &str) -> String {
    let response = call(
        client,
        "POST",
        &features_path(&f.a, project),
        &Auth::Key(&f.key_a),
        Some(&json!({ "name": unique("feature") })),
    );
    assert_eq!(
        response.status,
        201,
        "create a feature: {}",
        response.text()
    );
    response.json()["feature"]["id"]
        .as_str()
        .expect("the feature has an id")
        .to_string()
}

/// Renames a feature, as `auth`; returns the status.
fn rename(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: &str,
    name: &str,
    auth: &Auth,
) -> u16 {
    let path = format!("{}/{feature}", features_path(&f.a, project));
    call(client, "PATCH", &path, auth, Some(&json!({ "name": name }))).status
}

/// The agent renames a feature, which emits `feature.renamed`.
fn renamed(f: &Fixture, client: &Client, project: &str, feature: &str) {
    let name = unique("renamed");
    let status = rename(f, client, project, feature, &name, &Auth::Key(&f.key_a));
    assert_eq!(status, 200, "rename the feature as the agent");
}

/// The agent deletes a feature, which emits `feature.deleted`.
fn deleted(f: &Fixture, client: &Client, project: &str, feature: &str) {
    let path = format!("{}/{feature}", features_path(&f.a, project));
    let response = call(client, "DELETE", &path, &Auth::Key(&f.key_a), None);
    assert_eq!(response.status, 204, "{}", response.text());
}

// ---------------------------------------------------------------------------
// A small WebSocket client
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
    /// Opens the socket at `path`, which must work.
    fn open(client: &Client, path: &str, auth: &Auth) -> Ws {
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
        let header = |name: &str| -> Option<String> {
            head.lines()
                .filter_map(|l| l.split_once(':'))
                .find(|(n, _)| n.trim().eq_ignore_ascii_case(name))
                .map(|(_, v)| v.trim().to_string())
        };
        ws.buffer.drain(..split + 4);
        if status == 101 {
            assert_eq!(
                header("sec-websocket-accept").as_deref(),
                Some(HANDSHAKE_ACCEPT),
                "the accept value of the handshake: {head}"
            );
            return ws;
        }
        let length: usize = header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        while ws.buffer.len() < length {
            match ws.read_some(deadline) {
                Arrived::Data(bytes) => ws.buffer.extend(bytes),
                _ => break,
            }
        }
        panic!(
            "the socket should open, but {path} answered {status}: {}",
            String::from_utf8_lossy(&ws.buffer)
        );
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

    fn send_text(&mut self, text: &str) {
        self.send_frame(1, text.as_bytes());
    }

    fn send(&mut self, message: &Value) {
        self.send_text(&message.to_string());
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

    /// The next `count` events, which must all arrive in time.
    fn events(&mut self, count: usize) -> Vec<Value> {
        let mut found = Vec::new();
        while found.len() < count {
            match self.next_event(PATIENCE) {
                Some(event) => found.push(event),
                None => panic!(
                    "{count} events expected, only {} arrived: {found:#?}",
                    found.len()
                ),
            }
        }
        found
    }

    /// Every event that arrives until the socket has been quiet for `QUIET`.
    fn drain(&mut self) -> Vec<Value> {
        let mut found = Vec::new();
        while let Some(event) = self.next_event(QUIET) {
            found.push(event);
        }
        found
    }

    fn assert_no_event(&mut self, what: &str) {
        if let Some(event) = self.next_event(QUIET) {
            panic!("{what}, but an event arrived: {event}");
        }
    }

    /// The next message of any kind, which must arrive in time.
    fn reply(&mut self) -> Value {
        match self.next(PATIENCE) {
            Frame::Message(message) => message,
            Frame::Silence => panic!("the server did not answer"),
            Frame::Closed => panic!("the server closed the socket"),
        }
    }

    /// Subscribes and waits for the acknowledgement, which comes before any event.
    fn subscribe(&mut self, project: &str, after: Option<i64>) -> Value {
        let mut message = json!({ "type": "subscribe", "project_id": project });
        if let Some(after) = after {
            message["after"] = json!(after);
        }
        self.send(&message);
        let ack = self.reply();
        assert_eq!(ack["type"], "subscribed", "the acknowledgement: {ack}");
        assert_eq!(ack["project_id"], project, "{ack}");
        assert!(ack["last_seq"].is_i64(), "last_seq is a number: {ack}");
        ack
    }
}

fn path_of(workspace: &str) -> String {
    format!("/ws/w/{workspace}")
}

/// Opens the socket of `workspace`, which must work.
fn open(client: &Client, workspace: &str, auth: &Auth) -> Ws {
    Ws::open(client, &path_of(workspace), auth)
}

fn seq_of(event: &Value) -> i64 {
    event["seq"]
        .as_i64()
        .unwrap_or_else(|| panic!("an event has a numeric seq: {event}"))
}

fn name_of(event: &Value) -> &str {
    event["event"]
        .as_str()
        .unwrap_or_else(|| panic!("an event has a name: {event}"))
}

fn names(events: &[Value]) -> Vec<&str> {
    events.iter().map(name_of).collect()
}

fn assert_gapless(events: &[Value]) {
    for pair in events.windows(2) {
        assert_eq!(
            seq_of(&pair[1]),
            seq_of(&pair[0]) + 1,
            "sequence numbers go up by one: {pair:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Subscribing
// ---------------------------------------------------------------------------

#[test]
fn text_that_is_not_a_message_is_an_error_and_the_socket_stays_open() {
    let (f, client) = ready();
    let project = new_project(f, client, "garbage");
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));
    for text in [
        "this is not json",
        r#"{"type": "dance"}"#,
        r#"{"type": "subscribe"}"#,
    ] {
        ws.send_text(text);
        let reply = ws.reply();
        assert_eq!(reply["type"], "error", "{text}: {reply}");
        assert_eq!(reply["error"], "invalid_request", "{text}: {reply}");
    }
    ws.subscribe(&project, None);
}

// ---------------------------------------------------------------------------
// Delivery
// ---------------------------------------------------------------------------

#[test]
fn a_request_that_changes_nothing_pushes_no_event() {
    let (f, client) = ready();
    let project = new_project(f, client, "nothing");
    let feature = new_feature(f, client, &project);
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));
    ws.subscribe(&project, None);

    // A blank name is refused, and so is the human renaming.
    let key = Auth::Key(&f.key_a);
    let human = Auth::Session(&f.alice_token);
    assert_eq!(rename(f, client, &project, &feature, "  ", &key), 400);
    assert_eq!(rename(f, client, &project, &feature, "New", &human), 403);
    ws.assert_no_event("none of those requests changed anything");
}

#[test]
fn a_client_only_gets_the_events_of_the_projects_it_subscribed_to() {
    let (f, client) = ready();
    let watched = new_project(f, client, "watched");
    let other = new_project(f, client, "other");
    let mut subscribed = open(client, &f.a, &Auth::Key(&f.key_a));
    let mut unsubscribed = open(client, &f.a, &Auth::Session(&f.alice_token));
    subscribed.subscribe(&watched, None);

    new_feature(f, client, &other);
    let feature = new_feature(f, client, &watched);

    let events = subscribed.events(1);
    assert_eq!(events[0]["project_id"], watched.as_str());
    assert_eq!(events[0]["data"]["feature_id"], feature.as_str());
    subscribed.assert_no_event("the other project's feature was never announced");
    unsubscribed.assert_no_event("this client subscribed to no project");
}

#[test]
fn events_reach_a_subscribed_client_within_250_ms_of_the_change() {
    let (f, client) = ready();
    let project = new_project(f, client, "latency");
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));
    ws.subscribe(&project, None);

    let mut slowest = Duration::ZERO;
    for _ in 0..5 {
        let started = Instant::now();
        let feature = new_feature(f, client, &project);
        let created = ws.events(1);
        let created_in = started.elapsed();
        assert_eq!(name_of(&created[0]), "feature.created");

        let started = Instant::now();
        renamed(f, client, &project, &feature);
        let changed = ws.events(1);
        let changed_in = started.elapsed();
        assert_eq!(name_of(&changed[0]), "feature.renamed");
        slowest = slowest.max(created_in).max(changed_in);
    }
    assert!(
        slowest < Duration::from_millis(250),
        "an event took {slowest:?} from the request to the client"
    );
}

// ---------------------------------------------------------------------------
// Sequence numbers and ordering
// ---------------------------------------------------------------------------

#[test]
fn sequence_numbers_go_up_by_one_per_project() {
    let (f, client) = ready();
    let busy = new_project(f, client, "busy");
    let quiet = new_project(f, client, "quiet");
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));
    let busy_from = ws.subscribe(&busy, None)["last_seq"].as_i64().unwrap();
    let quiet_from = ws.subscribe(&quiet, None)["last_seq"].as_i64().unwrap();

    for _ in 0..3 {
        let feature = new_feature(f, client, &busy);
        renamed(f, client, &busy, &feature);
    }
    new_feature(f, client, &quiet);

    let events = ws.events(7);
    let of = |project: &str| -> Vec<Value> {
        events
            .iter()
            .filter(|e| e["project_id"] == project)
            .cloned()
            .collect()
    };
    let (busy_events, quiet_events) = (of(&busy), of(&quiet));
    assert_eq!(busy_events.len(), 6);
    assert_eq!(quiet_events.len(), 1);
    assert_eq!(seq_of(&busy_events[0]), busy_from + 1, "{busy_events:?}");
    assert_gapless(&busy_events);
    assert_eq!(
        seq_of(&quiet_events[0]),
        quiet_from + 1,
        "the numbers are per project, so the quiet project did not count the busy one's events: {quiet_events:?}"
    );
}

#[test]
fn events_of_changes_made_at_the_same_time_arrive_in_sequence_order_without_gaps() {
    let (f, client) = ready();
    let project = new_project(f, client, "burst");
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));
    ws.subscribe(&project, None);

    let workers = 8;
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                let feature = new_feature(f, client, &project);
                renamed(f, client, &project, &feature);
            });
        }
    });

    let events = ws.events(workers * 2);
    assert_gapless(&events);
    let mut seen_created = Vec::new();
    for event in &events {
        let id = event["data"]["feature_id"].as_str().unwrap().to_string();
        match name_of(event) {
            "feature.created" => seen_created.push(id),
            "feature.renamed" => assert!(
                seen_created.contains(&id),
                "a feature is announced as created before it is renamed: {events:#?}"
            ),
            other => panic!("unexpected event {other}"),
        }
    }
    assert_eq!(seen_created.len(), workers);
    ws.assert_no_event("no event is delivered twice");
}

// ---------------------------------------------------------------------------
// Resuming
// ---------------------------------------------------------------------------

#[test]
fn a_client_that_reconnects_with_its_last_sequence_number_gets_everything_after_it() {
    let (f, client) = ready();
    let project = new_project(f, client, "resume");
    let mut first = open(client, &f.a, &Auth::Key(&f.key_a));
    first.subscribe(&project, None);

    let one = new_feature(f, client, &project);
    renamed(f, client, &project, &one);
    let seen = first.events(2);
    let last_seen = seq_of(&seen[1]);
    drop(first);

    // Changes while the client is away.
    deleted(f, client, &project, &one);
    let two = new_feature(f, client, &project);
    renamed(f, client, &project, &two);

    // The human's browser comes back with its last number (the same client
    // type as before is not required: resuming is about the number).
    let mut second = open(client, &f.a, &Auth::Session(&f.alice_token));
    let ack = second.subscribe(&project, Some(last_seen));
    assert_eq!(ack["last_seq"], last_seen + 3, "{ack}");
    let missed = second.events(3);
    assert_eq!(
        names(&missed),
        ["feature.deleted", "feature.created", "feature.renamed"]
    );
    assert_eq!(seq_of(&missed[0]), last_seen + 1);
    assert_gapless(&missed);
    assert_eq!(missed[0]["data"]["feature_id"], one.as_str());
    assert_eq!(missed[2]["data"]["feature_id"], two.as_str());
    second.assert_no_event("each missed event is sent once");

    // Live events carry on right after the replay, without a gap or a repeat.
    renamed(f, client, &project, &two);
    let live = second.events(1);
    assert_eq!(name_of(&live[0]), "feature.renamed");
    assert_eq!(seq_of(&live[0]), last_seen + 4);
    second.assert_no_event("nothing is repeated after the replay");
}

#[test]
fn resuming_from_zero_replays_the_whole_history_as_it_was_first_sent() {
    let (f, client) = ready();
    let project = new_project(f, client, "history");
    let mut first = open(client, &f.a, &Auth::Key(&f.key_a));
    first.subscribe(&project, None);
    let feature = new_feature(f, client, &project);
    renamed(f, client, &project, &feature);
    deleted(f, client, &project, &feature);
    let original = first.events(3);

    let mut late = open(client, &f.a, &Auth::Session(&f.alice_token));
    late.subscribe(&project, Some(0));
    let replayed = late.drain();
    assert_gapless(&replayed);
    assert_eq!(
        replayed, original,
        "a replayed event is the event that was sent live, field for field"
    );
}

#[test]
fn resuming_from_the_newest_number_or_without_one_replays_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client, "caught-up");
    let feature = new_feature(f, client, &project);
    renamed(f, client, &project, &feature);
    let newest = open(client, &f.a, &Auth::Key(&f.key_a)).subscribe(&project, None)["last_seq"]
        .as_i64()
        .unwrap();
    assert!(newest >= 2, "the project has had events: {newest}");

    let mut caught_up = open(client, &f.a, &Auth::Key(&f.key_a));
    caught_up.subscribe(&project, Some(newest));
    caught_up.assert_no_event("the client has seen everything already");

    let mut fresh = open(client, &f.a, &Auth::Key(&f.key_a));
    fresh.subscribe(&project, None);
    fresh.assert_no_event("without a sequence number only new events are sent");

    // Both go on with the next event, which is newest + 1.
    renamed(f, client, &project, &feature);
    for ws in [&mut caught_up, &mut fresh] {
        let events = ws.events(1);
        assert_eq!(seq_of(&events[0]), newest + 1);
    }
}

// US-059: creating a feature is an event of its project, `feature.created`
// with `data` that has at least `{"feature_id", "name"}`, pushed live and
// replayed to a client that resumes from an earlier sequence number.
#[test]
fn a_new_feature_is_pushed_and_replayed_to_a_client_that_resumes_before_it() {
    let (f, client) = ready();
    let project = new_project(f, client, "features");
    new_feature(f, client, &project);
    let before = open(client, &f.a, &Auth::Key(&f.key_a)).subscribe(&project, None)["last_seq"]
        .as_i64()
        .unwrap();
    assert!(before >= 1, "the project has had events: {before}");
    let mut live = open(client, &f.a, &Auth::Session(&f.alice_token));
    live.subscribe(&project, None);

    let name = unique("Checkout");
    let response = call(
        client,
        "POST",
        &features_path(&f.a, &project),
        &Auth::Key(&f.key_a),
        Some(&json!({ "name": name })),
    );
    assert_eq!(
        response.status,
        201,
        "create a feature: {}",
        response.text()
    );
    let feature = response.json()["feature"]["id"]
        .as_str()
        .expect("the feature has an id")
        .to_string();

    let pushed = live.events(1);
    assert_eq!(pushed[0]["type"], "event", "{}", pushed[0]);
    assert_eq!(name_of(&pushed[0]), "feature.created", "{}", pushed[0]);
    assert!(
        pushed[0]["at"].as_str().is_some_and(|t| !t.is_empty()),
        "{}",
        pushed[0]
    );
    assert_eq!(seq_of(&pushed[0]), before + 1);
    assert_eq!(pushed[0]["project_id"], project.as_str());
    assert_eq!(pushed[0]["data"]["feature_id"], feature.as_str());
    assert_eq!(pushed[0]["data"]["name"], name.as_str());

    let mut late = open(client, &f.a, &Auth::Key(&f.key_a));
    late.subscribe(&project, Some(before));
    let replayed = late.events(1);
    assert_eq!(
        replayed[0], pushed[0],
        "the replayed event is the one that was pushed live"
    );
    late.assert_no_event("the feature's event is sent once");
}
