//! US-111: `omni doc activity`, tested as the built `omni` against a mock HTTP
//! server. None of these tests needs Postgres; the API's contract is in
//! crates/omni-api/tests/activity.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`):
//!
//! - `omni doc activity <doc>` reads `GET /docs/{doc}/activity`, which answers
//!   `{"entries": [Entry], "next": <seq> | null}` newest first, and prints one
//!   line per entry: `<seq>  <created_at>  <role>  <summary>` (two spaces),
//!   the role being the entry's `agent.role`. While an answer has a `next`,
//!   it reads the next page with `?before=<next>` and prints it too, so the
//!   whole activity is printed, newest first. `--json` prints the API's
//!   answer of the first page.
//! - Without the doc it is a one-line error that names the doc and sends
//!   nothing. An API error (a doc that is not there) is one line on standard
//!   error with the error's code (US-015). `omni --help` lists the command.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const GONE: &str = "01900000-0000-7000-8000-0000000000d9";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn entry(seq: u32, role: &str, at: &str, summary: &str) -> Value {
    json!({
        "seq": seq,
        "agent": { "run": "run-3", "role": role, "model": "m", "harness": "h" },
        "created_at": at,
        "summary": summary
    })
}

fn first_page() -> Value {
    json!({
        "entries": [
            entry(4, "designer", "2026-10-24T09:30:00.000000Z", "renamed to \"Checkout plan\""),
            entry(3, "writer", "2026-10-24T09:20:00.000000Z", "3 nodes changed, 1 inserted"),
        ],
        "next": 3
    })
}

fn second_page() -> Value {
    json!({
        "entries": [
            entry(2, "writer", "2026-10-24T09:10:00.000000Z", "2 nodes inserted"),
            entry(1, "writer", "2026-10-24T09:00:00.000000Z", "created"),
        ],
        "next": null
    })
}

/// An API with the doc `DOC`, whose activity has two pages; `GONE` is a doc
/// that is not there.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let (path, query) = request
            .path
            .split_once('?')
            .unwrap_or((request.path.as_str(), ""));
        let Some(rest) = path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        match (request.method.as_str(), rest) {
            ("GET", r) if r == format!("/docs/{DOC}/activity") => {
                if query.split('&').any(|pair| pair == "before=3") {
                    Reply::json(200, &second_page())
                } else {
                    Reply::json(200, &first_page())
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

#[test]
fn doc_activity_prints_every_write_newest_first_with_the_role_the_time_and_the_summary() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "activity", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        non_empty_lines(&out.stdout),
        [
            "4  2026-10-24T09:30:00.000000Z  designer  renamed to \"Checkout plan\"",
            "3  2026-10-24T09:20:00.000000Z  writer  3 nodes changed, 1 inserted",
            "2  2026-10-24T09:10:00.000000Z  writer  2 nodes inserted",
            "1  2026-10-24T09:00:00.000000Z  writer  created",
        ],
        "one line per write, across both pages"
    );
    let gets: Vec<Recorded> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "GET")
        .collect();
    assert_eq!(gets.len(), 2, "two pages are read: {gets:?}");
    for get in &gets {
        assert!(
            get.path
                .starts_with(&format!("{}/docs/{DOC}/activity", base())),
            "{get:?}"
        );
        assert_eq!(
            get.header("authorization"),
            Some(format!("Bearer {API_KEY}").as_str()),
            "{get:?}"
        );
    }
    assert!(
        gets[1].path.contains("before=3"),
        "the second page is read with the first page's next: {gets:?}"
    );
}

#[test]
fn doc_activity_json_prints_the_api_answer() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "activity", DOC, "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json(), first_page(), "--json prints the API's answer");
}

#[test]
fn doc_activity_without_a_doc_is_one_line_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "activity"]);
    assert_one_line_error(&out, &["doc"]);
    assert!(
        server.requests().is_empty(),
        "nothing is sent: {:?}",
        server.requests()
    );
}

#[test]
fn doc_activity_of_a_doc_that_is_not_there_is_one_line_with_the_code() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "activity", GONE]);
    assert_one_line_error(&out, &["not_found"]);
}

#[test]
fn help_lists_doc_activity() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    assert!(
        help.stdout.contains("doc activity"),
        "omni --help lists `omni doc activity`: {:?}",
        help.stdout
    );
}
