//! US-022: `omni doc` against the real API. Run with `--features stack-tests`
//! in the Compose test container, which reaches Postgres at `postgres:5432` and
//! RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key (the
//! agent), create a scratch RustFS bucket and serve the API's router on a local
//! port against both. The built `omni` runs against that address with the key
//! in its config file. The contract of the CLI is in tests/docs.rs and the
//! API's in crates/omni-api/tests/docs.rs.

mod common;

use common::*;
use omni_api::Config;
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us022_cli";
const TEST_BUCKET: &str = "omni-us022-cli";
const KEY_PREFIX: &str = "omniustest22";

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
            let workspace: String = sqlx::query_scalar(
                "insert into workspaces (name) values ('Alpha') returning id::text",
            )
            .fetch_one(&mut conn)
            .await
            .expect("seed a workspace");
            sqlx::query(
                "insert into api_keys (workspace_id, prefix, key_hash, scopes) \
                 values ($1::uuid, $2, encode(sha256(convert_to($3, 'UTF8')), 'hex'), '{*}')",
            )
            .bind(&workspace)
            .bind(KEY_PREFIX)
            .bind(&secret)
            .execute(&mut conn)
            .await
            .expect("seed an API key");
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

fn run_as(home: &Scratch, agent: &str, args: &[&str], stdin: &[u8]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", agent])
        .stdin(stdin)
        .run()
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    run_as(home, "run-7/author", args, b"")
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Node IDs in what `omni doc edit` printed, each once, in order: every
/// 8-character word of lowercase base32 that is not a plain number nor part of
/// a UUID.
fn printed_ids(text: &str) -> Vec<String> {
    let is_uuid = |w: &str| w.len() == 36 && w.matches('-').count() == 4;
    let mut found: Vec<String> = Vec::new();
    for token in text
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .filter(|t| !is_uuid(t))
    {
        for word in token.split('-') {
            let id = word.len() == 8
                && !word.chars().all(|c| c.is_ascii_digit())
                && word
                    .chars()
                    .all(|c| c.is_ascii_digit() || (c.is_ascii_lowercase() && !"ilou".contains(c)));
            if id && !found.iter().any(|f| f == word) {
                found.push(word.to_string());
            }
        }
    }
    found
}

// US-102: a doc is created at the app level or in a feature, and reads back at
// once, through the real API.
#[test]
fn an_agent_creates_docs_and_reads_them_at_once() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("live");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let p = ["--project", project.as_str()];
    let feature = run(
        &home,
        &[&["feature", "create", "Checkout", "--json"][..], &p].concat(),
    );
    assert_eq!(feature.code, 0, "feature create: {}", feature.stderr);
    let checkout = feature.json()["feature"]["id"]
        .as_str()
        .expect("a feature id")
        .to_string();

    let create = |extra: &[&str], kind: &str, pillar: &str, title: &str| -> Value {
        let out = run(
            &home,
            &[
                &["doc", "create"][..],
                extra,
                &[
                    "--kind", kind, "--pillar", pillar, "--title", title, "--json",
                ][..],
                &p,
            ]
            .concat(),
        );
        assert_eq!(out.code, 0, "doc create {title}: {}", out.stderr);
        out.json()["doc"].clone()
    };
    let plan = create(&[], "html", "spec", "Checkout plan");
    assert_eq!(plan["feature_id"], Value::Null, "{plan}");
    let architecture = create(
        &["--feature", "checkout"],
        "html",
        "architecture",
        "Checkout architecture",
    );
    assert_eq!(
        architecture["feature_id"],
        checkout.as_str(),
        "the doc is in the feature: {architecture}"
    );
    let screens = create(&["--feature", "Checkout"], "design", "ui", "Screens");
    assert_eq!(screens["feature_id"], checkout.as_str(), "{screens}");

    for doc in [&plan, &architecture] {
        let id = doc["id"].as_str().expect("a doc id");
        let read = run(&home, &[&["doc", "read", id][..], &p].concat());
        assert_eq!(read.code, 0, "{}", read.stderr);
        assert_eq!(read.stdout.trim(), "", "an empty doc: {:?}", read.stdout);
    }
    let id = screens["id"].as_str().expect("a doc id");
    let read = run(&home, &[&["doc", "read", id][..], &p].concat());
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert_eq!(
        read.stdout.matches("<Page").count(),
        1,
        "an empty design with one page: {:?}",
        read.stdout
    );
    assert!(!read.stdout.contains("<Board"), "{:?}", read.stdout);
}

// US-103: an agent edits a live doc and reads the change at once, through the
// real API.
#[test]
fn an_agent_edits_a_live_doc_and_reads_the_change() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("live-edit");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let p = ["--project", project.as_str()];
    let out = run(
        &home,
        &[
            &[
                "doc", "create", "--kind", "html", "--pillar", "spec", "--title", "Plan", "--json",
            ][..],
            &p,
        ]
        .concat(),
    );
    assert_eq!(out.code, 0, "doc create: {}", out.stderr);
    let doc = out.json()["doc"]["id"]
        .as_str()
        .expect("a doc id")
        .to_string();

    let ops = json!([{
        "op": "insert",
        "parent": null,
        "position": { "after": null },
        "content": "<p>Pay in one step</p>"
    }]);
    let edited = run_as(
        &home,
        "run-7/author",
        &[&["doc", "edit", &doc][..], &p].concat(),
        ops.to_string().as_bytes(),
    );
    assert_eq!(edited.code, 0, "doc edit: {}", edited.stderr);
    let nodes = printed_ids(&edited.stdout);
    assert_eq!(nodes.len(), 1, "the new paragraph: {:?}", edited.stdout);
    let node = &nodes[0];

    let read = run(&home, &[&["doc", "read", &doc][..], &p].concat());
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(
        read.stdout.contains("Pay in one step"),
        "the doc reads with the change at once: {:?}",
        read.stdout
    );

    let set = |rev: u64, text: &str| json!([{ "op": "set", "node": node, "rev": rev, "properties": { "html": text } }]);
    let edited = run_as(
        &home,
        "run-7/author",
        &[&["doc", "edit", &doc, "--json"][..], &p].concat(),
        set(1, "Pay in two steps").to_string().as_bytes(),
    );
    assert_eq!(edited.code, 0, "doc edit: {}", edited.stderr);
    assert_eq!(
        edited.json()["results"][0]["revision"],
        2,
        "{}",
        edited.stdout
    );
    let stale = run_as(
        &home,
        "run-8/editor",
        &[&["doc", "edit", &doc][..], &p].concat(),
        set(1, "Stale").to_string().as_bytes(),
    );
    assert_ne!(stale.code, 0, "a stale rev is refused: {:?}", stale.stdout);
    let lines = stale.stderr_lines();
    assert_eq!(lines.len(), 1, "{:?}", stale.stderr);
    assert!(
        lines[0].contains("conflict")
            && lines[0].contains(node.as_str())
            && lines[0].contains("revision 2"),
        "the conflict names the node and its current revision 2: {:?}",
        lines[0]
    );

    let read = run(&home, &[&["doc", "read", &doc][..], &p].concat());
    assert!(
        read.stdout.contains("Pay in two steps") && !read.stdout.contains("Stale"),
        "{:?}",
        read.stdout
    );
}

// US-110: an agent renames and deletes a live doc from the command line,
// through the real API. Both are logged in `doc_changes`.
#[test]
fn an_agent_renames_and_deletes_a_live_doc_from_the_command_line() {
    let f = fixture();
    let home = agent_home(f);
    let project = unique("tidy");
    let created = run(&home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let p = ["--project", project.as_str()];
    let create = |title: &str| -> String {
        let out = run(
            &home,
            &[
                &[
                    "doc", "create", "--kind", "html", "--pillar", "spec", "--title", title,
                    "--json",
                ][..],
                &p,
            ]
            .concat(),
        );
        assert_eq!(out.code, 0, "doc create: {}", out.stderr);
        out.json()["doc"]["id"]
            .as_str()
            .expect("a doc id")
            .to_string()
    };
    let doc = create("Draft plan");
    let kept = create("Kept plan");

    let renamed = run(
        &home,
        &[&["doc", "rename", &doc, "Checkout plan"][..], &p].concat(),
    );
    assert_eq!(renamed.code, 0, "doc rename: {}", renamed.stderr);
    let shown = run(&home, &[&["doc", "show", &doc][..], &p].concat());
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    assert!(
        shown.stdout.contains("Checkout plan") && !shown.stdout.contains("Draft plan"),
        "the doc shows its new title: {:?}",
        shown.stdout
    );

    let deleted = run(&home, &[&["doc", "delete", &doc][..], &p].concat());
    assert_eq!(deleted.code, 0, "doc delete: {}", deleted.stderr);
    let read = run(&home, &[&["doc", "read", &doc][..], &p].concat());
    assert_ne!(
        read.code, 0,
        "a deleted doc cannot be read: {:?}",
        read.stdout
    );
    assert!(
        read.stderr.contains("not_found"),
        "the error names the code: {:?}",
        read.stderr
    );
    let listed = run(&home, &[&["doc", "list"][..], &p].concat());
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    assert!(
        !listed.stdout.contains(&doc) && listed.stdout.contains(&kept),
        "the list has the kept doc and not the deleted one: {:?}",
        listed.stdout
    );

    let operations: Vec<String> = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar(
            "SELECT operations -> 0 ->> 'op' FROM doc_changes WHERE doc_id = $1::uuid ORDER BY seq",
        )
        .bind(&doc)
        .fetch_all(&mut conn)
        .await
        .expect("read the doc changes")
    });
    assert_eq!(
        operations,
        ["create", "rename", "delete"],
        "the creation, the rename and the delete are logged"
    );
}
