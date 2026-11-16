//! US-137: one smoke test of the `omni story` writes against the real API.
//! Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432`.
//!
//! The test migrates a scratch database, seeds a workspace with an API key
//! and serves the API's router on a local port against that database. The
//! CLI's contract is in tests/stories.rs, the API's rules in
//! crates/omni-api/tests/stories.rs. What this test adds is the round trip:
//! the bodies the CLI sends, with the version it read, are ones the API
//! takes, so a story added, claimed, ticked and moved to `done` reads back
//! as `done`.

mod common;

use common::*;
use omni_api::Config;
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us137_cli";
const KEY_PREFIX: &str = "omniustes137";

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
        // Nothing here reaches RustFS: stories only use Postgres.
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

const AGENT: &str = "run-1/implementer";

#[test]
fn a_story_added_claimed_ticked_and_moved_to_done_reads_back_as_done() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("board");
    let created = run_as(&home, AGENT, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let part = run_as(
        &home,
        AGENT,
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
    assert_eq!(part.code, 0, "part create: {}", part.stderr);
    let feature = run_as(
        &home,
        AGENT,
        &[
            "track",
            "create",
            "stories",
            "--part",
            "Wision",
            "--project",
            &project,
        ],
    );
    assert_eq!(feature.code, 0, "feature create: {}", feature.stderr);
    let file = home.join("story.md");
    std::fs::write(
        &file,
        "### Smoke story\n\
         **Description:** As the workflow, I want to finish a story.\n\
         \n\
         **Acceptance Criteria:**\n\
         - [ ] It is done\n",
    )
    .unwrap();
    let scope = ["--project", project.as_str(), "--track", "stories"];
    let story = |args: &[&str]| run_as(&home, AGENT, &[args, &scope[..]].concat());

    let added = story(&["story", "add", &file.display().to_string()]);
    assert_eq!(added.code, 0, "story add: {}", added.stderr);
    let key = added.stdout.trim().to_string();
    let claimed = story(&[
        "story",
        "claim",
        &key,
        "--run",
        "r1",
        "--stage",
        "implementing",
    ]);
    let ticked = story(&["story", "tick", &key, "--run", "r1", "--criterion", "0"]);
    let moved = story(&["story", "move", &key, "--run", "r1", "--to", "done"]);
    let read = story(&["story", "read", &key, "--json"]);

    assert_eq!(claimed.code, 0, "story claim: {}", claimed.stderr);
    assert_eq!(ticked.code, 0, "story tick: {}", ticked.stderr);
    assert_eq!(moved.code, 0, "story move: {}", moved.stderr);
    assert_eq!(read.code, 0, "story read: {}", read.stderr);
    let back = read.json();
    assert_eq!(back["key"], key.as_str());
    assert_eq!(back["status"], "done");
    assert_eq!(back["criteria"][0]["ticked"], true);
    assert_eq!(back["holder"], serde_json::Value::Null);
}

/// Adds a story with one criterion from a prd-format file; returns its key.
fn add_story(home: &Scratch, scope: &[&str], title: &str) -> String {
    let file = home.join(&format!("{}.md", unique("story")));
    std::fs::write(
        &file,
        format!(
            "### {title}\n\
             **Description:** As the workflow, I want {title}.\n\
             \n\
             **Acceptance Criteria:**\n\
             - [ ] It is done\n"
        ),
    )
    .unwrap();
    let added = run_as(
        home,
        AGENT,
        &[&["story", "add", &file.display().to_string()], scope].concat(),
    );
    assert_eq!(added.code, 0, "story add: {}", added.stderr);
    added.stdout.trim().to_string()
}

/// US-319: a story is blocked by a story on another part's track of the same
/// project; `read` names the blocker's track and status, and the story is not
/// pickable until the blocker is done.
#[test]
fn a_story_blocked_from_another_track_reads_its_track_and_status_and_waits_until_it_is_done() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("board");
    let created = run_as(&home, AGENT, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    for (part, kind, track) in [
        ("Wision", "app", "Analyze"),
        ("iot-service", "service", "Ingest"),
    ] {
        let made = run_as(
            &home,
            AGENT,
            &[
                "part",
                "create",
                part,
                "--kind",
                kind,
                "--project",
                &project,
            ],
        );
        assert_eq!(made.code, 0, "part create {part}: {}", made.stderr);
        let made = run_as(
            &home,
            AGENT,
            &[
                "track",
                "create",
                track,
                "--part",
                part,
                "--project",
                &project,
            ],
        );
        assert_eq!(made.code, 0, "track create {track}: {}", made.stderr);
    }
    let on_analyze = ["--project", project.as_str(), "--track", "Analyze"];
    let on_ingest = ["--project", project.as_str(), "--track", "Ingest"];
    let blocked = add_story(&home, &on_analyze, "Show the windows");
    let blocker = add_story(&home, &on_ingest, "Store the windows");
    let analyze = |args: &[&str]| run_as(&home, AGENT, &[args, &on_analyze[..]].concat());
    let ingest = |args: &[&str]| run_as(&home, AGENT, &[args, &on_ingest[..]].concat());
    let blocked_by_line = || {
        let read = analyze(&["story", "read", &blocked]);
        assert_eq!(read.code, 0, "story read: {}", read.stderr);
        read.stdout
            .lines()
            .find(|line| line.starts_with("blocked by:"))
            .unwrap_or_else(|| panic!("read shows no blocker: {}", read.stdout))
            .to_string()
    };
    let pickable = || {
        let listed = analyze(&["story", "list", "--pickable"]);
        assert_eq!(listed.code, 0, "story list: {}", listed.stderr);
        listed
            .stdout
            .lines()
            .any(|line| line.split_whitespace().next() == Some(blocked.as_str()))
    };

    let edited = analyze(&["story", "edit", &blocked, "--blocked-by", &blocker]);
    assert_eq!(edited.code, 0, "story edit: {}", edited.stderr);
    let line = blocked_by_line();
    for part in [blocker.as_str(), "Ingest", "backlog"] {
        assert!(line.contains(part), "{part:?} missing from {line:?}");
    }
    assert!(!pickable(), "{blocked} waits for {blocker}");

    for step in [
        &[
            "story",
            "claim",
            &blocker,
            "--run",
            "r1",
            "--stage",
            "implementing",
        ][..],
        &["story", "tick", &blocker, "--run", "r1", "--criterion", "0"],
        &["story", "move", &blocker, "--run", "r1", "--to", "done"],
    ] {
        let done = ingest(step);
        assert_eq!(done.code, 0, "{step:?}: {}", done.stderr);
    }
    let line = blocked_by_line();
    for part in [blocker.as_str(), "Ingest", "done"] {
        assert!(line.contains(part), "{part:?} missing from {line:?}");
    }
    assert!(pickable(), "{blocked} is pickable once {blocker} is done");
}

/// US-320: a track is split off onto another part, both tracks show the link,
/// and a story moved to the new track keeps its key and its blockers.
#[test]
fn a_track_is_split_off_and_a_story_moved_to_it_keeps_its_key_and_blockers() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("monitoring");
    let created = run_as(&home, AGENT, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    for (part, kind) in [("Wision", "app"), ("iot-service", "service")] {
        let made = run_as(
            &home,
            AGENT,
            &[
                "part",
                "create",
                part,
                "--kind",
                kind,
                "--project",
                &project,
            ],
        );
        assert_eq!(made.code, 0, "part create {part}: {}", made.stderr);
    }
    let made = run_as(
        &home,
        AGENT,
        &[
            "track",
            "create",
            "Long windows",
            "--part",
            "Wision",
            "--project",
            &project,
        ],
    );
    assert_eq!(made.code, 0, "track create: {}", made.stderr);
    let split = run_as(
        &home,
        AGENT,
        &[
            "track",
            "split",
            "Aggregated readings",
            "--part",
            "iot-service",
            "--from",
            "Long windows",
            "--project",
            &project,
        ],
    );
    assert_eq!(split.code, 0, "track split: {}", split.stderr);

    // Each track's show names the other under its side of the link.
    for (track, heading, other) in [
        ("Aggregated readings", "split off from:", "Long windows"),
        ("Long windows", "split off to:", "Aggregated readings"),
    ] {
        let shown = run_as(
            &home,
            AGENT,
            &["track", "show", track, "--project", &project],
        );
        assert_eq!(shown.code, 0, "track show {track}: {}", shown.stderr);
        let lines: Vec<&str> = shown.stdout.lines().collect();
        let at = lines
            .iter()
            .position(|line| line.trim() == heading)
            .unwrap_or_else(|| panic!("{track}: no {heading:?} in {:?}", shown.stdout));
        assert!(
            lines.get(at + 1).is_some_and(|line| line.contains(other)),
            "{track}: {other:?} follows {heading:?}: {:?}",
            shown.stdout
        );
    }
    let shown = run_as(
        &home,
        AGENT,
        &[
            "track",
            "show",
            "Aggregated readings",
            "--project",
            &project,
        ],
    );
    assert!(
        shown.stdout.contains("planning"),
        "the split-off track is planning: {:?}",
        shown.stdout
    );

    let on_long = ["--project", project.as_str(), "--track", "Long windows"];
    let on_aggregated = [
        "--project",
        project.as_str(),
        "--track",
        "Aggregated readings",
    ];
    let blocker = add_story(&home, &on_long, "Read the windows");
    let moving = add_story(&home, &on_long, "Aggregate the readings");
    let dependant = add_story(&home, &on_long, "Show the aggregates");
    let long = |args: &[&str]| run_as(&home, AGENT, &[args, &on_long[..]].concat());
    let aggregated = |args: &[&str]| run_as(&home, AGENT, &[args, &on_aggregated[..]].concat());
    let edited = long(&["story", "edit", &moving, "--blocked-by", &blocker]);
    assert_eq!(edited.code, 0, "story edit: {}", edited.stderr);
    let edited = long(&["story", "edit", &dependant, "--blocked-by", &moving]);
    assert_eq!(edited.code, 0, "story edit: {}", edited.stderr);

    let moved = long(&[
        "story",
        "move-track",
        &moving,
        "--to",
        "Aggregated readings",
    ]);
    assert_eq!(moved.code, 0, "story move-track: {}", moved.stderr);

    let keys = |out: Out| -> Vec<String> {
        assert_eq!(out.code, 0, "story list: {}", out.stderr);
        out.stdout
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .filter(|word| word.starts_with("US-"))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        keys(aggregated(&["story", "list"])),
        vec![moving.clone()],
        "the story is on Aggregated readings"
    );
    assert_eq!(
        keys(long(&["story", "list"])),
        vec![blocker.clone(), dependant.clone()],
        "the story has left Long windows"
    );
    let blocked_by_line = |read: Out| -> String {
        assert_eq!(read.code, 0, "story read: {}", read.stderr);
        read.stdout
            .lines()
            .find(|line| line.starts_with("blocked by:"))
            .unwrap_or_else(|| panic!("read shows no blocker: {}", read.stdout))
            .to_string()
    };
    let line = blocked_by_line(aggregated(&["story", "read", &moving]));
    assert!(
        line.contains(&blocker) && line.contains("Long windows"),
        "the moved story is still blocked by {blocker} on Long windows: {line:?}"
    );
    let line = blocked_by_line(long(&["story", "read", &dependant]));
    assert!(
        line.contains(&moving) && line.contains("Aggregated readings"),
        "{dependant} is still blocked by {moving}, now on Aggregated readings: {line:?}"
    );
}
