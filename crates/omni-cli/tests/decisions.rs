//! US-037: `omni decision propose`, `omni decision show` and `omni changeset
//! link-decision`, tested as the built binary against a mock HTTP server, and
//! the decision file's schema in docs/formats/decision.md. None of these tests
//! needs Postgres; the API's contract is in crates/omni-api/tests/decisions.rs,
//! and the round trip against the API is in tests/stack_decisions.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni decision propose <thread> --file <decision.json>` reads the file,
//!   which holds one JSON object (the decision file whose schema
//!   docs/formats/decision.md documents), and sends it unchanged as the body
//!   of `POST /threads/{thread}/decisions`. It prints the new decision's ID
//!   and its status; `--json` prints the API's JSON. A missing `<thread>` or
//!   `--file`, and a file that cannot be read or holds no JSON object, is a
//!   one-line error (naming the command, the option or the file) that sends
//!   nothing. An answer of 400 or 404 is a one-line error with the API's
//!   error code.
//! - `omni decision show <thread|id>` takes a thread's ID or a decision's and
//!   only reads: `GET /decisions/{id}` for a decision, `GET /threads/{id}` for
//!   a thread, whose `decision` is its current decision (in whichever order
//!   it tries them). It prints the decision: at least its ID, its status, the
//!   direction, the chosen option, each option's name and why each losing one
//!   lost, the assumptions, the open questions, the dissent and, for a
//!   rejected decision, the rejection reason. `--json` prints `{"decision":
//!   Decision}`. For a thread without a decision it exits 0 and says "no
//!   decision" (`--json`: `{"decision": null}`). An ID that is neither is a
//!   one-line error that names it; a missing ID is a one-line usage error
//!   naming the command, and sends nothing.
//! - `omni changeset link-decision <changeset> <decision>` sends `POST
//!   /changesets/{changeset}/decisions/{decision}` and nothing else that
//!   changes anything. It prints a line naming the decision; `--json` prints
//!   the API's JSON. A missing argument is a one-line usage error naming the
//!   command that sends nothing, and an unknown change set or decision (404)
//!   is a one-line error with `not_found`.
//! - `omni --help` lists the three commands.
//! - Every command is an agent request with the agent identity headers, and
//!   errors are one line on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};
use std::path::PathBuf;

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const THREAD: &str = "01900000-0000-7000-8000-0000000000e1";
/// A thread without a decision.
const QUIET_THREAD: &str = "01900000-0000-7000-8000-0000000000e2";
/// A thread for which the API refuses every proposal with 400.
const STRICT_THREAD: &str = "01900000-0000-7000-8000-0000000000e3";
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-0000000000e9";
/// The current decision of `THREAD`, proposed.
const CURRENT: &str = "01900000-0000-7000-8000-0000000000d1";
/// A rejected decision of `THREAD`, since superseded.
const REJECTED: &str = "01900000-0000-7000-8000-0000000000d2";
/// What the API answers for a new proposal.
const NEW_DECISION: &str = "01900000-0000-7000-8000-0000000000d3";
const POST: &str = "01900000-0000-7000-8000-0000000000f1";
const DIRECTION: &str = "Use the split hero: copy on the left, product shot on the right";
const REJECTION: &str = "The fold sits lower on the phones we sell to";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn propose_path(thread: &str) -> String {
    format!("{}/threads/{thread}/decisions", base())
}

fn link_path(changeset: &str, decision: &str) -> String {
    format!("{}/changesets/{changeset}/decisions/{decision}", base())
}

/// A decision file that keeps to the schema.
fn decision_file() -> Value {
    json!({
        "direction": DIRECTION,
        "chosen": "Split hero",
        "criteria": ["Readable at 320 px", "Fits above the fold"],
        "options": [
            {
                "name": "Split hero",
                "scores": { "Readable at 320 px": 8.5, "Fits above the fold": 9 },
                "lost_because": null
            },
            {
                "name": "Full-bleed photo",
                "scores": { "Readable at 320 px": 5, "Fits above the fold": 7 },
                "lost_because": "Text over the photo fails contrast at 320 px."
            },
            {
                "name": "Carousel",
                "scores": { "Readable at 320 px": 4, "Fits above the fold": 6 },
                "lost_because": "It hides two of the three messages."
            }
        ],
        "assumptions": ["Most visitors arrive on phones."],
        "open_questions": ["Is there a product shot for dark mode?"],
        "dissent": [{ "by": "run-7/scout", "text": "The carousel did better in last year's study." }],
        "posts": [POST]
    })
}

/// A Decision as the API answers it.
fn decision_json(id: &str, status: &str, reason: Option<&str>) -> Value {
    let mut decision = decision_file();
    let extra = json!({
        "id": id,
        "project_id": PROJECT,
        "thread_id": THREAD,
        "changeset_id": CHANGESET,
        "status": status,
        "author": {
            "kind": "agent",
            "run": "run-7",
            "role": "moderator",
            "model": "opus",
            "harness": "claude-code"
        },
        "rejection_reason": reason,
        "created_at": "2026-10-21T10:05:00.000000Z"
    });
    for (key, value) in extra.as_object().expect("an object") {
        decision[key] = value.clone();
    }
    decision
}

fn thread_json(id: &str, decision: Value) -> Value {
    json!({ "thread": {
        "id": id,
        "project_id": PROJECT,
        "changeset_id": CHANGESET,
        "question": "Which hero layout?",
        "criteria": ["Readable at 320 px", "Fits above the fold"],
        "run_tag": "run-7",
        "created_at": "2026-10-21T10:00:00.000000Z",
        "phases": [{
            "number": 1,
            "sealed": false,
            "started_at": "2026-10-21T10:00:00.000000Z",
            "posted_by": []
        }],
        "posts": [],
        "decision": decision
    } })
}

fn changeset_json(decisions: Value) -> Value {
    json!({ "changeset": {
        "id": CHANGESET,
        "project_id": PROJECT,
        "title": "Hero section",
        "description": null,
        "status": "open",
        "run_tag": "run-7",
        "created_at": "2026-10-21T10:00:00.000000Z",
        "history": [{
            "from_status": null,
            "to_status": "open",
            "actor": "agent",
            "reason": null,
            "at": "2026-10-21T10:00:00.000000Z"
        }],
        "approved_versions": [],
        "decisions": decisions
    } })
}

/// An API that knows `THREAD` (whose current decision is `CURRENT`),
/// `QUIET_THREAD` (no decision), `STRICT_THREAD` (refuses proposals), the
/// decisions `CURRENT` and `REJECTED`, and `CHANGESET`.
fn mock() -> MockServer {
    MockServer::start(move |request| {
        let path = request.path.as_str();
        let method = request.method.as_str();
        if method == "GET" && path == format!("/api/v1/w/{WORKSPACE}/projects") {
            return Reply::json(
                200,
                &json!({ "projects": [{
                    "id": PROJECT,
                    "name": "brand-site",
                    "created_at": "2026-10-15T10:00:00.000000Z"
                }] }),
            );
        }
        let Some(rest) = path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        match (method, rest) {
            ("POST", r) if r == format!("/threads/{THREAD}/decisions") => {
                let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
                let mut decision = decision_json(NEW_DECISION, "proposed", None);
                if let Some(fields) = body.as_object() {
                    for (key, value) in fields {
                        decision[key] = value.clone();
                    }
                }
                Reply::json(201, &json!({ "decision": decision }))
            }
            ("POST", r) if r == format!("/threads/{STRICT_THREAD}/decisions") => {
                Reply::json(400, &json!({ "error": "invalid_request" }))
            }
            ("GET", r) if r == format!("/threads/{THREAD}") => Reply::json(
                200,
                &thread_json(THREAD, decision_json(CURRENT, "proposed", None)),
            ),
            ("GET", r) if r == format!("/threads/{QUIET_THREAD}") => {
                Reply::json(200, &thread_json(QUIET_THREAD, Value::Null))
            }
            ("GET", r) if r == format!("/decisions/{CURRENT}") => Reply::json(
                200,
                &json!({ "decision": decision_json(CURRENT, "proposed", None) }),
            ),
            ("GET", r) if r == format!("/decisions/{REJECTED}") => Reply::json(
                200,
                &json!({ "decision": decision_json(REJECTED, "superseded", Some(REJECTION)) }),
            ),
            ("POST", r) if r == format!("/changesets/{CHANGESET}/decisions/{CURRENT}") => {
                Reply::json(
                    200,
                    &changeset_json(json!([{
                        "id": CURRENT,
                        "thread_id": THREAD,
                        "status": "proposed"
                    }])),
                )
            }
            ("GET", r) if r == format!("/changesets/{CHANGESET}") => {
                Reply::json(200, &changeset_json(json!([])))
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

/// A file in `home` with `bytes`; returns its path.
fn file(home: &Scratch, name: &str, bytes: &[u8]) -> String {
    let path = home.join(name);
    std::fs::write(&path, bytes).expect("write a file");
    path.to_str().expect("a UTF-8 path").to_string()
}

/// The requests other than the project lookup.
fn requests(server: &MockServer) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.path != format!("/api/v1/w/{WORKSPACE}/projects"))
        .collect()
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

fn assert_contains_all(text: &str, needles: &[&str], what: &str) {
    for needle in needles {
        assert!(
            text.contains(needle),
            "{what} should contain {needle:?}: {text:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// omni decision propose
// ---------------------------------------------------------------------------

#[test]
fn propose_sends_the_decision_file_unchanged_and_prints_the_new_decision() {
    let server = mock();
    let home = logged_in(&server.url);
    let text = serde_json::to_string_pretty(&decision_file()).expect("JSON");
    let path = file(&home, "decision.json", text.as_bytes());
    let out = run(&home, &["decision", "propose", THREAD, "--file", &path]);
    assert_eq!(out.code, 0, "{}", out.stderr);

    let sent = requests_to(&server, "POST", &propose_path(THREAD));
    assert_eq!(sent.len(), 1, "one proposal: {:?}", server.requests());
    assert_agent_request(&sent[0]);
    assert_eq!(
        body_of(&sent[0]),
        decision_file(),
        "the body is the decision file as it is"
    );
    assert_contains_all(
        &out.stdout,
        &[NEW_DECISION, "proposed"],
        "the output of propose",
    );
}

#[test]
fn propose_with_json_prints_the_apis_answer() {
    let server = mock();
    let home = logged_in(&server.url);
    let text = decision_file().to_string();
    let path = file(&home, "decision.json", text.as_bytes());
    let out = run(
        &home,
        &["decision", "propose", THREAD, "--file", &path, "--json"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let answer = out.json();
    assert_eq!(answer["decision"]["id"], NEW_DECISION, "{answer}");
    assert_eq!(answer["decision"]["status"], "proposed", "{answer}");
    assert_eq!(answer["decision"]["direction"], DIRECTION, "{answer}");
}

#[test]
fn propose_needs_a_thread_and_a_file() {
    let server = mock();
    let home = logged_in(&server.url);
    let path = file(
        &home,
        "decision.json",
        decision_file().to_string().as_bytes(),
    );

    let out = run(&home, &["decision", "propose", THREAD]);
    assert_one_line_error(&out, &["--file"]);
    let out = run(&home, &["decision", "propose", "--file", &path]);
    assert_one_line_error(&out, &["decision propose"]);
    assert!(
        requests(&server).is_empty(),
        "a usage error sends nothing: {:?}",
        server.requests()
    );
}

#[test]
fn propose_with_a_file_it_cannot_use_is_one_line_that_names_it_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let missing = home.join("missing.json");
    let missing = missing.to_str().expect("a UTF-8 path").to_string();
    let not_json = file(&home, "notes.json", b"direction: split hero\n");
    let not_object = file(&home, "list.json", b"[\"split hero\"]");
    for path in [&missing, &not_json, &not_object] {
        let out = run(&home, &["decision", "propose", THREAD, "--file", path]);
        assert_one_line_error(&out, &[path.as_str()]);
    }
    assert!(
        requests(&server).is_empty(),
        "nothing is sent: {:?}",
        server.requests()
    );
}

#[test]
fn propose_that_the_api_refuses_is_one_line_with_its_error_code() {
    let server = mock();
    let home = logged_in(&server.url);
    let path = file(
        &home,
        "decision.json",
        decision_file().to_string().as_bytes(),
    );
    let out = run(
        &home,
        &["decision", "propose", STRICT_THREAD, "--file", &path],
    );
    assert_one_line_error(&out, &["invalid_request"]);
    let out = run(&home, &["decision", "propose", UNKNOWN_ID, "--file", &path]);
    assert_one_line_error(&out, &["not_found"]);
}

// ---------------------------------------------------------------------------
// omni decision show
// ---------------------------------------------------------------------------

/// What the text of any shown decision of `decision_file()` holds.
fn decision_needles() -> Vec<&'static str> {
    vec![
        DIRECTION,
        "Split hero",
        "Full-bleed photo",
        "Carousel",
        "Text over the photo fails contrast at 320 px.",
        "It hides two of the three messages.",
        "Most visitors arrive on phones.",
        "Is there a product shot for dark mode?",
        "The carousel did better in last year's study.",
    ]
}

/// The requests of `server` other than the project lookup are all reads.
fn assert_only_reads(server: &MockServer) {
    let sent = requests(server);
    assert!(!sent.is_empty(), "it reads");
    for request in &sent {
        assert_eq!(request.method, "GET", "show only reads: {request:?}");
        assert_agent_request(request);
    }
}

#[test]
fn show_by_thread_prints_the_threads_current_decision() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["decision", "show", THREAD]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let mut needles = decision_needles();
    needles.extend([CURRENT, "proposed"]);
    assert_contains_all(&out.stdout, &needles, "the shown decision");
    assert_only_reads(&server);
}

#[test]
fn show_by_id_prints_that_decision_with_its_status_and_rejection_reason() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["decision", "show", REJECTED]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let mut needles = decision_needles();
    needles.extend([REJECTED, "superseded", REJECTION]);
    assert_contains_all(&out.stdout, &needles, "the shown decision");
    assert_only_reads(&server);
}

#[test]
fn show_without_a_rejection_prints_no_reason() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["decision", "show", CURRENT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_contains_all(&out.stdout, &[CURRENT, "proposed", DIRECTION], "the output");
    assert!(
        !out.stdout.contains(REJECTION),
        "a decision that was not rejected has no reason: {:?}",
        out.stdout
    );
}

#[test]
fn show_with_json_prints_the_decision() {
    let server = mock();
    let home = logged_in(&server.url);
    for (id, expected) in [(THREAD, CURRENT), (REJECTED, REJECTED)] {
        let out = run(&home, &["decision", "show", id, "--json"]);
        assert_eq!(out.code, 0, "{}", out.stderr);
        let answer = out.json();
        assert_eq!(answer["decision"]["id"], expected, "{answer}");
        assert_eq!(answer["decision"]["direction"], DIRECTION, "{answer}");
    }
    let out = run(&home, &["decision", "show", REJECTED, "--json"]);
    assert_eq!(out.json()["decision"]["rejection_reason"], REJECTION);
}

#[test]
fn show_for_a_thread_without_a_decision_says_so() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["decision", "show", QUIET_THREAD]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.to_lowercase().contains("no decision"),
        "it says the thread has no decision: {:?}",
        out.stdout
    );
    let out = run(&home, &["decision", "show", QUIET_THREAD, "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json(), json!({ "decision": null }));
}

#[test]
fn show_of_an_unknown_id_is_one_line_that_names_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["decision", "show", UNKNOWN_ID]);
    assert_one_line_error(&out, &[UNKNOWN_ID]);
}

#[test]
fn show_needs_an_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["decision", "show"]);
    assert_one_line_error(&out, &["decision show"]);
    assert!(
        requests(&server).is_empty(),
        "a usage error sends nothing: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni changeset link-decision
// ---------------------------------------------------------------------------

#[test]
fn link_decision_links_the_change_set_to_the_decision() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["changeset", "link-decision", CHANGESET, CURRENT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests_to(&server, "POST", &link_path(CHANGESET, CURRENT));
    assert_eq!(sent.len(), 1, "one link: {:?}", server.requests());
    assert_agent_request(&sent[0]);
    let others: Vec<Recorded> = requests(&server)
        .into_iter()
        .filter(|r| r.method != "GET" && r.path != link_path(CHANGESET, CURRENT))
        .collect();
    assert!(others.is_empty(), "it changes nothing else: {others:?}");
    assert!(
        out.stdout.contains(CURRENT),
        "the output names the decision: {:?}",
        out.stdout
    );
}

#[test]
fn link_decision_with_json_prints_the_apis_answer() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["changeset", "link-decision", CHANGESET, CURRENT, "--json"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let answer = out.json();
    assert_eq!(answer["changeset"]["id"], CHANGESET, "{answer}");
    assert_eq!(
        answer["changeset"]["decisions"][0]["id"], CURRENT,
        "{answer}"
    );
}

#[test]
fn link_decision_needs_a_change_set_and_a_decision() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        vec!["changeset", "link-decision"],
        vec!["changeset", "link-decision", CHANGESET],
    ] {
        let out = run(&home, &args);
        assert_one_line_error(&out, &["link-decision"]);
    }
    assert!(
        requests(&server).is_empty(),
        "a usage error sends nothing: {:?}",
        server.requests()
    );
}

#[test]
fn link_decision_to_an_unknown_decision_is_one_line_with_not_found() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["changeset", "link-decision", CHANGESET, UNKNOWN_ID],
    );
    assert_one_line_error(&out, &["not_found"]);
    assert_eq!(
        requests_to(&server, "POST", &link_path(CHANGESET, UNKNOWN_ID)).len(),
        1,
        "it asked the API: {:?}",
        server.requests()
    );
}

#[test]
fn help_lists_the_decision_commands() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    assert_contains_all(
        &help.stdout,
        &[
            "omni decision propose",
            "omni decision show",
            "omni changeset link-decision",
        ],
        "omni --help",
    );
}

// ---------------------------------------------------------------------------
// docs/formats/decision.md
// ---------------------------------------------------------------------------

fn decision_doc() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/formats/decision.md");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("docs/formats/decision.md documents the schema ({e})"))
}

#[test]
fn the_format_doc_names_every_field_and_status() {
    let doc = decision_doc();
    for name in [
        "direction",
        "chosen",
        "criteria",
        "options",
        "name",
        "scores",
        "lost_because",
        "assumptions",
        "open_questions",
        "dissent",
        "by",
        "text",
        "posts",
        "proposed",
        "approved",
        "rejected",
        "superseded",
        "rejection_reason",
    ] {
        assert!(
            doc.contains(&format!("`{name}`")),
            "docs/formats/decision.md names `{name}`"
        );
    }
    assert!(
        doc.contains("omni decision propose"),
        "docs/formats/decision.md says how a decision is proposed"
    );
}

#[test]
fn the_format_doc_has_an_example_decision_file_with_every_field() {
    let doc = decision_doc();
    let example = doc
        .split("```json")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .unwrap_or_else(|| panic!("docs/formats/decision.md has a ```json example"));
    let example: Value = serde_json::from_str(example)
        .unwrap_or_else(|e| panic!("the example of docs/formats/decision.md is JSON: {e}"));
    let mut keys: Vec<&str> = example
        .as_object()
        .unwrap_or_else(|| panic!("the example is a JSON object: {example}"))
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    let mut expected = vec![
        "direction",
        "chosen",
        "criteria",
        "options",
        "assumptions",
        "open_questions",
        "dissent",
        "posts",
    ];
    expected.sort();
    assert_eq!(keys, expected, "the example has every field and no other");
    let options = example["options"].as_array().expect("a list of options");
    assert!(
        options.len() >= 2,
        "the example shows an option that lost: {example}"
    );
    assert!(
        options.iter().any(|o| o["name"] == example["chosen"]),
        "the chosen option is among the options: {example}"
    );
    for option in options {
        assert!(
            option["scores"].is_object(),
            "every option has its scores: {option}"
        );
    }
}
