//! US-030: `omni design read`, tested as the built binary against a mock HTTP
//! server. None of these tests needs Postgres; the API's contract is in
//! crates/omni-api/tests/boards.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found from `--project` or `.omni.toml`):
//!
//! - `omni design read --doc <id> --board <id>` prints one board of the doc's
//!   head in the tag format (`GET /docs/{doc}/boards/{board}`). It prints the
//!   answer's `tags` as they are, and nothing else. `--json` prints the API's
//!   answer.
//! - A missing `--doc` or `--board`, a doc that is no UUID, or a board that is
//!   no node ID (8 lowercase base32 characters) is a one-line error that names
//!   the problem and sends nothing. A board that the doc does not have (404
//!   `unknown_board`) is a one-line error that names it.
//! - Every command is an agent request with the agent identity headers, and
//!   errors are one line on standard error, as in US-015
//!   (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const PAGE: &str = "page0001";
const BOARD: &str = "brd00001";
/// A board ID that the doc does not have.
const UNKNOWN: &str = "zzzzzzzz";

const TAGS: &str = r#"<Board id="brd00001" rev={3} name="Cart" x={0} y={0} width={360} height={640} dir="col">
  <Text id="txt00001" rev={2} name="Total" content="Summa 12 kr" fontFamily="Inter" fontSize={16}/>
</Board>"#;

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn board_path(doc: &str, board: &str) -> String {
    format!("{}/docs/{doc}/boards/{board}", base())
}

fn board_answer() -> Value {
    json!({
        "doc_id": DOC,
        "page_id": PAGE,
        "board_id": BOARD,
        "tags": TAGS
    })
}

fn mock() -> MockServer {
    MockServer::start(move |request| {
        if request.method != "GET" {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        let path = request.path.as_str();
        if path == board_path(DOC, BOARD) {
            Reply::json(200, &board_answer())
        } else if path == board_path(DOC, UNKNOWN) {
            Reply::json(404, &json!({ "error": "unknown_board" }))
        } else {
            Reply::json(404, &json!({ "error": "not_found" }))
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
        .args(&["--project", PROJECT, "--as", "run-30/designer"])
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
    assert_eq!(request.header("x-omni-run"), Some("run-30"), "{request:?}");
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

#[test]
fn design_read_prints_the_board_of_the_head() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["design", "read", "--doc", DOC, "--board", BOARD]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, board_path(DOC, BOARD), "{requests:?}");
    assert_agent_request(&requests[0]);
    assert_eq!(
        out.stdout.trim_end(),
        TAGS,
        "the board's tags as the API gave them, and nothing else"
    );
}

#[test]
fn design_read_prints_the_answer_with_json() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["design", "read", "--doc", DOC, "--board", BOARD, "--json"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json(), board_answer());
}

#[test]
fn design_read_needs_a_doc_and_a_board_before_sending_anything() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(
        &run(&home, &["design", "read", "--board", BOARD]),
        &["--doc"],
    );
    assert_one_line_error(&run(&home, &["design", "read", "--doc", DOC]), &["--board"]);
    assert_one_line_error(
        &run(
            &home,
            &["design", "read", "--doc", "checkout", "--board", BOARD],
        ),
        &["checkout"],
    );
    assert_one_line_error(
        &run(&home, &["design", "read", "--doc", DOC, "--board", "Cart!"]),
        &["Cart!"],
    );
    assert_nothing_sent(&server);
}

#[test]
fn design_read_of_a_board_the_doc_does_not_have_names_it() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(
        &run(&home, &["design", "read", "--doc", DOC, "--board", UNKNOWN]),
        &[UNKNOWN],
    );
}
