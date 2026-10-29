//! US-111: `omni doc activity`, tested as the built `omni` against a mock HTTP
//! server; the API's contract is in crates/omni-api/tests/activity.rs.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const GONE: &str = "01900000-0000-7000-8000-0000000000d9";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn entry(seq: u32, kind: &str, role: &str, at: &str, summary: &str) -> Value {
    json!({
        "seq": seq,
        "kind": kind,
        "agent": { "run": "run-3", "role": role, "model": "m", "harness": "h" },
        "created_at": at,
        "summary": summary
    })
}

fn page() -> Value {
    json!({
        "entries": [
            entry(4, "rename", "designer", "2026-10-24T09:30:00.000000Z", "Renamed from 'Draft' to 'Checkout plan'"),
            entry(3, "edit", "writer", "2026-10-24T09:20:00.000000Z", "3 nodes changed, 1 inserted"),
        ],
        "next_before": null
    })
}

/// An API with the doc `DOC`, whose activity is one page; `GONE` is a doc that
/// is not there.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let path = request
            .path
            .split_once('?')
            .map_or(request.path.as_str(), |(path, _)| path);
        let Some(rest) = path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        match (request.method.as_str(), rest) {
            ("GET", r) if r == format!("/docs/{DOC}/activity") => Reply::json(200, &page()),
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
fn doc_activity_prints_one_line_per_entry_with_the_seq_the_time_the_role_and_the_summary() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "activity", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        non_empty_lines(&out.stdout),
        [
            "4  2026-10-24T09:30:00.000000Z  designer  Renamed from 'Draft' to 'Checkout plan'",
            "3  2026-10-24T09:20:00.000000Z  writer  3 nodes changed, 1 inserted",
        ]
    );
}

#[test]
fn doc_activity_reads_the_page_that_limit_and_before_name() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["doc", "activity", DOC, "--limit", "2", "--before", "5"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let gets: Vec<Recorded> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "GET")
        .collect();
    assert_eq!(gets.len(), 1, "{gets:?}");
    let (path, query) = gets[0].path.split_once('?').expect("a query");
    assert_eq!(path, format!("{}/docs/{DOC}/activity", base()));
    let mut pairs: Vec<&str> = query.split('&').collect();
    pairs.sort();
    assert_eq!(pairs, ["before=5", "limit=2"], "{gets:?}");
}

#[test]
fn doc_activity_json_prints_the_api_answer() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "activity", DOC, "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json(), page(), "--json prints the API's answer");
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
