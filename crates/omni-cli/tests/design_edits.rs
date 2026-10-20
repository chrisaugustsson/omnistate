//! US-031: `omni design edit`, tested as the built binary against a mock HTTP
//! server. None of these tests needs Postgres; the API's contract is in
//! crates/omni-api/tests/design_edits.rs, and the round trip against the real
//! API, with claims on boards and the time targets, in
//! tests/stack_design_edits.rs and tests/stack_bench_design.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni design edit --changeset <id> --doc <id>` reads a batch of
//!   operations (a JSON array: inserts with content in the tag format,
//!   replaces, sets, moves and deletes, each with its expected `rev`) from
//!   `--body-file <path>`, or standard input without it or with `--body-file
//!   -`, and sends `POST /changesets/{changeset}/docs/{doc}/ops` with
//!   `{"ops": <the batch>}`. With `--board <id>` the body also carries
//!   `"board": "<id>"`, which keeps the batch inside that board.
//! - It prints each result's node and new revision and the IDs that the
//!   server assigned to new nodes; `--json` prints the API's answer.
//! - A missing `--changeset` or `--doc`, a doc or change set that is no UUID,
//!   a board that is no node ID, and input that is not a JSON array are
//!   one-line errors that send nothing.
//! - A refused operation (400 `invalid_operation`) is a one-line error with
//!   the API's message, which names the unknown ID; a claim of another agent
//!   (409 `claimed`) one that names the holder; a board that the doc does not
//!   have (404 `unknown_board`) one that names it.
//! - `omni --help` lists `omni design edit`.
//! - Every command is an agent request with the agent identity headers, and
//!   errors are one line on standard error, as in US-015
//!   (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const PAGE: &str = "page0001";
const BOARD: &str = "brd00001";
const TOTAL: &str = "txt00001";
/// The node that the mock answers is claimed by another agent.
const CLAIMED: &str = "c1a1med0";
/// An ID that no node has.
const UNKNOWN: &str = "zzzzzzzz";
const NEW_BOARD: &str = "n3wb0ard";
const NEW_TEXT: &str = "n3wtext0";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn ops_path() -> String {
    format!("{}/changesets/{CHANGESET}/docs/{DOC}/ops", base())
}

/// A batch with a set and an insert of a new board under a page.
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

/// The node of the first operation of a batch.
fn first_node(body: &Value) -> Option<&str> {
    body["ops"][0]["node"].as_str()
}

fn mock() -> MockServer {
    MockServer::start(move |request| {
        if request.method != "POST" || request.path != ops_path() {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        if body["board"] == UNKNOWN {
            return Reply::json(404, &json!({ "error": "unknown_board" }));
        }
        match first_node(&body) {
            Some(UNKNOWN) => Reply::json(
                400,
                &json!({
                    "error": "invalid_operation",
                    "index": 0,
                    "message": format!("the node `{UNKNOWN}` is not in the draft")
                }),
            ),
            Some(CLAIMED) => Reply::json(
                409,
                &json!({
                    "error": "claimed",
                    "index": 0,
                    "message": "the board brd00001 is claimed by run-9/painter",
                    "claim": {
                        "id": "01900000-0000-7000-8000-0000000000e9",
                        "doc_id": DOC,
                        "draft_id": "01900000-0000-7000-8000-0000000000e1",
                        "changeset_id": CHANGESET,
                        "node": BOARD,
                        "agent": { "run": "run-9", "role": "painter", "model": null, "harness": null },
                        "taken_at": "2026-10-21T10:00:00.000000Z",
                        "expires_at": "2026-10-21T10:10:00.000000Z"
                    }
                }),
            ),
            _ => Reply::json(200, &edit_answer()),
        }
    })
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

/// `omni design edit` with `args`, as run-31/designer, with `stdin`.
fn run(home: &Scratch, args: &[&str], stdin: &str) -> Out {
    omni(&home.path)
        .args(&["design", "edit"])
        .args(args)
        .args(&["--project", PROJECT, "--as", "run-31/designer"])
        .stdin(stdin.as_bytes())
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

fn assert_nothing_sent(server: &MockServer) {
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

/// The batches that reached the operations route, as JSON bodies.
fn sent_bodies(server: &MockServer) -> Vec<Value> {
    server
        .requests()
        .iter()
        .filter(|request| request.method == "POST" && request.path == ops_path())
        .map(|request| {
            assert_agent_request(request);
            serde_json::from_slice(&request.body).unwrap_or_else(|e| {
                panic!(
                    "the body is JSON ({e}): {}",
                    String::from_utf8_lossy(&request.body)
                )
            })
        })
        .collect()
}

const TARGET: [&str; 4] = ["--changeset", CHANGESET, "--doc", DOC];

#[test]
fn design_edit_sends_the_batch_from_a_file_or_standard_input() {
    let server = mock();
    let home = logged_in(&server.url);
    let file = home.join("ops.json");
    std::fs::write(&file, serde_json::to_string_pretty(&batch()).unwrap()).unwrap();
    let from_file = run(
        &home,
        &[&TARGET[..], &["--body-file", file.to_str().unwrap()]].concat(),
        "",
    );
    assert_eq!(from_file.code, 0, "a file: {}", from_file.stderr);
    let from_stdin = run(&home, &TARGET, &batch().to_string());
    assert_eq!(from_stdin.code, 0, "standard input: {}", from_stdin.stderr);
    let from_dash = run(
        &home,
        &[&TARGET[..], &["--body-file", "-"]].concat(),
        &batch().to_string(),
    );
    assert_eq!(from_dash.code, 0, "--body-file -: {}", from_dash.stderr);

    let bodies = sent_bodies(&server);
    assert_eq!(bodies.len(), 3, "{:?}", server.requests());
    for body in bodies {
        assert_eq!(
            body,
            json!({ "ops": batch() }),
            "the batch goes to the API as it is, with no board"
        );
    }
}

#[test]
fn design_edit_with_a_board_sends_the_board_along() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[&TARGET[..], &["--board", BOARD]].concat(),
        &batch().to_string(),
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        sent_bodies(&server),
        vec![json!({ "ops": batch(), "board": BOARD })]
    );
}

#[test]
fn design_edit_prints_the_new_revisions_and_the_ids_of_new_nodes() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &TARGET, &batch().to_string());
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

    let json = run(
        &home,
        &[&TARGET[..], &["--json"]].concat(),
        &batch().to_string(),
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), edit_answer());
}

#[test]
fn design_edit_needs_a_changeset_a_doc_and_a_batch_before_sending_anything() {
    let server = mock();
    let home = logged_in(&server.url);
    let ops = batch().to_string();
    assert_one_line_error(&run(&home, &["--doc", DOC], &ops), &["--changeset"]);
    assert_one_line_error(&run(&home, &["--changeset", CHANGESET], &ops), &["--doc"]);
    assert_one_line_error(
        &run(
            &home,
            &["--changeset", CHANGESET, "--doc", "checkout"],
            &ops,
        ),
        &["checkout"],
    );
    assert_one_line_error(
        &run(&home, &["--changeset", "draft-1", "--doc", DOC], &ops),
        &["draft-1"],
    );
    assert_one_line_error(
        &run(&home, &[&TARGET[..], &["--board", "Cart!"]].concat(), &ops),
        &["Cart!"],
    );
    for input in ["", "not json", "{\"op\": \"delete\"}"] {
        assert_one_line_error(&run(&home, &TARGET, input), &[]);
    }
    assert_nothing_sent(&server);
}

#[test]
fn a_refused_operation_is_one_line_that_names_the_unknown_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let ops = json!([{ "op": "delete", "node": UNKNOWN, "rev": 1 }]);
    let out = run(&home, &TARGET, &ops.to_string());
    assert_one_line_error(&out, &[UNKNOWN]);
    assert_eq!(sent_bodies(&server).len(), 1, "the batch was sent");
}

#[test]
fn an_edit_of_a_board_that_another_agent_claimed_is_one_line_that_names_the_holder() {
    let server = mock();
    let home = logged_in(&server.url);
    let ops = json!([{ "op": "set", "node": CLAIMED, "rev": 1, "properties": { "content": "x" } }]);
    let out = run(&home, &TARGET, &ops.to_string());
    assert_one_line_error(&out, &["claimed", "run-9", "painter"]);
    assert_eq!(sent_bodies(&server).len(), 1, "the batch was sent");
}

#[test]
fn an_edit_inside_a_board_that_the_doc_does_not_have_names_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[&TARGET[..], &["--board", UNKNOWN]].concat(),
        &batch().to_string(),
    );
    assert_one_line_error(&out, &[UNKNOWN]);
    assert_eq!(sent_bodies(&server).len(), 1, "the batch was sent");
}

#[test]
fn help_lists_design_edit() {
    let home = Scratch::new("help");
    let out = omni(&home.path).args(&["--help"]).run();
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains("omni design edit"),
        "omni --help lists `omni design edit`: {}",
        out.stdout
    );
}
