//! US-104: `omni design edit` and `omni tokens import`, tested as the built
//! `omni` against a mock HTTP server. None of these tests needs Postgres; the
//! API's contract is in crates/omni-api/tests/live_designs.rs.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace}/projects/{project_id}`):
//!
//! - `omni design edit <doc>` (or `--doc <doc>`) sends the batch (a JSON array,
//!   from `--body-file <path|->` or standard input) to `POST /docs/{doc}/ops` as
//!   `{"ops": [...]}`, with `"board"` added for `--board <id>`, as the agent. It
//!   prints each node with its new revision and the IDs of new nodes; `--json`
//!   prints the API's answer. A conflict (409) is a one-line error that names
//!   the conflict, the node and its current revision.
//! - `omni tokens import <file>` sends `PUT /tokens` with `{"tokens": <the
//!   file's JSON>}`, which replaces the token tree of the design system's head.
//!   It prints a short plain-text line or two with the modes; `--json` prints
//!   the API's answer. A refusal (400 `invalid_tokens`) is a one-line error that
//!   carries the API's message.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const DESIGN_SYSTEM: &str = "01900000-0000-7000-8000-0000000000d2";
const PAGE: &str = "page0001";
const BOARD: &str = "brd00001";
const TOTAL: &str = "txt00001";
const NEW_BOARD: &str = "n3wb0ard";
const NEW_TEXT: &str = "n3wtext0";
/// A token name that the mock API refuses.
const BROKEN: &str = "broken";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn ops_path() -> String {
    format!("{}/docs/{DOC}/ops", base())
}

fn tokens_path() -> String {
    format!("{}/tokens", base())
}

fn batch() -> Value {
    json!([
        { "op": "set", "node": TOTAL, "rev": 3, "properties": { "content": "Summa 12 kr" } },
        {
            "op": "insert",
            "parent": PAGE,
            "position": { "after": BOARD },
            "content": "<Board name=\"Receipt\" x={800} y={0} width={360} height={640}>\n  <Text name=\"Thanks\" content=\"Tack\" fontFamily=\"Inter\" fontSize={14}/>\n</Board>"
        }
    ])
}

fn edit_answer() -> Value {
    json!({ "results": [
        { "node": TOTAL, "revision": 4, "created": [], "removed": [] },
        { "node": NEW_BOARD, "revision": 1, "created": [NEW_BOARD, NEW_TEXT], "removed": [] }
    ]})
}

fn tree() -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "$type": "color",
            "brand": {
                "$value": "#2563eb",
                "$extensions": {"omnistate": {"modes": {"light": "#2563eb", "dark": "#0f172a"}}}
            }
        }
    })
}

/// An API with a design `DOC` and a design system `DESIGN_SYSTEM` that it
/// writes directly. A token tree with a `color.broken` token is refused.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", path) if path == ops_path() => Reply::json(200, &edit_answer()),
            ("PUT", path) if path == tokens_path() => {
                if body["tokens"]["color"].get(BROKEN).is_some() {
                    Reply::json(
                        400,
                        &json!({
                            "error": "invalid_tokens",
                            "message": "color.broken refers to color.missing, which is no token"
                        }),
                    )
                } else if body["tokens"].is_object() {
                    Reply::json(
                        200,
                        &json!({
                            "project_id": PROJECT,
                            "doc_id": DESIGN_SYSTEM,
                            "modes": body["tokens"]["$extensions"]["omnistate"]["modes"],
                            "tokens": body["tokens"]
                        }),
                    )
                } else {
                    Reply::json(400, &json!({ "error": "invalid_request" }))
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

/// `omni design edit` with `args`, as run-31/designer, with `stdin`.
fn design_edit(home: &Scratch, args: &[&str], stdin: &str) -> Out {
    omni(&home.path)
        .args(&["design", "edit"])
        .args(args)
        .args(&["--project", PROJECT, "--as", "run-31/designer"])
        .stdin(stdin.as_bytes())
        .run()
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--project", PROJECT, "--as", "run-31/designer"])
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

fn assert_agent_request(request: &Recorded) {
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str()),
        "{request:?}"
    );
    assert_eq!(request.header("x-omni-run"), Some("run-31"), "{request:?}");
    assert_eq!(
        request.header("x-omni-role"),
        Some("designer"),
        "{request:?}"
    );
}

fn body_of(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body))
}

fn write_file(home: &Scratch, name: &str, text: &str) -> String {
    let path = home.join(name);
    std::fs::write(&path, text).expect("write the file");
    path.display().to_string()
}

// ---------------------------------------------------------------------------
// omni design edit
// ---------------------------------------------------------------------------

#[test]
fn design_edit_sends_the_batch_to_the_designs_head() {
    let server = mock();
    let home = logged_in(&server.url);
    let file = write_file(
        &home,
        "ops.json",
        &serde_json::to_string_pretty(&batch()).unwrap(),
    );
    let stdin = batch().to_string();

    for (args, input) in [
        (vec![DOC, "--body-file", file.as_str()], ""),
        (vec![DOC], stdin.as_str()),
        (vec![DOC, "--body-file", "-"], stdin.as_str()),
        (vec!["--doc", DOC], stdin.as_str()),
    ] {
        let out = design_edit(&home, &args, input);
        assert_eq!(out.code, 0, "{args:?}: {}", out.stderr);
    }

    let requests = server.requests();
    assert_eq!(requests.len(), 4, "{requests:?}");
    for request in &requests {
        assert_eq!(request.method, "POST", "{request:?}");
        assert_eq!(
            request.path,
            ops_path(),
            "the design's own ops: {request:?}"
        );
        assert_agent_request(request);
        assert_eq!(
            body_of(request),
            json!({ "ops": batch() }),
            "the batch goes to the API as it is, with no board"
        );
    }
}

#[test]
fn design_edit_sends_the_board_along() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = design_edit(&home, &[DOC, "--board", BOARD], &batch().to_string());
    assert_eq!(out.code, 0, "{}", out.stderr);
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].path, ops_path(), "{requests:?}");
    assert_eq!(
        body_of(&requests[0]),
        json!({ "ops": batch(), "board": BOARD })
    );
}

#[test]
fn design_edit_prints_the_new_revisions() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = design_edit(&home, &[DOC], &batch().to_string());
    assert_eq!(out.code, 0, "{}", out.stderr);
    let lines = non_empty_lines(&out.stdout);
    assert!(
        lines.iter().any(|l| l.contains(TOTAL) && l.contains('4')),
        "the set node and its new revision 4 are on one line: {:?}",
        out.stdout
    );
    for id in [NEW_BOARD, NEW_TEXT] {
        assert!(
            out.stdout.contains(id),
            "the new node {id} is printed: {:?}",
            out.stdout
        );
    }

    let json = design_edit(&home, &[DOC, "--json"], &batch().to_string());
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), edit_answer());
}

// ---------------------------------------------------------------------------
// omni tokens import
// ---------------------------------------------------------------------------

#[test]
fn tokens_import_replaces_the_tokens_of_the_design_systems_head() {
    let server = mock();
    let home = logged_in(&server.url);
    let file = write_file(&home, "tokens.json", &tree().to_string());
    let out = run(&home, &["tokens", "import", &file]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].method, "PUT", "{requests:?}");
    assert_eq!(requests[0].path, tokens_path(), "{requests:?}");
    assert_agent_request(&requests[0]);
    assert_eq!(
        body_of(&requests[0]),
        json!({ "tokens": tree() }),
        "the whole file goes in one request"
    );
    assert!(
        out.stdout.contains("light") && out.stdout.contains("dark"),
        "the text names the modes: {:?}",
        out.stdout
    );
    assert!(
        serde_json::from_str::<Value>(&out.stdout).is_err(),
        "the default output is plain text: {:?}",
        out.stdout
    );
    assert!(non_empty_lines(&out.stdout).len() <= 5, "{:?}", out.stdout);

    let json = run(&home, &["tokens", "import", &file, "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    let answer = json.json();
    assert_eq!(answer["doc_id"], DESIGN_SYSTEM, "{answer}");
    assert_eq!(answer["tokens"], tree(), "{answer}");
}

#[test]
fn tokens_import_reports_a_refusal_of_the_api() {
    let server = mock();
    let home = logged_in(&server.url);
    let mut broken = tree();
    broken["color"][BROKEN] = json!({"$value": "{color.missing}"});
    let file = write_file(&home, "broken.tokens.json", &broken.to_string());
    assert_one_line_error(
        &run(&home, &["tokens", "import", &file]),
        &["color.broken refers to color.missing"],
    );
}
