//! US-108: threads from the command line, against the real API. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key and
//! serve the API's router on a local port against that database. The built
//! `omni` runs against that address with the key in its config file; agents
//! share the key and tell themselves apart with `--as <run>/<role>`. The
//! CLI's contract is in tests/threads.rs, tests/posts.rs and
//! tests/thread_waits.rs, the API's in crates/omni-api/tests/live_threads.rs.
//!
//! What these tests add is the round trip: `omni thread create` starts
//! threads on a track (`--track` or `OMNI_TRACK`) and never without one
//! (US-325), `omni thread read` and `omni wait --thread --posts` keep to a sealed thread (an agent
//! that has not posted reads no other agent's posts, and counts none of
//! them), and `omni thread conclude` writes a conclusion that leaves the
//! thread open to posts.

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

const TEST_DB: &str = "omni_us108_cli";
const KEY_PREFIX: &str = "omniustes108";

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
                "with agent as (insert into users (email) values ($2 || '@agents.example.test') returning id), \
                 member as (insert into memberships (workspace_id, user_id) select $1::uuid, id from agent) \
                 insert into api_keys (user_id, prefix, key_hash, scopes) \
                 select id, $2, encode(sha256(convert_to($3, 'UTF8')), 'hex'), '{*}' from agent \
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

const MODERATOR: &str = "run-7/moderator";

/// A new project made by the moderator from the command line, with the part
/// "Wision" and its track "Checkout" for threads to start on; its name.
fn new_project(home: &Scratch) -> String {
    let project = unique("forum-site");
    let created = run_as(home, MODERATOR, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let out = run_as(
        home,
        MODERATOR,
        &[
            "part",
            "create",
            "Wision",
            "--kind",
            "app",
            "--project",
            &project,
        ],
    );
    assert_eq!(out.code, 0, "part create: {}", out.stderr);
    let out = run_as(
        home,
        MODERATOR,
        &[
            "track",
            "create",
            "Checkout",
            "--part",
            "Wision",
            "--project",
            &project,
        ],
    );
    assert_eq!(out.code, 0, "track create: {}", out.stderr);
    project
}

/// The ID of the project's track "Checkout".
fn checkout_id(home: &Scratch, project: &str) -> String {
    let out = run_as(
        home,
        MODERATOR,
        &["track", "list", "--project", project, "--json"],
    );
    assert_eq!(out.code, 0, "track list: {}", out.stderr);
    let listed = out.json();
    listed["tracks"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of tracks: {listed}"))
        .iter()
        .find(|track| track["name"] == "Checkout")
        .and_then(|track| track["id"].as_str())
        .unwrap_or_else(|| panic!("the track Checkout is listed: {listed}"))
        .to_string()
}

/// Starts a thread from the command line with `extra` options; its ID.
fn new_thread(home: &Scratch, project: &str, question: &str, extra: &[&str]) -> String {
    let mut args = vec!["thread", "create", question, "--project", project, "--json"];
    args.extend_from_slice(extra);
    let out = run_as(home, MODERATOR, &args);
    assert_eq!(out.code, 0, "thread create: {}", out.stderr);
    out.json()["thread"]["id"]
        .as_str()
        .expect("the thread's id")
        .to_string()
}

/// What a test knows of a post it made.
struct Made {
    id: String,
    summary: String,
    body: String,
}

/// Posts as `who` with a unique summary and body, the body as an argument.
fn post(home: &Scratch, who: &str, project: &str, thread: &str) -> Made {
    let summary = unique(&format!("{who} says"));
    let body = unique(&format!("{who} explains"));
    let out = run_as(
        home,
        who,
        &[
            "post",
            thread,
            "--summary",
            &summary,
            &body,
            "--project",
            project,
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

/// `omni thread read` as `who`; what it printed.
fn read(home: &Scratch, who: &str, project: &str, thread: &str) -> String {
    let out = run_as(home, who, &["thread", "read", thread, "--project", project]);
    assert_eq!(out.code, 0, "{who} reads the thread: {}", out.stderr);
    out.stdout
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

fn assert_all_of(text: &str, made: &Made, what: &str) {
    for marker in [&made.summary, &made.body] {
        assert!(
            text.contains(marker.as_str()),
            "{what}: {marker:?} shows: {text}"
        );
    }
}

/// `omni wait --thread <thread> --posts <posts> --timeout <timeout>` as `who`.
fn wait_posts(
    home: &Scratch,
    who: &str,
    project: &str,
    thread: &str,
    posts: &str,
    timeout: &str,
) -> Out {
    run_as(
        home,
        who,
        &[
            "wait",
            "--thread",
            thread,
            "--posts",
            posts,
            "--timeout",
            timeout,
            "--project",
            project,
        ],
    )
}

#[test]
fn an_agent_starts_threads_on_a_track_and_never_without_one() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let checkout = checkout_id(&home, &project);

    let out = run_as(
        &home,
        MODERATOR,
        &[
            "thread",
            "create",
            "Which hero layout?",
            "--project",
            &project,
        ],
    );
    assert_ne!(out.code, 0, "no track: {:?}", out.stdout);
    assert!(
        out.stderr.contains("track_required"),
        "the error names track_required: {:?}",
        out.stderr
    );

    let flag_question = unique("Do we need a cart page?");
    let by_flag = new_thread(
        &home,
        &project,
        &flag_question,
        &["--track", "checkout", "--sealed"],
    );
    let env_question = unique("Which hero layout?");
    let out = omni(&home.path)
        .args(&[
            "thread",
            "create",
            &env_question,
            "--project",
            &project,
            "--json",
            "--as",
            MODERATOR,
        ])
        .env("OMNI_TRACK", "Checkout")
        .run();
    assert_eq!(out.code, 0, "thread create with OMNI_TRACK: {}", out.stderr);
    let by_env = out.json()["thread"]["id"]
        .as_str()
        .expect("the thread's id")
        .to_string();

    let out = run_as(&home, MODERATOR, &["thread", "list", "--project", &project]);
    assert_eq!(out.code, 0, "thread list: {}", out.stderr);
    let listed = non_empty_lines(&out.stdout);
    assert_eq!(
        listed.len(),
        2,
        "only the two threads on the track: {:?}",
        out.stdout
    );
    for (id, question, sealed) in [
        (&by_flag, &flag_question, true),
        (&by_env, &env_question, false),
    ] {
        let lines: Vec<&str> = listed
            .iter()
            .copied()
            .filter(|line| line.contains(id.as_str()))
            .collect();
        assert_eq!(lines.len(), 1, "one line for {id}: {:?}", out.stdout);
        assert!(lines[0].contains(question.as_str()), "{:?}", lines[0]);
        let shown = run_as(
            &home,
            MODERATOR,
            &["thread", "read", id, "--project", &project, "--json"],
        );
        assert_eq!(shown.code, 0, "thread read: {}", shown.stderr);
        let thread = &shown.json()["thread"];
        assert_eq!(thread["scope"], "track", "{thread}");
        assert_eq!(thread["track_id"], checkout.as_str(), "{thread}");
        assert_eq!(thread["sealed"], sealed, "{thread}");
    }
}

#[test]
fn in_a_sealed_thread_read_and_wait_show_an_agent_no_other_agents_posts_until_it_posts() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let thread = new_thread(
        &home,
        &project,
        "Which hero layout?",
        &["--track", "Checkout", "--sealed"],
    );
    let a = post(&home, "run-7/critic-a", &project, &thread);
    let b = post(&home, "run-8/critic-b", &project, &thread);

    let seen = read(&home, MODERATOR, &project, &thread);
    assert_nothing_of(&seen, &a, "the moderator's thread read");
    assert_nothing_of(&seen, &b, "the moderator's thread read");
    let out = wait_posts(&home, MODERATOR, &project, &thread, "1", "2");
    assert_eq!(
        out.code, 2,
        "the moderator reads no post yet, so even one post is not there for it; \
         stdout: {:?}; stderr: {:?}",
        out.stdout, out.stderr
    );
    let seen = read(&home, "run-7/critic-a", &project, &thread);
    assert_all_of(&seen, &a, "critic-a reads its own post");
    assert_all_of(
        &seen,
        &b,
        "critic-a has posted, so it reads critic-b's later post too",
    );

    let own = post(&home, MODERATOR, &project, &thread);
    let out = wait_posts(&home, MODERATOR, &project, &thread, "3", "10");
    assert_eq!(
        out.code, 0,
        "once it has posted, the moderator reads all three; stdout: {:?}; stderr: {:?}",
        out.stdout, out.stderr
    );
    let seen = read(&home, MODERATOR, &project, &thread);
    for made in [&a, &b, &own] {
        assert_all_of(&seen, made, "the moderator's thread read after posting");
    }
}

#[test]
fn thread_conclude_sets_the_conclusion_and_posts_can_still_be_added() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let thread = new_thread(
        &home,
        &project,
        "Which hero layout?",
        &["--track", "Checkout"],
    );
    post(&home, "run-7/critic-a", &project, &thread);

    let conclusion = unique("We take the left layout");
    let out = run_as(
        &home,
        MODERATOR,
        &[
            "thread",
            "conclude",
            &thread,
            &conclusion,
            "--project",
            &project,
        ],
    );
    assert_eq!(out.code, 0, "thread conclude: {}", out.stderr);
    let seen = read(&home, "run-7/critic-a", &project, &thread);
    assert!(
        seen.contains(&conclusion),
        "thread read shows the conclusion: {seen}"
    );

    let later = post(&home, "run-7/critic-b", &project, &thread);
    let seen = read(&home, MODERATOR, &project, &thread);
    assert_all_of(&seen, &later, "a post after the conclusion");
    assert!(seen.contains(&conclusion), "{seen}");
}
