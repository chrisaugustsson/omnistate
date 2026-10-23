//! US-038: `omni comment list|reply|addressed` and `omni version save
//! --remove-commented` against the real API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key and
//! a member (alice) with a live session, create a scratch RustFS bucket and
//! serve the API's router on a local port against both. The built `omni`
//! runs against that address with the key in its config file; the human's
//! steps (commenting on a node of a saved version), which no command does,
//! are plain HTTP requests with alice's session cookie. The CLI's contract is
//! in tests/comments.rs, the API's in crates/omni-api/tests/comments.rs.
//!
//! What these tests add is the round trip: a comment the human pins to a node
//! is listed by `omni comment list` for its change set and doc, an agent's
//! reply and its `addressed` mark reach the API and show in the list, a node
//! changed by a later version shows as `changed`, and `omni version save`
//! refuses to drop a commented node, naming the comment, until
//! `--remove-commented` is given.

mod common;

use common::*;
use omni_api::Config;
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us038_cli";
const TEST_BUCKET: &str = "omni-us038-cli";
const KEY_PREFIX: &str = "omniustest38";
const AUTHOR: &str = "run-7/author";
/// The API's session cookie (crates/omni-api/src/session.rs).
const SESSION_COOKIE: &str = "omni_session";

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
    /// Alice's session token, for the human's steps.
    session: String,
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
        let session = format!("us038-cli-session-{}", "0123456789abcdef".repeat(3));
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
            let user = text(
                "insert into users (email) values ('alice@example.test') returning id::text",
                &[],
            )
            .await;
            text(
                "insert into sessions (user_id, token_hash, expires_at) \
                 values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
                 returning id::text",
                &[&user, &session],
            )
            .await;
            text(
                "insert into memberships (workspace_id, user_id) \
                 values ($1::uuid, $2::uuid) returning id::text",
                &[&workspace, &user],
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
            session,
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

fn run_with(home: &Scratch, args: &[&str], stdin: &[u8]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", AUTHOR])
        .stdin(stdin)
        .run()
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    run_with(home, args, b"")
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// An HTML doc that the author started from the command line in a new
/// project's open change set, with two paragraphs, saved as `v1`.
struct Paper {
    /// The project's name, for `--project`.
    project: String,
    project_id: String,
    changeset: String,
    doc: String,
    v1: String,
    a: String,
    b: String,
}

impl Paper {
    fn edit(&self, home: &Scratch, ops: Value) -> Value {
        let out = run_with(
            home,
            &[
                "doc",
                "edit",
                &self.doc,
                "--changeset",
                &self.changeset,
                "--project",
                &self.project,
                "--json",
            ],
            ops.to_string().as_bytes(),
        );
        assert_eq!(out.code, 0, "doc edit: {}", out.stderr);
        out.json()
    }

    /// `omni version save` with `extra` arguments.
    fn save(&self, home: &Scratch, extra: &[&str]) -> Out {
        run(
            home,
            &[
                &[
                    "version",
                    "save",
                    &self.doc,
                    "--changeset",
                    &self.changeset,
                    "--project",
                    &self.project,
                ][..],
                extra,
            ]
            .concat(),
        )
    }

    fn saved(&self, home: &Scratch, extra: &[&str]) -> String {
        let out = self.save(home, &[extra, &["--json"]].concat());
        assert_eq!(out.code, 0, "version save: {}", out.stderr);
        out.json()["version"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("a version id: {}", out.stdout))
            .to_string()
    }

    /// `omni comment <args> --project <project>`.
    fn comment(&self, home: &Scratch, args: &[&str], stdin: &[u8]) -> Out {
        run_with(
            home,
            &[&["comment"][..], args, &["--project", &self.project]].concat(),
            stdin,
        )
    }

    /// The comments that `omni comment list <args> --json` prints.
    fn listed(&self, home: &Scratch, args: &[&str]) -> Vec<Value> {
        let out = self.comment(home, &[&["list"][..], args, &["--json"]].concat(), b"");
        assert_eq!(out.code, 0, "comment list {args:?}: {}", out.stderr);
        out.json()["comments"]
            .as_array()
            .unwrap_or_else(|| panic!("the answer lists comments: {}", out.stdout))
            .clone()
    }
}

fn new_paper(home: &Scratch) -> Paper {
    let project = unique("comment-site");
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
            "Hero PRD",
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "changeset create: {}", out.stderr);
    let changeset = out.json()["changeset"].clone();
    let changeset_id = changeset["id"].as_str().expect("an id").to_string();
    let out = run(
        home,
        &[
            "doc",
            "create",
            "--project",
            &project,
            "--changeset",
            &changeset_id,
            "--kind",
            "html",
            "--pillar",
            "spec",
            "--title",
            "Hero PRD",
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "doc create: {}", out.stderr);
    let doc = out.json()["doc"]["id"]
        .as_str()
        .expect("the doc's id")
        .to_string();
    let mut paper = Paper {
        project,
        project_id: changeset["project_id"]
            .as_str()
            .expect("the change set's project")
            .to_string(),
        changeset: changeset_id,
        doc,
        v1: String::new(),
        a: String::new(),
        b: String::new(),
    };
    let answer = paper.edit(
        home,
        json!([
            { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>The hero is split.</p>" },
            { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>The copy sits on the left.</p>" }
        ]),
    );
    let node = |i: usize| {
        answer["results"][i]["node"]
            .as_str()
            .unwrap_or_else(|| panic!("an insert names its node: {answer}"))
            .to_string()
    };
    paper.a = node(0);
    paper.b = node(1);
    paper.v1 = paper.saved(home, &[]);
    paper
}

/// The human comments `text` on `node` of `version`, as a plain HTTP request
/// with alice's session cookie; returns the comment's ID.
fn human_comments(f: &Fixture, paper: &Paper, version: &str, node: &str, text: &str) -> String {
    let authority = f.url.trim_start_matches("http://");
    let path = format!(
        "/api/v1/w/{}/projects/{}/versions/{version}/comments",
        f.workspace, paper.project_id
    );
    let body = json!({ "node": node, "text": text }).to_string();
    let mut stream = TcpStream::connect(authority).expect("connect to the API");
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nCookie: {SESSION_COOKIE}={}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        f.session,
        body.len()
    )
    .expect("send the request");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).expect("read the answer");
    let status: u16 = answer
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("an HTTP answer: {answer}"));
    let body = answer
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    assert_eq!(status, 201, "the human comments: {body}");
    let comment: Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("a JSON answer ({e}): {body}"));
    comment["comment"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the comment's id: {comment}"))
        .to_string()
}

fn find<'a>(comments: &'a [Value], id: &str) -> Option<&'a Value> {
    comments.iter().find(|c| c["id"] == id)
}

#[test]
fn an_agent_lists_replies_to_and_addresses_the_humans_comment() {
    let f = fixture();
    let home = agent_home(f);
    let paper = new_paper(&home);
    let text = unique("The copy runs too long at 320 px");
    let comment = human_comments(f, &paper, &paper.v1, &paper.b, &text);

    let out = paper.comment(&home, &["list", "--changeset", &paper.changeset], b"");
    assert_eq!(out.code, 0, "comment list: {}", out.stderr);
    let line = non_empty_lines(&out.stdout)
        .into_iter()
        .find(|l| l.contains(&comment))
        .unwrap_or_else(|| panic!("the comment is listed: {:?}", out.stdout));
    for needle in ["open", paper.b.as_str(), text.as_str()] {
        assert!(line.contains(needle), "{needle:?} in {line:?}");
    }

    let out = paper.comment(
        &home,
        &["reply", &comment],
        b"Shortened in the next version.",
    );
    assert_eq!(out.code, 0, "comment reply: {}", out.stderr);
    let listed = paper.listed(&home, &["--doc", &paper.doc]);
    let read = find(&listed, &comment).unwrap_or_else(|| panic!("listed: {listed:?}"));
    assert_eq!(
        read["replies"][0]["text"].as_str().map(str::trim),
        Some("Shortened in the next version."),
        "{read}"
    );
    assert_eq!(read["replies"][0]["author"]["role"], "author", "{read}");

    paper.edit(
        &home,
        json!([{ "op": "set", "node": paper.b, "rev": 1, "properties": { "html": "Copy left." } }]),
    );
    let v2 = paper.saved(&home, &[]);
    let out = paper.comment(&home, &["addressed", &comment, "--version", &v2], b"");
    assert_eq!(out.code, 0, "comment addressed: {}", out.stderr);

    let open = paper.listed(
        &home,
        &["--changeset", &paper.changeset, "--status", "open"],
    );
    assert!(
        find(&open, &comment).is_none(),
        "an addressed comment is no longer open: {open:?}"
    );
    let addressed = paper.listed(
        &home,
        &["--changeset", &paper.changeset, "--status", "addressed"],
    );
    let read = find(&addressed, &comment).unwrap_or_else(|| panic!("listed: {addressed:?}"));
    assert_eq!(read["status"], "addressed", "{read}");
    assert_eq!(read["addressed_in"], v2.as_str(), "{read}");
    assert_eq!(read["node_status"], "changed", "{read}");

    let out = paper.comment(&home, &["list", "--doc", &paper.doc], b"");
    assert_eq!(out.code, 0, "comment list: {}", out.stderr);
    let line = non_empty_lines(&out.stdout)
        .into_iter()
        .find(|l| l.contains(&comment))
        .unwrap_or_else(|| panic!("the comment is listed: {:?}", out.stdout));
    assert!(
        line.contains("changed"),
        "the node changed since the comment: {line:?}"
    );
}

#[test]
fn saving_a_version_that_removes_a_commented_node_needs_remove_commented() {
    let f = fixture();
    let home = agent_home(f);
    let paper = new_paper(&home);
    let comment = human_comments(f, &paper, &paper.v1, &paper.a, "Say who the hero is for");
    paper.edit(
        &home,
        json!([{ "op": "delete", "node": paper.a, "rev": 1 }]),
    );

    let out = paper.save(&home, &[]);
    assert_ne!(out.code, 0, "the save is refused: {:?}", out.stdout);
    assert_eq!(out.stdout, "", "nothing on standard output");
    for needle in ["--remove-commented", comment.as_str(), paper.a.as_str()] {
        assert!(
            out.stderr.contains(needle),
            "standard error names {needle:?}: {:?}",
            out.stderr
        );
    }
    let before = paper.listed(&home, &["--doc", &paper.doc]);
    assert_eq!(
        find(&before, &comment).map(|c| c["node_status"].clone()),
        Some(json!("current")),
        "nothing was saved, so v1 is still the newest version: {before:?}"
    );

    let saved = paper.saved(&home, &["--remove-commented"]);
    let after = paper.listed(&home, &["--doc", &paper.doc]);
    let read = find(&after, &comment).unwrap_or_else(|| panic!("the comment stays: {after:?}"));
    assert_eq!(read["node_status"], "removed", "{read}");
    assert_eq!(read["compared_to"], saved.as_str(), "{read}");
    assert_eq!(read["status"], "open", "{read}");
}
