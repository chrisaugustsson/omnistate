//! US-108: `omni thread create`, `list`, `read` and `conclude`, threads at the
//! app level or in a feature with no change set, tested as the built binary
//! against a mock HTTP server. None of these tests needs Postgres; the API's
//! contract is in crates/omni-api/tests/live_threads.rs, and the round trip
//! against the API in tests/stack_threads.rs. `omni post` is in
//! tests/posts.rs and `omni wait --thread` in tests/thread_waits.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found from `--project` or `.omni.toml`):
//!
//! - `omni thread create <question> [--feature <name or id>] [--sealed]`
//!   sends `POST /threads` with `{"question", "sealed"}` (`sealed` is `true`
//!   with `--sealed`, else `false` or left out): a thread at the app level,
//!   with no change set and no criteria. With `--feature` it sends `POST
//!   /features/{feature_id}/threads` instead; a name is looked up in the
//!   project's features (`GET /features`) ignoring case, and a name the
//!   project does not have is a one-line error that names it and starts
//!   nothing. A missing question is a one-line error that sends nothing. It
//!   prints the new thread's ID.
//! - `omni thread list` reads `GET /threads` and prints one line per thread
//!   with its ID and its question.
//! - `omni thread read <id>` reads `GET /threads/{id}` and prints the
//!   question, every post the agent may read (its summary, its body and its
//!   author: the role of an agent, `human` for the human) and the conclusion
//!   when there is one. An unknown thread is a one-line error that names it.
//! - `omni thread conclude <id> <text>` sends `PUT /threads/{id}/conclusion`
//!   with `{"conclusion": <text>}`. A missing ID or text is a one-line error
//!   that sends nothing; an unknown thread is a one-line error that names it.
//! - The API answers `{"thread": Thread}` or `{"threads": [Thread, ...]}` with
//!   the Thread of the API's tests. Every command is an agent request with the
//!   agent identity headers, `--json` prints the API's JSON, and errors are
//!   one line on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHECKOUT: &str = "01900000-0000-7000-8000-0000000000f1";
const SEARCH: &str = "01900000-0000-7000-8000-0000000000f2";
/// The thread that a create answers with.
const NEW_THREAD: &str = "01900000-0000-7000-8000-0000000000e2";
/// A concluded thread in Checkout with an agent's post and the human's.
const THREAD: &str = "01900000-0000-7000-8000-0000000000e1";
const UNKNOWN_THREAD: &str = "01900000-0000-7000-8000-0000000000e9";
const QUESTION: &str = "Which hero layout should the landing page use?";
const OTHER_QUESTION: &str = "Do we need a cart page at all?";
const CONCLUSION: &str = "We take the left layout: it reads better at 320 px.";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn feature_json(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "name": name,
        "description": "",
        "created_at": "2026-10-23T10:00:00.000000Z"
    })
}

fn thread_json(
    id: &str,
    feature: Option<&str>,
    question: &Value,
    sealed: bool,
    conclusion: Value,
) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "scope": if feature.is_some() { "feature" } else { "app" },
        "feature_id": feature,
        "question": question,
        "sealed": sealed,
        "conclusion": conclusion,
        "run_tag": "run-7",
        "created_at": "2026-10-23T10:00:00.000000Z"
    })
}

fn post_json(id: &str, author: Value, summary: &str, body: &str) -> Value {
    json!({
        "id": id,
        "thread_id": THREAD,
        "author": author,
        "summary": summary,
        "body": body,
        "body_html": format!("<p>{body}</p>"),
        "created_at": "2026-10-23T10:01:00.000000Z"
    })
}

/// `THREAD` as a read answers it: two posts and a conclusion.
fn read_json() -> Value {
    let mut thread = thread_json(
        THREAD,
        Some(CHECKOUT),
        &json!(QUESTION),
        true,
        json!(CONCLUSION),
    );
    thread["posts"] = json!([
        post_json(
            "01900000-0000-7000-8000-0000000000a1",
            json!({ "kind": "agent", "run": "run-7", "role": "critic", "model": null, "harness": null }),
            "Pick the left layout",
            "It reads better at 320 px.",
        ),
        post_json(
            "01900000-0000-7000-8000-0000000000a2",
            json!({ "kind": "human", "user_id": "01900000-0000-7000-8000-0000000000ab" }),
            "Left it is",
            "Agreed, ship the left one.",
        ),
    ]);
    json!({ "thread": thread })
}

/// An API with the features "Checkout" and "Search", which starts threads at
/// the app level and in either feature, lists `THREAD` and an app-level
/// thread, reads `THREAD` and concludes it.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let Some(rest) = request.path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        let created = |feature: Option<&str>| {
            let sealed = body["sealed"] == true;
            Reply::json(
                201,
                &json!({ "thread": thread_json(NEW_THREAD, feature, &body["question"], sealed, Value::Null) }),
            )
        };
        match (request.method.as_str(), rest) {
            ("GET", "/features") => Reply::json(
                200,
                &json!({ "features": [
                    feature_json(CHECKOUT, "Checkout"),
                    feature_json(SEARCH, "Search")
                ] }),
            ),
            ("POST", "/threads") => created(None),
            ("POST", r) if r == format!("/features/{CHECKOUT}/threads") => created(Some(CHECKOUT)),
            ("POST", r) if r == format!("/features/{SEARCH}/threads") => created(Some(SEARCH)),
            ("GET", "/threads") => Reply::json(
                200,
                &json!({ "threads": [
                    thread_json(THREAD, Some(CHECKOUT), &json!(QUESTION), true, json!(CONCLUSION)),
                    thread_json(NEW_THREAD, None, &json!(OTHER_QUESTION), false, Value::Null)
                ] }),
            ),
            ("GET", r) if r == format!("/threads/{THREAD}") => Reply::json(200, &read_json()),
            ("PUT", r) if r == format!("/threads/{THREAD}/conclusion") => Reply::json(
                200,
                &json!({ "thread": thread_json(
                    THREAD,
                    Some(CHECKOUT),
                    &json!(QUESTION),
                    true,
                    body["conclusion"].clone()
                ) }),
            ),
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--project", PROJECT, "--as", "run-7/moderator"])
        .run()
}

fn requests_to(server: &MockServer, method: &str, path: &str) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == method && r.path == path)
        .collect()
}

/// The requests that are no read.
fn writes(server: &MockServer) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method != "GET")
        .collect()
}

fn assert_agent_request(request: &Recorded) {
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str()),
        "{request:?}"
    );
    assert_eq!(request.header("x-omni-run"), Some("run-7"), "{request:?}");
    assert_eq!(
        request.header("x-omni-role"),
        Some("moderator"),
        "{request:?}"
    );
}

fn body_of(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body))
}

/// An error as US-015 defines it: a documented non-zero exit code, nothing on
/// standard output and exactly one line on standard error that has every
/// needle in it (ignoring case).
fn assert_one_line_error(out: &Out, needles: &[&str]) {
    assert_ne!(out.code, 0, "expected a failure; stdout: {:?}", out.stdout);
    assert!(
        documented_exit_codes().contains(&out.code),
        "exit code {} is not documented under \"Exit codes\" in omni --help",
        out.code
    );
    assert_eq!(out.stdout, "", "an error prints nothing on standard output");
    let lines = out.stderr_lines();
    assert_eq!(
        lines.len(),
        1,
        "one line on standard error: {:?}",
        out.stderr
    );
    let line = lines[0].to_lowercase();
    for needle in needles {
        assert!(
            line.contains(&needle.to_lowercase()),
            "the error line should contain {needle:?}: {:?}",
            lines[0]
        );
    }
}

/// The body of the one thread the server was asked to start, at `path`.
fn started(server: &MockServer, path: &str) -> Value {
    let sent = writes(server);
    assert_eq!(sent.len(), 1, "one write: {sent:?}");
    assert_eq!(sent[0].method, "POST", "{:?}", sent[0]);
    assert_eq!(sent[0].path, path, "{:?}", sent[0]);
    assert_agent_request(&sent[0]);
    body_of(&sent[0])
}

/// The body has no change set, no criteria and nothing but the question and
/// `sealed`.
fn assert_only_question_and_sealed(body: &Value) {
    let keys: Vec<&str> = body
        .as_object()
        .unwrap_or_else(|| panic!("an object: {body}"))
        .keys()
        .map(String::as_str)
        .filter(|key| *key != "question" && *key != "sealed")
        .collect();
    assert!(
        keys.is_empty(),
        "a thread is started with its question and `sealed` only: {body}"
    );
}

// ---------------------------------------------------------------------------
// omni thread create
// ---------------------------------------------------------------------------

#[test]
fn thread_create_starts_a_thread_at_the_app_level_or_in_a_feature_without_a_change_set() {
    // (extra arguments, path under base(), whether the body says sealed)
    for (extra, path, sealed) in [
        (vec![], "/threads".to_string(), false),
        (
            vec!["--feature", "checkout", "--sealed"],
            format!("/features/{CHECKOUT}/threads"),
            true,
        ),
        (
            vec!["--feature", CHECKOUT, "--sealed"],
            format!("/features/{CHECKOUT}/threads"),
            true,
        ),
    ] {
        let server = mock();
        let home = logged_in(&server.url);
        let mut args = vec!["thread", "create", QUESTION];
        args.extend_from_slice(&extra);
        let out = run(&home, &args);
        assert_eq!(out.code, 0, "{extra:?}: {}", out.stderr);
        let body = started(&server, &format!("{}{path}", base()));
        assert_eq!(body["question"], QUESTION, "{extra:?}: {body}");
        assert_eq!(body["sealed"] == true, sealed, "{extra:?}: {body}");
        assert_only_question_and_sealed(&body);
        assert!(
            out.stdout.contains(NEW_THREAD),
            "thread create prints the new thread's ID: {:?}",
            out.stdout
        );
    }
}

#[test]
fn thread_create_needs_a_question() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        vec!["thread", "create"],
        vec!["thread", "create", "   "],
        vec!["thread", "create", "--sealed"],
    ] {
        let out = run(&home, &args);
        assert_one_line_error(&out, &["question"]);
    }
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni thread list and omni thread read
// ---------------------------------------------------------------------------

#[test]
fn thread_list_prints_one_line_per_thread_with_its_id_and_question() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "list"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    for (id, question) in [(THREAD, QUESTION), (NEW_THREAD, OTHER_QUESTION)] {
        let lines: Vec<&str> = non_empty_lines(&out.stdout)
            .into_iter()
            .filter(|line| line.contains(id))
            .collect();
        assert_eq!(lines.len(), 1, "one line for {id}: {:?}", out.stdout);
        assert!(
            lines[0].contains(question),
            "the line of {id} has its question: {:?}",
            lines[0]
        );
    }
    let reads = requests_to(&server, "GET", &format!("{}/threads", base()));
    assert_eq!(reads.len(), 1, "{:?}", server.requests());
    assert_agent_request(&reads[0]);
    assert!(writes(&server).is_empty(), "{:?}", server.requests());
}

#[test]
fn thread_read_prints_the_question_the_posts_and_the_conclusion() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "read", THREAD]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    for text in [
        QUESTION,
        "Pick the left layout",
        "It reads better at 320 px.",
        "critic",
        "Left it is",
        "Agreed, ship the left one.",
        "human",
        CONCLUSION,
    ] {
        assert!(
            out.stdout.contains(text),
            "thread read prints {text:?}: {:?}",
            out.stdout
        );
    }
    let reads = requests_to(&server, "GET", &format!("{}/threads/{THREAD}", base()));
    assert_eq!(reads.len(), 1, "{:?}", server.requests());
    assert_agent_request(&reads[0]);
    assert!(writes(&server).is_empty(), "{:?}", server.requests());
}

#[test]
fn thread_read_and_conclude_of_an_unknown_thread_are_one_line_that_names_it() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        vec!["thread", "read", UNKNOWN_THREAD],
        vec!["thread", "conclude", UNKNOWN_THREAD, CONCLUSION],
    ] {
        let out = run(&home, &args);
        assert_one_line_error(&out, &[UNKNOWN_THREAD]);
    }
}

// ---------------------------------------------------------------------------
// omni thread conclude
// ---------------------------------------------------------------------------

#[test]
fn thread_conclude_sends_the_conclusion() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "conclude", THREAD, CONCLUSION]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = writes(&server);
    assert_eq!(sent.len(), 1, "one write: {sent:?}");
    assert_eq!(sent[0].method, "PUT", "{:?}", sent[0]);
    assert_eq!(
        sent[0].path,
        format!("{}/threads/{THREAD}/conclusion", base()),
        "{:?}",
        sent[0]
    );
    assert_agent_request(&sent[0]);
    assert_eq!(body_of(&sent[0]), json!({ "conclusion": CONCLUSION }));

    let out = run(&home, &["thread", "conclude", THREAD, CONCLUSION, "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json()["thread"]["conclusion"], CONCLUSION);
}

#[test]
fn thread_conclude_needs_a_thread_and_a_text() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        vec!["thread", "conclude"],
        vec!["thread", "conclude", THREAD],
        vec!["thread", "conclude", THREAD, "  "],
    ] {
        let out = run(&home, &args);
        assert_one_line_error(&out, &[]);
    }
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}
