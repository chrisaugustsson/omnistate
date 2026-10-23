//! US-023: `omni claim` and `omni release`, and claims in `omni doc show` and
//! `omni doc edit`, tested as the built binary against a mock HTTP server.
//! None of these tests needs Postgres; the real round trip against the API is
//! in tests/stack_docs.rs, and the API's contract in
//! crates/omni-api/tests/claims.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni claim <node> --changeset <id>` sends `POST
//!   /changesets/{changeset}/claims` with `{"node"}` and prints the claimed
//!   node and when the claim expires. `omni release <node> --changeset <id>`
//!   sends `POST /changesets/{changeset}/claims/release` with `{"node"}` and
//!   prints the released node. A missing node or change set is a one-line
//!   error that sends nothing.
//! - A claim held by another agent (409 `{"error": "claimed", "message",
//!   "claim"}`) is a one-line error that names the holder as `<run>/<role>`
//!   and the claimed node; so is an operation of `omni doc edit` that the API
//!   refuses because of a claim. A node that the change set does not have (404)
//!   is a one-line error that names the node.
//! - `omni doc show` prints each claim on a line of its own with the claimed
//!   node and the holder as `<run>/<role>`, as plain text, not JSON.
//! - Every command is an agent request with the agent identity headers,
//!   `--json` prints the API's JSON, and errors are one line on standard
//!   error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const DRAFT: &str = "01900000-0000-7000-8000-0000000000e2";
const MAINLINE: &str = "01900000-0000-7000-8000-0000000000f1";
const CLAIM: &str = "01900000-0000-7000-8000-0000000000a7";
/// A section that the agent of these tests may claim.
const FREE_NODE: &str = "s3ct10na";
/// A section that another agent (run-2/reviewer) holds.
const HELD_NODE: &str = "s3ct10nb";
const UNKNOWN_NODE: &str = "zzzzzzzz";
const EXPIRES_AT: &str = "2026-10-15T10:10:00.000000Z";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn claim_json(node: &str, run: &str, role: &str) -> Value {
    json!({
        "id": CLAIM,
        "doc_id": DOC,
        "draft_id": DRAFT,
        "changeset_id": CHANGESET,
        "node": node,
        "agent": { "run": run, "role": role, "model": "test-model", "harness": "test-harness" },
        "taken_at": "2026-10-15T10:00:00.000000Z",
        "expires_at": EXPIRES_AT
    })
}

/// The refusal of a claim on `HELD_NODE`; an operation's refusal also has
/// its `index`.
fn held(index: Option<u64>) -> Reply {
    let mut answer = json!({
        "error": "claimed",
        "message": format!("{HELD_NODE} is claimed by run-2/reviewer until {EXPIRES_AT}"),
        "claim": claim_json(HELD_NODE, "run-2", "reviewer")
    });
    if let Some(index) = index {
        answer["index"] = json!(index);
    }
    Reply::json(409, &answer)
}

fn doc_json() -> Value {
    json!({
        "id": DOC,
        "project_id": PROJECT,
        "kind": "html",
        "pillar": "spec",
        "title": "Checkout PRD",
        "created_at": "2026-10-15T10:00:00.000000Z",
        "mainline_version_id": MAINLINE,
        "drafts": [{ "id": DRAFT, "doc_id": DOC, "changeset_id": CHANGESET, "base_version_id": MAINLINE }],
        "claims": [claim_json(HELD_NODE, "run-2", "reviewer")]
    })
}

/// An API where `FREE_NODE` can be claimed and released, `HELD_NODE` is held
/// by run-2/reviewer, and `UNKNOWN_NODE` is in no draft of the change set.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let path = request.path.as_str();
        let method = request.method.as_str();
        let Some(rest) = path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        let claims = format!("/changesets/{CHANGESET}/claims");
        let node = body["node"].as_str().unwrap_or("");
        match (method, rest) {
            ("POST", r) if r == claims || r == format!("{claims}/release") => match node {
                FREE_NODE if r == claims => Reply::json(
                    201,
                    &json!({ "claim": claim_json(FREE_NODE, "run-7", "author") }),
                ),
                FREE_NODE => Reply::json(
                    200,
                    &json!({ "claim": claim_json(FREE_NODE, "run-7", "author") }),
                ),
                HELD_NODE => held(None),
                _ => Reply::json(404, &json!({ "error": "not_found" })),
            },
            ("POST", r) if r == format!("/changesets/{CHANGESET}/docs/{DOC}/ops") => held(Some(0)),
            ("GET", r) if r == format!("/docs/{DOC}") => {
                Reply::json(200, &json!({ "doc": doc_json() }))
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
        .args(&["--project", PROJECT, "--as", "run-7/author"])
        .run()
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
    assert_eq!(request.header("x-omni-role"), Some("author"), "{request:?}");
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

fn claims_path() -> String {
    format!("{}/changesets/{CHANGESET}/claims", base())
}

fn release_path() -> String {
    format!("{}/release", claims_path())
}

// ---------------------------------------------------------------------------
// omni claim
// ---------------------------------------------------------------------------

#[test]
fn claim_sends_the_node_and_prints_the_claim() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["claim", FREE_NODE, "--changeset", CHANGESET]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests_to(&server, "POST", &claims_path());
    assert_eq!(sent.len(), 1, "{:?}", server.requests());
    assert_agent_request(&sent[0]);
    assert_eq!(body_of(&sent[0]), json!({ "node": FREE_NODE }));
    assert!(
        out.stdout.contains(FREE_NODE),
        "claim prints the node: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains(EXPIRES_AT) || out.stdout.contains("10:10"),
        "claim prints when the claim expires: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &["claim", FREE_NODE, "--changeset", CHANGESET, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "claim": claim_json(FREE_NODE, "run-7", "author") })
    );
}

#[test]
fn claim_of_a_node_another_agent_holds_is_one_line_that_names_the_holder() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["claim", HELD_NODE, "--changeset", CHANGESET]);
    assert_one_line_error(&out, &["run-2/reviewer", HELD_NODE]);
}

#[test]
fn claim_of_a_node_the_change_set_does_not_have_is_one_line_that_names_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["claim", UNKNOWN_NODE, "--changeset", CHANGESET]);
    assert_one_line_error(&out, &[UNKNOWN_NODE]);
}

#[test]
fn claim_and_release_need_a_node_and_a_change_set() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        vec!["claim", FREE_NODE],
        vec!["claim", "--changeset", CHANGESET],
        vec!["release", FREE_NODE],
        vec!["release", "--changeset", CHANGESET],
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

// ---------------------------------------------------------------------------
// omni release
// ---------------------------------------------------------------------------

#[test]
fn release_sends_the_node_and_prints_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["release", FREE_NODE, "--changeset", CHANGESET]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests_to(&server, "POST", &release_path());
    assert_eq!(sent.len(), 1, "{:?}", server.requests());
    assert_agent_request(&sent[0]);
    assert_eq!(body_of(&sent[0]), json!({ "node": FREE_NODE }));
    assert!(
        out.stdout.contains(FREE_NODE),
        "release prints the node: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &["release", FREE_NODE, "--changeset", CHANGESET, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "claim": claim_json(FREE_NODE, "run-7", "author") })
    );
}

#[test]
fn release_of_another_agents_claim_is_one_line_that_names_the_holder() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["release", HELD_NODE, "--changeset", CHANGESET]);
    assert_one_line_error(&out, &["run-2/reviewer", HELD_NODE]);
}

#[test]
fn release_of_a_node_without_a_claim_is_one_line_that_names_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["release", UNKNOWN_NODE, "--changeset", CHANGESET]);
    assert_one_line_error(&out, &[UNKNOWN_NODE]);
}

// ---------------------------------------------------------------------------
// Claims in omni doc show and omni doc edit
// ---------------------------------------------------------------------------

#[test]
fn doc_show_prints_each_claim_with_its_node_and_holder() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "show", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let lines = non_empty_lines(&out.stdout);
    let line = lines
        .iter()
        .find(|l| l.contains(HELD_NODE))
        .unwrap_or_else(|| panic!("show prints the claimed node: {:?}", out.stdout));
    assert!(
        line.contains("run-2/reviewer"),
        "the claim's line names its holder: {line:?}"
    );
    assert!(
        !line.contains('{') && !line.contains("\"node\""),
        "the claim is plain text, not JSON: {line:?}"
    );
}

#[test]
fn an_operation_refused_because_of_a_claim_is_one_line_that_names_the_holder() {
    let server = mock();
    let home = logged_in(&server.url);
    let ops = json!([{ "op": "set", "node": HELD_NODE, "rev": 1, "properties": { "html": "x" } }]);
    let out = omni(&home.path)
        .args(&[
            "doc",
            "edit",
            DOC,
            "--changeset",
            CHANGESET,
            "--project",
            PROJECT,
            "--as",
            "run-7/author",
        ])
        .stdin(ops.to_string().as_bytes())
        .run();
    assert_one_line_error(&out, &["run-2/reviewer"]);
}

#[test]
fn help_lists_the_claim_commands() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    for command in ["omni claim", "omni release"] {
        assert!(
            help.stdout.contains(command),
            "omni --help lists `{command}`: {:?}",
            help.stdout
        );
    }
}
