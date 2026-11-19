//! US-059: `omni track` against the real API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key (the
//! agent) and a user with a live session (the human), and serve the API's
//! router on a local port against that database. The built `omni` runs against
//! that address with the key in its config file. The contract of the CLI is in
//! tests/tracks.rs, that of the API in crates/omni-api/tests/tracks.rs.

mod common;

use common::*;
use omni_api::Config;
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us059_cli";
const TEST_BUCKET: &str = "omni-us059-cli";
const KEY_PREFIX: &str = "omniustest59";
const HUMAN_TOKEN: &str = "us059-cli-human-session-0123456789abcdef0123456789abcdef0123456789";

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
        // Creating a live doc reaches RustFS, so it needs a scratch bucket.
        s3_endpoint: std::env::var("OMNI_S3_ENDPOINT")
            .unwrap_or_else(|_| "http://rustfs:9000".to_string()),
        s3_access_key: std::env::var("OMNI_S3_ACCESS_KEY")
            .unwrap_or_else(|_| "omni-dev".to_string()),
        s3_secret_key: std::env::var("OMNI_S3_SECRET_KEY")
            .unwrap_or_else(|_| "omni-dev-secret".to_string()),
        s3_bucket: TEST_BUCKET.to_string(),
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
            let user = text(
                "insert into users (email) values ('hanna@example.test') returning id::text",
                &[],
            )
            .await;
            text(
                "insert into sessions (user_id, token_hash, expires_at) \
                 values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
                 returning id::text",
                &[&user, HUMAN_TOKEN],
            )
            .await;
            text(
                "insert into memberships (workspace_id, user_id) \
                 values ($1::uuid, $2::uuid) returning id::text",
                &[&workspace, &user],
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
            runtime
                .block_on(state.ensure_bucket(&config(), 30))
                .expect("create the scratch bucket");
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

/// A home directory whose config file holds the agent's key, as `omni login`
/// leaves it.
fn agent_home(f: &Fixture) -> Scratch {
    let home = Scratch::new("agent");
    write_config(&home.path, &f.url, &f.workspace, &f.secret);
    home
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", "run-7/planner"])
        .run()
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[test]
fn a_track_is_created_listed_and_shown_from_the_command_line() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("shop");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let part = run(
        &home,
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

    let out = run(
        &home,
        &[
            "track",
            "create",
            "Checkout",
            "--description",
            "Pay for the basket in one step.",
            "--part",
            "Wision",
            "--project",
            &project,
        ],
    );
    assert_eq!(out.code, 0, "track create: {}", out.stderr);
    let lines = non_empty_lines(&out.stdout);
    assert_eq!(lines.len(), 1, "{:?}", out.stdout);
    let (checkout, name) = lines[0]
        .split_once("  ")
        .unwrap_or_else(|| panic!("`<id>  <name>`: {:?}", lines[0]));
    assert_eq!(name, "Checkout");
    let checkout = checkout.to_string();

    let out = run(
        &home,
        &[
            "track",
            "create",
            "Wish list",
            "--part",
            "Wision",
            "--project",
            &project,
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "track create --json: {}", out.stderr);
    let wish_list = out.json()["track"].clone();
    assert!(
        wish_list.get("status").is_none(),
        "a track has no status (US-110): {wish_list}"
    );
    assert_eq!(wish_list["description"], "", "{wish_list}");
    let wish_list = wish_list["id"].as_str().expect("an id").to_string();

    // Names are unique in the project, ignoring case.
    let taken = run(
        &home,
        &[
            "track",
            "create",
            "checkout",
            "--part",
            "Wision",
            "--project",
            &project,
        ],
    );
    assert_ne!(taken.code, 0, "{:?}", taken.stdout);
    assert!(
        taken.stderr.contains("name_taken"),
        "the error names the code: {:?}",
        taken.stderr
    );

    let listed = run(&home, &["track", "list", "--project", &project]);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert_eq!(
        non_empty_lines(&listed.stdout),
        [
            format!("{checkout}  Checkout  Wision  planning  -"),
            format!("{wish_list}  Wish list  Wision  planning  -"),
        ],
        "the tracks in creation order, `<id>  <name>  <part>  <state>  <owner>`; \
         the fixture's key has no creator, so no member owns them"
    );

    for which in [checkout.as_str(), "Checkout"] {
        let shown = run(&home, &["track", "show", which, "--project", &project]);
        assert_eq!(shown.code, 0, "show {which}: {}", shown.stderr);
        for needle in [
            checkout.as_str(),
            "Checkout",
            "Pay for the basket in one step.",
        ] {
            assert!(
                shown.stdout.contains(needle),
                "show {which} prints {needle:?}: {:?}",
                shown.stdout
            );
        }
    }

    // --project works the same way.
    let shown = omni(&home.path)
        .args(&[
            "track",
            "show",
            "Wish list",
            "--json",
            "--project",
            &project,
        ])
        .args(&["--as", "run-7/planner"])
        .run();
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    assert_eq!(shown.json()["track"]["id"], wish_list.as_str());
}

// US-061: `omni doc list` against the real API: a doc takes the scope it was
// created in, and the list prints it.
#[test]
fn doc_list_prints_the_scope_of_each_doc_from_the_command_line() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("shop");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let created = run(
        &home,
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
    assert_eq!(created.code, 0, "part create: {}", created.stderr);
    let created = run(
        &home,
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
    assert_eq!(created.code, 0, "track create: {}", created.stderr);

    let doc = |scope: &[&str], kind: &str, pillar: &str, title: &str| {
        let out = run(
            &home,
            &[
                &["doc", "create"][..],
                scope,
                &[
                    "--kind",
                    kind,
                    "--pillar",
                    pillar,
                    "--title",
                    title,
                    "--project",
                    &project,
                    "--json",
                ][..],
            ]
            .concat(),
        );
        assert_eq!(out.code, 0, "doc create {title}: {}", out.stderr);
        out.json()["doc"]["id"].as_str().expect("an id").to_string()
    };
    let prd = doc(&["--global"], "html", "spec", "Brand PRD");
    let architecture = doc(
        &["--track", "Checkout"],
        "html",
        "architecture",
        "Checkout architecture",
    );

    let listed = run(&home, &["doc", "list", "--project", &project]);
    assert_eq!(listed.code, 0, "doc list: {}", listed.stderr);
    assert_eq!(
        non_empty_lines(&listed.stdout),
        [
            format!("{prd}  project  html  spec  Brand PRD"),
            format!("{architecture}  Checkout  html  architecture  Checkout architecture"),
        ],
        "the docs in creation order with their scope"
    );
    for (filter, expected) in [
        (
            &["--track", "Checkout"][..],
            format!("{architecture}  Checkout  html  architecture  Checkout architecture"),
        ),
        (
            &["--global"][..],
            format!("{prd}  project  html  spec  Brand PRD"),
        ),
    ] {
        let out = run(
            &home,
            &[&["doc", "list", "--project", &project][..], filter].concat(),
        );
        assert_eq!(out.code, 0, "doc list {filter:?}: {}", out.stderr);
        assert_eq!(
            non_empty_lines(&out.stdout),
            [expected],
            "doc list {filter:?}"
        );
    }
}

// US-110: an agent renames and deletes a track from the command line. A
// deleted track is in no list, cannot be shown and leaves its name free.
#[test]
fn a_track_is_renamed_and_deleted_from_the_command_line() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("shop");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let part = run(
        &home,
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
    let create = |name: &str| -> String {
        let out = run(
            &home,
            &[
                "track",
                "create",
                name,
                "--part",
                "Wision",
                "--project",
                &project,
            ],
        );
        assert_eq!(out.code, 0, "track create: {}", out.stderr);
        non_empty_lines(&out.stdout)[0]
            .split_once("  ")
            .map(|(id, _)| id.to_string())
            .unwrap_or_else(|| panic!("`<id>  <name>`: {:?}", out.stdout))
    };
    let checkout = create("Checkout");
    let search = create("Search");

    let renamed = run(
        &home,
        &[
            "track",
            "rename",
            "Checkout",
            "Express checkout",
            "--project",
            &project,
        ],
    );
    assert_eq!(renamed.code, 0, "track rename: {}", renamed.stderr);
    let listed = run(&home, &["track", "list", "--project", &project]);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert_eq!(
        non_empty_lines(&listed.stdout),
        [
            format!("{checkout}  Express checkout  Wision  planning  -"),
            format!("{search}  Search  Wision  planning  -"),
        ],
        "the renamed track keeps its ID"
    );

    let taken = run(
        &home,
        &[
            "track",
            "rename",
            "Search",
            "express CHECKOUT",
            "--project",
            &project,
        ],
    );
    assert_ne!(taken.code, 0, "{:?}", taken.stdout);
    assert!(
        taken.stderr.contains("name_taken"),
        "the error names the code: {:?}",
        taken.stderr
    );

    let deleted = run(
        &home,
        &["track", "delete", "Express checkout", "--project", &project],
    );
    assert_eq!(deleted.code, 0, "track delete: {}", deleted.stderr);
    let listed = run(&home, &["track", "list", "--project", &project]);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert_eq!(
        non_empty_lines(&listed.stdout),
        [format!("{search}  Search  Wision  planning  -")],
        "the deleted track is in no list"
    );
    let shown = run(&home, &["track", "show", &checkout, "--project", &project]);
    assert_ne!(shown.code, 0, "{:?}", shown.stdout);
    assert!(
        shown.stderr.contains("not_found"),
        "the error names the code: {:?}",
        shown.stderr
    );

    // The name is free again.
    let again = create("Express checkout");
    assert_ne!(again, checkout, "a new track takes the name");
}

// US-309: a track points at the features of its part that it changes (D15).
#[test]
fn a_track_is_linked_to_a_feature_of_its_part_from_the_command_line() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("monitoring");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    for (part, kind) in [("Wision", "app"), ("iot-service", "service")] {
        let out = run(
            &home,
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
        assert_eq!(out.code, 0, "part create {part}: {}", out.stderr);
    }
    for (feature, part) in [("Analyze", "Wision"), ("Uplinks", "iot-service")] {
        let out = run(
            &home,
            &[
                "feature",
                "create",
                feature,
                "--part",
                part,
                "--project",
                &project,
            ],
        );
        assert_eq!(out.code, 0, "feature create {feature}: {}", out.stderr);
    }
    let out = run(
        &home,
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
    assert_eq!(out.code, 0, "track create: {}", out.stderr);

    // Linking the same feature twice is a no-op, not an error.
    for attempt in ["first", "second"] {
        let out = run(
            &home,
            &[
                "track",
                "link-feature",
                "Long windows",
                "Analyze",
                "--project",
                &project,
            ],
        );
        assert_eq!(out.code, 0, "{attempt} link-feature: {}", out.stderr);
    }

    let shown = run(
        &home,
        &["track", "show", "Long windows", "--project", &project],
    );
    assert_eq!(shown.code, 0, "track show: {}", shown.stderr);
    assert_eq!(
        shown.stdout.matches("Analyze").count(),
        1,
        "track show lists the linked feature once: {:?}",
        shown.stdout
    );
    let shown = run(
        &home,
        &["feature", "show", "Analyze", "--project", &project],
    );
    assert_eq!(shown.code, 0, "feature show: {}", shown.stderr);
    assert!(
        shown.stdout.contains("Long windows"),
        "feature show lists the linked track: {:?}",
        shown.stdout
    );

    // A feature of another part is refused by the API.
    let refused = run(
        &home,
        &[
            "track",
            "link-feature",
            "Long windows",
            "Uplinks",
            "--project",
            &project,
        ],
    );
    assert_ne!(refused.code, 0, "{:?}", refused.stdout);
    assert!(
        refused.stderr.contains("feature_not_on_part"),
        "the error names the code: {:?}",
        refused.stderr
    );
    let shown = run(
        &home,
        &["track", "show", "Long windows", "--project", &project],
    );
    assert_eq!(shown.code, 0, "track show: {}", shown.stderr);
    assert!(
        !shown.stdout.contains("Uplinks"),
        "the refused feature is not linked: {:?}",
        shown.stdout
    );
}

// US-322: a decision is recorded on the current track, about a feature, and
// listed for the track and for the feature; a later decision replaces it.
#[test]
fn a_decision_is_recorded_on_the_track_and_listed_with_the_one_that_replaces_it() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("monitoring");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let out = run(
        &home,
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
    let out = run(
        &home,
        &[
            "feature",
            "create",
            "Analyze",
            "--part",
            "Wision",
            "--project",
            &project,
        ],
    );
    assert_eq!(out.code, 0, "feature create: {}", out.stderr);
    let out = run(
        &home,
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
    assert_eq!(out.code, 0, "track create: {}", out.stderr);
    // The current track comes from OMNI_TRACK.
    let on_track = |args: &[&str]| {
        omni(&home.path)
            .args(args)
            .args(&["--project", &project, "--as", "run-7/planner"])
            .env("OMNI_TRACK", "Long windows")
            .run()
    };

    let added = on_track(&[
        "decision",
        "add",
        "--question",
        "Raw or aggregated?",
        "--decision",
        "Aggregate per hour",
        "--why",
        "Two years raw is too slow",
        "--feature",
        "Analyze",
        "--json",
    ]);
    assert_eq!(added.code, 0, "decision add: {}", added.stderr);
    let first = added.json()["decision"].clone();
    assert_eq!(first["author"]["kind"], "agent", "{first}");
    assert_eq!(first["author"]["run"], "run-7", "{first}");
    assert_eq!(first["author"]["role"], "planner", "{first}");
    let first = first["id"].as_str().expect("an id").to_string();

    for filter in [&[][..], &["--feature", "Analyze"][..]] {
        let listed = on_track(&[&["decision", "list"][..], filter].concat());
        assert_eq!(
            listed.code, 0,
            "decision list {filter:?}: {}",
            listed.stderr
        );
        let lines = non_empty_lines(&listed.stdout);
        assert_eq!(
            lines.len(),
            1,
            "decision list {filter:?}: {:?}",
            listed.stdout
        );
        for needle in [
            first.as_str(),
            "Analyze",
            "Raw or aggregated?",
            "Aggregate per hour",
            "Two years raw is too slow",
        ] {
            assert!(
                lines[0].contains(needle),
                "decision list {filter:?} prints {needle:?}: {:?}",
                lines[0]
            );
        }
    }

    // Without --feature, the new decision takes the replaced one's feature.
    let added = on_track(&[
        "decision",
        "add",
        "--question",
        "Raw or aggregated?",
        "--decision",
        "Aggregate per quarter hour",
        "--why",
        "An hour hides the peaks",
        "--replaces",
        &first,
        "--json",
    ]);
    assert_eq!(added.code, 0, "decision add --replaces: {}", added.stderr);
    let second = added.json()["decision"]["id"]
        .as_str()
        .expect("an id")
        .to_string();

    let listed = on_track(&["decision", "list", "--feature", "Analyze"]);
    assert_eq!(listed.code, 0, "decision list --feature: {}", listed.stderr);
    let lines = non_empty_lines(&listed.stdout);
    assert_eq!(lines.len(), 2, "both decisions: {:?}", listed.stdout);
    assert!(
        lines[0].starts_with(&second)
            && lines[0].contains("Aggregate per quarter hour")
            && lines[0].contains(&format!("replaces {first}"))
            && !lines[0].contains("replaced by"),
        "the newer decision comes first, marked as replacing the older: {:?}",
        lines[0]
    );
    assert!(
        lines[1].starts_with(&first)
            && lines[1].contains(&format!("replaced by {second}"))
            && !lines[1].contains("replaces "),
        "the older decision is marked as replaced by the newer: {:?}",
        lines[1]
    );
}

// US-329: a decision about a part is listed by that part and by no feature, one
// about the project by the project, the track lists both, and two levels at
// once is one_scope_only.
#[test]
fn a_decision_about_a_part_or_the_project_is_listed_at_that_level_only() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("monitoring");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    for (part, kind) in [("Wision", "app"), ("iot-service", "service")] {
        let out = run(
            &home,
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
        assert_eq!(out.code, 0, "part create {part}: {}", out.stderr);
    }
    for (feature, part) in [("Analyze", "Wision"), ("Ingest", "iot-service")] {
        let out = run(
            &home,
            &[
                "feature",
                "create",
                feature,
                "--part",
                part,
                "--project",
                &project,
            ],
        );
        assert_eq!(out.code, 0, "feature create {feature}: {}", out.stderr);
    }
    let out = run(
        &home,
        &[
            "track",
            "create",
            "Readings",
            "--part",
            "iot-service",
            "--project",
            &project,
        ],
    );
    assert_eq!(out.code, 0, "track create: {}", out.stderr);
    let on_track = |args: &[&str]| {
        omni(&home.path)
            .args(args)
            .args(&["--project", &project, "--as", "run-7/planner"])
            .env("OMNI_TRACK", "Readings")
            .run()
    };
    let add = |level: &[&str], decision: &str| {
        let base = [
            "decision",
            "add",
            "--question",
            "Q",
            "--decision",
            decision,
            "--why",
            "W",
        ];
        let added = on_track(&[&base[..], level, &["--json"][..]].concat());
        assert_eq!(added.code, 0, "decision add {level:?}: {}", added.stderr);
        added.json()["decision"]["id"]
            .as_str()
            .expect("an id")
            .to_string()
    };
    let listed = |filter: &[&str]| {
        let out = on_track(&[&["decision", "list"][..], filter].concat());
        assert_eq!(out.code, 0, "decision list {filter:?}: {}", out.stderr);
        out.stdout
    };

    let part = add(&["--part", "iot-service"], "Only writer of readings");
    // A bare --project, before the --project <name> that selects the project.
    let whole = add(&["--project"], "One stack per customer");

    let of_part = listed(&["--part", "iot-service"]);
    assert!(
        of_part.contains(&part) && of_part.contains("Only writer of readings"),
        "the part lists its decision: {of_part:?}"
    );
    assert!(
        !of_part.contains(&whole),
        "the part leaves out the project's decision: {of_part:?}"
    );
    for feature in ["Analyze", "Ingest"] {
        let of_feature = listed(&["--feature", feature]);
        assert!(
            !of_feature.contains(&part) && !of_feature.contains(&whole),
            "feature {feature} lists neither: {of_feature:?}"
        );
    }
    let of_project = listed(&["--project"]);
    assert!(
        of_project.contains(&whole) && of_project.contains("One stack per customer"),
        "the project lists its decision: {of_project:?}"
    );
    assert!(
        !of_project.contains(&part),
        "the project leaves out the part's decision: {of_project:?}"
    );
    let of_track = listed(&[]);
    assert!(
        of_track.contains(&part) && of_track.contains(&whole),
        "the track lists every decision recorded on it: {of_track:?}"
    );

    for levels in [
        &["--feature", "Ingest", "--part", "iot-service"][..],
        &["--part", "iot-service", "--project"][..],
    ] {
        let base = [
            "decision",
            "add",
            "--question",
            "Q",
            "--decision",
            "Both",
            "--why",
            "W",
        ];
        let out = on_track(&[&base[..], levels].concat());
        assert_ne!(out.code, 0, "{levels:?} is refused: {}", out.stdout);
        assert!(
            out.stderr.contains("one_scope_only"),
            "{levels:?}: {}",
            out.stderr
        );
    }
    assert!(
        !listed(&[]).contains("Both"),
        "a refused decision is not recorded"
    );
}
