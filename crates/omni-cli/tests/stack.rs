//! US-015: `omni login` and `omni key` against the real API. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed two workspaces (Alpha and Beta)
//! and two users with a password login (dana is a member of Alpha only, erin
//! of both), and serve the API's router on a local port against that database.
//! The built `omni` then runs against that address, with an empty home
//! directory per test. Prompts and the password are read from standard input
//! when it is not a terminal: `omni login` reads the email, the password and
//! the workspace choice (its number in the list it prints) as three lines;
//! `omni key list|revoke` reads the password as one line. The prompts are
//! written to standard error. The contract of the CLI is in tests/common/mod.rs.

mod common;

use common::*;
use omni_api::Config;
use serde_json::Value;
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard, OnceLock};

const TEST_DB: &str = "omni_us015_cli";
const PASSWORD: &str = "correct horse battery staple 15";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";

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
        // Nothing here reaches RustFS: login and keys only use Postgres.
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
    url: String,
    alpha: String,
    beta: String,
    /// Keeps the API's runtime, and so the server, alive.
    _runtime: tokio::runtime::Runtime,
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
        let (alpha, beta) = block(async {
            let hash = omni_api::password::hash_password(PASSWORD).expect("hash a password");
            let mut conn = admin_connection().await;
            let mut text = async |sql: &str, binds: &[&str]| -> String {
                let mut query = sqlx::query_scalar::<_, String>(sql);
                for bind in binds {
                    query = query.bind(*bind);
                }
                query.fetch_one(&mut conn).await.expect("seed")
            };
            let alpha = text(
                "insert into workspaces (name) values ('Alpha') returning id::text",
                &[],
            )
            .await;
            let beta = text(
                "insert into workspaces (name) values ('Beta') returning id::text",
                &[],
            )
            .await;
            let mut users = Vec::new();
            for name in ["dana", "erin"] {
                let email = format!("{name}@example.test");
                let id = text(
                    "insert into users (email) values ($1) returning id::text",
                    &[&email],
                )
                .await;
                text(
                    "insert into login_identities (user_id, provider, subject, secret_hash) \
                     values ($1::uuid, $2, lower($3), $4) returning id::text",
                    &[&id, omni_api::password::PASSWORD_PROVIDER, &email, &hash],
                )
                .await;
                users.push(id);
            }
            let member = "insert into memberships (workspace_id, user_id) \
                          values ($1::uuid, $2::uuid) returning id::text";
            text(member, &[&alpha, &users[0]]).await;
            text(member, &[&alpha, &users[1]]).await;
            text(member, &[&beta, &users[1]]).await;
            (alpha, beta)
        });

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("bind a local port");
        let port = listener.local_addr().unwrap().port();
        let router = {
            // The connection pool needs a runtime to be created in.
            let _guard = runtime.enter();
            let state = omni_api::AppState::new(&config()).expect("build the API state");
            omni_api::router(state)
        };
        runtime.spawn(async move {
            axum::serve(listener, router).await.expect("serve the API");
        });
        Fixture {
            url: format!("http://127.0.0.1:{port}"),
            alpha,
            beta,
            _runtime: runtime,
        }
    })
}

/// The fixture. The guard serialises the tests, because some of them count the
/// sessions of a user.
fn ready() -> (MutexGuard<'static, ()>, &'static Fixture) {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    (guard, fixture())
}

/// Runs `omni login` against the fixture with the answers on standard input.
fn login(home: &Scratch, server: &str, email: &str, password: &str, choice: &str) -> Out {
    omni(&home.path)
        .args(&["login", "--server", server])
        .stdin(format!("{email}\n{password}\n{choice}\n").as_bytes())
        .run()
}

fn secret_of(home: &Scratch) -> String {
    config_value(&home.path, "api_key").unwrap_or_else(|| {
        panic!(
            "the config file has an api_key: {:?}",
            config_path(&home.path)
        )
    })
}

fn prefix_of(secret: &str) -> String {
    secret
        .rsplit_once('_')
        .expect("a key has a prefix")
        .0
        .to_string()
}

fn live_sessions(email: &str) -> String {
    db_text(
        "select count(*)::text from sessions s join users u on u.id = s.user_id \
         where u.email = $1 and s.revoked_at is null and s.expires_at > now()",
        &[email],
    )
    .unwrap_or_default()
}

fn last_line(out: &Out) -> String {
    out.stderr_lines()
        .last()
        .map(|line| line.to_lowercase())
        .unwrap_or_default()
}

#[test]
fn login_asks_for_the_credentials_and_stores_a_working_key_with_mode_600() {
    let (_guard, f) = ready();
    let home = Scratch::new("login");
    let out = login(&home, &f.url, "dana@example.test", PASSWORD, "1");
    assert_eq!(out.code, 0, "omni login: {}", out.stderr);

    let prompts = out.stderr.to_lowercase();
    assert!(
        prompts.contains("email") && prompts.contains("password"),
        "login asks for the email and the password on standard error: {:?}",
        out.stderr
    );

    let path = config_path(&home.path);
    assert!(path.exists(), "login writes {}", path.display());
    assert_eq!(
        file_mode(&path),
        0o600,
        "the config file holds a secret and is mode 600"
    );
    let secret = secret_of(&home);
    assert_eq!(
        config_value(&home.path, "workspace_id"),
        Some(f.alpha.clone())
    );
    assert_eq!(config_value(&home.path, "server"), Some(f.url.clone()));
    assert_eq!(
        config_value(&home.path, "email"),
        Some("dana@example.test".to_string())
    );
    let config_text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !config_text.contains(PASSWORD) && !config_text.contains("omni_session"),
        "the password and the session are not stored: {config_text}"
    );
    assert!(
        !out.stdout.contains(&secret),
        "the secret is only stored, never printed: {:?}",
        out.stdout
    );

    // The stored key is a working agent key for that workspace.
    let whoami = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer", "--json"])
        .run();
    assert_eq!(
        whoami.code, 0,
        "whoami with the stored key: {}",
        whoami.stderr
    );
    let body = whoami.json();
    assert_eq!(body["workspace_id"], f.alpha.as_str(), "{body}");
    assert_eq!(body["actor"]["kind"], "agent", "{body}");
}

#[test]
fn login_lets_you_pick_a_workspace() {
    let (_guard, f) = ready();
    let home = Scratch::new("pick");
    let out = login(&home, &f.url, "erin@example.test", PASSWORD, "2");
    assert_eq!(out.code, 0, "omni login: {}", out.stderr);
    let listed = format!("{}{}", out.stdout, out.stderr);
    assert!(
        listed.contains("Alpha") && listed.contains("Beta"),
        "login lists the workspaces to pick from: {listed:?}"
    );
    assert_eq!(
        config_value(&home.path, "workspace_id"),
        Some(f.beta.clone()),
        "choice 2 is Beta"
    );

    let whoami = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer", "--json"])
        .run();
    assert_eq!(whoami.code, 0, "{}", whoami.stderr);
    assert_eq!(whoami.json()["workspace_id"], f.beta.as_str());
}

#[test]
fn login_takes_the_server_from_the_environment_and_json_output() {
    let (_guard, f) = ready();
    let home = Scratch::new("loginjson");
    let out = omni(&home.path)
        .env("OMNI_SERVER", &f.url)
        .args(&["login", "--json"])
        .stdin(format!("dana@example.test\n{PASSWORD}\n1\n").as_bytes())
        .run();
    assert_eq!(out.code, 0, "omni login --json: {}", out.stderr);
    let body = out.json();
    assert_eq!(body["workspace_id"], f.alpha.as_str(), "{body}");
    assert!(
        !out.stdout.contains(&secret_of(&home)),
        "even --json never prints the secret"
    );
}

#[test]
fn login_with_a_wrong_password_fails_and_stores_nothing() {
    let (_guard, f) = ready();
    let home = Scratch::new("badlogin");
    let out = login(&home, &f.url, "dana@example.test", "not the password", "1");
    assert_ne!(out.code, 0, "{:?}", out.stdout);
    assert!(
        documented_exit_codes().contains(&out.code),
        "exit code {} is documented",
        out.code
    );
    assert_eq!(out.stdout, "");
    assert!(
        last_line(&out).contains("password"),
        "the last line says what went wrong: {:?}",
        out.stderr
    );
    assert!(
        !config_path(&home.path).exists(),
        "a failed login stores nothing"
    );
}

#[test]
fn login_replaces_a_config_file_with_a_looser_mode() {
    let (_guard, f) = ready();
    let home = Scratch::new("loose");
    write_config(&home.path, &f.url, &f.alpha, API_KEY);
    let path = config_path(&home.path);
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();

    let out = login(&home, &f.url, "dana@example.test", PASSWORD, "1");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(file_mode(&path), 0o600, "the file is mode 600 afterwards");
    assert_ne!(
        secret_of(&home),
        API_KEY,
        "the new key replaced the old one"
    );
}

#[test]
fn key_list_asks_for_the_password_and_acts_as_the_human_for_that_command_only() {
    let (_guard, f) = ready();
    let home = Scratch::new("keylist");
    let out = login(&home, &f.url, "dana@example.test", PASSWORD, "1");
    assert_eq!(out.code, 0, "{}", out.stderr);
    let prefix = prefix_of(&secret_of(&home));
    let before = std::fs::read(config_path(&home.path)).unwrap();
    let sessions_before = live_sessions("dana@example.test");

    // No agent identity here: key commands act as the human, not as an agent.
    let listed = omni(&home.path)
        .args(&["key", "list", "--json"])
        .stdin(format!("{PASSWORD}\n").as_bytes())
        .run();
    assert_eq!(listed.code, 0, "omni key list: {}", listed.stderr);
    assert!(
        listed.stderr.to_lowercase().contains("password"),
        "key list asks for the password: {:?}",
        listed.stderr
    );
    let body = listed.json();
    let keys = body["keys"].as_array().expect("a keys array");
    let key = keys
        .iter()
        .find(|key| key["prefix"] == prefix.as_str())
        .unwrap_or_else(|| panic!("the key {prefix} of the login is listed: {body}"));
    assert!(key["revoked_at"].is_null(), "{key}");
    assert!(
        key.get("secret").is_none() && !listed.stdout.contains(&secret_of(&home)),
        "a listing never shows secrets"
    );

    let text = omni(&home.path)
        .args(&["key", "list"])
        .stdin(format!("{PASSWORD}\n").as_bytes())
        .run();
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert!(
        serde_json::from_str::<Value>(&text.stdout).is_err(),
        "the default output is text: {:?}",
        text.stdout
    );
    assert!(
        text.stdout.contains(&prefix) && text.stdout.contains(key["id"].as_str().unwrap()),
        "the text lists the key's prefix and ID: {:?}",
        text.stdout
    );

    // "For that command only": the session is not kept anywhere.
    assert_eq!(
        std::fs::read(config_path(&home.path)).unwrap(),
        before,
        "the config file is untouched"
    );
    let files: Vec<_> = std::fs::read_dir(config_path(&home.path).parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(files.len(), 1, "no session file is left behind: {files:?}");
    assert_eq!(
        live_sessions("dana@example.test"),
        sessions_before,
        "the session that the command opened is ended"
    );
}

#[test]
fn key_revoke_revokes_the_key_and_it_stops_working() {
    let (_guard, f) = ready();
    let home = Scratch::new("keyrevoke");
    let out = login(&home, &f.url, "dana@example.test", PASSWORD, "1");
    assert_eq!(out.code, 0, "{}", out.stderr);
    let prefix = prefix_of(&secret_of(&home));

    let listed = omni(&home.path)
        .args(&["key", "list", "--json"])
        .stdin(format!("{PASSWORD}\n").as_bytes())
        .run();
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    let id = listed.json()["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|key| key["prefix"] == prefix.as_str())
        .and_then(|key| key["id"].as_str().map(str::to_string))
        .expect("the key of the login is listed");

    let revoked = omni(&home.path)
        .args(&["key", "revoke", &id])
        .stdin(format!("{PASSWORD}\n").as_bytes())
        .run();
    assert_eq!(revoked.code, 0, "omni key revoke: {}", revoked.stderr);
    assert_eq!(
        db_text(
            "select (revoked_at is not null)::text from api_keys where id = $1::uuid",
            &[&id]
        )
        .as_deref(),
        Some("true"),
        "the key is revoked in the database"
    );

    let whoami = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer"])
        .run();
    assert_ne!(whoami.code, 0, "a revoked key no longer works");
    assert_eq!(whoami.stderr_lines().len(), 1, "{:?}", whoami.stderr);
    assert!(
        last_line(&whoami).contains("omni login"),
        "{:?}",
        whoami.stderr
    );

    // An unknown key is an error, not a silent success.
    let unknown = omni(&home.path)
        .args(&["key", "revoke", UNKNOWN_ID])
        .stdin(format!("{PASSWORD}\n").as_bytes())
        .run();
    assert_ne!(unknown.code, 0, "{:?}", unknown.stdout);
    assert!(
        last_line(&unknown).contains("not_found") || last_line(&unknown).contains("not found"),
        "{:?}",
        unknown.stderr
    );
}
