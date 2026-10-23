//! US-108: `omni wait --thread <id> --posts <n>`, tested as the built binary
//! against a mock HTTP server. None of these tests needs Postgres; the API's
//! side (which posts an agent may read in a sealed thread) is in
//! crates/omni-api/tests/live_threads.rs, and the round trip against the API
//! in tests/stack_threads.rs. `omni wait --changeset` (US-025) is in
//! tests/wait.rs and stays as it is.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni wait --thread <id> --posts <n> [--timeout <seconds>] [--project
//!   <p>]` reads the thread with `GET
//!   /api/v1/w/{workspace}/projects/{project_id}/threads/{id}` as the agent,
//!   again at least once a second, until the thread's `posts` (the posts this
//!   agent may read, which is what the API answers it) number at least `n`.
//!   It sends nothing but reads.
//! - Met: exit 0.
//! - `--timeout <seconds>` ran out first: exit 2, nothing on standard output
//!   and one line on standard error.
//! - Usage errors are exit 1, one line on standard error that names the
//!   offending text, and no request at all: a `--posts` that is not a whole
//!   number of at least 1, and a `--timeout` that is not a number. An unknown
//!   thread is exit 1 with `not_found`.
//! - The conditions of the phases and decisions that US-108 removed are not
//!   pinned any more.
//! - `omni --help` lists `omni wait --thread` and `--posts`.

mod common;

use common::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";

/// Two posts; it never changes.
const QUIET: &str = "01900000-0000-7000-8000-0000000000e1";
/// One post; three after `LATER`.
const GROWING: &str = "01900000-0000-7000-8000-0000000000e4";
/// Three posts from the start.
const THREE: &str = "01900000-0000-7000-8000-0000000000e5";
/// No such thread.
const MISSING: &str = "01900000-0000-7000-8000-0000000000e9";

const LATER: Duration = Duration::from_millis(1500);

fn thread_path(id: &str) -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/threads/{id}")
}

fn post_json(thread: &str, number: usize) -> Value {
    json!({
        "id": format!("01900000-0000-7000-8000-0000000001{number:02}"),
        "thread_id": thread,
        "author": { "kind": "agent", "run": "run-7", "role": format!("critic-{number}") },
        "summary": format!("Post {number}"),
        "body": "Why.",
        "body_html": "<p>Why.</p>",
        "created_at": "2026-10-23T10:00:00.000000Z"
    })
}

fn thread_json(id: &str, posts: usize) -> Value {
    json!({ "thread": {
        "id": id,
        "project_id": PROJECT,
        "scope": "app",
        "feature_id": null,
        "question": "Which hero layout?",
        "sealed": true,
        "conclusion": null,
        "run_tag": "run-7",
        "created_at": "2026-10-23T10:00:00.000000Z",
        "posts": (1..=posts).map(|n| post_json(id, n)).collect::<Vec<_>>()
    } })
}

/// An API that knows the threads above in project `PROJECT`.
fn mock() -> MockServer {
    let first_reads: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
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
        let prefix = thread_path("");
        let Some(id) = request.path.strip_prefix(&prefix) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        if request.method != "GET" {
            return Reply::json(405, &json!({ "error": "method_not_allowed" }));
        }
        let later = {
            let mut reads = first_reads.lock().unwrap();
            reads
                .entry(id.to_string())
                .or_insert_with(Instant::now)
                .elapsed()
                >= LATER
        };
        let thread = match id {
            QUIET => thread_json(id, 2),
            GROWING if later => thread_json(id, 3),
            GROWING => thread_json(id, 1),
            THREE => thread_json(id, 3),
            _ => return Reply::json(404, &json!({ "error": "not_found" })),
        };
        Reply::json(200, &thread)
    })
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

fn wait(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(&["wait", "--project", PROJECT])
        .args(args)
        .args(&["--as", "run-7/moderator"])
        .run()
}

/// `omni wait --thread <thread> --posts <posts>` with `extra` options.
fn wait_for(home: &Scratch, thread: &str, posts: &str, extra: &[&str]) -> Out {
    let mut args = vec!["--thread", thread, "--posts", posts];
    args.extend_from_slice(extra);
    wait(home, &args)
}

/// A failure as US-015 defines it, with this exit code: nothing on standard
/// output and exactly one line on standard error that has every needle in it.
fn assert_one_line_failure(out: &Out, code: i32, needles: &[&str]) {
    assert_eq!(
        out.code, code,
        "exit code; stdout: {:?}; stderr: {:?}",
        out.stdout, out.stderr
    );
    assert!(
        documented_exit_codes().contains(&out.code),
        "exit code {} is not documented under \"Exit codes\" in omni --help",
        out.code
    );
    assert_eq!(
        out.stdout, "",
        "a failure prints nothing on standard output"
    );
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

fn reads_of(server: &MockServer, id: &str) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.path == thread_path(id))
        .count()
}

/// The server got reads of the thread only, each as the agent.
fn assert_only_agent_reads(server: &MockServer) {
    let requests = server.requests();
    assert!(!requests.is_empty(), "the wait read the thread");
    for request in &requests {
        assert_eq!(request.method, "GET", "a wait only reads: {request:?}");
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
}

// ---------------------------------------------------------------------------
// omni wait --thread --posts
// ---------------------------------------------------------------------------

#[test]
fn wait_posts_exits_0_once_the_thread_has_that_many_posts() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, GROWING, "3", &["--timeout", "20"]);
    assert_eq!(
        out.code, 0,
        "three posts came; stdout: {:?}; stderr: {:?}",
        out.stdout, out.stderr
    );
    assert!(
        out.elapsed >= LATER - Duration::from_millis(100),
        "it waited for the posts: {:?}",
        out.elapsed
    );
    assert!(
        out.elapsed < LATER + Duration::from_millis(2500),
        "it noticed the posts late: {:?}",
        out.elapsed
    );
    assert!(
        reads_of(&server, GROWING) >= 2,
        "it read the thread again: {:?}",
        server.requests()
    );
    assert_only_agent_reads(&server);
}

#[test]
fn wait_posts_that_are_already_there_exits_0_at_once() {
    let server = mock();
    let home = logged_in(&server.url);
    for posts in ["1", "3"] {
        let out = wait_for(&home, THREE, posts, &["--timeout", "20"]);
        assert_eq!(
            out.code, 0,
            "--posts {posts}; stdout: {:?}; stderr: {:?}",
            out.stdout, out.stderr
        );
        assert!(
            out.elapsed < Duration::from_secs(2),
            "--posts {posts}: it ended at once: {:?}",
            out.elapsed
        );
    }
    assert_only_agent_reads(&server);
}

#[test]
fn wait_posts_exits_2_when_the_timeout_runs_out_first() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, QUIET, "3", &["--timeout", "1"]);
    assert_one_line_failure(&out, 2, &[]);
    assert!(
        out.elapsed >= Duration::from_millis(900),
        "it waited for the timeout: {:?}",
        out.elapsed
    );
    assert!(
        out.elapsed < Duration::from_secs(4),
        "it stopped at the timeout: {:?}",
        out.elapsed
    );
    assert_only_agent_reads(&server);
}

#[test]
fn wait_posts_needs_a_whole_number_of_at_least_1_and_sends_nothing_otherwise() {
    let server = mock();
    let home = logged_in(&server.url);
    for posts in ["0", "-1", "abc", "2.5"] {
        let out = wait_for(&home, THREE, posts, &[]);
        assert_one_line_failure(&out, 1, &[posts]);
    }
    let out = wait_for(&home, THREE, "3", &["--timeout", "soon"]);
    assert_one_line_failure(&out, 1, &["soon"]);
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

#[test]
fn wait_posts_on_an_unknown_thread_is_not_found() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, MISSING, "1", &["--timeout", "5"]);
    assert_one_line_failure(&out, 1, &["not_found"]);
}

#[test]
fn help_lists_the_thread_wait_on_posts() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    for text in ["omni wait --thread", "--posts"] {
        assert!(
            help.stdout.contains(text),
            "omni --help names {text:?}: {:?}",
            help.stdout
        );
    }
}
