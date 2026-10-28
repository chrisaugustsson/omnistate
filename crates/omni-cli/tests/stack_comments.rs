//! US-106: `omni comment list` and `omni comment reply` on live docs against
//! the real API. Run with `--features stack-tests` in the Compose test
//! container, which reaches Postgres at `postgres:5432` and RustFS at
//! `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key and
//! a member (alice) with a live session, create a scratch RustFS bucket and
//! serve the API's router on a local port against both. The built `omni`
//! runs against that address with the key in its config file and writes the
//! docs straight to their heads (no change set); the human's steps
//! (commenting on a node of a doc's head, replying, deleting a comment), which
//! no command does, are plain HTTP requests with alice's session cookie. The
//! CLI's contract is in tests/comments.rs, the API's in
//! crates/omni-api/tests/comments.rs.
//!
//! What these tests add is the round trip: the comments the human pins to
//! nodes of a live doc are listed by `omni comment list` with their doc, node,
//! text and replies, oldest first; `omni comment reply <id> <text>` reaches
//! the API and makes the comment wait on the human; `--doc` and `--feature`
//! narrow the list; and a comment the human deleted is no longer listed and
//! cannot be replied to.

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

const TEST_DB: &str = "omni_us106_cli";
const TEST_BUCKET: &str = "omni-us106-cli";
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
        let session = format!("us106-cli-session-{}", "0123456789abcdef".repeat(3));
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

/// A live HTML doc that the author created from the command line in a new
/// project, at the app level or in a feature, with two paragraphs.
struct Paper {
    /// The project's name, for `--project`.
    project: String,
    project_id: String,
    doc: String,
    a: String,
    b: String,
}

impl Paper {
    /// `omni comment <args> --project <project>`.
    fn comment(&self, home: &Scratch, args: &[&str]) -> Out {
        run(
            home,
            &[&["comment"][..], args, &["--project", &self.project]].concat(),
        )
    }

    /// The comments that `omni comment list <args> --json` prints.
    fn listed(&self, home: &Scratch, args: &[&str]) -> Vec<Value> {
        let out = self.comment(home, &[&["list"][..], args, &["--json"]].concat());
        assert_eq!(out.code, 0, "comment list {args:?}: {}", out.stderr);
        out.json()["comments"]
            .as_array()
            .unwrap_or_else(|| panic!("the answer lists comments: {}", out.stdout))
            .clone()
    }
}

fn new_project(home: &Scratch) -> String {
    let project = unique("comment-site");
    let created = run(home, &["project", "create", &project]);
    assert_eq!(created.code, 0, "project create: {}", created.stderr);
    project
}

/// A live doc in `project` (in `feature` when given, else at the app level)
/// with two paragraphs written straight to its head.
fn new_paper(home: &Scratch, project: &str, feature: Option<&str>) -> Paper {
    let scope = match feature {
        Some(feature) => vec!["--feature", feature],
        None => vec!["--global"],
    };
    let out = run(
        home,
        &[
            &[
                "doc",
                "create",
                "--project",
                project,
                "--kind",
                "html",
                "--pillar",
                "spec",
                "--title",
                "Hero spec",
                "--json",
            ][..],
            &scope,
        ]
        .concat(),
    );
    assert_eq!(out.code, 0, "doc create: {}", out.stderr);
    let doc = out.json()["doc"].clone();
    let doc_id = doc["id"].as_str().expect("the doc's id").to_string();
    let ops = json!([
        { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>The hero is split.</p>" },
        { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>The copy sits on the left.</p>" }
    ]);
    let out = run_with(
        home,
        &["doc", "edit", &doc_id, "--project", project, "--json"],
        ops.to_string().as_bytes(),
    );
    assert_eq!(out.code, 0, "doc edit: {}", out.stderr);
    let answer = out.json();
    let node = |i: usize| {
        answer["results"][i]["node"]
            .as_str()
            .unwrap_or_else(|| panic!("an insert names its node: {answer}"))
            .to_string()
    };
    Paper {
        project: project.to_string(),
        project_id: doc["project_id"]
            .as_str()
            .expect("the doc's project")
            .to_string(),
        doc: doc_id,
        a: node(0),
        b: node(1),
    }
}

/// A request of the human's, as plain HTTP with alice's session cookie;
/// returns the status and the body.
fn human_request(f: &Fixture, method: &str, path: &str, body: Option<&Value>) -> (u16, String) {
    let authority = f.url.trim_start_matches("http://");
    let body = body.map(Value::to_string).unwrap_or_default();
    let mut stream = TcpStream::connect(authority).expect("connect to the API");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nCookie: {SESSION_COOKIE}={}\r\n\
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
    (status, body)
}

fn project_path(f: &Fixture, paper: &Paper) -> String {
    format!("/api/v1/w/{}/projects/{}", f.workspace, paper.project_id)
}

/// The human pins `text` to `node` of the paper's head; returns the comment's
/// ID.
fn human_comments(f: &Fixture, paper: &Paper, node: &str, text: &str) -> String {
    let (status, body) = human_request(
        f,
        "POST",
        &format!("{}/docs/{}/comments", project_path(f, paper), paper.doc),
        Some(&json!({ "node": node, "text": text })),
    );
    assert_eq!(status, 201, "the human comments: {body}");
    let comment: Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("a JSON answer ({e}): {body}"));
    comment["comment"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the comment's id: {comment}"))
        .to_string()
}

fn human_replies(f: &Fixture, paper: &Paper, comment: &str, text: &str) {
    let (status, body) = human_request(
        f,
        "POST",
        &format!("{}/comments/{comment}/replies", project_path(f, paper)),
        Some(&json!({ "text": text })),
    );
    assert_eq!(status, 201, "the human replies: {body}");
}

fn find<'a>(comments: &'a [Value], id: &str) -> Option<&'a Value> {
    comments.iter().find(|c| c["id"] == id)
}

fn ids(comments: &[Value]) -> Vec<String> {
    comments
        .iter()
        .map(|c| c["id"].as_str().expect("an id").to_string())
        .collect()
}

#[test]
fn an_agent_lists_the_humans_comments_on_a_live_doc_and_replies() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let paper = new_paper(&home, &project, None);
    let first_text = unique("The copy runs too long at 320 px");
    let second_text = unique("Say who the hero is for");
    let first = human_comments(f, &paper, &paper.b, &first_text);
    let second = human_comments(f, &paper, &paper.a, &second_text);
    human_replies(f, &paper, &first, "Is that clear?");

    let out = paper.comment(&home, &["reply", &first, "Shortened to one line."]);
    assert_eq!(out.code, 0, "comment reply: {}", out.stderr);

    let out = paper.comment(&home, &["list"]);
    assert_eq!(out.code, 0, "comment list: {}", out.stderr);
    let text = &out.stdout;
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("the list shows {needle:?}: {text:?}"))
    };
    let order = [
        at(&first),
        at(&first_text),
        at("Is that clear?"),
        at("Shortened to one line."),
        at(&second),
        at(&second_text),
    ];
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "the comments with their replies, oldest first: {text:?}"
    );
    let first_part = &text[order[0]..order[4]];
    for needle in [paper.doc.as_str(), paper.b.as_str()] {
        assert!(
            first_part.contains(needle),
            "the first comment shows {needle:?}: {first_part:?}"
        );
    }
    assert!(
        first_part.contains("waiting on you"),
        "an agent replied last, so it waits: {first_part:?}"
    );
    let second_part = &text[order[4]..];
    assert!(
        second_part.contains(&paper.a) && !second_part.contains("waiting on you"),
        "{second_part:?}"
    );

    let listed = paper.listed(&home, &["--doc", &paper.doc]);
    assert_eq!(ids(&listed), [first.clone(), second.clone()], "{listed:?}");
    let read = find(&listed, &first).expect("the first comment");
    let replies = read["replies"].as_array().expect("replies");
    assert_eq!(replies.len(), 2, "{read}");
    assert_eq!(replies[1]["text"], "Shortened to one line.", "{read}");
    assert_eq!(replies[1]["author"]["kind"], "agent", "{read}");
    assert_eq!(replies[1]["author"]["role"], "author", "{read}");
    assert_eq!(read["waiting_on_you"], true, "{read}");
}
