//! US-013: API keys, workspace resolution and human-only actions. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed two workspaces with three users
//! (alice is a member of A only, bob of B only, carol of both) and a live session
//! for each, put a probe trigger on `api_keys`, and serve the API's router on a
//! local port against that database with the built-in password login.
//!
//! The contract these tests pin down:
//!
//! - `POST /api/v1/w/{workspace_id}/api-keys` (human) answers 201
//!   `{"key": {...}, "secret": "<the whole key>"}`. The key object has `id`,
//!   `prefix`, `scopes` (`["*"]`: v1 issues full-access keys only), `created_at`,
//!   `last_used_at` and `revoked_at`. `secret` is the public `prefix`, then one
//!   optional separator character, then the 32 random bytes as 64 lowercase hex
//!   digits. It is in no other response. `api_keys.key_hash` holds the lowercase
//!   hex SHA-256 of the whole `secret`.
//! - `GET /api/v1/w/{workspace_id}/api-keys` (human) answers `{"keys": [...]}`
//!   with the key objects of that workspace, revoked ones included.
//! - `POST /api/v1/w/{workspace_id}/api-keys/{key_id}/revoke` (human) answers
//!   200 `{"key": {...}}` with `revoked_at` set. The row is kept.
//! - `GET /api/v1/w/{workspace_id}/whoami` (both) answers 200
//!   `{"workspace_id", "actor": {"kind": "agent"}}` for an API key and
//!   `{"workspace_id", "actor": {"kind": "human", "user_id"}}` for a session.
//! - An API key goes in `Authorization: Bearer <secret>`. A request with no
//!   valid credentials is 401 `{"error": "unauthorized"}`; a workspace the
//!   credentials do not reach is 404 `{"error": "not_found"}`; a `human` route
//!   called with an API key is 403 `{"error": "forbidden"}`.
//! - One "who is calling" step decides agent versus human for every route, the
//!   routes outside `/api/v1/w/{workspace_id}` included: a request that carries
//!   an `Authorization` header is an agent's, even with a session cookie too,
//!   so `/api/v1/auth/me`, `/api/v1/auth/logout`, `/api/v1/auth/workspaces` and
//!   every other `human` route answer it 403. Only `POST /api/v1/auth/login`,
//!   which checks a password, is exempt.
//! - `AppState::with_pool(config, pool)` builds the state over a pool the
//!   caller made, so a test can size it and keep a handle on its connections.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::password::hash_password;
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::{Barrier, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

const TEST_DB: &str = "omni_us013_keys";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";

fn block<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(future)
}

fn admin_options() -> PgConnectOptions {
    let url = std::env::var("OMNI_ADMIN_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://omni:omni-dev@postgres:5432/omni".to_string());
    PgConnectOptions::from_str(&url).expect("OMNI_ADMIN_DATABASE_URL")
}

fn api_password() -> String {
    // The password that the stack's API gives omni_api (docker-compose.yml). The
    // role is cluster-wide, so the scratch database must use the same one.
    std::env::var("OMNI_API_DB_PASSWORD").unwrap_or_else(|_| "omni-api-dev".to_string())
}

fn config() -> Config {
    let admin = admin_options();
    let url = std::env::var("OMNI_ADMIN_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://omni:omni-dev@postgres:5432/omni".to_string());
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

fn db_exec(sql: &str, binds: &[&str]) {
    block(async {
        let mut conn = admin_connection().await;
        let mut query = sqlx::query(sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("statement failed ({e}): {sql}"));
    })
}

/// Waits up to 3 s for `sql` to answer `true`, for writes the API may do just
/// after it has answered.
fn eventually(sql: &str, binds: &[&str]) -> bool {
    let started = Instant::now();
    loop {
        if db_text(sql, binds).as_deref() == Some("true") {
            return true;
        }
        if started.elapsed() > Duration::from_secs(3) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct Fixture {
    a: String,
    b: String,
    alice_id: String,
    alice_token: String,
    bob_token: String,
    carol_token: String,
    server: TestServer,
}

/// Records, for every insert and update of an API key, the `app.workspace_id`
/// setting of the transaction that did it.
const PROBE_SQL: [&str; 3] = [
    "CREATE TABLE us013_probe (
        n bigserial PRIMARY KEY,
        op text NOT NULL,
        setting text,
        key_workspace text NOT NULL,
        key_id text NOT NULL,
        last_used_changed boolean NOT NULL,
        revoked_changed boolean NOT NULL,
        pid integer NOT NULL DEFAULT pg_backend_pid()
    )",
    "CREATE FUNCTION us013_probe_fn() RETURNS trigger
        LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public
        AS $$
        BEGIN
            INSERT INTO us013_probe (op, setting, key_workspace, key_id, last_used_changed, revoked_changed)
            VALUES (
                TG_OP,
                current_setting('app.workspace_id', true),
                NEW.workspace_id::text,
                NEW.id::text,
                TG_OP = 'UPDATE' AND NEW.last_used_at IS DISTINCT FROM OLD.last_used_at,
                TG_OP = 'UPDATE' AND NEW.revoked_at IS DISTINCT FROM OLD.revoked_at
            );
            RETURN NEW;
        END
        $$",
    "CREATE TRIGGER us013_probe BEFORE INSERT OR UPDATE ON api_keys
        FOR EACH ROW EXECUTE FUNCTION us013_probe_fn()",
];

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
        let (a, b, alice_id) = block(async {
            let mut conn = admin_connection().await;
            for statement in PROBE_SQL {
                sqlx::query(statement)
                    .execute(&mut conn)
                    .await
                    .expect("install the probe");
            }
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
            let mut users = Vec::new();
            for name in ["alice", "bob", "carol"] {
                let id = text(
                    "insert into users (email) values ($1) returning id::text",
                    &[&format!("{name}@example.test")],
                )
                .await;
                text(
                    "insert into sessions (user_id, token_hash, expires_at) \
                     values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
                     returning id::text",
                    &[&id, &session_token(name)],
                )
                .await;
                users.push(id);
            }
            let member = "insert into memberships (workspace_id, user_id) \
                          values ($1::uuid, $2::uuid) returning id::text";
            text(member, &[&a, &users[0]]).await;
            text(member, &[&b, &users[1]]).await;
            text(member, &[&a, &users[2]]).await;
            text(member, &[&b, &users[2]]).await;
            (a, b, users[0].clone())
        });
        Fixture {
            a,
            b,
            alice_id,
            alice_token: session_token("alice"),
            bob_token: session_token("bob"),
            carol_token: session_token("carol"),
            server: TestServer::start(&config()),
        }
    })
}

fn session_token(name: &str) -> String {
    format!("us013-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
}

/// The fixture. The guard serialises the tests, because several of them count
/// the writes that the probe saw.
fn ready() -> (MutexGuard<'static, ()>, &'static Fixture, &'static Client) {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = fixture();
    (guard, fixture, &fixture.server.client)
}

fn with_cookie(token: &str) -> Vec<(String, String)> {
    vec![("Cookie".to_string(), cookie(token))]
}

fn with_bearer(secret: &str) -> Vec<(String, String)> {
    vec![("Authorization".to_string(), format!("Bearer {secret}"))]
}

fn call(
    client: &Client,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<&Value>,
) -> Response {
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    let body = body.map(Value::to_string);
    client.request(method, path, &headers, body.as_deref())
}

fn whoami(client: &Client, workspace: &str, headers: &[(String, String)]) -> Response {
    call(
        client,
        "GET",
        &format!("/api/v1/w/{workspace}/whoami"),
        headers,
        None,
    )
}

/// Creates a key in `workspace` as the session `token`; returns (secret, key).
fn create_key(client: &Client, token: &str, workspace: &str) -> (String, Value) {
    let response = call(
        client,
        "POST",
        &format!("/api/v1/w/{workspace}/api-keys"),
        &with_cookie(token),
        Some(&json!({})),
    );
    assert_eq!(response.status, 201, "create a key: {}", response.text());
    let body = response.json();
    let secret = body["secret"]
        .as_str()
        .unwrap_or_else(|| panic!("the response has a `secret`: {body}"))
        .to_string();
    (secret, body["key"].clone())
}

fn key_id(key: &Value) -> String {
    key["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the key has an `id`: {key}"))
        .to_string()
}

fn assert_error(response: &Response, status: u16, code: &str) {
    assert_eq!(response.status, status, "body: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": code }),
        "the error body carries the code and nothing else"
    );
}

// ---------------------------------------------------------------------------
// Key format, hashing and lookup
// ---------------------------------------------------------------------------

#[test]
fn a_key_is_a_public_prefix_plus_32_random_bytes() {
    let (_guard, f, client) = ready();
    let (secret_1, key_1) = create_key(client, &f.carol_token, &f.a);
    let (secret_2, key_2) = create_key(client, &f.carol_token, &f.a);
    for (secret, key) in [(&secret_1, &key_1), (&secret_2, &key_2)] {
        let prefix = key["prefix"]
            .as_str()
            .unwrap_or_else(|| panic!("the key has a `prefix`: {key}"));
        assert!(
            !prefix.is_empty() && prefix.len() <= 32,
            "prefix {prefix:?}"
        );
        assert!(
            prefix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "the prefix is plain ASCII: {prefix:?}"
        );
        let rest = secret
            .strip_prefix(prefix)
            .unwrap_or_else(|| panic!("the secret {secret:?} starts with its prefix {prefix:?}"));
        let random = rest.strip_prefix(['_', '-', '.', ':']).unwrap_or(rest);
        assert!(
            random.len() == 64
                && random
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "after the prefix come 32 random bytes as 64 lowercase hex digits: {random:?}"
        );
    }
    assert_ne!(secret_1, secret_2, "every key is random");
    assert_ne!(key_1["prefix"], key_2["prefix"], "prefixes are unique");
}

#[test]
fn a_key_is_shown_once_and_stored_as_a_sha256_hash() {
    let (_guard, f, client) = ready();
    let (secret, key) = create_key(client, &f.carol_token, &f.a);
    let id = key_id(&key);
    let create_text = serde_json::to_string(&key).unwrap();
    assert!(
        !create_text.contains("key_hash"),
        "the key object does not expose the hash: {create_text}"
    );

    let stored = db_text(
        "select (key_hash = encode(sha256(convert_to($2, 'UTF8')), 'hex'))::text from api_keys where id = $1::uuid",
        &[&id, &secret],
    );
    assert_eq!(
        stored.as_deref(),
        Some("true"),
        "key_hash is the lowercase hex SHA-256 of the whole secret"
    );
    let tail = &secret[secret.len() - 64..];
    let leaked = db_text(
        "select count(*)::text from api_keys where api_keys::text like '%' || $1 || '%'",
        &[tail],
    );
    assert_eq!(
        leaked.as_deref(),
        Some("0"),
        "no column of any row holds the random part of the key"
    );

    // Neither the list nor the revoke response shows the secret or the hash again.
    let hash = db_text("select key_hash from api_keys where id = $1::uuid", &[&id]).unwrap();
    let list = call(
        client,
        "GET",
        &format!("/api/v1/w/{}/api-keys", f.a),
        &with_cookie(&f.carol_token),
        None,
    );
    assert_eq!(list.status, 200, "list keys: {}", list.text());
    let revoke = call(
        client,
        "POST",
        &format!("/api/v1/w/{}/api-keys/{id}/revoke", f.a),
        &with_cookie(&f.carol_token),
        Some(&json!({})),
    );
    assert_eq!(revoke.status, 200, "revoke: {}", revoke.text());
    for response in [&list, &revoke] {
        let text = response.text();
        assert!(!text.contains(tail), "the secret shows again: {text}");
        assert!(!text.contains(&hash), "the hash shows: {text}");
        assert!(!text.contains("key_hash"), "the hash shows: {text}");
    }
}

#[test]
fn a_key_is_found_by_prefix_and_its_secret_is_checked() {
    let (_guard, f, client) = ready();
    let (secret, key) = create_key(client, &f.carol_token, &f.a);
    let prefix = key["prefix"].as_str().unwrap().to_string();
    let ok = whoami(client, &f.a, &with_bearer(&secret));
    assert_eq!(ok.status, 200, "the right secret works: {}", ok.text());

    let last = secret.chars().last().unwrap();
    let flipped = if last == '0' { '1' } else { '0' };
    let wrong_tail = format!("{}{flipped}", &secret[..secret.len() - 1]);
    let unknown_prefix = format!("nokey{}", &secret[prefix.len().min(secret.len())..]);
    let attempts = [
        ("a wrong secret with the right prefix", wrong_tail),
        ("an unknown prefix", unknown_prefix),
        ("the prefix alone", prefix.clone()),
        ("an empty key", String::new()),
    ];
    for (what, bad) in attempts {
        let response = whoami(client, &f.a, &with_bearer(&bad));
        assert_error(&response, 401, "unauthorized");
        assert!(
            response.session_set_cookie().is_none(),
            "{what} starts no session"
        );
    }
    for header in [
        format!("Basic {secret}"),
        secret.clone(),
        format!("Bearer{secret}"),
    ] {
        let response = whoami(
            client,
            &f.a,
            &[("Authorization".to_string(), header.clone())],
        );
        assert_error(&response, 401, "unauthorized");
    }
}

// ---------------------------------------------------------------------------
// One workspace, scopes, revoking and last use
// ---------------------------------------------------------------------------

#[test]
fn a_revoked_key_stops_working_and_its_row_stays() {
    let (_guard, f, client) = ready();
    let (secret, key) = create_key(client, &f.carol_token, &f.a);
    let id = key_id(&key);
    assert_eq!(whoami(client, &f.a, &with_bearer(&secret)).status, 200);

    // A key of workspace B cannot be revoked through workspace A's path.
    let (other_secret, other_key) = create_key(client, &f.carol_token, &f.b);
    let wrong_path = call(
        client,
        "POST",
        &format!("/api/v1/w/{}/api-keys/{}/revoke", f.a, key_id(&other_key)),
        &with_cookie(&f.carol_token),
        Some(&json!({})),
    );
    assert_error(&wrong_path, 404, "not_found");
    assert_eq!(
        whoami(client, &f.b, &with_bearer(&other_secret)).status,
        200
    );
    let unknown = call(
        client,
        "POST",
        &format!("/api/v1/w/{}/api-keys/{UNKNOWN_ID}/revoke", f.a),
        &with_cookie(&f.carol_token),
        Some(&json!({})),
    );
    assert_error(&unknown, 404, "not_found");

    let revoke = call(
        client,
        "POST",
        &format!("/api/v1/w/{}/api-keys/{id}/revoke", f.a),
        &with_cookie(&f.carol_token),
        Some(&json!({})),
    );
    assert_eq!(revoke.status, 200, "revoke: {}", revoke.text());
    assert!(
        revoke.json()["key"]["revoked_at"].is_string(),
        "the revoked key shows revoked_at: {}",
        revoke.text()
    );
    assert_error(
        &whoami(client, &f.a, &with_bearer(&secret)),
        401,
        "unauthorized",
    );

    assert_eq!(
        db_text(
            "select (revoked_at is not null)::text from api_keys where id = $1::uuid",
            &[&id]
        )
        .as_deref(),
        Some("true"),
        "the row is kept with revoked_at set"
    );
    let list = call(
        client,
        "GET",
        &format!("/api/v1/w/{}/api-keys", f.a),
        &with_cookie(&f.carol_token),
        None,
    );
    let body = list.json();
    let listed = body["keys"]
        .as_array()
        .and_then(|keys| keys.iter().find(|k| k["id"] == key["id"]))
        .unwrap_or_else(|| panic!("the revoked key is still listed: {body}"));
    assert!(
        listed["revoked_at"].is_string(),
        "listed as revoked: {listed}"
    );
}

#[test]
fn last_used_at_is_written_at_most_once_a_minute() {
    let (_guard, f, client) = ready();
    let (secret, key) = create_key(client, &f.carol_token, &f.a);
    let id = key_id(&key);
    let used_in = |window: &str| {
        format!(
            "select (last_used_at > now() - interval '{window}')::text from api_keys where id = $1::uuid"
        )
    };
    let writes = "select count(*)::text from us013_probe \
                  where key_id = $1 and op = 'UPDATE' and last_used_changed";
    db_exec("delete from us013_probe", &[]);

    assert_eq!(whoami(client, &f.a, &with_bearer(&secret)).status, 200);
    assert!(
        eventually(&used_in("1 minute"), &[&id]),
        "the first use sets last_used_at"
    );
    for _ in 0..4 {
        assert_eq!(whoami(client, &f.a, &with_bearer(&secret)).status, 200);
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        db_text(writes, &[&id]).as_deref(),
        Some("1"),
        "five calls within a minute write last_used_at once"
    );
    assert_eq!(
        db_text(
            "select setting from us013_probe where key_id = $1 and last_used_changed",
            &[&id]
        )
        .as_deref(),
        Some(f.a.as_str()),
        "last_used_at is written inside a transaction of the key's workspace"
    );

    // Two minutes later, the next call writes it again.
    db_exec(
        "update api_keys set last_used_at = now() - interval '2 minutes' where id = $1::uuid",
        &[&id],
    );
    db_exec("delete from us013_probe", &[]);
    assert_eq!(whoami(client, &f.a, &with_bearer(&secret)).status, 200);
    assert!(
        eventually(&used_in("30 seconds"), &[&id]),
        "a call after a minute writes last_used_at again"
    );
    assert_eq!(db_text(writes, &[&id]).as_deref(), Some("1"));

    let list = call(
        client,
        "GET",
        &format!("/api/v1/w/{}/api-keys", f.a),
        &with_cookie(&f.carol_token),
        None,
    );
    let body = list.json();
    let listed = body["keys"]
        .as_array()
        .and_then(|keys| keys.iter().find(|k| k["id"] == key["id"]))
        .unwrap_or_else(|| panic!("the key is listed: {body}"));
    assert!(
        listed["last_used_at"].is_string(),
        "the list shows last_used_at: {listed}"
    );
}

// ---------------------------------------------------------------------------
// Workspace resolution
// ---------------------------------------------------------------------------

#[test]
fn a_key_reaches_its_own_workspace_and_any_other_path_is_404() {
    let (_guard, f, client) = ready();
    let (secret_a, _) = create_key(client, &f.carol_token, &f.a);
    let (secret_b, _) = create_key(client, &f.carol_token, &f.b);

    for (secret, own, other) in [(&secret_a, &f.a, &f.b), (&secret_b, &f.b, &f.a)] {
        let ok = whoami(client, own, &with_bearer(secret));
        assert_eq!(ok.status, 200, "own workspace: {}", ok.text());
        assert_eq!(ok.json()["workspace_id"], json!(own));
        assert_error(
            &whoami(client, other, &with_bearer(secret)),
            404,
            "not_found",
        );
    }
    let unknown = whoami(client, UNKNOWN_ID, &with_bearer(&secret_a));
    assert_error(&unknown, 404, "not_found");
    let malformed = whoami(client, "not-a-uuid", &with_bearer(&secret_a));
    assert_error(&malformed, 404, "not_found");
}

#[test]
fn a_session_reaches_only_the_workspaces_it_is_a_member_of() {
    let (_guard, f, client) = ready();
    for (token, workspace) in [
        (&f.alice_token, f.a.as_str()),
        (&f.bob_token, f.b.as_str()),
        (&f.carol_token, f.a.as_str()),
        (&f.carol_token, f.b.as_str()),
    ] {
        let response = whoami(client, workspace, &with_cookie(token));
        assert_eq!(
            response.status,
            200,
            "session {token} on workspace {workspace}: {}",
            response.text()
        );
        assert_eq!(response.json()["workspace_id"], json!(workspace));
    }
    for (token, workspace) in [
        (&f.alice_token, f.b.as_str()),
        (&f.bob_token, f.a.as_str()),
        (&f.carol_token, UNKNOWN_ID),
    ] {
        let response = whoami(client, workspace, &with_cookie(token));
        assert_error(&response, 404, "not_found");
    }
}

#[test]
fn a_request_without_valid_credentials_is_401_whatever_the_workspace() {
    let (_guard, f, client) = ready();
    for workspace in [f.a.as_str(), UNKNOWN_ID, "not-a-uuid"] {
        assert_error(&whoami(client, workspace, &[]), 401, "unauthorized");
    }
    let bad_cookie = whoami(client, &f.a, &with_cookie("not-a-session"));
    assert_error(&bad_cookie, 401, "unauthorized");
}

// ---------------------------------------------------------------------------
// Agents and the human
// ---------------------------------------------------------------------------

#[test]
fn api_key_requests_act_as_an_agent_and_only_sessions_act_as_the_human() {
    let (_guard, f, client) = ready();
    let (secret, _) = create_key(client, &f.carol_token, &f.a);

    let agent = whoami(client, &f.a, &with_bearer(&secret));
    assert_eq!(agent.status, 200, "agent: {}", agent.text());
    assert_eq!(agent.json()["actor"]["kind"], "agent");

    let human = whoami(client, &f.a, &with_cookie(&f.alice_token));
    assert_eq!(human.status, 200, "human: {}", human.text());
    assert_eq!(human.json()["actor"]["kind"], "human");
    assert_eq!(human.json()["actor"]["user_id"], json!(f.alice_id));

    // A request that carries both is an agent's: the key wins over the cookie.
    let mut both = with_bearer(&secret);
    both.extend(with_cookie(&f.carol_token));
    let mixed = whoami(client, &f.a, &both);
    assert_eq!(mixed.status, 200, "both: {}", mixed.text());
    assert_eq!(mixed.json()["actor"]["kind"], "agent");
    let list = call(
        client,
        "GET",
        &format!("/api/v1/w/{}/api-keys", f.a),
        &both,
        None,
    );
    assert_error(&list, 403, "forbidden");
}

#[test]
fn human_routes_return_403_to_api_keys() {
    let (_guard, f, client) = ready();
    let (secret, key) = create_key(client, &f.carol_token, &f.a);
    let id = key_id(&key);
    let keys = with_bearer(&secret);

    let attempts = [
        ("GET", format!("/api/v1/w/{}/api-keys", f.a), None),
        (
            "POST",
            format!("/api/v1/w/{}/api-keys", f.a),
            Some(json!({})),
        ),
        (
            "POST",
            format!("/api/v1/w/{}/api-keys/{id}/revoke", f.a),
            Some(json!({})),
        ),
        (
            "POST",
            format!("/api/v1/w/{}/api-keys/{UNKNOWN_ID}/revoke", f.a),
            Some(json!({})),
        ),
    ];
    for (method, path, body) in &attempts {
        let response = call(client, method, path, &keys, body.as_ref());
        assert_error(&response, 403, "forbidden");
    }
    // The 403 changed nothing: the key still works and is not revoked.
    assert_eq!(
        whoami(client, &f.a, &keys).status,
        200,
        "the key is not revoked"
    );
    assert_eq!(
        db_text(
            "select (revoked_at is null)::text from api_keys where id = $1::uuid",
            &[&id]
        )
        .as_deref(),
        Some("true")
    );

    // The same calls work for the session, and without credentials they are 401.
    let session = call(
        client,
        "GET",
        &format!("/api/v1/w/{}/api-keys", f.a),
        &with_cookie(&f.alice_token),
        None,
    );
    assert_eq!(
        session.status,
        200,
        "the session lists keys: {}",
        session.text()
    );
    let anonymous = call(
        client,
        "GET",
        &format!("/api/v1/w/{}/api-keys", f.a),
        &[],
        None,
    );
    assert_error(&anonymous, 401, "unauthorized");
}

// ---------------------------------------------------------------------------
// Reopened after the independent check
// ---------------------------------------------------------------------------

/// A new live session of `user_id` under a token of its own, so that a test
/// that may end its session (logout) cannot take the shared ones with it.
fn fresh_session(user_id: &str, label: &str) -> String {
    let token = format!("us013-fresh-{label}-0123456789abcdef0123456789abcdef0123456789abcd");
    db_text(
        "insert into sessions (user_id, token_hash, expires_at) \
         values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
         returning id::text",
        &[user_id, &token],
    );
    token
}

#[test]
fn the_auth_routes_never_answer_a_request_with_an_authorization_header_as_the_human() {
    let (_guard, f, client) = ready();
    let (secret, key) = create_key(client, &f.carol_token, &f.a);
    let token = fresh_session(&f.alice_id, "auth-routes");
    let routes = [
        ("GET", "/api/v1/auth/me"),
        ("POST", "/api/v1/auth/logout"),
        ("GET", "/api/v1/auth/workspaces"),
    ];

    // The session alone is the human's, on these routes as on the workspace ones.
    let me = call(client, "GET", "/api/v1/auth/me", &with_cookie(&token), None);
    assert_eq!(me.status, 200, "the session is live: {}", me.text());

    // A key, with or without a session cookie, is an agent: 403, and the 403
    // changes nothing (a logout that carries a key does not end the session).
    for (method, path) in routes {
        let body = (method != "GET").then(|| json!({}));
        let mut headers = with_bearer(&secret);
        assert_error(
            &call(client, method, path, &headers, body.as_ref()),
            403,
            "forbidden",
        );
        headers.extend(with_cookie(&token));
        assert_error(
            &call(client, method, path, &headers, body.as_ref()),
            403,
            "forbidden",
        );
    }
    let still = call(client, "GET", "/api/v1/auth/me", &with_cookie(&token), None);
    assert_eq!(
        still.status,
        200,
        "the logout that carried a key did not end the session: {}",
        still.text()
    );
    assert_eq!(still.json()["user"]["id"], json!(f.alice_id));

    // A header that is not a valid key never turns into the human either: the
    // cookie next to it is not used, so the request is not answered as alice.
    let revoke = call(
        client,
        "POST",
        &format!("/api/v1/w/{}/api-keys/{}/revoke", f.a, key_id(&key)),
        &with_cookie(&f.carol_token),
        Some(&json!({})),
    );
    assert_eq!(revoke.status, 200, "revoke: {}", revoke.text());
    for authorization in [
        format!("Bearer {secret}"),
        "Bearer omni_not-a-key".to_string(),
        "Basic YWxpY2U6c2VjcmV0".to_string(),
    ] {
        let mut headers = vec![("Authorization".to_string(), authorization.clone())];
        headers.extend(with_cookie(&token));
        for (method, path) in routes {
            let body = (method != "GET").then(|| json!({}));
            let response = call(client, method, path, &headers, body.as_ref());
            assert_error(&response, 401, "unauthorized");
        }
    }
    let after = call(client, "GET", "/api/v1/auth/me", &with_cookie(&token), None);
    assert_eq!(
        after.status,
        200,
        "none of those calls ended the session: {}",
        after.text()
    );
    assert_eq!(after.json()["user"]["id"], json!(f.alice_id));
}

#[test]
fn login_is_the_one_route_that_an_authorization_header_does_not_block() {
    let (_guard, f, client) = ready();
    let (secret, _) = create_key(client, &f.carol_token, &f.a);
    let user_id = db_text(
        "insert into users (email) values ('dave@example.test') returning id::text",
        &[],
    )
    .expect("insert a user");
    let hash = hash_password("us013 correct horse battery staple").expect("hash a password");
    db_text(
        "insert into login_identities (user_id, provider, subject, secret_hash) \
         values ($1::uuid, 'password', 'dave@example.test', $2) returning id::text",
        &[&user_id, &hash],
    );

    let login = call(
        client,
        "POST",
        "/api/v1/auth/login",
        &with_bearer(&secret),
        Some(&json!({
            "email": "dave@example.test",
            "password": "us013 correct horse battery staple",
        })),
    );
    assert_eq!(
        login.status,
        200,
        "login checks a password, so a key header does not block it: {}",
        login.text()
    );
    assert_eq!(login.json()["user"]["email"], "dave@example.test");
    assert!(login.session_token().is_some(), "login starts a session");

    // The header does not make a wrong password right either.
    let wrong = call(
        client,
        "POST",
        "/api/v1/auth/login",
        &with_bearer(&secret),
        Some(&json!({ "email": "dave@example.test", "password": "wrong" })),
    );
    assert_error(&wrong, 401, "invalid_credentials");
}

#[test]
fn app_workspace_id_lasts_only_for_its_transaction() {
    let (_guard, f, _client) = ready();
    // The API gets one connection, so every query it runs, and the checks
    // below, use the same physical connection.
    let (server, pool) = TestServer::start_with_pool_size(&config(), 1);
    let client = &server.client;
    let setting = || -> Option<String> {
        server.block_on(async {
            sqlx::query_scalar::<_, Option<String>>(
                "select current_setting('app.workspace_id', true)",
            )
            .fetch_one(&pool)
            .await
            .expect("read the setting on the API's connection")
        })
    };
    let backend_pid = || -> String {
        server.block_on(async {
            sqlx::query_scalar::<_, String>("select pg_backend_pid()::text")
                .fetch_one(&pool)
                .await
                .expect("read the backend pid of the API's connection")
        })
    };
    let assert_unset = |after: &str| {
        let value = setting();
        assert!(
            value.as_deref().is_none_or(str::is_empty),
            "after {after}, a connection of the pool still has app.workspace_id = {value:?}"
        );
    };

    db_exec("delete from us013_probe", &[]);
    let created = call(
        client,
        "POST",
        &format!("/api/v1/w/{}/api-keys", f.b),
        &with_cookie(&f.carol_token),
        Some(&json!({})),
    );
    assert_eq!(created.status, 201, "create: {}", created.text());
    let secret = created.json()["secret"].as_str().unwrap().to_string();
    let id = key_id(&created.json()["key"]);
    assert_eq!(
        db_text(
            "select setting from us013_probe where key_id = $1 and op = 'INSERT'",
            &[&id]
        )
        .as_deref(),
        Some(f.b.as_str()),
        "the request did set app.workspace_id to B inside its transaction"
    );
    assert_eq!(
        db_text(
            "select pid::text from us013_probe where key_id = $1 and op = 'INSERT'",
            &[&id]
        ),
        Some(backend_pid()),
        "the request ran on the one connection of the pool"
    );
    assert_eq!(server.block_on(async { pool.size() }), 1);
    assert_unset("a committed request");

    // A request that fails after its transaction began leaves nothing behind.
    let missing = call(
        client,
        "POST",
        &format!("/api/v1/w/{}/api-keys/{UNKNOWN_ID}/revoke", f.b),
        &with_cookie(&f.carol_token),
        Some(&json!({})),
    );
    assert_error(&missing, 404, "not_found");
    assert_unset("a request that ended in 404");

    let listed = call(
        client,
        "GET",
        &format!("/api/v1/w/{}/api-keys", f.b),
        &with_cookie(&f.carol_token),
        None,
    );
    assert_eq!(listed.status, 200, "list: {}", listed.text());
    assert_unset("a list");

    // An agent's request, which also writes last_used_at in its own transaction.
    assert_eq!(whoami(client, &f.b, &with_bearer(&secret)).status, 200);
    assert!(
        eventually(
            "select (last_used_at is not null)::text from api_keys where id = $1::uuid",
            &[&id]
        ),
        "the first use of the key is recorded"
    );
    assert_unset("an agent request and its last_used_at write");
    assert_eq!(server.block_on(async { pool.size() }), 1);
}

#[test]
fn ten_concurrent_requests_with_a_stale_key_write_last_used_at_once() {
    let (_guard, f, client) = ready();
    let (secret, key) = create_key(client, &f.carol_token, &f.a);
    let id = key_id(&key);
    db_exec(
        "update api_keys set last_used_at = now() - interval '2 minutes' where id = $1::uuid",
        &[&id],
    );
    db_exec("delete from us013_probe", &[]);

    let barrier = Barrier::new(10);
    let statuses: Vec<u16> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..10)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    whoami(client, &f.a, &with_bearer(&secret)).status
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(statuses, vec![200; 10], "every request is let in");

    assert!(
        eventually(
            "select (last_used_at > now() - interval '30 seconds')::text from api_keys \
             where id = $1::uuid",
            &[&id]
        ),
        "the stale last_used_at is refreshed"
    );
    // Let any further write that the other requests would do land.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        db_text(
            "select count(*)::text from us013_probe \
             where key_id = $1 and op = 'UPDATE' and last_used_changed",
            &[&id]
        )
        .as_deref(),
        Some("1"),
        "ten concurrent requests write last_used_at exactly once"
    );
}
