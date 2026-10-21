//! US-037: `omni decision propose`, `omni decision show` and `omni changeset
//! link-decision` against the real API. Run with `--features stack-tests` in
//! the Compose test container, which reaches Postgres at `postgres:5432`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key and
//! a member (alice) with a live session, and serve the API's router on a local
//! port against that database. The built `omni` runs against that address with
//! the key in its config file; the human's steps (approving and rejecting a
//! decision), which no command does, are plain HTTP requests with alice's
//! session cookie. The CLI's contract is in tests/decisions.rs, the API's in
//! crates/omni-api/tests/decisions.rs.
//!
//! What these tests add is the round trip: a decision file proposed from the
//! command line is the decision that `omni decision show` prints, by thread
//! and by ID, with the status and reason that the human's steps gave it; a
//! later proposal supersedes it; `omni changeset link-decision` links a change
//! set to it; and `omni wait --until decision:approved` (US-036) ends when the
//! human approves it, and with exit 3 when the human rejects it.

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
use std::time::Duration;

const TEST_DB: &str = "omni_us037_cli";
const KEY_PREFIX: &str = "omniustest37";
/// How long after the wait starts the human acts.
const LATER: Duration = Duration::from_millis(1500);
const MODERATOR: &str = "run-7/moderator";
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
        // Nothing here reaches RustFS: threads, posts and decisions only use
        // Postgres.
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
        let session = format!("us037-cli-session-{}", "0123456789abcdef".repeat(3));
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

/// What a test made from the command line.
struct Forum {
    /// The project's name, for `--project`.
    project: String,
    project_id: String,
    changeset: String,
    thread: String,
}

/// A new project with a change set and an open thread, made by the moderator
/// from the command line.
fn new_forum(home: &Scratch) -> Forum {
    let project = unique("decision-site");
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
    let changeset: Value = out.json()["changeset"].clone();
    let criteria = home.join(&format!("{}.txt", unique("criteria")));
    std::fs::write(&criteria, "Readable at 320 px\nFits above the fold\n")
        .expect("write the criteria file");
    let out = run_as(
        home,
        MODERATOR,
        &[
            "thread",
            "create",
            "--project",
            &project,
            "--changeset",
            changeset["id"].as_str().expect("the change set's id"),
            "--question",
            "Which hero layout?",
            "--criteria-file",
            criteria.to_str().expect("a UTF-8 path"),
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "thread create: {}", out.stderr);
    Forum {
        project,
        project_id: changeset["project_id"]
            .as_str()
            .expect("the change set's project")
            .to_string(),
        changeset: changeset["id"].as_str().expect("an id").to_string(),
        thread: out.json()["thread"]["id"]
            .as_str()
            .expect("the thread's id")
            .to_string(),
    }
}

/// A critic posts a position; returns the post's ID.
fn post(home: &Scratch, forum: &Forum) -> String {
    let file = home.join(&format!("{}.md", unique("body")));
    std::fs::write(&file, "The split hero reads best at 320 px.").expect("write the body");
    let out = run_as(
        home,
        "run-7/critic-a",
        &[
            "post",
            &forum.thread,
            "--project",
            &forum.project,
            "--type",
            "position",
            "--summary",
            "Pick the split hero",
            "--body-file",
            file.to_str().expect("a UTF-8 path"),
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "the critic posts: {}", out.stderr);
    out.json()["post"]["id"]
        .as_str()
        .expect("the post's id")
        .to_string()
}

/// A decision file relying on `posts`, with a unique direction.
fn decision_file(posts: &[&str]) -> (Value, String) {
    let direction = unique("Use the split hero");
    let file = json!({
        "direction": direction,
        "chosen": "Split hero",
        "criteria": ["Readable at 320 px", "Fits above the fold"],
        "options": [
            {
                "name": "Split hero",
                "scores": { "Readable at 320 px": 8, "Fits above the fold": 9 },
                "lost_because": null
            },
            {
                "name": "Carousel",
                "scores": { "Readable at 320 px": 4, "Fits above the fold": 6 },
                "lost_because": "It hides two of the three messages."
            }
        ],
        "assumptions": ["Most visitors arrive on phones."],
        "open_questions": [],
        "dissent": [],
        "posts": posts
    });
    (file, direction)
}

/// The moderator proposes a decision from a file; returns (its ID, its
/// direction).
fn propose(home: &Scratch, forum: &Forum, posts: &[&str]) -> (String, String) {
    let (decision, direction) = decision_file(posts);
    let path = home.join(&format!("{}.json", unique("decision")));
    std::fs::write(&path, decision.to_string()).expect("write the decision file");
    let out = run_as(
        home,
        MODERATOR,
        &[
            "decision",
            "propose",
            &forum.thread,
            "--file",
            path.to_str().expect("a UTF-8 path"),
            "--project",
            &forum.project,
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "decision propose: {}", out.stderr);
    let answer = out.json();
    assert_eq!(answer["decision"]["status"], "proposed", "{answer}");
    assert_eq!(
        answer["decision"]["direction"],
        direction.as_str(),
        "{answer}"
    );
    (
        answer["decision"]["id"]
            .as_str()
            .expect("the decision's id")
            .to_string(),
        direction,
    )
}

fn show(home: &Scratch, forum: &Forum, id: &str) -> String {
    let out = run_as(
        home,
        MODERATOR,
        &["decision", "show", id, "--project", &forum.project],
    );
    assert_eq!(out.code, 0, "decision show {id}: {}", out.stderr);
    out.stdout
}

/// The human's step on a decision (`approve` or `reject`), as a plain HTTP
/// request with alice's session cookie; returns the status and the body.
fn human_acts(
    f: &Fixture,
    forum: &Forum,
    decision: &str,
    action: &str,
    body: &Value,
) -> (u16, String) {
    let authority = f.url.trim_start_matches("http://");
    let path = format!(
        "/api/v1/w/{}/projects/{}/decisions/{decision}/{action}",
        f.workspace, forum.project_id
    );
    let body = body.to_string();
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
    let status = answer
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

fn assert_contains_all(text: &str, needles: &[&str], what: &str) {
    for needle in needles {
        assert!(
            text.contains(needle),
            "{what} should contain {needle:?}: {text:?}"
        );
    }
}

#[test]
fn a_proposed_decision_is_shown_by_thread_and_by_id_with_its_status_and_reason() {
    let f = fixture();
    let home = agent_home(f);
    let forum = new_forum(&home);
    let relied_on = post(&home, &forum);
    let (first, direction) = propose(&home, &forum, &[&relied_on]);

    let by_thread = show(&home, &forum, &forum.thread);
    assert_contains_all(
        &by_thread,
        &[&first, "proposed", &direction, "Split hero", "Carousel"],
        "the thread's decision",
    );

    let reason = unique("The fold sits lower on the phones we sell to");
    let (status, body) = human_acts(f, &forum, &first, "reject", &json!({ "reason": reason }));
    assert_eq!(status, 200, "the human rejects: {body}");
    let by_id = show(&home, &forum, &first);
    assert_contains_all(
        &by_id,
        &[&first, "rejected", &reason, &direction],
        "the rejected decision",
    );

    // A new proposal supersedes the rejected one and is the thread's
    // decision now; the old one keeps its reason.
    let (second, second_direction) = propose(&home, &forum, &[&relied_on]);
    assert_contains_all(
        &show(&home, &forum, &first),
        &[&first, "superseded", &reason],
        "the superseded decision",
    );
    let by_thread = show(&home, &forum, &forum.thread);
    assert_contains_all(
        &by_thread,
        &[&second, "proposed", &second_direction],
        "the thread's new decision",
    );
    assert!(
        !by_thread.contains(&first),
        "the thread shows only its current decision: {by_thread:?}"
    );
}

#[test]
fn a_linked_change_set_and_a_decision_wait_follow_the_humans_approval() {
    let f = fixture();
    let home = agent_home(f);
    let forum = new_forum(&home);
    let (decision, _) = propose(&home, &forum, &[]);

    let out = run_as(
        &home,
        MODERATOR,
        &[
            "changeset",
            "link-decision",
            &forum.changeset,
            &decision,
            "--project",
            &forum.project,
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "changeset link-decision: {}", out.stderr);
    let answer = out.json();
    assert_eq!(
        answer["changeset"]["decisions"],
        json!([{ "id": decision, "thread_id": forum.thread, "status": "proposed" }]),
        "{answer}"
    );

    let (human_forum, human_decision) = (
        Forum {
            project: forum.project.clone(),
            project_id: forum.project_id.clone(),
            changeset: forum.changeset.clone(),
            thread: forum.thread.clone(),
        },
        decision.clone(),
    );
    let human = std::thread::spawn(move || {
        std::thread::sleep(LATER);
        human_acts(
            fixture(),
            &human_forum,
            &human_decision,
            "approve",
            &json!({}),
        )
    });
    let waited = run_as(
        &home,
        MODERATOR,
        &[
            "wait",
            "--thread",
            &forum.thread,
            "--until",
            "decision:approved",
            "--timeout",
            "30",
            "--project",
            &forum.project,
        ],
    );
    let (status, body) = human.join().expect("the human approves");
    assert_eq!(status, 200, "the human approves: {body}");
    assert_eq!(waited.code, 0, "{} {}", waited.stdout, waited.stderr);

    let out = run_as(
        &home,
        MODERATOR,
        &[
            "changeset",
            "show",
            &forum.changeset,
            "--project",
            &forum.project,
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "changeset show: {}", out.stderr);
    let shown = out.json();
    assert_eq!(
        shown["changeset"]["decisions"][0]["status"], "approved",
        "{shown}"
    );
}

#[test]
fn a_rejection_ends_a_wait_for_approval_with_3() {
    let f = fixture();
    let home = agent_home(f);
    let forum = new_forum(&home);
    let (decision, _) = propose(&home, &forum, &[]);
    let (human_forum, human_decision) = (
        Forum {
            project: forum.project.clone(),
            project_id: forum.project_id.clone(),
            changeset: forum.changeset.clone(),
            thread: forum.thread.clone(),
        },
        decision,
    );
    let human = std::thread::spawn(move || {
        std::thread::sleep(LATER);
        human_acts(
            fixture(),
            &human_forum,
            &human_decision,
            "reject",
            &json!({ "reason": "Not this direction." }),
        )
    });
    let waited = run_as(
        &home,
        MODERATOR,
        &[
            "wait",
            "--thread",
            &forum.thread,
            "--until",
            "decision:approved",
            "--timeout",
            "30",
            "--project",
            &forum.project,
        ],
    );
    let (status, body) = human.join().expect("the human rejects");
    assert_eq!(status, 200, "the human rejects: {body}");
    assert_eq!(waited.code, 3, "{} {}", waited.stdout, waited.stderr);
    assert_eq!(waited.stdout, "");
    let lines = waited.stderr_lines();
    assert_eq!(lines.len(), 1, "{:?}", waited.stderr);
    assert!(lines[0].contains("rejected"), "{:?}", lines[0]);
}
