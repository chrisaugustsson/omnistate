//! US-103: `omni doc edit` without a change set, tested as the built `omni`
//! against a mock HTTP server. None of these tests needs Postgres; the real
//! round trip against the API is in tests/stack_docs.rs, and the API's
//! contract in crates/omni-api/tests/live_edits.rs.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace}/projects/{project_id}`):
//!
//! - `omni doc edit <doc>`, without `--changeset`, sends the batch (a JSON
//!   array, from `--body-file <path|->` or standard input) to `POST
//!   /docs/{doc}/ops` as `{"ops": [...]}`, with the agent's key and identity,
//!   and touches no change set. It prints each node with its new revision and
//!   the IDs of new nodes; `--json` prints the API's answer. `--changeset <id>`
//!   still edits that change set's draft.
//! - A conflict (409) is a one-line error that names the conflict, the node
//!   and its current revision; a refused operation (400) is a one-line error
//!   that names the unknown ID. Input that is not a batch is refused before
//!   anything is sent.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const CONFLICT_NODE: &str = "c0nf1ct0";
const UNKNOWN_NODE: &str = "zzzzzzzz";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn ops_path() -> String {
    format!("{}/docs/{DOC}/ops", base())
}

fn edit_answer() -> Value {
    json!({ "results": [
        { "node": "a0000002", "revision": 4, "created": [], "removed": [] },
        { "node": "n3wa1st0", "revision": 1, "created": ["n3wa1st0", "n3w1tem0"], "removed": [] }
    ] })
}

fn batch() -> Value {
    json!([
        { "op": "set", "node": "a0000002", "rev": 3, "properties": { "html": "Changed" } },
        {
            "op": "insert",
            "parent": null,
            "position": { "after": "a0000002" },
            "content": "<ul><li>New</li></ul>"
        }
    ])
}

/// An API that applies batches to `DOC` directly. A batch that mentions
/// `CONFLICT_NODE` is a conflict, one that mentions `UNKNOWN_NODE` is refused.
fn mock() -> MockServer {
    MockServer::start(|request| {
        if request.method != "POST" || request.path != ops_path() {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        let text = String::from_utf8_lossy(&request.body).into_owned();
        if text.contains(CONFLICT_NODE) {
            Reply::json(
                409,
                &json!({ "error": "conflict", "conflict": {
                    "index": 0,
                    "node": CONFLICT_NODE,
                    "expected": 1,
                    "current_revision": 3,
                    "current_html": "<p data-node-id=\"c0nf1ct0\">Changed by someone else</p>"
                } }),
            )
        } else if text.contains(UNKNOWN_NODE) {
            Reply::json(
                400,
                &json!({
                    "error": "invalid_operation",
                    "index": 0,
                    "message": format!("the node `{UNKNOWN_NODE}` is not in the doc")
                }),
            )
        } else {
            Reply::json(200, &edit_answer())
        }
    })
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

fn edit_with(home: &Scratch, extra: &[&str], stdin: &[u8]) -> Out {
    omni(&home.path)
        .args(&["doc", "edit", DOC])
        .args(extra)
        .args(&["--project", PROJECT, "--as", "run-7/author"])
        .stdin(stdin)
        .run()
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
    let line = lines[0].to_lowercase();
    for needle in needles {
        assert!(
            line.contains(&needle.to_lowercase()),
            "the error line should contain {needle:?}: {:?}",
            lines[0]
        );
    }
}

fn body_of(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body))
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

fn non_empty_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| !l.trim().is_empty()).collect()
}

#[test]
fn doc_edit_without_a_change_set_sends_the_batch_to_the_doc() {
    let server = mock();
    let home = logged_in(&server.url);
    let file = home.join("ops.json");
    std::fs::write(&file, serde_json::to_string_pretty(&batch()).unwrap()).unwrap();
    let stdin = batch().to_string();

    let from_file = edit_with(&home, &["--body-file", file.to_str().unwrap()], b"");
    assert_eq!(from_file.code, 0, "a file: {}", from_file.stderr);
    let from_stdin = edit_with(&home, &[], stdin.as_bytes());
    assert_eq!(from_stdin.code, 0, "standard input: {}", from_stdin.stderr);
    let from_dash = edit_with(&home, &["--body-file", "-"], stdin.as_bytes());
    assert_eq!(from_dash.code, 0, "--body-file -: {}", from_dash.stderr);

    let requests = server.requests();
    assert_eq!(requests.len(), 3, "{requests:?}");
    for request in &requests {
        assert_eq!(request.method, "POST", "{request:?}");
        assert_eq!(request.path, ops_path(), "the doc's own ops: {request:?}");
        assert_agent_request(request);
        assert_eq!(
            body_of(request),
            json!({ "ops": batch() }),
            "the batch goes to the API as it is"
        );
    }
    assert!(
        requests.iter().all(|r| !r.path.contains("/changesets")),
        "no change set is involved: {requests:?}"
    );
}

#[test]
fn doc_edit_without_a_change_set_prints_the_new_revisions() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = edit_with(&home, &[], batch().to_string().as_bytes());
    assert_eq!(out.code, 0, "{}", out.stderr);
    let lines = non_empty_lines(&out.stdout);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("a0000002") && l.contains('4')),
        "the set node and its new revision 4 are on one line: {:?}",
        out.stdout
    );
    for id in ["n3wa1st0", "n3w1tem0"] {
        assert!(
            out.stdout.contains(id),
            "the new node {id} is printed: {:?}",
            out.stdout
        );
    }

    let json = edit_with(&home, &["--json"], batch().to_string().as_bytes());
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), edit_answer());
}

#[test]
fn a_conflict_on_a_live_doc_is_one_line_with_the_current_revision() {
    let server = mock();
    let home = logged_in(&server.url);
    let ops =
        json!([{ "op": "set", "node": CONFLICT_NODE, "rev": 1, "properties": { "html": "x" } }]);
    let out = edit_with(&home, &[], ops.to_string().as_bytes());
    assert_one_line_error(&out, &["conflict", CONFLICT_NODE, "revision 3"]);
}

#[test]
fn a_refused_operation_on_a_live_doc_is_one_line_that_names_the_unknown_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let ops = json!([{ "op": "delete", "node": UNKNOWN_NODE, "rev": 1 }]);
    let out = edit_with(&home, &[], ops.to_string().as_bytes());
    assert_one_line_error(&out, &[UNKNOWN_NODE]);
}

#[test]
fn doc_edit_without_a_change_set_refuses_input_that_is_not_a_batch_before_sending_it() {
    let server = mock();
    let home = logged_in(&server.url);
    for input in ["", "not json", "{\"op\": \"delete\"}"] {
        let out = edit_with(&home, &[], input.as_bytes());
        assert_one_line_error(&out, &[]);
    }
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}
