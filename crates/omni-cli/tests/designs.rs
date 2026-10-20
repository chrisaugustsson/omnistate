//! US-030: `omni design read`, tested as the built binary against a mock HTTP
//! server. None of these tests needs Postgres; the API's contract is in
//! crates/omni-api/tests/boards.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni design read --doc <id> --board <id>` prints one board of the doc in
//!   the tag format: with `--changeset <id>` of that change set's draft (`GET
//!   /changesets/{changeset}/docs/{doc}/boards/{board}`), with `--version <v>`
//!   of that saved version (`GET /docs/{doc}/boards/{board}?version=<v>`), and
//!   without either of the mainline version (`GET
//!   /docs/{doc}/boards/{board}`). It prints the answer's `tags` as they are,
//!   which for a draft carry a `rev` on every tag, and nothing else. `--json`
//!   prints the API's answer.
//! - A missing `--doc` or `--board`, a doc that is no UUID, a board that is no
//!   node ID (8 lowercase base32 characters), or both `--changeset` and
//!   `--version`, is a one-line error that names the problem and sends
//!   nothing. A board that the doc does not have (404 `unknown_board`) is a
//!   one-line error that names it, and a mainline read of a doc without a
//!   mainline version (409 `no_mainline_version`) one that says so and points
//!   to `--changeset`.
//! - `omni --help` lists `omni design read`.
//! - Every command is an agent request with the agent identity headers, and
//!   errors are one line on standard error, as in US-015
//!   (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
/// A doc that has no mainline version yet.
const NEW_DOC: &str = "01900000-0000-7000-8000-0000000000d2";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const VERSION: &str = "01900000-0000-7000-8000-0000000000f1";
const MAINLINE: &str = "01900000-0000-7000-8000-0000000000f2";
const PAGE: &str = "page0001";
const BOARD: &str = "brd00001";
/// A board ID that the doc does not have.
const UNKNOWN: &str = "zzzzzzzz";

const DRAFT_TAGS: &str = r#"<Board id="brd00001" rev={3} name="Cart" x={0} y={0} width={360} height={640} dir="col">
  <Text id="txt00001" rev={2} name="Total" content="Summa 12 kr" fontFamily="Inter" fontSize={16}/>
</Board>"#;
const VERSION_TAGS: &str = r#"<Board id="brd00001" name="Cart" x={0} y={0} width={360} height={640} dir="col">
  <Text id="txt00001" name="Total" content="Summa 11 kr" fontFamily="Inter" fontSize={16}/>
</Board>"#;
const MAINLINE_TAGS: &str = r#"<Board id="brd00001" name="Cart" x={0} y={0} width={360} height={640} dir="col">
  <Text id="txt00001" name="Total" content="Summa 10 kr" fontFamily="Inter" fontSize={16}/>
</Board>"#;

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn draft_path(doc: &str, board: &str) -> String {
    format!(
        "{}/changesets/{CHANGESET}/docs/{doc}/boards/{board}",
        base()
    )
}

fn board_path(doc: &str, board: &str) -> String {
    format!("{}/docs/{doc}/boards/{board}", base())
}

fn draft_answer() -> Value {
    json!({
        "doc_id": DOC,
        "changeset_id": CHANGESET,
        "draft_id": "01900000-0000-7000-8000-0000000000e1",
        "page_id": PAGE,
        "board_id": BOARD,
        "version_id": null,
        "tags": DRAFT_TAGS
    })
}

fn version_answer(version: &str, tags: &str) -> Value {
    json!({
        "doc_id": DOC,
        "page_id": PAGE,
        "board_id": BOARD,
        "version_id": version,
        "tags": tags
    })
}

fn mock() -> MockServer {
    MockServer::start(move |request| {
        if request.method != "GET" {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        let path = request.path.as_str();
        if path == draft_path(DOC, BOARD) {
            Reply::json(200, &draft_answer())
        } else if path == format!("{}?version={VERSION}", board_path(DOC, BOARD)) {
            Reply::json(200, &version_answer(VERSION, VERSION_TAGS))
        } else if path == board_path(DOC, BOARD) {
            Reply::json(200, &version_answer(MAINLINE, MAINLINE_TAGS))
        } else if path == board_path(DOC, UNKNOWN) || path == draft_path(DOC, UNKNOWN) {
            Reply::json(404, &json!({ "error": "unknown_board" }))
        } else if path == board_path(NEW_DOC, BOARD) {
            Reply::json(409, &json!({ "error": "no_mainline_version" }))
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

/// Asserts that the command asked for `path` once and printed `tags` and
/// nothing else.
fn assert_printed(server: &MockServer, out: &Out, path: &str, tags: &str) {
    assert_eq!(out.code, 0, "{}", out.stderr);
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, path, "{requests:?}");
    assert_agent_request(&requests[0]);
    assert_eq!(
        out.stdout.trim_end(),
        tags,
        "the board's tags as the API gave them, and nothing else"
    );
}

#[test]
fn design_read_with_a_changeset_prints_the_board_of_the_draft_with_revs() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "design",
            "read",
            "--doc",
            DOC,
            "--board",
            BOARD,
            "--changeset",
            CHANGESET,
        ],
    );
    assert_printed(&server, &out, &draft_path(DOC, BOARD), DRAFT_TAGS);
    assert!(
        opening_lines(&out.stdout)
            .iter()
            .all(|line| line.contains(" rev=")),
        "every tag of a draft carries its rev: {:?}",
        out.stdout
    );
}

#[test]
fn design_read_with_a_version_prints_the_board_of_that_version() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "design",
            "read",
            "--doc",
            DOC,
            "--board",
            BOARD,
            "--version",
            VERSION,
        ],
    );
    assert_printed(
        &server,
        &out,
        &format!("{}?version={VERSION}", board_path(DOC, BOARD)),
        VERSION_TAGS,
    );
}

#[test]
fn design_read_without_options_prints_the_board_of_the_mainline() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["design", "read", "--doc", DOC, "--board", BOARD]);
    assert_printed(&server, &out, &board_path(DOC, BOARD), MAINLINE_TAGS);
}

#[test]
fn design_read_prints_the_answer_with_json() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "design",
            "read",
            "--doc",
            DOC,
            "--board",
            BOARD,
            "--changeset",
            CHANGESET,
            "--json",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json(), draft_answer());
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
    assert_one_line_error(
        &run(
            &home,
            &[
                "design",
                "read",
                "--doc",
                DOC,
                "--board",
                BOARD,
                "--changeset",
                CHANGESET,
                "--version",
                VERSION,
            ],
        ),
        &["--changeset", "--version"],
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
    assert_one_line_error(
        &run(
            &home,
            &[
                "design",
                "read",
                "--doc",
                DOC,
                "--board",
                UNKNOWN,
                "--changeset",
                CHANGESET,
            ],
        ),
        &[UNKNOWN],
    );
}

#[test]
fn design_read_of_the_mainline_of_a_doc_without_one_points_to_the_draft() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(
        &run(
            &home,
            &["design", "read", "--doc", NEW_DOC, "--board", BOARD],
        ),
        &["mainline", "--changeset"],
    );
}

#[test]
fn help_lists_design_read() {
    let home = Scratch::new("help");
    let out = omni(&home.path).args(&["--help"]).run();
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains("omni design read"),
        "omni --help lists `omni design read`: {}",
        out.stdout
    );
}

/// The lines of `text` that open a tag.
fn opening_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with('<') && !line.starts_with("</"))
        .collect()
}
