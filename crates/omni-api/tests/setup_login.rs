//! US-012: first-run setup, Argon2id password identities, and cookie sessions.
//! Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432`.
//!
//! The tests create a scratch database, run the `omni-api setup` command on it
//! (which applies the migrations like start-up does, then creates the first
//! user, the password identity, the workspace and the membership, reading the
//! password from standard input), run the command a second time, and then serve
//! the API's router on a local port against that database with the built-in
//! password login. The environment the command and the router get is the one
//! of the API container: OMNI_DATABASE_URL (the `omni_api` login),
//! OMNI_ADMIN_DATABASE_URL, OMNI_API_DB_PASSWORD and the S3 settings.

mod common;

use common::{Client, Response, SESSION_COOKIE, TestServer, cookie};
use omni_api::Config;
use serde_json::json;
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

const TEST_DB: &str = "omni_us012_auth";
const OWNER_EMAIL: &str = "Owner@Example.test";
const OWNER_PASSWORD: &str = "correct horse battery staple 7";
const WORKSPACE_NAME: &str = "Acme Studio";
const SETUP_TIMEOUT: Duration = Duration::from_secs(25);

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

/// A connection URL for the scratch database, as `user`.
fn scratch_url(user: &str, password: &str) -> String {
    let admin = admin_options();
    format!(
        "postgres://{user}:{password}@{}:{}/{TEST_DB}",
        admin.get_host(),
        admin.get_port()
    )
}

fn admin_scratch_url() -> String {
    let url = std::env::var("OMNI_ADMIN_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://omni:omni-dev@postgres:5432/omni".to_string());
    let (server, _database) = url.rsplit_once('/').expect("OMNI_ADMIN_DATABASE_URL");
    format!("{server}/{TEST_DB}")
}

fn config() -> Config {
    Config {
        bind: "127.0.0.1:0".to_string(),
        database_url: scratch_url("omni_api", &api_password()),
        admin_database_url: admin_scratch_url(),
        api_db_password: api_password(),
        // Nothing here reaches RustFS: the setup command and the login routes
        // only use Postgres.
        s3_endpoint: "http://127.0.0.1:1".to_string(),
        s3_access_key: "none".to_string(),
        s3_secret_key: "none".to_string(),
        s3_bucket: "omni".to_string(),
    }
}

struct SetupRun {
    success: bool,
    timed_out: bool,
    stdout: String,
    stderr: String,
}

impl SetupRun {
    fn output(&self) -> String {
        format!(
            "timed out: {}\nstdout: {}\nstderr: {}",
            self.timed_out, self.stdout, self.stderr
        )
    }
}

/// Runs `omni-api setup` against the scratch database, killing it after
/// SETUP_TIMEOUT, and feeds `password` to its standard input.
fn run_setup(password: &str) -> SetupRun {
    let config = config();
    let mut child = Command::new(env!("CARGO_BIN_EXE_omni-api"))
        .args([
            "setup",
            "--email",
            OWNER_EMAIL,
            "--workspace",
            WORKSPACE_NAME,
            "--password-stdin",
        ])
        .env("OMNI_DATABASE_URL", &config.database_url)
        .env("OMNI_ADMIN_DATABASE_URL", &config.admin_database_url)
        .env("OMNI_API_DB_PASSWORD", &config.api_db_password)
        .env("OMNI_S3_ENDPOINT", &config.s3_endpoint)
        .env("OMNI_S3_ACCESS_KEY", &config.s3_access_key)
        .env("OMNI_S3_SECRET_KEY", &config.s3_secret_key)
        .env("OMNI_S3_BUCKET", &config.s3_bucket)
        .env("OMNI_API_BIND", "127.0.0.1:0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start omni-api");
    let mut stdin = child.stdin.take().unwrap();
    let _ = writeln!(stdin, "{password}");
    drop(stdin);

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for omni-api") {
            break status;
        }
        if started.elapsed() > SETUP_TIMEOUT {
            timed_out = true;
            let _ = child.kill();
            break child.wait().expect("reap omni-api");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    let _ = child.stdout.take().unwrap().read_to_string(&mut stdout);
    let _ = child.stderr.take().unwrap().read_to_string(&mut stderr);
    SetupRun {
        success: status.success() && !timed_out,
        timed_out,
        stdout,
        stderr,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Counts {
    users: i64,
    identities: i64,
    workspaces: i64,
    memberships: i64,
}

async fn count(conn: &mut PgConnection, table: &str) -> i64 {
    // -1 when the table is missing, so the assertions say what is wrong.
    sqlx::query_scalar(&format!("select count(*) from {table}"))
        .fetch_one(conn)
        .await
        .unwrap_or(-1)
}

async fn admin_connection() -> PgConnection {
    let options = admin_options().database(TEST_DB);
    PgConnection::connect_with(&options)
        .await
        .expect("connect to the scratch database as the admin role")
}

fn counts() -> Counts {
    block(async {
        let mut conn = admin_connection().await;
        Counts {
            users: count(&mut conn, "users").await,
            identities: count(&mut conn, "login_identities").await,
            workspaces: count(&mut conn, "workspaces").await,
            memberships: count(&mut conn, "memberships").await,
        }
    })
}

fn session_count() -> i64 {
    block(async { count(&mut admin_connection().await, "sessions").await })
}

struct Fixture {
    first: SetupRun,
    counts_after_first: Counts,
    second: Option<SetupRun>,
    counts_after_second: Option<Counts>,
    server: Option<TestServer>,
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
        });
        let first = run_setup(OWNER_PASSWORD);
        let counts_after_first = counts();
        let (second, counts_after_second) = if first.success {
            (Some(run_setup("another password entirely")), Some(counts()))
        } else {
            (None, None)
        };
        let server = first.success.then(|| TestServer::start(&config()));
        Fixture {
            first,
            counts_after_first,
            second,
            counts_after_second,
            server,
        }
    })
}

/// The fixture, once the setup command has worked. The guard serialises the
/// tests, because several of them count sessions.
fn ready() -> (MutexGuard<'static, ()>, &'static Fixture, &'static Client) {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = fixture();
    assert!(
        fixture.first.success,
        "`omni-api setup` must succeed on a fresh database:\n{}",
        fixture.first.output()
    );
    let client = &fixture.server.as_ref().expect("API server").client;
    (guard, fixture, client)
}

fn login(client: &Client, email: &str, password: &str) -> Response {
    client.post_json(
        "/api/v1/auth/login",
        &json!({ "email": email, "password": password }),
        &[],
    )
}

fn owner_login(client: &Client) -> Response {
    let response = login(client, OWNER_EMAIL, OWNER_PASSWORD);
    assert_eq!(response.status, 200, "login body: {}", response.text());
    response
}

fn token_state(token: &str) -> Option<(bool, bool)> {
    // (not revoked, not expired) of the session whose token hash is SHA-256(token).
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_as::<_, (bool, bool)>(
            "select revoked_at is null, expires_at > now() from sessions \
             where token_hash = sha256($1)",
        )
        .bind(token.as_bytes().to_vec())
        .fetch_optional(&mut conn)
        .await
        .expect("query sessions")
    })
}

// ---------------------------------------------------------------------------
// The one-time setup command
// ---------------------------------------------------------------------------

#[test]
fn setup_creates_the_first_user_password_and_workspace() {
    let (_guard, fixture, _client) = ready();
    assert_eq!(
        fixture.counts_after_first,
        Counts {
            users: 1,
            identities: 1,
            workspaces: 1,
            memberships: 1
        },
        "rows after the first setup run"
    );
    block(async {
        let mut conn = admin_connection().await;
        let (email, workspace, member): (String, String, bool) = sqlx::query_as(
            "select u.email, w.name, m.workspace_id = w.id and m.user_id = u.id \
             from users u, workspaces w, memberships m",
        )
        .fetch_one(&mut conn)
        .await
        .expect("the user, workspace and membership");
        assert_eq!(email.to_lowercase(), OWNER_EMAIL.to_lowercase());
        assert_eq!(workspace, WORKSPACE_NAME);
        assert!(member, "the user is a member of the workspace");
    });
}

#[test]
fn setup_never_prints_the_password() {
    let (_guard, fixture, _client) = ready();
    for run in [Some(&fixture.first), fixture.second.as_ref()]
        .into_iter()
        .flatten()
    {
        assert!(
            !run.stdout.contains(OWNER_PASSWORD) && !run.stderr.contains(OWNER_PASSWORD),
            "the password appears in the output of setup:\n{}",
            run.output()
        );
    }
}

#[test]
fn setup_runs_only_once() {
    let (_guard, fixture, _client) = ready();
    let second = fixture.second.as_ref().expect("a second setup run");
    assert!(
        !second.success,
        "a second `omni-api setup` must fail once a user exists:\n{}",
        second.output()
    );
    assert!(
        !second.timed_out && second.stderr.to_lowercase().contains("already"),
        "the second run explains that setup was done already:\n{}",
        second.output()
    );
    assert_eq!(
        fixture.counts_after_second.as_ref(),
        Some(&fixture.counts_after_first),
        "the second run creates nothing"
    );
}

// ---------------------------------------------------------------------------
// Argon2id and login identities
// ---------------------------------------------------------------------------

#[test]
fn the_password_is_stored_as_an_argon2id_hash_in_a_password_login_identity() {
    let (_guard, _fixture, _client) = ready();
    block(async {
        let mut conn = admin_connection().await;
        let (provider, secret_hash, linked): (String, Option<String>, bool) = sqlx::query_as(
            "select i.provider, i.secret_hash, i.user_id = u.id \
             from login_identities i, users u",
        )
        .fetch_one(&mut conn)
        .await
        .expect("the login identity");
        assert_eq!(provider, "password");
        assert!(linked, "the identity belongs to the user");

        let hash = secret_hash.expect("the password identity has a secret_hash");
        let parts: Vec<&str> = hash.split('$').collect();
        // $argon2id$v=19$m=<kib>,t=<passes>,p=<lanes>$<salt>$<hash>
        assert_eq!(parts.len(), 6, "a PHC string: {hash}");
        assert_eq!(parts[1], "argon2id", "the algorithm: {hash}");
        let memory_kib: u32 = parts[3]
            .split(',')
            .find_map(|p| p.strip_prefix("m="))
            .and_then(|m| m.parse().ok())
            .unwrap_or_else(|| panic!("no memory cost in {hash}"));
        assert!(
            memory_kib >= 19456,
            "Argon2id memory cost of at least 19 MiB, got {memory_kib} KiB"
        );
        assert!(
            !parts[4].is_empty() && !parts[5].is_empty(),
            "a salt and a hash: {hash}"
        );
        assert!(
            !hash.contains(OWNER_PASSWORD),
            "the password is not stored in the clear"
        );
    });
}

#[test]
fn a_login_identity_of_another_provider_cannot_log_in_with_a_password() {
    let (_guard, _fixture, client) = ready();
    block(async {
        let mut conn = admin_connection().await;
        let user_id: String = sqlx::query_scalar(
            "insert into users (email) values ('github-user@example.test') returning id::text",
        )
        .fetch_one(&mut conn)
        .await
        .expect("insert a user");
        sqlx::query(
            "insert into login_identities (user_id, provider, subject) \
             values ($1::uuid, 'github', 'octocat')",
        )
        .bind(&user_id)
        .execute(&mut conn)
        .await
        .expect("insert a GitHub identity");
    });
    for password in ["", "octocat", "anything"] {
        let response = login(client, "github-user@example.test", password);
        assert_eq!(
            response.status, 401,
            "a user without a password identity cannot log in with a password"
        );
        assert!(response.session_set_cookie().is_none());
    }
}

// ---------------------------------------------------------------------------
// Login, sessions and logout
// ---------------------------------------------------------------------------

#[test]
fn login_sets_an_httponly_secure_samesite_lax_session_cookie() {
    let (_guard, _fixture, client) = ready();
    let response = owner_login(client);
    let body = response.json();
    assert_eq!(
        body["user"]["email"].as_str().map(str::to_lowercase),
        Some(OWNER_EMAIL.to_lowercase()),
        "login body: {body}"
    );
    assert!(
        body["user"]["id"].as_str().is_some_and(|id| id.len() == 36),
        "the user id is a UUID: {body}"
    );

    let line = response
        .session_set_cookie()
        .unwrap_or_else(|| panic!("no {SESSION_COOKIE} cookie: {:?}", response.headers));
    let attributes: Vec<String> = line
        .split(';')
        .skip(1)
        .map(|a| a.trim().to_lowercase())
        .collect();
    for required in ["httponly", "secure", "samesite=lax", "path=/"] {
        assert!(
            attributes.iter().any(|a| a == required),
            "the session cookie needs {required}: {line}"
        );
    }
    let token = response.session_token().unwrap();
    assert!(
        token.len() >= 32 && token.chars().all(|c| c.is_ascii_graphic() && c != ';'),
        "an opaque token of at least 32 characters: {token:?}"
    );
}

#[test]
fn the_session_is_stored_on_the_server_by_the_hash_of_its_token() {
    let (_guard, _fixture, client) = ready();
    let before = session_count();
    let token = owner_login(client).session_token().unwrap();
    assert_eq!(
        session_count(),
        before + 1,
        "a login creates one session row"
    );
    assert_eq!(
        token_state(&token),
        Some((true, true)),
        "a live session whose token_hash is the SHA-256 of the cookie value"
    );
}

#[test]
fn the_session_cookie_authenticates_later_requests() {
    let (_guard, _fixture, client) = ready();
    let token = owner_login(client).session_token().unwrap();
    let me = client.get_with("/api/v1/auth/me", &[("Cookie", &cookie(&token))]);
    assert_eq!(me.status, 200, "me body: {}", me.text());
    assert_eq!(
        me.json()["user"]["email"]
            .as_str()
            .map(str::to_lowercase)
            .as_deref(),
        Some(OWNER_EMAIL.to_lowercase().as_str())
    );

    let forged = client.get_with(
        "/api/v1/auth/me",
        &[("Cookie", &cookie("0123456789abcdef0123456789abcdef"))],
    );
    assert_eq!(forged.status, 401, "a token that was never issued");
    assert_eq!(client.get("/api/v1/auth/me").status, 401, "no cookie");
}

#[test]
fn an_expired_session_is_not_accepted() {
    let (_guard, _fixture, client) = ready();
    let token = owner_login(client).session_token().unwrap();
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query(
            "update sessions set expires_at = now() - interval '1 minute' \
             where token_hash = sha256($1)",
        )
        .bind(token.as_bytes().to_vec())
        .execute(&mut conn)
        .await
        .expect("expire the session");
    });
    let me = client.get_with("/api/v1/auth/me", &[("Cookie", &cookie(&token))]);
    assert_eq!(me.status, 401, "an expired session");
}

#[test]
fn wrong_passwords_and_unknown_emails_are_the_same_401_and_create_no_session() {
    let (_guard, _fixture, client) = ready();
    let before = session_count();
    let wrong_password = login(client, OWNER_EMAIL, "not the password");
    let unknown_email = login(client, "nobody@example.test", OWNER_PASSWORD);
    assert_eq!(wrong_password.status, 401);
    assert_eq!(unknown_email.status, 401);
    assert_eq!(wrong_password.json()["error"], "invalid_credentials");
    assert_eq!(
        wrong_password.json(),
        unknown_email.json(),
        "the two failures cannot be told apart"
    );
    assert!(wrong_password.session_set_cookie().is_none());
    assert!(unknown_email.session_set_cookie().is_none());
    assert_eq!(session_count(), before, "failed logins store no session");
}

#[test]
fn the_email_is_matched_without_regard_to_case() {
    let (_guard, _fixture, client) = ready();
    let response = login(client, &OWNER_EMAIL.to_uppercase(), OWNER_PASSWORD);
    assert_eq!(response.status, 200, "login body: {}", response.text());
}

#[test]
fn logout_ends_the_session_on_the_server_and_clears_the_cookie() {
    let (_guard, _fixture, client) = ready();
    let token = owner_login(client).session_token().unwrap();
    let other = owner_login(client).session_token().unwrap();

    let response = client.request(
        "POST",
        "/api/v1/auth/logout",
        &[("Cookie", &cookie(&token))],
        None,
    );
    assert!(
        [200, 204].contains(&response.status),
        "logout status {}: {}",
        response.status,
        response.text()
    );
    let cleared = response
        .session_set_cookie()
        .unwrap_or_else(|| panic!("logout must clear the cookie: {:?}", response.headers));
    assert!(
        cleared.starts_with(&format!("{SESSION_COOKIE}=;"))
            && cleared.to_lowercase().contains("max-age=0"),
        "an empty cookie that expires at once: {cleared}"
    );

    assert_eq!(
        token_state(&token).map(|(live, _)| live),
        Some(false),
        "the session row stays but is revoked"
    );
    let replay = client.get_with("/api/v1/auth/me", &[("Cookie", &cookie(&token))]);
    assert_eq!(replay.status, 401, "the old cookie no longer works");

    let still_live = client.get_with("/api/v1/auth/me", &[("Cookie", &cookie(&other))]);
    assert_eq!(still_live.status, 200, "other sessions are not affected");
}

#[test]
fn logout_without_a_session_is_401() {
    let (_guard, _fixture, client) = ready();
    let response = client.request("POST", "/api/v1/auth/logout", &[], None);
    assert_eq!(response.status, 401);
    assert_eq!(response.json()["error"], "unauthorized");
}
