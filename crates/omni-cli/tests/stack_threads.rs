//! US-036: `omni thread digest`, `omni post show` and `omni wait --thread`
//! against the real API. Run with `--features stack-tests` in the Compose
//! test container, which reaches Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key and
//! serve the API's router on a local port against that database. The built
//! `omni` runs against that address with the key in its config file; agents
//! share the key and tell themselves apart with `--as <run>/<role>`. The
//! CLI's contract is in tests/digests.rs and tests/thread_waits.rs, the API's
//! in crates/omni-api/tests/thread_waits.rs and sealed_phases.rs.
//!
//! What these tests add is the round trip: the waits see who has posted in a
//! sealed phase without showing the agent what it may not read yet, the
//! digest and `post show` keep to the sealed phase (US-035), and a thread
//! without a decision is one that `decision:` waits wait on.

mod common;

use common::*;
use omni_api::Config;
use serde_json::Value;
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const TEST_DB: &str = "omni_us036_cli";
const KEY_PREFIX: &str = "omniustest36";
/// How long after the wait starts the other agents act.
const LATER: Duration = Duration::from_millis(1500);

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
        // Nothing here reaches RustFS: threads and posts without attachments
        // only use Postgres.
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
    url: String,
    workspace: String,
    secret: String,
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
        let secret = format!("{KEY_PREFIX}_{}", "0123456789abcdef".repeat(4));
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
            text(
                "insert into api_keys (workspace_id, prefix, key_hash, scopes) \
                 values ($1::uuid, $2, encode(sha256(convert_to($3, 'UTF8')), 'hex'), '{*}') \
                 returning id::text",
                &[&workspace, KEY_PREFIX, &secret],
            )
            .await;
            workspace
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
            workspace,
            secret,
            _runtime: runtime,
        }
    })
}

/// A home directory whose config file holds the agents' key, as `omni login`
/// leaves it.
fn agent_home(f: &Fixture) -> Scratch {
    let home = Scratch::new("agent");
    write_config(&home.path, &f.url, &f.workspace, &f.secret);
    home
}

/// Runs `omni` as the agent `who` (`<run>/<role>`).
fn run_as(home: &Scratch, who: &str, args: &[&str]) -> Out {
    omni(&home.path).args(args).args(&["--as", who]).run()
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// A new project with a change set and a thread, made by the moderator from
/// the command line; returns (project name, thread ID).
fn new_thread(home: &Scratch, sealed: bool) -> (String, String) {
    let project = unique("forum-site");
    let created = run_as(home, MODERATOR, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let out = run_as(
        home,
        MODERATOR,
        &[
            "changeset",
            "create",
            "--project",
            &project,
            "--title",
            "Hero section",
            "--run",
            "run-7",
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "changeset create: {}", out.stderr);
    let changeset = out.json()["changeset"]["id"]
        .as_str()
        .expect("the change set's id")
        .to_string();
    let criteria = home.join(&format!("{}.txt", unique("criteria")));
    std::fs::write(&criteria, "Readable at 320 px\n").expect("write the criteria file");
    let mut args = vec![
        "thread",
        "create",
        "--project",
        project.as_str(),
        "--changeset",
        changeset.as_str(),
        "--question",
        "Which hero layout?",
        "--criteria-file",
        criteria.to_str().expect("a UTF-8 path"),
        "--json",
    ];
    if sealed {
        args.push("--sealed");
    }
    let out = run_as(home, MODERATOR, &args);
    assert_eq!(out.code, 0, "thread create: {}", out.stderr);
    let thread = out.json()["thread"]["id"]
        .as_str()
        .expect("the thread's id")
        .to_string();
    (project, thread)
}

const MODERATOR: &str = "run-7/moderator";

/// What a test knows of a post it made.
struct Made {
    id: String,
    summary: String,
    body: String,
}

/// Posts a position as `who` with a unique summary and body.
fn post(home: &Scratch, who: &str, project: &str, thread: &str) -> Made {
    let summary = unique(&format!("{who} says"));
    let body = unique(&format!("{who} explains"));
    let file = home.join(&format!("{}.md", unique("body")));
    std::fs::write(&file, &body).expect("write the body file");
    let out = run_as(
        home,
        who,
        &[
            "post",
            thread,
            "--project",
            project,
            "--type",
            "position",
            "--summary",
            &summary,
            "--body-file",
            file.to_str().expect("a UTF-8 path"),
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "{who} posts: {}", out.stderr);
    let answer: Value = out.json();
    Made {
        id: answer["post"]["id"]
            .as_str()
            .expect("the post's id")
            .to_string(),
        summary,
        body,
    }
}

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

#[test]
fn a_moderator_waits_for_the_critics_of_a_sealed_phase_without_reading_their_posts() {
    let f = fixture();
    let home = agent_home(f);
    let (project, thread) = new_thread(&home, true);

    let (critics_home, critics_project, critics_thread) =
        (agent_home(f), project.clone(), thread.clone());
    let critics = std::thread::spawn(move || {
        std::thread::sleep(LATER);
        let a = post(
            &critics_home,
            "run-7/critic-a",
            &critics_project,
            &critics_thread,
        );
        let b = post(
            &critics_home,
            "run-8/critic-b",
            &critics_project,
            &critics_thread,
        );
        (a, b)
    });
    let out = run_as(
        &home,
        MODERATOR,
        &[
            "wait",
            "--thread",
            &thread,
            "--until",
            "roles:critic-*,*-b",
            "--timeout",
            "20",
            "--project",
            &project,
            "--json",
        ],
    );
    let (a, b) = critics.join().expect("the critics post");
    assert_eq!(out.code, 0, "omni wait: {} {}", out.stdout, out.stderr);
    assert!(
        out.elapsed >= LATER - Duration::from_millis(100),
        "it waited for the critics: {:?}",
        out.elapsed
    );
    // The moderator has not posted in the sealed phase: what it got shows
    // that the critics posted, not what they wrote.
    assert_nothing_of(&out.stdout, &a, "the wait's output");
    assert_nothing_of(&out.stdout, &b, "the wait's output");
    let answer = out.json();
    let roles: Vec<&str> = answer["thread"]["phases"][0]["posted_by"]
        .as_array()
        .unwrap_or_else(|| panic!("phase 1 tells who has posted: {answer}"))
        .iter()
        .filter_map(|who| who["role"].as_str())
        .collect();
    assert_eq!(roles, vec!["critic-a", "critic-b"], "{answer}");
}

#[test]
fn a_wait_for_posts_ends_with_3_when_the_next_phase_starts_first() {
    let f = fixture();
    let home = agent_home(f);
    let (project, thread) = new_thread(&home, false);
    post(&home, "run-7/critic-a", &project, &thread);

    let (other_home, other_project, other_thread) =
        (agent_home(f), project.clone(), thread.clone());
    let next_phase = std::thread::spawn(move || {
        std::thread::sleep(LATER);
        run_as(
            &other_home,
            MODERATOR,
            &[
                "thread",
                "phase",
                &other_thread,
                "--sealed",
                "--project",
                &other_project,
            ],
        )
    });
    let out = run_as(
        &home,
        "run-7/scribe",
        &[
            "wait",
            "--thread",
            &thread,
            "--until",
            "posts:2",
            "--timeout",
            "20",
            "--project",
            &project,
        ],
    );
    let started = next_phase.join().expect("the next phase");
    assert_eq!(started.code, 0, "thread phase: {}", started.stderr);
    assert_eq!(out.code, 3, "{} {}", out.stdout, out.stderr);
    assert_eq!(out.stdout, "");
    let lines = out.stderr_lines();
    assert_eq!(lines.len(), 1, "one line: {:?}", out.stderr);
    assert!(lines[0].contains("phase"), "{:?}", lines[0]);
}

#[test]
fn phase_change_and_decision_waits_time_out_on_a_quiet_thread() {
    let f = fixture();
    let home = agent_home(f);
    let (project, thread) = new_thread(&home, false);
    for until in ["phase-change", "decision:approved", "posts:1"] {
        let out = run_as(
            &home,
            MODERATOR,
            &[
                "wait",
                "--thread",
                &thread,
                "--until",
                until,
                "--timeout",
                "1",
                "--project",
                &project,
            ],
        );
        assert_eq!(out.code, 2, "{until}: {} {}", out.stdout, out.stderr);
        assert_eq!(out.stdout, "", "{until}");
        assert_eq!(out.stderr_lines().len(), 1, "{until}: {:?}", out.stderr);
    }
}

#[test]
fn digest_and_post_show_keep_to_the_sealed_phase() {
    let f = fixture();
    let home = agent_home(f);
    let (project, thread) = new_thread(&home, true);
    let critic = "run-7/critic-a";
    let made = post(&home, critic, &project, &thread);

    // The critic reads its own post in the digest and in full.
    let digest = run_as(
        &home,
        critic,
        &["thread", "digest", &thread, "--project", &project],
    );
    assert_eq!(digest.code, 0, "{}", digest.stderr);
    let lines = non_empty_lines(&digest.stdout);
    assert_eq!(lines.len(), 1, "one post: {:?}", digest.stdout);
    for needle in [
        "position",
        "critic-a",
        made.summary.as_str(),
        made.id.as_str(),
    ] {
        assert!(lines[0].contains(needle), "{needle:?}: {:?}", lines[0]);
    }
    let shown = run_as(
        &home,
        critic,
        &["post", "show", &made.id, "--project", &project],
    );
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    assert!(shown.stdout.contains(&made.body), "{:?}", shown.stdout);

    // The moderator has not posted in the sealed phase: it sees nothing of it.
    let digest = run_as(
        &home,
        MODERATOR,
        &["thread", "digest", &thread, "--project", &project],
    );
    assert_eq!(digest.code, 0, "{}", digest.stderr);
    assert!(
        non_empty_lines(&digest.stdout).is_empty(),
        "no post the moderator may see: {:?}",
        digest.stdout
    );
    assert_nothing_of(&digest.stdout, &made, "the moderator's digest");
    let hidden = run_as(
        &home,
        MODERATOR,
        &["post", "show", &made.id, "--project", &project],
    );
    assert_eq!(hidden.code, 1, "{} {}", hidden.stdout, hidden.stderr);
    assert_eq!(hidden.stdout, "");
    assert_eq!(hidden.stderr_lines().len(), 1, "{:?}", hidden.stderr);
    assert!(
        !hidden.stderr.contains(&made.summary) && !hidden.stderr.contains(&made.body),
        "{:?}",
        hidden.stderr
    );
}
