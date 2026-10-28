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
use omni_api::password::hash_password;
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
        let workspace: String = sqlx::query_scalar(
            "select w.name from users u \
             join memberships m on m.user_id = u.id \
             join workspaces w on w.id = m.workspace_id \
             where lower(u.email) = lower($1)",
        )
        .bind(OWNER_EMAIL)
        .fetch_one(&mut conn)
        .await
        .expect("the owner's workspace");
        assert_eq!(
            workspace, WORKSPACE_NAME,
            "the owner is a member of the workspace setup made"
        );
    });
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
        !second.timed_out,
        "the second run refuses rather than hangs:\n{}",
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
        let (provider, secret_hash): (String, Option<String>) = sqlx::query_as(
            "select i.provider, i.secret_hash from login_identities i \
             join users u on u.id = i.user_id where lower(u.email) = lower($1)",
        )
        .bind(OWNER_EMAIL)
        .fetch_one(&mut conn)
        .await
        .expect("the owner's login identity");
        assert_eq!(provider, "password");
        let hash = secret_hash.expect("the password identity has a secret_hash");
        assert!(
            hash.starts_with("$argon2id$"),
            "an Argon2id PHC string: {hash}"
        );
    });
}

#[test]
fn a_login_identity_of_another_provider_cannot_log_in_with_a_password() {
    let (_guard, _fixture, client) = ready();
    let secret_hash = hash_password("octocat-password").expect("hash a password");
    block(async {
        let mut conn = admin_connection().await;
        let user_id: String = sqlx::query_scalar(
            "insert into users (email) values ('github-user@example.test') returning id::text",
        )
        .fetch_one(&mut conn)
        .await
        .expect("insert a user");
        sqlx::query(
            "insert into login_identities (user_id, provider, subject, secret_hash) \
             values ($1::uuid, 'github', 'github-user@example.test', $2)",
        )
        .bind(&user_id)
        .bind(&secret_hash)
        .execute(&mut conn)
        .await
        .expect("insert a GitHub identity");
    });
    let response = login(client, "github-user@example.test", "octocat-password");
    assert_eq!(
        response.status, 401,
        "only a password identity can log in with a password"
    );
    assert_eq!(response.json()["error"], "invalid_credentials");
    assert!(response.session_set_cookie().is_none());
}

// ---------------------------------------------------------------------------
// Login, sessions and logout
// ---------------------------------------------------------------------------

#[test]
fn login_sets_an_httponly_secure_samesite_lax_session_cookie() {
    let (_guard, _fixture, client) = ready();
    let response = owner_login(client);
    let line = response
        .session_set_cookie()
        .unwrap_or_else(|| panic!("no {SESSION_COOKIE} cookie: {:?}", response.headers));
    let attributes: Vec<String> = line
        .split(';')
        .skip(1)
        .map(|a| a.trim().to_lowercase())
        .collect();
    // Path=/ makes the cookie reach every /api route.
    for required in ["httponly", "secure", "samesite=lax", "path=/"] {
        assert!(
            attributes.iter().any(|a| a == required),
            "the session cookie needs {required}: {line}"
        );
    }
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

    let replay = client.get_with("/api/v1/auth/me", &[("Cookie", &cookie(&token))]);
    assert_eq!(replay.status, 401, "the old cookie no longer works");

    let still_live = client.get_with("/api/v1/auth/me", &[("Cookie", &cookie(&other))]);
    assert_eq!(still_live.status, 200, "other sessions are not affected");
}
