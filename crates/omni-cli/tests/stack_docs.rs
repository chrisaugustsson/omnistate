//! US-022: `omni doc` against the real API. Run with `--features stack-tests`
//! in the Compose test container, which reaches Postgres at `postgres:5432` and
//! RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key (the
//! agent), create a scratch RustFS bucket and serve the API's router on a local
//! port against both. The built `omni` runs against that address with the key
//! in its config file. The contract of the CLI is in tests/docs.rs,
//! tests/claims.rs (US-023) and tests/versions.rs (US-024), and the API's in
//! crates/omni-api/tests/docs.rs, crates/omni-api/tests/claims.rs and
//! crates/omni-api/tests/versions.rs.

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

/// A project with an open change set and an HTML PRD started in it, made from
/// the command line; returns (project name, change set ID, the `doc create
/// --json` answer).
fn project_with_doc(home: &Scratch) -> (String, String, Value) {
    let project = unique("checkout");
    let created = run(home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    let out = run(
        home,
        &[
            "changeset",
            "create",
            "--project",
            &project,
            "--title",
            "Checkout PRD",
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "changeset create: {}", out.stderr);
    let changeset = out.json()["changeset"]["id"]
        .as_str()
        .expect("a change set id")
        .to_string();
    let out = run(
        home,
        &[
            "doc",
            "create",
            "--project",
            &project,
            "--changeset",
            &changeset,
            "--kind",
            "html",
            "--module",
            "prd",
            "--title",
            "Checkout PRD",
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "doc create: {}", out.stderr);
    let answer = out.json();
    assert!(answer["doc"]["id"].is_string(), "{answer}");
    (project, changeset, answer)
}

/// Every start tag of `html` that carries `data-node-id`, as (ID, `data-rev`).
fn tagged(html: &str) -> Vec<(String, Option<String>)> {
    let attribute = |tag: &str, name: &str| -> Option<String> {
        let needle = format!("{name}=\"");
        let start = tag.find(&needle)? + needle.len();
        let end = tag[start..].find('"')?;
        Some(tag[start..start + end].to_string())
    };
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start..start + end];
        if let Some(id) = attribute(tag, "data-node-id") {
            found.push((id, attribute(tag, "data-rev")));
        }
        rest = &rest[start + end + 1..];
    }
    found
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

#[test]
fn an_agent_starts_a_doc_edits_its_draft_and_reads_it_back() {
    let f = fixture();
    let home = agent_home(f);
    let (project, changeset, created) = project_with_doc(&home);
    let doc = created["doc"]["id"].as_str().unwrap().to_string();
    let draft = created["draft"]["id"].as_str().unwrap().to_string();
    let p = ["--project", project.as_str()];

    // Show: no mainline version yet, and the change set's draft.
    let shown = run(&home, &[&["doc", "show", &doc][..], &p].concat());
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    assert!(
        shown.stdout.contains("Checkout PRD")
            && shown.stdout.contains(&draft)
            && shown.stdout.contains(&changeset),
        "{:?}",
        shown.stdout
    );
    assert!(
        non_empty_lines(&shown.stdout)
            .iter()
            .any(|l| l.to_lowercase().contains("mainline") && l.to_lowercase().contains("none")),
        "{:?}",
        shown.stdout
    );

    // Mainline has nothing of the doc until the change set merges.
    let mainline = run(&home, &[&["doc", "read", &doc][..], &p].concat());
    assert_ne!(mainline.code, 0, "{:?}", mainline.stdout);
    assert!(
        mainline.stderr.to_lowercase().contains("mainline"),
        "{:?}",
        mainline.stderr
    );

    // Edit from standard input.
    let ops = json!([
        { "op": "insert", "parent": null, "position": { "after": null }, "content": "<h1>Checkout</h1>" },
        { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>Pay in one step.</p>" }
    ]);
    let edited = run_as(
        &home,
        "run-7/author",
        &[&["doc", "edit", &doc, "--changeset", &changeset][..], &p].concat(),
        ops.to_string().as_bytes(),
    );
    assert_eq!(edited.code, 0, "{}", edited.stderr);
    let new_ids = printed_ids(&edited.stdout);
    assert_eq!(
        new_ids.len(),
        2,
        "the IDs of the two new nodes are printed: {:?}",
        edited.stdout
    );

    let read = run(
        &home,
        &[&["doc", "read", &doc, "--changeset", &changeset][..], &p].concat(),
    );
    assert_eq!(read.code, 0, "{}", read.stderr);
    let tags = tagged(&read.stdout);
    let ids: Vec<&str> = tags.iter().map(|(id, _)| id.as_str()).collect();
    for id in &new_ids {
        assert!(
            ids.contains(&id.as_str()),
            "{id} is in the draft: {:?}",
            read.stdout
        );
    }
    assert!(
        tags.iter().all(|(_, rev)| rev.as_deref() == Some("1")),
        "{:?}",
        read.stdout
    );
    assert!(
        read.stdout.contains("Pay in one step."),
        "{:?}",
        read.stdout
    );

    // Edit from a file: the paragraph is the first block.
    let paragraph = ids[0].to_string();
    let file = home.join("ops.json");
    std::fs::write(
        &file,
        json!([{ "op": "set", "node": paragraph, "rev": 1, "properties": { "html": "Pay in <strong>one</strong> step." } }])
            .to_string(),
    )
    .unwrap();
    let edited = run(
        &home,
        &[
            &[
                "doc",
                "edit",
                &doc,
                "--changeset",
                &changeset,
                "--body-file",
                file.to_str().unwrap(),
            ][..],
            &p,
        ]
        .concat(),
    );
    assert_eq!(edited.code, 0, "{}", edited.stderr);
    assert!(
        non_empty_lines(&edited.stdout)
            .iter()
            .any(|l| l.contains(&paragraph) && l.contains('2')),
        "the new revision of {paragraph}: {:?}",
        edited.stdout
    );
    let read = run(
        &home,
        &[&["doc", "read", &doc, "--changeset", &changeset][..], &p].concat(),
    );
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(
        tagged(&read.stdout)
            .iter()
            .any(|(id, rev)| id == &paragraph && rev.as_deref() == Some("2")),
        "{:?}",
        read.stdout
    );
    assert!(
        read.stdout.contains("<strong>one</strong>"),
        "{:?}",
        read.stdout
    );

    // Opening the doc in its change set reuses the draft.
    let opened = run(
        &home,
        &[&["doc", "open", &doc, "--changeset", &changeset][..], &p].concat(),
    );
    assert_eq!(opened.code, 0, "{}", opened.stderr);
    assert!(opened.stdout.contains(&draft), "{:?}", opened.stdout);
}

#[test]
fn an_unknown_node_id_is_one_line_that_names_it() {
    let f = fixture();
    let home = agent_home(f);
    let (project, changeset, created) = project_with_doc(&home);
    let doc = created["doc"]["id"].as_str().unwrap().to_string();
    let ops = json!([{ "op": "delete", "node": "zzzzzzzz", "rev": 1 }]);
    let out = run_as(
        &home,
        "run-7/author",
        &[
            "doc",
            "edit",
            &doc,
            "--changeset",
            &changeset,
            "--project",
            &project,
        ],
        ops.to_string().as_bytes(),
    );
    assert_ne!(out.code, 0, "{:?}", out.stdout);
    assert_eq!(out.stdout, "");
    let lines = out.stderr_lines();
    assert_eq!(lines.len(), 1, "{:?}", out.stderr);
    assert!(lines[0].contains("zzzzzzzz"), "{:?}", lines[0]);
}

#[test]
fn two_agents_edit_different_nodes_of_one_draft_at_the_same_time() {
    let f = fixture();
    let home = agent_home(f);
    let (project, changeset, created) = project_with_doc(&home);
    let doc = created["doc"]["id"].as_str().unwrap().to_string();
    let ops = json!([
        { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>A</p>" },
        { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>B</p>" }
    ]);
    let common = [
        "--changeset",
        changeset.as_str(),
        "--project",
        project.as_str(),
    ];
    let edited = run_as(
        &home,
        "run-7/author",
        &[&["doc", "edit", &doc][..], &common].concat(),
        ops.to_string().as_bytes(),
    );
    assert_eq!(edited.code, 0, "{}", edited.stderr);
    let nodes = printed_ids(&edited.stdout);
    assert_eq!(nodes.len(), 2, "{:?}", edited.stdout);

    const ROUNDS: u64 = 8;
    std::thread::scope(|scope| {
        for (node, agent) in nodes.iter().zip(["run-a/writer", "run-b/editor"]) {
            let (home, doc, common) = (&home, &doc, &common);
            scope.spawn(move || {
                for round in 1..=ROUNDS {
                    let ops = json!([{
                        "op": "set",
                        "node": node,
                        "rev": round,
                        "properties": { "html": format!("{agent} round {round}") }
                    }]);
                    let out = run_as(
                        home,
                        agent,
                        &[&["doc", "edit", doc.as_str()][..], common].concat(),
                        ops.to_string().as_bytes(),
                    );
                    assert_eq!(out.code, 0, "{agent} round {round}: {}", out.stderr);
                }
            });
        }
    });
    let read = run(&home, &[&["doc", "read", &doc][..], &common].concat());
    assert_eq!(read.code, 0, "{}", read.stderr);
    for agent in ["run-a/writer", "run-b/editor"] {
        assert!(
            read.stdout.contains(&format!("{agent} round {ROUNDS}")),
            "{agent}'s last edit applied: {:?}",
            read.stdout
        );
    }
    let revs: Vec<Option<String>> = tagged(&read.stdout).into_iter().map(|(_, r)| r).collect();
    assert_eq!(
        revs,
        vec![Some((ROUNDS + 1).to_string()); 2],
        "{:?}",
        read.stdout
    );
}

/// Seconds since the epoch of an RFC 3339 time, as Postgres reads it.
fn epoch(time: &Value) -> f64 {
    let time = time
        .as_str()
        .unwrap_or_else(|| panic!("a time is a string: {time}"));
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar::<_, f64>("select extract(epoch from $1::text::timestamptz)::float8")
            .bind(time)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("{time:?} is not a time: {e}"))
    })
}

/// The names of the live events of a project with that change set and node.
fn claim_events(changeset: &str, node: &str) -> Vec<String> {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar::<_, String>(
            "select event from live_events \
             where event like 'claim.%' and data->>'changeset_id' = $1 and data->>'node' = $2 \
             order by seq",
        )
        .bind(changeset)
        .bind(node)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the live events: {e}"))
    })
}

#[test]
fn an_agent_claims_a_section_and_another_agent_cannot_change_it_until_it_is_released() {
    // US-023: claims through the command line, with the default claim TTL.
    let f = fixture();
    let home = agent_home(f);
    let (project, changeset, created) = project_with_doc(&home);
    let doc = created["doc"]["id"].as_str().unwrap().to_string();
    let common = [
        "--changeset",
        changeset.as_str(),
        "--project",
        project.as_str(),
    ];
    let ops = json!([{
        "op": "insert",
        "parent": null,
        "position": { "after": null },
        "content": "<section><h2>Pricing</h2><p>Pay monthly.</p></section>"
    }]);
    let edited = run_as(
        &home,
        "run-7/author",
        &[&["doc", "edit", &doc][..], &common, &["--json"]].concat(),
        ops.to_string().as_bytes(),
    );
    assert_eq!(edited.code, 0, "{}", edited.stderr);
    let created_ids: Vec<String> = edited.json()["results"][0]["created"]
        .as_array()
        .expect("the IDs of the new nodes")
        .iter()
        .map(|id| id.as_str().expect("a node ID").to_string())
        .collect();
    assert_eq!(created_ids.len(), 3, "{created_ids:?}");
    let (section, paragraph) = (created_ids[0].clone(), created_ids[2].clone());

    // The author claims the section; a claim lasts 10 minutes by default.
    let claimed = run(
        &home,
        &[&["claim", &section][..], &common, &["--json"]].concat(),
    );
    assert_eq!(claimed.code, 0, "{}", claimed.stderr);
    let claim = claimed.json()["claim"].clone();
    assert_eq!(claim["node"], section.as_str(), "{claim}");
    let ttl = epoch(&claim["expires_at"]) - epoch(&claim["taken_at"]);
    assert!(
        (595.0..=605.0).contains(&ttl),
        "a claim lasts 10 minutes, not {ttl} s: {claim}"
    );

    // Another agent can neither claim nor change it, and is told who holds it.
    let refused = run_as(
        &home,
        "run-8/editor",
        &[&["claim", &section][..], &common].concat(),
        b"",
    );
    assert_ne!(refused.code, 0, "{:?}", refused.stdout);
    assert_eq!(refused.stderr_lines().len(), 1, "{:?}", refused.stderr);
    assert!(
        refused.stderr.contains("run-7/author"),
        "{:?}",
        refused.stderr
    );
    let change = json!([{ "op": "set", "node": paragraph, "rev": 1, "properties": { "html": "Pay yearly." } }]);
    let refused = run_as(
        &home,
        "run-8/editor",
        &[&["doc", "edit", &doc][..], &common].concat(),
        change.to_string().as_bytes(),
    );
    assert_ne!(refused.code, 0, "{:?}", refused.stdout);
    assert_eq!(refused.stderr_lines().len(), 1, "{:?}", refused.stderr);
    assert!(
        refused.stderr.contains("run-7/author"),
        "{:?}",
        refused.stderr
    );

    // The claim shows in doc show.
    let shown = run(&home, &["doc", "show", &doc, "--project", &project]);
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    assert!(
        non_empty_lines(&shown.stdout)
            .iter()
            .any(|l| l.contains(&section) && l.contains("run-7/author")),
        "{:?}",
        shown.stdout
    );

    // Released, the section is free again.
    let released = run(&home, &[&["release", &section][..], &common].concat());
    assert_eq!(released.code, 0, "{}", released.stderr);
    let applied = run_as(
        &home,
        "run-8/editor",
        &[&["doc", "edit", &doc][..], &common].concat(),
        change.to_string().as_bytes(),
    );
    assert_eq!(applied.code, 0, "{}", applied.stderr);
    let shown = run(&home, &["doc", "show", &doc, "--project", &project]);
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    assert!(
        !shown.stdout.contains("run-7/author"),
        "a released claim no longer shows: {:?}",
        shown.stdout
    );
    assert_eq!(
        claim_events(&changeset, &section),
        ["claim.taken", "claim.released"]
    );
}

#[test]
fn an_agent_saves_versions_compares_them_and_removes_the_doc() {
    let f = fixture();
    let home = agent_home(f);
    let (project, changeset, created) = project_with_doc(&home);
    let doc = created["doc"]["id"].as_str().unwrap().to_string();
    let p = ["--project", project.as_str()];
    let edit = |ops: Value| -> Value {
        let out = run_as(
            &home,
            "run-7/author",
            &[
                &["doc", "edit", &doc, "--changeset", &changeset, "--json"][..],
                &p,
            ]
            .concat(),
            ops.to_string().as_bytes(),
        );
        assert_eq!(out.code, 0, "doc edit: {}", out.stderr);
        out.json()
    };
    let save = || -> String {
        let out = run(
            &home,
            &[
                &["version", "save", &doc, "--changeset", &changeset, "--json"][..],
                &p,
            ]
            .concat(),
        );
        assert_eq!(out.code, 0, "version save: {}", out.stderr);
        out.json()["version"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("a version id: {}", out.stdout))
            .to_string()
    };

    let answer = edit(json!([
        { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>Keep this.</p>" },
        { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>Drop this.</p>" },
        { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>Change this.</p>" }
    ]));
    let node = |i: usize| answer["results"][i]["node"].as_str().unwrap().to_string();
    let (kept, dropped, changed) = (node(0), node(1), node(2));
    let v1 = save();

    let answer = edit(json!([
        { "op": "delete", "node": dropped, "rev": 1 },
        { "op": "set", "node": changed, "rev": 1, "properties": { "html": "Changed." } },
        { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>Added.</p>" }
    ]));
    let added = answer["results"][2]["node"].as_str().unwrap().to_string();
    let v2 = save();

    // The diff lists each difference on a line of its own.
    let diff = run(&home, &[&["version", "diff", &v1, &v2][..], &p].concat());
    assert_eq!(diff.code, 0, "{}", diff.stderr);
    let lines = non_empty_lines(&diff.stdout);
    for (word, id) in [
        ("added", &added),
        ("removed", &dropped),
        ("changed", &changed),
    ] {
        assert!(
            lines
                .iter()
                .any(|l| l.contains(id.as_str()) && l.contains(word)),
            "{id} is {word}: {:?}",
            diff.stdout
        );
    }
    assert!(
        !diff.stdout.contains(&kept),
        "an unchanged node is not listed: {:?}",
        diff.stdout
    );

    // Version 1 reads as it was saved.
    let read = run(
        &home,
        &[&["doc", "read", &doc, "--version", &v1][..], &p].concat(),
    );
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(
        read.stdout.contains("Drop this.") && read.stdout.contains("Change this."),
        "{:?}",
        read.stdout
    );
    assert!(!read.stdout.contains("Added."), "{:?}", read.stdout);

    // Removing the doc saves a version marking it removed; the history stays.
    let removed = run(
        &home,
        &[
            &["doc", "remove", &doc, "--changeset", &changeset, "--json"][..],
            &p,
        ]
        .concat(),
    );
    assert_eq!(removed.code, 0, "{}", removed.stderr);
    let removal = removed.json();
    assert_eq!(removal["version"]["removed"], true, "{removal}");
    let removal = removal["version"]["id"].as_str().unwrap().to_string();
    let diff = run(
        &home,
        &[&["version", "diff", &v2, &removal][..], &p].concat(),
    );
    assert_eq!(diff.code, 0, "{}", diff.stderr);
    for id in [&kept, &changed, &added] {
        assert!(
            non_empty_lines(&diff.stdout)
                .iter()
                .any(|l| l.contains(id.as_str()) && l.contains("removed")),
            "{id} is removed with the doc: {:?}",
            diff.stdout
        );
    }
    let read = run(
        &home,
        &[&["doc", "read", &doc, "--version", &v2][..], &p].concat(),
    );
    assert_eq!(read.code, 0, "{}", read.stderr);
    assert!(read.stdout.contains("Added."), "{:?}", read.stdout);
}
