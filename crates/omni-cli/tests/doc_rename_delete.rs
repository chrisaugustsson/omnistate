//! US-110: `omni doc rename` and `omni doc delete`, tested as the built `omni`
//! against a mock HTTP server. None of these tests needs Postgres; the real
//! round trip against the API is in tests/stack_docs.rs, and the API's
//! contract in crates/omni-api/tests/deletes_renames.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`):
//!
//! - `omni doc rename <doc> <title>` sends `PATCH /docs/{doc}` with
//!   `{"title": <title>}` and prints `<id>  <title>` (two spaces) from the
//!   answer `{"doc": Doc}`; `--json` prints the API's answer. Without the doc
//!   or with a missing or blank title it is a one-line error that sends
//!   nothing.
//! - `omni doc delete <doc>` sends `DELETE /docs/{doc}`, which answers 204,
//!   and prints a line with the doc's ID. Without the doc it is a one-line
//!   error that sends nothing.
//! - An API error (a doc that is not there, a refusal) is one line on standard
//!   error with the error's code (US-015). `omni --help` lists both commands.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const GONE: &str = "01900000-0000-7000-8000-0000000000d9";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn doc_json(title: &str) -> Value {
    json!({
        "id": DOC,
        "project_id": PROJECT,
        "kind": "html",
        "pillar": "spec",
        "title": title,
        "created_at": "2026-10-23T10:00:00.000000Z",
        "mainline_version_id": null,
        "feature_id": null,
        "drafts": [],
        "claims": []
    })
}

/// An API with the doc `DOC`, which it renames and deletes; `GONE` is a doc
/// that is not there.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let Some(rest) = request.path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        match (request.method.as_str(), rest) {
            ("PATCH", r) if r == format!("/docs/{DOC}") => {
                let title = body["title"].as_str().unwrap_or("");
                Reply::json(200, &json!({ "doc": doc_json(title) }))
            }
            ("DELETE", r) if r == format!("/docs/{DOC}") => Reply {
                status: 204,
                headers: Vec::new(),
                body: String::new(),
            },
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

fn sent(server: &MockServer, method: &str) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == method)
        .collect()
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

// ---------------------------------------------------------------------------
// omni doc rename
// ---------------------------------------------------------------------------

#[test]
fn doc_rename_sends_the_new_title_and_prints_the_id_and_title() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "rename", DOC, "Checkout plan"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let patches = sent(&server, "PATCH");
    assert_eq!(patches.len(), 1, "{:?}", server.requests());
    assert_eq!(patches[0].path, format!("{}/docs/{DOC}", base()));
    assert_agent_request(&patches[0]);
    assert_eq!(body_of(&patches[0]), json!({ "title": "Checkout plan" }));
    assert_eq!(
        non_empty_lines(&out.stdout),
        [format!("{DOC}  Checkout plan")],
        "rename prints `<id>  <title>`"
    );

    let json = run(&home, &["doc", "rename", DOC, "Pay plan", "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "doc": doc_json("Pay plan") }),
        "--json prints the API's answer"
    );
}

#[test]
fn doc_rename_without_a_doc_or_a_title_is_one_line_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        &["doc", "rename"][..],
        &["doc", "rename", DOC][..],
        &["doc", "rename", DOC, "   "][..],
    ] {
        let out = run(&home, args);
        assert_one_line_error(&out, &["title"]);
    }
    assert!(
        server.requests().is_empty(),
        "nothing is sent: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni doc delete
// ---------------------------------------------------------------------------

#[test]
fn doc_delete_sends_a_delete_and_prints_the_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "delete", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let deletes = sent(&server, "DELETE");
    assert_eq!(deletes.len(), 1, "{:?}", server.requests());
    assert_eq!(deletes[0].path, format!("{}/docs/{DOC}", base()));
    assert_agent_request(&deletes[0]);
    assert!(
        out.stdout.contains(DOC),
        "delete prints the doc's ID: {:?}",
        out.stdout
    );
    assert_eq!(out.stderr, "", "{:?}", out.stderr);
}

#[test]
fn doc_delete_of_a_doc_that_is_not_there_is_one_line_with_the_code() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "delete", GONE]);
    assert_one_line_error(&out, &["not_found"]);
}
