//! US-017: live updates over WebSockets. Run with `--features stack-tests` in
//! the Compose test container, which reaches Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed two workspaces (A and B) with one
//! member each (alice in A, bob in B) and a live session for each, mint an API
//! key per workspace through the API, and serve the API's router on a local
//! port. They speak WebSocket to it with a small client of their own (a TCP
//! stream, the handshake and the frames of RFC 6455).
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
//! - Every status change of a change set (every entry of its history, the
//!   creation included) emits one `changeset.<status>` event with the new
//!   status, so a new change set is `changeset.open`, as is a reopened one. Its
//!   `data` is `{"changeset_id", "from_status", "status", "actor"}`, plus
//!   `reason` when the human rejected. A request that changes nothing (a 409)
//!   emits nothing.
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
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
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

/// The first column of the first row of `sql` as text, or None for no row or NULL.
fn db_text(sql: &str, binds: &[&str]) -> Option<String> {
    block(async {
        let mut conn = admin_connection().await;
        let mut query = sqlx::query_scalar::<_, Option<String>>(sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_optional(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("query failed ({e}): {sql}"))
            .flatten()
    })
}

struct Fixture {
    a: String,
    b: String,
    alice_token: String,
    bob_token: String,
    key_a: String,
    key_b: String,
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
        let (a, b) = block(async {
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
            let b = text(
                "insert into workspaces (name) values ('Beta') returning id::text",
                &[],
            )
            .await;
            for (name, workspace) in [("alice", &a), ("bob", &b)] {
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
                    &[workspace, &user],
                )
                .await;
            }
            (a, b)
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
        let key_b = mint(&session_token("bob"), &b);
        Fixture {
            a,
            b,
            alice_token: session_token("alice"),
            bob_token: session_token("bob"),
            key_a,
            key_b,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
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
            Auth::KeyAndSession(secret, token) => {
                [vec![key(secret), ("Cookie", cookie(token))], identity()].concat()
            }
            Auth::Nobody => vec![],
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

fn changesets_path(workspace: &str, project: &str) -> String {
    format!("/api/v1/w/{workspace}/projects/{project}/changesets")
}

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

/// Creates a change set in a project of workspace A as its agent; returns its ID.
fn new_changeset(f: &Fixture, client: &Client, project: &str) -> String {
    let response = call(
        client,
        "POST",
        &changesets_path(&f.a, project),
        &Auth::Key(&f.key_a),
        Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
    );
    assert_eq!(
        response.status,
        201,
        "create a change set: {}",
        response.text()
    );
    response.json()["changeset"]["id"]
        .as_str()
        .expect("the change set has an id")
        .to_string()
}

/// Calls a lifecycle action of a change set of workspace A and returns the status.
fn act(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    action: &str,
    auth: &Auth,
) -> u16 {
    let path = format!("{}/{changeset}/{action}", changesets_path(&f.a, project));
    let body = json!({ "reason": "not good enough yet" });
    call(client, "POST", &path, auth, Some(&body)).status
}

fn agent_acts(f: &Fixture, client: &Client, project: &str, changeset: &str, action: &str) {
    let status = act(f, client, project, changeset, action, &Auth::Key(&f.key_a));
    assert_eq!(status, 200, "{action} as the agent");
}

fn human_rejects(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let status = act(
        f,
        client,
        project,
        changeset,
        "reject",
        &Auth::Session(&f.alice_token),
    );
    assert_eq!(status, 200, "reject as the human");
}

// ---------------------------------------------------------------------------
// A small WebSocket client
// ---------------------------------------------------------------------------

/// The answer to a handshake that did not open the socket.
#[derive(Debug)]
struct Refused {
    status: u16,
    body: String,
}

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
            return Ok(ws);
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
        .unwrap_or_else(|refused| panic!("the socket should open: {refused:?}"))
}

fn assert_refused(result: Result<Ws, Refused>, status: u16, code: &str) {
    let refused = match result {
        Ok(_) => panic!("the socket opened, but {status} {code} was expected"),
        Err(refused) => refused,
    };
    assert_eq!(refused.status, status, "{refused:?}");
    let body: Value = serde_json::from_str(&refused.body)
        .unwrap_or_else(|e| panic!("the refusal is JSON ({e}): {refused:?}"));
    assert_eq!(
        body,
        json!({ "error": code }),
        "the error body carries the code and nothing else"
    );
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

fn changeset_events(events: &[Value]) -> Vec<Value> {
    events
        .iter()
        .filter(|e| name_of(e).starts_with("changeset."))
        .cloned()
        .collect()
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
// Opening the socket
// ---------------------------------------------------------------------------

#[test]
fn an_agent_with_a_key_and_the_human_with_a_session_open_the_socket_of_their_workspace() {
    let (f, client) = ready();
    open(client, &f.a, &Auth::Key(&f.key_a));
    open(client, &f.a, &Auth::Session(&f.alice_token));
    open(client, &f.b, &Auth::Key(&f.key_b));
    open(client, &f.b, &Auth::Session(&f.bob_token));
}

#[test]
fn the_socket_needs_valid_credentials_and_a_bad_key_is_never_rescued_by_a_cookie() {
    let (f, client) = ready();
    let path = path_of(&f.a);
    assert_refused(Ws::open(client, &path, &Auth::Nobody), 401, "unauthorized");
    assert_refused(
        Ws::open(client, &path, &Auth::Key("omni_not-a-real-key")),
        401,
        "unauthorized",
    );
    assert_refused(
        Ws::open(client, &path, &Auth::Session("not-a-real-session")),
        401,
        "unauthorized",
    );
    // Any Authorization header means an agent: the valid cookie next to the bad
    // key does not make the request the human's.
    assert_refused(
        Ws::open(
            client,
            &path,
            &Auth::KeyAndSession("omni_not-a-real-key", &f.alice_token),
        ),
        401,
        "unauthorized",
    );
}

#[test]
fn a_caller_cannot_open_the_socket_of_a_workspace_it_cannot_reach() {
    let (f, client) = ready();
    // A key belongs to one workspace; a session to the workspaces of its user.
    assert_refused(
        Ws::open(client, &path_of(&f.b), &Auth::Key(&f.key_a)),
        404,
        "not_found",
    );
    assert_refused(
        Ws::open(client, &path_of(&f.a), &Auth::Key(&f.key_b)),
        404,
        "not_found",
    );
    assert_refused(
        Ws::open(client, &path_of(&f.b), &Auth::Session(&f.alice_token)),
        404,
        "not_found",
    );
    assert_refused(
        Ws::open(client, &path_of(&f.a), &Auth::Session(&f.bob_token)),
        404,
        "not_found",
    );
    assert_refused(
        Ws::open(client, &path_of(UNKNOWN_ID), &Auth::Key(&f.key_a)),
        404,
        "not_found",
    );
    assert_refused(
        Ws::open(client, "/ws/w/not-a-uuid", &Auth::Session(&f.alice_token)),
        404,
        "not_found",
    );
    // The workspace is the one in the path, never one named in the query.
    let path = format!("{}?workspace_id={}", path_of(&f.a), f.b);
    assert_refused(
        Ws::open(client, &path, &Auth::Key(&f.key_b)),
        404,
        "not_found",
    );
}

// ---------------------------------------------------------------------------
// Subscribing
// ---------------------------------------------------------------------------

#[test]
fn subscribing_to_a_project_is_acknowledged_with_its_newest_sequence_number() {
    let (f, client) = ready();
    let project = new_project(f, client, "ack");
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));
    let fresh = ws.subscribe(&project, None);
    let before = fresh["last_seq"].as_i64().unwrap();

    let changeset = new_changeset(f, client, &project);
    agent_acts(f, client, &project, &changeset, "submit");
    let events = ws.events(2);

    let again = open(client, &f.a, &Auth::Key(&f.key_a)).subscribe(&project, None)["last_seq"]
        .as_i64()
        .unwrap();
    assert_eq!(
        again,
        seq_of(&events[1]),
        "last_seq is the newest event's sequence number (it was {before} before)"
    );
}

#[test]
fn a_subscription_to_an_unknown_or_foreign_project_is_an_error_and_leaks_nothing() {
    let (f, client) = ready();
    let own = new_project(f, client, "own");
    let foreign = new_project_in(client, &f.b, &f.key_b, "foreign");
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));

    for project in [UNKNOWN_ID, foreign.as_str()] {
        ws.send(&json!({ "type": "subscribe", "project_id": project }));
        let reply = ws.reply();
        assert_eq!(reply["type"], "error", "{reply}");
        assert_eq!(reply["error"], "not_found", "{reply}");
    }

    // The socket stays open and works for a project of the workspace.
    ws.subscribe(&own, None);

    // Workspace B's agent changes its project: A's client sees none of it.
    let key_b = Auth::Key(&f.key_b);
    let body = json!({ "title": "theirs" });
    let path = changesets_path(&f.b, &foreign);
    let created = call(client, "POST", &path, &key_b, Some(&body));
    assert_eq!(created.status, 201, "{}", created.text());
    ws.assert_no_event("the client subscribed to no project of workspace B");

    // Resuming a foreign project from the start does not work either.
    ws.send(&json!({ "type": "subscribe", "project_id": foreign, "after": 0 }));
    assert_eq!(ws.reply()["error"], "not_found");
    ws.assert_no_event("a foreign project cannot be replayed");
}

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
fn every_status_change_of_a_change_set_is_pushed_as_a_changeset_status_event() {
    let (f, client) = ready();
    let project = new_project(f, client, "delivery");
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));
    ws.subscribe(&project, None);

    let changeset = new_changeset(f, client, &project);
    agent_acts(f, client, &project, &changeset, "submit");
    human_rejects(f, client, &project, &changeset);
    agent_acts(f, client, &project, &changeset, "reopen");
    agent_acts(f, client, &project, &changeset, "abandon");

    let events = ws.events(5);
    assert_eq!(
        names(&events),
        [
            "changeset.open",
            "changeset.in_review",
            "changeset.rejected",
            "changeset.open",
            "changeset.abandoned"
        ]
    );
    let steps = [
        (None, "open", "agent"),
        (Some("open"), "in_review", "agent"),
        (Some("in_review"), "rejected", "human"),
        (Some("rejected"), "open", "agent"),
        (Some("open"), "abandoned", "agent"),
    ];
    for (event, (from, status, actor)) in events.iter().zip(steps) {
        assert_eq!(event["type"], "event", "{event}");
        assert_eq!(event["project_id"], project.as_str(), "{event}");
        assert!(
            event["at"].as_str().is_some_and(|t| !t.is_empty()),
            "{event}"
        );
        let data = &event["data"];
        assert_eq!(data["changeset_id"], changeset.as_str(), "{event}");
        assert_eq!(data["from_status"], json!(from), "{event}");
        assert_eq!(data["status"], status, "{event}");
        assert_eq!(data["actor"], actor, "{event}");
    }
    assert_eq!(
        events[2]["data"]["reason"], "not good enough yet",
        "a rejection carries its reason"
    );
    ws.assert_no_event("nothing else happened");
}

#[test]
fn a_request_that_changes_nothing_pushes_no_event() {
    let (f, client) = ready();
    let project = new_project(f, client, "nothing");
    let changeset = new_changeset(f, client, &project);
    agent_acts(f, client, &project, &changeset, "submit");
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));
    ws.subscribe(&project, None);

    // Submitting twice is a conflict, and so is the agent rejecting.
    let key = Auth::Key(&f.key_a);
    assert_eq!(act(f, client, &project, &changeset, "submit", &key), 409);
    assert_eq!(act(f, client, &project, &changeset, "reject", &key), 403);
    assert_eq!(act(f, client, &project, &changeset, "reopen", &key), 409);
    ws.assert_no_event("none of those requests changed anything");
}

#[test]
fn the_agent_and_the_human_both_receive_the_same_events() {
    let (f, client) = ready();
    let project = new_project(f, client, "both");
    let mut agent = open(client, &f.a, &Auth::Key(&f.key_a));
    let mut human = open(client, &f.a, &Auth::Session(&f.alice_token));
    agent.subscribe(&project, None);
    human.subscribe(&project, None);

    let changeset = new_changeset(f, client, &project);
    agent_acts(f, client, &project, &changeset, "submit");
    human_rejects(f, client, &project, &changeset);

    let seen_by_agent = agent.events(3);
    let seen_by_human = human.events(3);
    assert_eq!(
        seen_by_agent, seen_by_human,
        "same events, same sequence numbers"
    );
    assert_eq!(seen_by_human[2]["data"]["actor"], "human");
}

#[test]
fn a_client_only_gets_the_events_of_the_projects_it_subscribed_to() {
    let (f, client) = ready();
    let watched = new_project(f, client, "watched");
    let other = new_project(f, client, "other");
    let mut subscribed = open(client, &f.a, &Auth::Key(&f.key_a));
    let mut unsubscribed = open(client, &f.a, &Auth::Session(&f.alice_token));
    subscribed.subscribe(&watched, None);

    new_changeset(f, client, &other);
    let changeset = new_changeset(f, client, &watched);

    let events = subscribed.events(1);
    assert_eq!(events[0]["project_id"], watched.as_str());
    assert_eq!(events[0]["data"]["changeset_id"], changeset.as_str());
    subscribed.assert_no_event("the other project's change set was never announced");
    unsubscribed.assert_no_event("this client subscribed to no project");
}

#[test]
fn a_client_can_follow_several_projects_on_one_socket() {
    let (f, client) = ready();
    let first = new_project(f, client, "first");
    let second = new_project(f, client, "second");
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));
    ws.subscribe(&first, None);
    ws.subscribe(&second, None);

    new_changeset(f, client, &first);
    new_changeset(f, client, &second);
    let events = ws.events(2);
    let mut projects: Vec<&str> = events
        .iter()
        .map(|e| e["project_id"].as_str().unwrap())
        .collect();
    projects.sort();
    let mut expected = [first.as_str(), second.as_str()];
    expected.sort();
    assert_eq!(projects, expected);
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
        let changeset = new_changeset(f, client, &project);
        let created = ws.events(1);
        let created_in = started.elapsed();
        assert_eq!(name_of(&created[0]), "changeset.open");

        let started = Instant::now();
        agent_acts(f, client, &project, &changeset, "submit");
        let submitted = ws.events(1);
        let submitted_in = started.elapsed();
        assert_eq!(name_of(&submitted[0]), "changeset.in_review");
        slowest = slowest.max(created_in).max(submitted_in);
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
    ws.subscribe(&busy, None);
    ws.subscribe(&quiet, None);

    for _ in 0..3 {
        let changeset = new_changeset(f, client, &busy);
        agent_acts(f, client, &busy, &changeset, "submit");
    }
    new_changeset(f, client, &quiet);

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
    assert_gapless(&busy_events);
    assert!(
        seq_of(&quiet_events[0]) <= 2,
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
                let changeset = new_changeset(f, client, &project);
                agent_acts(f, client, &project, &changeset, "submit");
            });
        }
    });

    let events = ws.events(workers * 2);
    assert_gapless(&events);
    let mut seen_open = Vec::new();
    for event in &events {
        let id = event["data"]["changeset_id"].as_str().unwrap().to_string();
        match name_of(event) {
            "changeset.open" => seen_open.push(id),
            "changeset.in_review" => assert!(
                seen_open.contains(&id),
                "a change set is announced as open before it is in review: {events:#?}"
            ),
            other => panic!("unexpected event {other}"),
        }
    }
    assert_eq!(seen_open.len(), workers);
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

    let one = new_changeset(f, client, &project);
    agent_acts(f, client, &project, &one, "submit");
    let seen = first.events(2);
    let last_seen = seq_of(&seen[1]);
    drop(first);

    // Changes while the client is away.
    agent_acts(f, client, &project, &one, "abandon");
    let two = new_changeset(f, client, &project);
    agent_acts(f, client, &project, &two, "submit");

    // The human's browser comes back with its last number (the same client
    // type as before is not required: resuming is about the number).
    let mut second = open(client, &f.a, &Auth::Session(&f.alice_token));
    let ack = second.subscribe(&project, Some(last_seen));
    assert_eq!(ack["last_seq"], last_seen + 3, "{ack}");
    let missed = second.events(3);
    assert_eq!(
        names(&missed),
        [
            "changeset.abandoned",
            "changeset.open",
            "changeset.in_review"
        ]
    );
    assert_eq!(seq_of(&missed[0]), last_seen + 1);
    assert_gapless(&missed);
    assert_eq!(missed[0]["data"]["changeset_id"], one.as_str());
    assert_eq!(missed[2]["data"]["changeset_id"], two.as_str());
    second.assert_no_event("each missed event is sent once");

    // Live events carry on right after the replay, without a gap or a repeat.
    human_rejects(f, client, &project, &two);
    let live = second.events(1);
    assert_eq!(name_of(&live[0]), "changeset.rejected");
    assert_eq!(seq_of(&live[0]), last_seen + 4);
    second.assert_no_event("nothing is repeated after the replay");
}

#[test]
fn resuming_from_zero_replays_the_whole_history_as_it_was_first_sent() {
    let (f, client) = ready();
    let project = new_project(f, client, "history");
    let mut first = open(client, &f.a, &Auth::Key(&f.key_a));
    first.subscribe(&project, None);
    let changeset = new_changeset(f, client, &project);
    agent_acts(f, client, &project, &changeset, "submit");
    human_rejects(f, client, &project, &changeset);
    let original = first.events(3);

    let mut late = open(client, &f.a, &Auth::Session(&f.alice_token));
    late.subscribe(&project, Some(0));
    let replayed = late.drain();
    assert_gapless(&replayed);
    assert_eq!(
        changeset_events(&replayed),
        original,
        "a replayed event is the event that was sent live, field for field"
    );
}

#[test]
fn resuming_from_the_newest_number_or_without_one_replays_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client, "caught-up");
    let changeset = new_changeset(f, client, &project);
    agent_acts(f, client, &project, &changeset, "submit");
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
    human_rejects(f, client, &project, &changeset);
    for ws in [&mut caught_up, &mut fresh] {
        let events = ws.events(1);
        assert_eq!(seq_of(&events[0]), newest + 1);
    }
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

#[test]
fn events_are_kept_in_the_tenant_table_live_events_under_their_sequence_numbers() {
    let (f, client) = ready();
    let project = new_project(f, client, "stored");
    let mut ws = open(client, &f.a, &Auth::Key(&f.key_a));
    ws.subscribe(&project, None);
    let changeset = new_changeset(f, client, &project);
    agent_acts(f, client, &project, &changeset, "submit");
    let events = ws.events(2);

    for event in &events {
        let stored = db_text(
            "select workspace_id::text || ' ' || event from live_events \
             where project_id = $1::uuid and seq = $2::bigint",
            &[&project, &seq_of(event).to_string()],
        );
        assert_eq!(
            stored,
            Some(format!("{} {}", f.a, name_of(event))),
            "the event with seq {} is stored in workspace A with its name",
            seq_of(event)
        );
    }
}
