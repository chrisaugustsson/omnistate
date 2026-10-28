//! US-025: `omni wait --changeset`, tested as the built binary against a mock
//! HTTP server. None of these tests needs Postgres; the round trip against the
//! API is in tests/stack_changesets.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni wait --changeset <id> --until approved|rejected|merged|needs_rebase
//!   [--timeout <seconds>] [--project <p>]` reads the change set with `GET
//!   /api/v1/w/{workspace}/projects/{project_id}/changesets/{id}` (US-016) as
//!   the agent, again at least once a second, until the condition is met. It
//!   sends nothing but those reads. The project is found as for `omni
//!   changeset` (tests/changesets.rs).
//! - `--until rejected`, `merged` and `needs_rebase` are met when the change
//!   set has that status. `--until approved` is met when the change set
//!   carries the human's approval: its status is `approved`, `needs_rebase` or
//!   `merged`, because the server merges an approved change set at once or
//!   moves it to `needs_rebase` (US-025), so it never rests in `approved`.
//! - Met: exit 0, and standard output names the status (the API's JSON of the
//!   change set, `{"changeset": ...}`, with `--json`).
//! - `--timeout <seconds>` ran out first: exit 2. Without `--timeout` it waits
//!   as long as it takes.
//! - The condition can no longer be met: exit 3. That is when the change set
//!   is `abandoned`, or `merged` while waiting for `rejected` or
//!   `needs_rebase`; both statuses are final (FR-20).
//! - Exits 2 and 3 print nothing on standard output and one line on standard
//!   error: for 2 it says the wait timed out, for 3 it names the status that
//!   ended the wait. Both are documented under "Exit codes" in `omni --help`.
//! - A missing `--changeset` or `--until`, an `--until` outside the four
//!   statuses, a `--timeout` that is not a number, and an API error are
//!   errors as in US-015: exit 1, one line on standard error, and for the
//!   usage errors no request at all.

mod common;

use common::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const MERGED: &str = "01900000-0000-7000-8000-0000000000d1";
const APPROVED: &str = "01900000-0000-7000-8000-0000000000d2";
const NEEDS_REBASE: &str = "01900000-0000-7000-8000-0000000000d3";
const REJECTED: &str = "01900000-0000-7000-8000-0000000000d4";
const ABANDONED: &str = "01900000-0000-7000-8000-0000000000d5";
/// Stays `in_review` for good.
const STUCK: &str = "01900000-0000-7000-8000-0000000000d6";
/// `in_review` until `LATER` after its first read, then `merged`.
const LATER_MERGED: &str = "01900000-0000-7000-8000-0000000000d7";

const LATER: Duration = Duration::from_millis(1500);

fn changeset_path(id: &str) -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/changesets/{id}")
}

fn changeset_json(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "title": "Hero section",
        "description": "Landing hero, first pass.",
        "status": status,
        "run_tag": "ui-research-7",
        "created_at": "2026-10-15T10:00:00.000000Z",
        "approved_versions": [],
        "history": [
            {
                "from_status": null,
                "to_status": "open",
                "actor": "agent",
                "reason": null,
                "at": "2026-10-15T10:00:00.000000Z"
            }
        ]
    })
}

/// An API that knows the change sets above in project `PROJECT`.
fn mock() -> MockServer {
    let first_reads: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    MockServer::start(move |request| {
        if request.method == "GET" && request.path == format!("/api/v1/w/{WORKSPACE}/projects") {
            return Reply::json(
                200,
                &json!({ "projects": [{
                    "id": PROJECT,
                    "name": "brand-site",
                    "created_at": "2026-10-15T10:00:00.000000Z"
                }] }),
            );
        }
        let prefix = changeset_path("");
        let Some(id) = request.path.strip_prefix(&prefix) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        if request.method != "GET" {
            return Reply::json(405, &json!({ "error": "method_not_allowed" }));
        }
        let since_first = {
            let mut reads = first_reads.lock().unwrap();
            reads
                .entry(id.to_string())
                .or_insert_with(Instant::now)
                .elapsed()
        };
        let later = |status: &'static str| {
            if since_first >= LATER {
                status
            } else {
                "in_review"
            }
        };
        let status = match id {
            MERGED => "merged",
            APPROVED => "approved",
            NEEDS_REBASE => "needs_rebase",
            REJECTED => "rejected",
            ABANDONED => "abandoned",
            STUCK => "in_review",
            LATER_MERGED => later("merged"),
            _ => return Reply::json(404, &json!({ "error": "not_found" })),
        };
        Reply::json(200, &json!({ "changeset": changeset_json(id, status) }))
    })
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

fn wait(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(&["wait", "--project", PROJECT])
        .args(args)
        .args(&["--as", "run-7/author"])
        .run()
}

/// A failure as US-015 defines it, with this exit code: nothing on standard
/// output and exactly one line on standard error that has every needle in it.
fn assert_one_line_failure(out: &Out, code: i32, needles: &[&str]) {
    assert_eq!(
        out.code, code,
        "exit code; stdout: {:?}; stderr: {:?}",
        out.stdout, out.stderr
    );
    assert!(
        documented_exit_codes().contains(&out.code),
        "exit code {} is not documented under \"Exit codes\" in omni --help",
        out.code
    );
    assert_eq!(
        out.stdout, "",
        "a failure prints nothing on standard output"
    );
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

fn reads_of(server: &MockServer, id: &str) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.path == changeset_path(id))
        .count()
}

fn assert_only_reads(server: &MockServer) {
    let writes: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.method != "GET")
        .collect();
    assert!(writes.is_empty(), "omni wait only reads: {writes:?}");
}

// ---------------------------------------------------------------------------
// Met
// ---------------------------------------------------------------------------

#[test]
fn json_output_is_the_change_set_as_the_api_answers_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait(
        &home,
        &["--changeset", REJECTED, "--until", "rejected", "--json"],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let answer = out.json();
    assert_eq!(answer["changeset"]["id"], REJECTED, "{answer}");
    assert_eq!(answer["changeset"]["status"], "rejected", "{answer}");
}

#[test]
fn each_status_is_met_by_itself() {
    let server = mock();
    let home = logged_in(&server.url);
    for (changeset, until) in [
        (REJECTED, "rejected"),
        (MERGED, "merged"),
        (NEEDS_REBASE, "needs_rebase"),
        (APPROVED, "approved"),
    ] {
        let out = wait(
            &home,
            &["--changeset", changeset, "--until", until, "--timeout", "5"],
        );
        assert_eq!(out.code, 0, "--until {until}: {}", out.stderr);
        assert!(
            out.stdout.contains(until),
            "--until {until}: {:?}",
            out.stdout
        );
    }
}

#[test]
fn it_blocks_until_the_status_is_reached() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait(&home, &["--changeset", LATER_MERGED, "--until", "merged"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("merged"), "{:?}", out.stdout);
    assert!(
        out.elapsed >= LATER - Duration::from_millis(100),
        "it waited for the change set to merge: {:?}",
        out.elapsed
    );
    // It reads at least once a second, so it notices within about a second.
    assert!(
        out.elapsed < LATER + Duration::from_millis(2500),
        "it noticed the merge late: {:?}",
        out.elapsed
    );
    assert!(
        reads_of(&server, LATER_MERGED) >= 2,
        "it read the change set again: {:?}",
        server.requests()
    );
    assert_only_reads(&server);
}

// ---------------------------------------------------------------------------
// Timed out
// ---------------------------------------------------------------------------

#[test]
fn a_timeout_exits_2_with_one_line() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait(
        &home,
        &["--changeset", STUCK, "--until", "merged", "--timeout", "1"],
    );
    assert_one_line_failure(&out, 2, &["time"]);
    assert!(
        out.elapsed >= Duration::from_millis(900),
        "it waited for the timeout: {:?}",
        out.elapsed
    );
    assert!(
        out.elapsed < Duration::from_secs(4),
        "it stopped at the timeout: {:?}",
        out.elapsed
    );
    assert!(reads_of(&server, STUCK) >= 1, "{:?}", server.requests());
    assert_only_reads(&server);
}

// ---------------------------------------------------------------------------
// Can no longer be met
// ---------------------------------------------------------------------------

#[test]
fn an_abandoned_change_set_meets_no_condition_and_exits_3() {
    let server = mock();
    let home = logged_in(&server.url);
    for until in ["approved", "rejected", "merged", "needs_rebase"] {
        let out = wait(
            &home,
            &[
                "--changeset",
                ABANDONED,
                "--until",
                until,
                "--timeout",
                "10",
            ],
        );
        assert_one_line_failure(&out, 3, &["abandoned"]);
        assert!(
            out.elapsed < Duration::from_secs(3),
            "--until {until}: it gave up at once: {:?}",
            out.elapsed
        );
    }
}

// ---------------------------------------------------------------------------
// Usage and API errors
// ---------------------------------------------------------------------------

#[test]
fn a_wait_without_its_options_is_a_usage_error_that_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let cases: [(&[&str], &str); 4] = [
        (&["--until", "merged"], "--changeset"),
        (&["--changeset", MERGED], "--until"),
        (&["--changeset", MERGED, "--until", "open"], "--until"),
        (
            &[
                "--changeset",
                MERGED,
                "--until",
                "merged",
                "--timeout",
                "soon",
            ],
            "--timeout",
        ),
    ];
    for (args, needle) in cases {
        let out = wait(&home, args);
        assert_one_line_failure(&out, 1, &[needle]);
    }
    assert!(
        server.requests().is_empty(),
        "a usage error sends nothing: {:?}",
        server.requests()
    );
}
