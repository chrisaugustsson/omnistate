//! US-033: `omni thread create` and `omni thread phase`, tested as the built
//! binary against a mock HTTP server. None of these tests needs Postgres; the
//! API's contract is in crates/omni-api/tests/threads.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni thread create --changeset <id> --question <text> --criteria-file
//!   <path> [--sealed]` reads the criteria file, one criterion per line (lines
//!   that are empty or only whitespace are no criterion), and sends `POST
//!   /changesets/{changeset}/threads` with `{"question", "criteria",
//!   "sealed"}`: `sealed` is `true` with `--sealed`, and `false` or left out
//!   without it. It prints the new thread's ID and whether its first phase is
//!   open or sealed. `--changeset`, `--question` and `--criteria-file` are
//!   required; a missing one, or a criteria file that cannot be read, is a
//!   one-line error (naming the file, for a file) that sends nothing.
//! - `omni thread phase <thread> --open|--sealed` sends `POST
//!   /threads/{thread}/phases` with `{"sealed": false}` or `{"sealed": true}`
//!   and prints the new phase's number and whether it is open or sealed.
//!   Exactly one of `--open` and `--sealed` is required; neither or both is a
//!   one-line error that sends nothing. A thread that the API does not know
//!   (404) is a one-line error that names the thread.
//! - The API answers both with `{"thread": Thread}` (Thread as in the API's
//!   tests). Every command is an agent request with the agent identity
//!   headers, `--json` prints the API's JSON, and errors are one line on
//!   standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const THREAD: &str = "01900000-0000-7000-8000-0000000000e1";
const UNKNOWN_THREAD: &str = "01900000-0000-7000-8000-0000000000e9";
const QUESTION: &str = "Which hero layout should the landing page use?";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn threads_path() -> String {
    format!("{}/changesets/{CHANGESET}/threads", base())
}

fn phases_path(thread: &str) -> String {
    format!("{}/threads/{thread}/phases", base())
}

fn phase_json(number: u64, sealed: bool) -> Value {
    json!({ "number": number, "sealed": sealed, "started_at": "2026-10-21T10:00:00.000000Z" })
}

fn thread_json(criteria: &Value, phases: Vec<Value>) -> Value {
    json!({
        "id": THREAD,
        "project_id": PROJECT,
        "changeset_id": CHANGESET,
        "question": QUESTION,
        "criteria": criteria,
        "run_tag": "run-7",
        "created_at": "2026-10-21T10:00:00.000000Z",
        "phases": phases
    })
}

/// An API that starts threads in `CHANGESET` and phases in `THREAD`, and knows
/// no other thread. Its answers follow what the request asked for.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let path = request.path.as_str();
        let method = request.method.as_str();
        let Some(rest) = path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        match (method, rest) {
            ("POST", r) if r == format!("/changesets/{CHANGESET}/threads") => {
                let sealed = body["sealed"].as_bool().unwrap_or(false);
                Reply::json(
                    201,
                    &json!({ "thread": thread_json(&body["criteria"], vec![phase_json(1, sealed)]) }),
                )
            }
            ("POST", r) if r == format!("/threads/{THREAD}/phases") => {
                match body["sealed"].as_bool() {
                    Some(sealed) => Reply::json(
                        201,
                        &json!({ "thread": thread_json(
                            &json!(["Readable"]),
                            vec![phase_json(1, false), phase_json(2, sealed)],
                        ) }),
                    ),
                    None => Reply::json(400, &json!({ "error": "invalid_request" })),
                }
            }
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

/// A criteria file in `home` with `text`; returns its path.
fn criteria_file(home: &Scratch, text: &str) -> String {
    let path = home.join("criteria.txt");
    std::fs::write(&path, text).expect("write the criteria file");
    path.to_str().expect("a UTF-8 path").to_string()
}

fn requests_to(server: &MockServer, method: &str, path: &str) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == method && r.path == path)
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
// omni thread create
// ---------------------------------------------------------------------------

#[test]
fn thread_create_sends_the_question_and_one_criterion_per_line() {
    let server = mock();
    let home = logged_in(&server.url);
    let criteria = criteria_file(
        &home,
        "Fits the brand's tone\nReadable at 320 px\n\n   \nCosts no more than one sprint\n",
    );
    let out = run(
        &home,
        &[
            "thread",
            "create",
            "--changeset",
            CHANGESET,
            "--question",
            QUESTION,
            "--criteria-file",
            &criteria,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests_to(&server, "POST", &threads_path());
    assert_eq!(sent.len(), 1, "{:?}", server.requests());
    assert_agent_request(&sent[0]);
    let body = body_of(&sent[0]);
    assert_eq!(body["question"], QUESTION, "{body}");
    assert_eq!(
        body["criteria"],
        json!([
            "Fits the brand's tone",
            "Readable at 320 px",
            "Costs no more than one sprint"
        ]),
        "each non-empty line is one criterion, in order: {body}"
    );
    assert!(
        body.get("sealed").is_none_or(|sealed| sealed == false),
        "without --sealed the first phase is open: {body}"
    );
    assert!(
        out.stdout.contains(THREAD),
        "create prints the thread's ID: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("open") && !out.stdout.contains("sealed"),
        "create says that the first phase is open: {:?}",
        out.stdout
    );
}

#[test]
fn thread_create_with_sealed_starts_with_a_sealed_phase() {
    let server = mock();
    let home = logged_in(&server.url);
    let criteria = criteria_file(&home, "Readable\n");
    let out = run(
        &home,
        &[
            "thread",
            "create",
            "--changeset",
            CHANGESET,
            "--question",
            QUESTION,
            "--criteria-file",
            &criteria,
            "--sealed",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests_to(&server, "POST", &threads_path());
    assert_eq!(sent.len(), 1, "{:?}", server.requests());
    let body = body_of(&sent[0]);
    assert_eq!(body["sealed"], true, "{body}");
    assert_eq!(body["criteria"], json!(["Readable"]), "{body}");
    assert!(
        out.stdout.contains(THREAD) && out.stdout.contains("sealed"),
        "create prints the thread's ID and that its first phase is sealed: {:?}",
        out.stdout
    );
}

#[test]
fn thread_create_with_json_prints_the_apis_answer() {
    let server = mock();
    let home = logged_in(&server.url);
    let criteria = criteria_file(&home, "Readable\nCheap\n");
    let out = run(
        &home,
        &[
            "thread",
            "create",
            "--changeset",
            CHANGESET,
            "--question",
            QUESTION,
            "--criteria-file",
            &criteria,
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.json(),
        json!({ "thread": thread_json(&json!(["Readable", "Cheap"]), vec![phase_json(1, false)]) })
    );
}

#[test]
fn thread_create_needs_a_change_set_a_question_and_a_criteria_file() {
    let server = mock();
    let home = logged_in(&server.url);
    let criteria = criteria_file(&home, "Readable\n");
    for args in [
        vec![
            "thread",
            "create",
            "--question",
            QUESTION,
            "--criteria-file",
            &criteria,
        ],
        vec![
            "thread",
            "create",
            "--changeset",
            CHANGESET,
            "--criteria-file",
            &criteria,
        ],
        vec![
            "thread",
            "create",
            "--changeset",
            CHANGESET,
            "--question",
            QUESTION,
        ],
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

#[test]
fn thread_create_with_a_criteria_file_that_cannot_be_read_is_one_line_that_names_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let missing = home.join("no-such-criteria.txt");
    let missing = missing.to_str().expect("a UTF-8 path");
    let out = run(
        &home,
        &[
            "thread",
            "create",
            "--changeset",
            CHANGESET,
            "--question",
            QUESTION,
            "--criteria-file",
            missing,
        ],
    );
    assert_one_line_error(&out, &["no-such-criteria.txt"]);
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni thread phase
// ---------------------------------------------------------------------------

#[test]
fn thread_phase_sealed_starts_a_sealed_phase() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "phase", THREAD, "--sealed"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests_to(&server, "POST", &phases_path(THREAD));
    assert_eq!(sent.len(), 1, "{:?}", server.requests());
    assert_agent_request(&sent[0]);
    assert_eq!(body_of(&sent[0]), json!({ "sealed": true }));
    let lines = non_empty_lines(&out.stdout);
    assert!(
        lines
            .iter()
            .any(|l| l.contains('2') && l.contains("sealed")),
        "phase prints the new phase's number and that it is sealed: {:?}",
        out.stdout
    );
}

#[test]
fn thread_phase_open_starts_an_open_phase() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "phase", THREAD, "--open"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests_to(&server, "POST", &phases_path(THREAD));
    assert_eq!(sent.len(), 1, "{:?}", server.requests());
    assert_agent_request(&sent[0]);
    assert_eq!(body_of(&sent[0]), json!({ "sealed": false }));
    let lines = non_empty_lines(&out.stdout);
    assert!(
        lines.iter().any(|l| l.contains('2') && l.contains("open")),
        "phase prints the new phase's number and that it is open: {:?}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("sealed"),
        "an open phase is not called sealed: {:?}",
        out.stdout
    );
}

#[test]
fn thread_phase_with_json_prints_the_apis_answer() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "phase", THREAD, "--sealed", "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.json(),
        json!({ "thread": thread_json(
            &json!(["Readable"]),
            vec![phase_json(1, false), phase_json(2, true)],
        ) })
    );
}

#[test]
fn thread_phase_needs_exactly_one_of_open_and_sealed() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        vec!["thread", "phase", THREAD],
        vec!["thread", "phase", THREAD, "--open", "--sealed"],
        vec!["thread", "phase", "--sealed"],
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

#[test]
fn thread_phase_of_an_unknown_thread_is_one_line_that_names_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["thread", "phase", UNKNOWN_THREAD, "--open"]);
    assert_one_line_error(&out, &[UNKNOWN_THREAD]);
}

#[test]
fn help_lists_the_thread_commands() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    for command in ["omni thread create", "omni thread phase"] {
        assert!(
            help.stdout.contains(command),
            "omni --help lists `{command}`: {:?}",
            help.stdout
        );
    }
}
