//! US-029: `omni component usage`, tested as the built binary against a mock
//! HTTP server. None of these tests needs Postgres; the API's contract is in
//! crates/omni-api/tests/components.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni component usage <component>` takes a component's node ID and sends
//!   `GET /components/usage?component=<component>`. It prints one line per
//!   board of the answer's `boards`, with the board's name and ID and the
//!   doc's title, as plain text; without boards it prints a line that says no
//!   board uses the component. `--json` prints the API's answer.
//! - A missing component argument, or one that is no node ID (8 lowercase
//!   base32 characters), is a one-line error that sends nothing. A component
//!   that the design system does not define (404 `unknown_component`) is a
//!   one-line error that names it.
//! - `omni --help` lists `omni component usage`.
//! - Every command is an agent request with the agent identity headers, and
//!   errors are one line on standard error, as in US-015
//!   (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const SHOP: &str = "01900000-0000-7000-8000-0000000000d1";
const ADMIN: &str = "01900000-0000-7000-8000-0000000000d2";
const SHOP_VERSION: &str = "01900000-0000-7000-8000-0000000000f1";
const ADMIN_VERSION: &str = "01900000-0000-7000-8000-0000000000f2";
/// The button, which three boards use.
const BUTTON: &str = "btn00001";
/// A tag, which no board uses.
const TAG: &str = "tag00001";
/// A component ID that the design system does not define.
const UNKNOWN: &str = "zzzzzzzz";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn usage_path(component: &str) -> String {
    format!("{}/components/usage?component={component}", base())
}

fn board(doc: &str, title: &str, version: &str, page: (&str, &str), id: &str, name: &str) -> Value {
    json!({
        "doc_id": doc,
        "doc_title": title,
        "version_id": version,
        "page_id": page.0,
        "page_name": page.1,
        "board_id": id,
        "board_name": name,
        "components": [BUTTON]
    })
}

fn button_usage() -> Value {
    json!({
        "project_id": PROJECT,
        "component": BUTTON,
        "boards": [
            board(SHOP, "Shop", SHOP_VERSION, ("page0001", "Home"), "brd00001", "Checkout"),
            board(SHOP, "Shop", SHOP_VERSION, ("page0001", "Home"), "brd00002", "Product"),
            board(ADMIN, "Admin", ADMIN_VERSION, ("page0002", "Admin"), "brd00003", "Settings")
        ]
    })
}

fn mock() -> MockServer {
    MockServer::start(move |request| {
        if request.method != "GET" {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        if request.path == usage_path(BUTTON) {
            Reply::json(200, &button_usage())
        } else if request.path == usage_path(TAG) {
            Reply::json(
                200,
                &json!({ "project_id": PROJECT, "component": TAG, "boards": [] }),
            )
        } else if request.path == usage_path(UNKNOWN) {
            Reply::json(404, &json!({ "error": "unknown_component" }))
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
        .args(&["--project", PROJECT, "--as", "run-29/designer"])
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
    assert_eq!(request.header("x-omni-run"), Some("run-29"), "{request:?}");
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
fn component_usage_lists_every_board_that_uses_the_component() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["component", "usage", BUTTON]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, usage_path(BUTTON), "{requests:?}");
    assert_agent_request(&requests[0]);
    assert!(
        serde_json::from_str::<Value>(&out.stdout).is_err(),
        "the default output is plain text: {:?}",
        out.stdout
    );
    let lines = non_empty_lines(&out.stdout);
    for (name, id, title) in [
        ("Checkout", "brd00001", "Shop"),
        ("Product", "brd00002", "Shop"),
        ("Settings", "brd00003", "Admin"),
    ] {
        assert!(
            lines
                .iter()
                .any(|line| line.contains(name) && line.contains(id) && line.contains(title)),
            "one line names the board {name}, its ID {id} and its doc {title}: {:?}",
            out.stdout
        );
    }
    assert!(lines.len() <= 5, "{:?}", out.stdout);
}

#[test]
fn component_usage_says_when_no_board_uses_the_component() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["component", "usage", TAG]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.to_lowercase().contains("no board"),
        "the output says that no board uses it: {:?}",
        out.stdout
    );
    assert_eq!(server.requests().len(), 1, "{:?}", server.requests());
}

#[test]
fn component_usage_prints_the_answer_with_json() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["component", "usage", BUTTON, "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json(), button_usage());
}

#[test]
fn component_usage_needs_a_component_id_before_sending_anything() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(&run(&home, &["component", "usage"]), &["component"]);
    assert_one_line_error(
        &run(&home, &["component", "usage", "Button!"]),
        &["Button!"],
    );
    assert_nothing_sent(&server);
}

#[test]
fn component_usage_of_an_unknown_component_names_it() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(&run(&home, &["component", "usage", UNKNOWN]), &[UNKNOWN]);
}

#[test]
fn help_lists_component_usage() {
    let home = Scratch::new("help");
    let out = omni(&home.path).args(&["--help"]).run();
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains("omni component usage"),
        "omni --help lists `omni component usage`: {}",
        out.stdout
    );
}
