//! US-022: `omni doc`, tested as the built binary against a mock HTTP server.
//! None of these tests needs Postgres; the real round trip against the API is
//! in tests/stack_docs.rs, and the API's contract in
//! crates/omni-api/tests/docs.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found from `--project`):
//!
//! - `omni doc create` with a missing kind, pillar or title, or a kind outside
//!   `html`, `design` and `design-system`, is a one-line error that sends
//!   nothing; so is a pillar that is not one of the five (the error lists the
//!   five pillar names) and the old `--module` option (the error names
//!   `--pillar`, US-101). The doc it creates is in tests/live_docs.rs.
//! - `omni doc read <doc>` prints the doc's head from `GET /docs/{doc}/html`:
//!   the HTML as the API gave it, and nothing else is on standard output.
//! - `omni doc show <doc>` sends `GET /docs/{doc}` and prints the doc's title,
//!   kind and pillar.
//! - `omni doc edit` is in tests/live_edits.rs (US-103).
//! - `<doc>` is a doc ID (a UUID). Every command is an agent request with the
//!   agent identity headers, `--json` prints the API's JSON, and errors are one
//!   line on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const HEAD_HTML: &str = "<h1 data-omni-id=\"a0000001\" data-rev=\"1\">Checkout</h1><p data-omni-id=\"a0000002\" data-rev=\"3\">Head text</p>";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn doc_json() -> Value {
    json!({
        "id": DOC,
        "project_id": PROJECT,
        "kind": "html",
        "pillar": "research",
        "title": "Checkout PRD",
        "created_at": "2026-10-15T10:00:00.000000Z",
        "feature_id": null
    })
}

fn head_json() -> Value {
    json!({ "doc_id": DOC, "html": HEAD_HTML })
}

/// An API with the doc `DOC`.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let Some(rest) = request.path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        match (request.method.as_str(), rest) {
            ("GET", r) if r == format!("/docs/{DOC}") => {
                Reply::json(200, &json!({ "doc": doc_json() }))
            }
            ("GET", r) if r == format!("/docs/{DOC}/html") => Reply::json(200, &head_json()),
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

fn assert_nothing_sent(server: &MockServer) {
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni doc create
// ---------------------------------------------------------------------------

#[test]
fn doc_create_refuses_bad_options_before_sending_anything() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(
        &run(
            &home,
            &["doc", "create", "--pillar", "spec", "--title", "T"],
        ),
        &["kind"],
    );
    assert_one_line_error(
        &run(&home, &["doc", "create", "--kind", "html", "--title", "T"]),
        &["pillar"],
    );
    assert_one_line_error(
        &run(
            &home,
            &["doc", "create", "--kind", "html", "--pillar", "spec"],
        ),
        &["title"],
    );
    assert_one_line_error(
        &run(
            &home,
            &[
                "doc", "create", "--kind", "pdf", "--pillar", "spec", "--title", "T",
            ],
        ),
        &["pdf"],
    );
    assert_nothing_sent(&server);
}

#[test]
fn doc_create_refuses_a_pillar_that_is_not_one_of_the_five() {
    let server = mock();
    let home = logged_in(&server.url);
    for pillar in ["other", "prd"] {
        let out = run(
            &home,
            &[
                "doc", "create", "--kind", "html", "--pillar", pillar, "--title", "T",
            ],
        );
        assert_one_line_error(
            &out,
            &[pillar, "spec", "ui", "architecture", "db", "research"],
        );
    }
    assert_nothing_sent(&server);
}

#[test]
fn doc_create_refuses_the_old_module_option_and_names_pillar() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "doc", "create", "--kind", "html", "--module", "prd", "--title", "T",
        ],
    );
    assert_one_line_error(&out, &["--module", "--pillar"]);
    assert_nothing_sent(&server);
}

// ---------------------------------------------------------------------------
// omni doc read
// ---------------------------------------------------------------------------

#[test]
fn doc_read_prints_the_head() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let gets = requests_to(&server, "GET", &format!("{}/docs/{DOC}/html", base()));
    assert_eq!(gets.len(), 1, "{:?}", server.requests());
    assert_agent_request(&gets[0]);
    assert_eq!(
        out.stdout.trim_end(),
        HEAD_HTML,
        "the HTML with data-omni-id and data-rev, and nothing else"
    );
}

#[test]
fn doc_read_with_json_prints_the_api_answer() {
    let server = mock();
    let home = logged_in(&server.url);
    let json = run(&home, &["doc", "read", DOC, "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), head_json());
}

#[test]
fn doc_read_needs_a_doc() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(&run(&home, &["doc", "read"]), &["doc"]);
    assert_nothing_sent(&server);
}

// ---------------------------------------------------------------------------
// omni doc show
// ---------------------------------------------------------------------------

#[test]
fn doc_show_prints_the_metadata() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "show", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let gets = requests_to(&server, "GET", &format!("{}/docs/{DOC}", base()));
    assert_eq!(gets.len(), 1, "{:?}", server.requests());
    assert_agent_request(&gets[0]);
    for needle in [DOC, "Checkout PRD", "html", "research"] {
        assert!(
            out.stdout.contains(needle),
            "show prints {needle:?}: {:?}",
            out.stdout
        );
    }

    let json = run(&home, &["doc", "show", DOC, "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), json!({ "doc": doc_json() }));
}
