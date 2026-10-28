//! US-036: `omni thread digest` and `omni post show`, tested as the built
//! binary against a mock HTTP server. None of these tests needs Postgres; the
//! routes they read are those of US-035 (crates/omni-api/tests/sealed_phases.rs),
//! and the round trip against the API is in tests/stack_threads.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni thread digest <thread>` reads `GET /threads/{thread}/digest` and
//!   nothing else: no read of the thread or of single posts, since the digest
//!   is the cheap view. It prints one line per entry of the answer's `posts`,
//!   in the API's order, and nothing else on standard output. Each line holds
//!   the post's type, its author's role (`human` for the human's posts), its
//!   summary and its ID. `--json` prints the API's JSON.
//! - `omni post show <id>` reads `GET /posts/{id}` and prints the one post:
//!   at least its type, its author's role (`human` for the human), its
//!   summary, its ID and its Markdown body. `--json` prints the API's JSON.
//!   It sends nothing but that read.
//! - A thread or post the API does not know (404, which is also the answer for
//!   a post the caller may not see yet) is a one-line error that names it. A
//!   missing `<thread>` or `<id>` is a one-line usage error that names the
//!   command and sends nothing.
//! - Both are agent requests with the agent identity headers, directives in an
//!   answer are printed on standard error as for every command (US-034), and
//!   errors are one line on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const THREAD: &str = "01900000-0000-7000-8000-0000000000e1";
const UNKNOWN_THREAD: &str = "01900000-0000-7000-8000-0000000000e9";
const LEFT: &str = "01900000-0000-7000-8000-0000000000f1";
const FOLD: &str = "01900000-0000-7000-8000-0000000000f2";
const CONTRAST: &str = "01900000-0000-7000-8000-0000000000f3";
const UNKNOWN_POST: &str = "01900000-0000-7000-8000-0000000000f9";
const DIRECTIVE: &str = "01900000-0000-7000-8000-0000000000f7";
const HUMAN_ID: &str = "01900000-0000-7000-8000-0000000000ab";
const DIRECTIVE_SUMMARY: &str = "Keep the hero under 600 px";
const LEFT_BODY: &str = "## Why\n\nThe *left* layout reads better at 320 px.\n";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn digest_path(thread: &str) -> String {
    format!("{}/threads/{thread}/digest", base())
}

fn post_path(post: &str) -> String {
    format!("{}/posts/{post}", base())
}

fn agent(run: &str, role: &str) -> Value {
    json!({ "kind": "agent", "run": run, "role": role, "model": "opus", "harness": "claude-code" })
}

fn human() -> Value {
    json!({ "kind": "human", "user_id": HUMAN_ID })
}

/// The visible posts of `THREAD`: (id, type, author, summary).
fn posts() -> Vec<(&'static str, &'static str, Value, &'static str)> {
    vec![
        (
            LEFT,
            "position",
            agent("run-7", "critic-a"),
            "Pick the left layout",
        ),
        (FOLD, "note", human(), "Mind the fold on small phones"),
        (
            CONTRAST,
            "finding",
            agent("run-8", "scout"),
            "Contrast of the CTA is 3.1:1",
        ),
    ]
}

fn digest_json() -> Value {
    let entries: Vec<Value> = posts()
        .into_iter()
        .map(|(id, kind, author, summary)| {
            json!({
                "id": id,
                "phase": 1,
                "type": kind,
                "author": author,
                "summary": summary,
                "created_at": "2026-10-21T10:01:00.000000Z"
            })
        })
        .collect();
    json!({ "project_id": PROJECT, "thread_id": THREAD, "posts": entries })
}

fn post_json(id: &str) -> Option<Value> {
    let (id, kind, author, summary) = posts().into_iter().find(|p| p.0 == id)?;
    let body = if id == LEFT {
        LEFT_BODY.to_string()
    } else {
        format!("Body of {summary}.")
    };
    Some(json!({
        "id": id,
        "thread_id": THREAD,
        "phase": 1,
        "type": kind,
        "author": author,
        "summary": summary,
        "body": body,
        "body_html": "<p>rendered</p>",
        "links": [{ "kind": "url", "url": "https://example.com/hero-study" }],
        "attachments": [],
        "confidence": 0.8,
        "replaces": null,
        "created_at": "2026-10-21T10:01:00.000000Z"
    }))
}

fn directive_json() -> Value {
    json!({
        "id": DIRECTIVE,
        "thread_id": THREAD,
        "phase": 1,
        "type": "directive",
        "author": human(),
        "summary": DIRECTIVE_SUMMARY,
        "body": "It must fit above the fold.",
        "body_html": "<p>It must fit above the fold.</p>",
        "links": [],
        "attachments": [],
        "confidence": null,
        "replaces": null,
        "created_at": "2026-10-21T10:00:00.000000Z"
    })
}

/// An API that knows `THREAD` and its posts. With `directives`, every answer
/// in the thread carries the directive.
fn mock(directives: bool) -> MockServer {
    MockServer::start(move |request| {
        if request.method == "GET" && request.path == format!("/api/v1/w/{WORKSPACE}/projects") {
            return Reply::json(
                200,
                &json!({ "projects": [{
                    "id": PROJECT,
                    "name": "brand-site",
                    "created_at": "2026-10-15T10:00:00.000000Z"
                }] }),
            );
        }
        if request.method != "GET" {
            return Reply::json(405, &json!({ "error": "method_not_allowed" }));
        }
        let path = request.path.as_str();
        if path == digest_path(THREAD) {
            let mut answer = digest_json();
            if directives {
                answer["directives"] = json!([directive_json()]);
            }
            return Reply::json(200, &answer);
        }
        if let Some(id) = path.strip_prefix(&post_path("")) {
            if let Some(post) = post_json(id) {
                return Reply::json(200, &json!({ "project_id": PROJECT, "post": post }));
            }
        }
        if path == format!("{}/threads/{THREAD}", base()) {
            return Reply::json(
                200,
                &json!({ "thread": {
                    "id": THREAD,
                    "project_id": PROJECT,
                    "changeset_id": CHANGESET,
                    "question": "Which hero layout?",
                    "criteria": [],
                    "run_tag": "run-7",
                    "created_at": "2026-10-21T10:00:00.000000Z",
                    "phases": [],
                    "posts": []
                } }),
            );
        }
        Reply::json(404, &json!({ "error": "not_found" }))
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

/// The requests other than the project lookup.
fn requests(server: &MockServer) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.path != format!("/api/v1/w/{WORKSPACE}/projects"))
        .collect()
}

/// An error as US-015 defines it: a documented non-zero exit code, nothing on
/// standard output and exactly one line on standard error that has every
/// needle in it.
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
    for needle in needles {
        assert!(
            lines[0].contains(needle),
            "the error line should contain {needle:?}: {:?}",
            lines[0]
        );
    }
}

// ---------------------------------------------------------------------------
// omni thread digest
// ---------------------------------------------------------------------------

#[test]
fn digest_prints_one_line_per_visible_post_with_type_role_summary_and_id() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "digest", THREAD]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let lines = non_empty_lines(&out.stdout);
    assert_eq!(
        lines.len(),
        3,
        "one line per post and nothing else: {:?}",
        out.stdout
    );
    for (line, (id, kind, author, summary)) in lines.iter().zip(posts()) {
        let role = author["role"].as_str().unwrap_or("human");
        for (what, needle) in [
            ("type", kind),
            ("author role", role),
            ("summary", summary),
            ("ID", id),
        ] {
            assert!(
                line.contains(needle),
                "the line of post {id} has its {what} {needle:?}: {line:?}"
            );
        }
    }
}

#[test]
fn digest_reads_only_the_digest_as_the_agent() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "digest", THREAD]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests(&server);
    assert!(!sent.is_empty(), "it reads the digest");
    for request in &sent {
        assert_eq!(
            (request.method.as_str(), request.path.as_str()),
            ("GET", digest_path(THREAD).as_str()),
            "the digest is the only read: {sent:?}"
        );
        assert_agent_request(request);
    }
}

#[test]
fn digest_json_is_the_api_answer() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "digest", THREAD, "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let answer = out.json();
    let ids: Vec<&str> = answer["posts"]
        .as_array()
        .unwrap_or_else(|| panic!("the digest's posts: {answer}"))
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert_eq!(ids, vec![LEFT, FOLD, CONTRAST], "{answer}");
}

#[test]
fn digest_prints_directives_on_standard_error_only() {
    let server = mock(true);
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "digest", THREAD]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        non_empty_lines(&out.stdout).len(),
        3,
        "standard output is the digest alone: {:?}",
        out.stdout
    );
    assert!(!out.stdout.contains(DIRECTIVE_SUMMARY), "{:?}", out.stdout);
    assert!(
        out.stderr.contains(DIRECTIVE_SUMMARY),
        "the directive is on standard error: {:?}",
        out.stderr
    );
}

#[test]
fn digest_of_an_unknown_thread_is_one_line_that_names_it() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "digest", UNKNOWN_THREAD]);
    assert_one_line_error(&out, &[UNKNOWN_THREAD]);
    assert!(
        requests(&server)
            .iter()
            .any(|r| r.path == digest_path(UNKNOWN_THREAD)),
        "it asked the API: {:?}",
        server.requests()
    );
}

#[test]
fn digest_without_a_thread_is_a_usage_error_that_sends_nothing() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "digest"]);
    assert_one_line_error(&out, &["digest"]);
    assert!(
        requests(&server).is_empty(),
        "a usage error sends nothing: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni post show
// ---------------------------------------------------------------------------

#[test]
fn post_show_prints_the_whole_post() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run(&home, &["post", "show", LEFT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    for (what, needle) in [
        ("type", "position"),
        ("author role", "critic-a"),
        ("summary", "Pick the left layout"),
        ("ID", LEFT),
        ("body", "The *left* layout reads better at 320 px."),
    ] {
        assert!(
            out.stdout.contains(needle),
            "post show prints the {what} {needle:?}: {:?}",
            out.stdout
        );
    }
    let sent = requests(&server);
    assert!(!sent.is_empty(), "it reads the post");
    for request in &sent {
        assert_eq!(
            (request.method.as_str(), request.path.as_str()),
            ("GET", post_path(LEFT).as_str()),
            "post show reads the post and nothing else: {sent:?}"
        );
        assert_agent_request(request);
    }
}

#[test]
fn post_show_json_is_the_api_answer() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run(&home, &["post", "show", CONTRAST, "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let answer = out.json();
    assert_eq!(answer["post"]["id"], CONTRAST, "{answer}");
    assert_eq!(answer["post"]["type"], "finding", "{answer}");
}

#[test]
fn post_show_of_an_unknown_or_hidden_post_is_one_line_that_names_it() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run(&home, &["post", "show", UNKNOWN_POST]);
    assert_one_line_error(&out, &[UNKNOWN_POST]);
    assert!(
        requests(&server)
            .iter()
            .any(|r| r.method == "GET" && r.path == post_path(UNKNOWN_POST)),
        "it asked the API: {:?}",
        server.requests()
    );
    assert!(
        requests(&server).iter().all(|r| r.method == "GET"),
        "post show only reads: {:?}",
        server.requests()
    );
}

#[test]
fn post_show_without_an_id_is_a_usage_error_that_sends_nothing() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run(&home, &["post", "show"]);
    assert_one_line_error(&out, &["post show"]);
    assert!(
        requests(&server).is_empty(),
        "a usage error sends nothing: {:?}",
        server.requests()
    );
}
