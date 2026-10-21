//! US-036: `omni wait --thread`, tested as the built binary against a mock
//! HTTP server. None of these tests needs Postgres; the API's side (who has
//! posted in each phase, the thread's decision) is in
//! crates/omni-api/tests/thread_waits.rs, and the round trip against the API
//! in tests/stack_threads.rs. `omni wait --changeset` (US-025) is in
//! tests/wait.rs and stays as it is.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni wait --thread <id> --until <condition> [--timeout <seconds>]
//!   [--project <p>]` reads the thread with `GET
//!   /api/v1/w/{workspace}/projects/{project_id}/threads/{id}` (US-035) as the
//!   agent, again at least once a second, until the condition is met. It sends
//!   nothing but reads. The project is found as for `omni changeset`.
//! - The API answers `{"thread": Thread}`. Each entry of the Thread's `phases`
//!   has `number`, `sealed` and `posted_by`: the distinct agent identities
//!   (`{"run", "role"}`) that have posted in that phase; the human is never in
//!   it. The Thread's `decision` is `null` or the thread's current decision,
//!   with at least `id` and `status` (US-037).
//! - "The current phase" is the thread's last phase at the first read: the
//!   wait keeps to that phase even when a later one starts.
//! - Conditions:
//!   - `posts:<n>` (n a whole number of at least 1): at least n different
//!     agent identities (run and role) have posted in that phase. Two runs
//!     with the same role are two identities; posts of earlier phases do not
//!     count.
//!   - `roles:<pattern>[,<pattern>...]`: for every pattern, an identity whose
//!     whole role matches it has posted in that phase. `*` matches any run of
//!     characters (also none); everything else matches itself.
//!   - `phase-change`: a phase after that one has started.
//!   - `decision:approved` and `decision:rejected`: the thread's decision has
//!     that status.
//! - Met: exit 0. `--json` prints the API's JSON of the thread as last read.
//! - `--timeout <seconds>` ran out first: exit 2. Without `--timeout` it waits
//!   as long as it takes.
//! - The condition can no longer be met: exit 3. That is when a later phase
//!   has started while `posts:` or `roles:` is not met in the phase the wait
//!   keeps to (the one-line message says so and has the word "phase"), and
//!   when the decision has the other final status: `rejected` while waiting
//!   for `decision:approved`, `approved` while waiting for
//!   `decision:rejected` (the message names that status).
//! - Exits 2 and 3 print nothing on standard output and one line on standard
//!   error.
//! - Usage errors are exit 1, one line on standard error that names the
//!   offending text, and no request at all: a missing `--until`, a condition
//!   that is none of the above (such as `posts:abc`, `posts:`, `roles:`,
//!   `decision:maybe`, or a change set status such as `approved`), a
//!   `--timeout` that is not a number, and `--thread` together with
//!   `--changeset`. An unknown thread is exit 1 with `not_found`.
//! - `omni --help` lists `omni wait --thread` and the conditions.

mod common;

use common::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const DECISION: &str = "01900000-0000-7000-8000-0000000000dd";

/// Phase 1 with one poster, run-7/critic-a, and no decision; it never changes.
const QUIET: &str = "01900000-0000-7000-8000-0000000000e1";
/// Phase 1 with run-7/critic and run-8/critic.
const SAME_ROLE: &str = "01900000-0000-7000-8000-0000000000e2";
/// Phase 1 with three posters (one a scout), phase 2 (current) with one.
const EARLIER_PHASE: &str = "01900000-0000-7000-8000-0000000000e3";
/// Phase 1 with critic-a; after `LATER` also critic-b and moderator.
const GROWING: &str = "01900000-0000-7000-8000-0000000000e4";
/// Phase 1 with critic-a; after `LATER` phase 2 starts and phase 1 is as it was.
const MOVED_ON: &str = "01900000-0000-7000-8000-0000000000e5";
/// Phase 1 with critic-a; after `LATER` phase 1 has three posters and phase 2
/// has started.
const MET_THEN_MOVED: &str = "01900000-0000-7000-8000-0000000000e6";
/// A proposed decision; approved after `LATER`.
const APPROVED_LATER: &str = "01900000-0000-7000-8000-0000000000e7";
/// A proposed decision; rejected after `LATER`.
const REJECTED_LATER: &str = "01900000-0000-7000-8000-0000000000e8";
/// An approved decision from the start.
const APPROVED: &str = "01900000-0000-7000-8000-0000000000ea";
/// No such thread.
const MISSING: &str = "01900000-0000-7000-8000-0000000000e9";

const LATER: Duration = Duration::from_millis(1500);

fn thread_path(id: &str) -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/threads/{id}")
}

fn identity(spec: &str) -> Value {
    let (run, role) = spec.split_once('/').expect("run/role");
    json!({ "run": run, "role": role })
}

/// A phase: its number, whether it is sealed, and who posted in it.
fn phase(number: u64, sealed: bool, posted_by: &[&str]) -> Value {
    json!({
        "number": number,
        "sealed": sealed,
        "started_at": "2026-10-21T10:00:00.000000Z",
        "posted_by": posted_by.iter().map(|s| identity(s)).collect::<Vec<_>>()
    })
}

fn decision(status: &str) -> Value {
    json!({ "id": DECISION, "thread_id": "", "status": status })
}

fn thread_json(id: &str, phases: Vec<Value>, decision: Value) -> Value {
    let mut decision = decision;
    if decision.is_object() {
        decision["thread_id"] = json!(id);
    }
    json!({ "thread": {
        "id": id,
        "project_id": PROJECT,
        "changeset_id": CHANGESET,
        "question": "Which hero layout?",
        "criteria": ["Readable at 320 px"],
        "run_tag": "run-7",
        "created_at": "2026-10-21T10:00:00.000000Z",
        "phases": phases,
        "posts": [],
        "decision": decision
    } })
}

/// An API that knows the threads above in project `PROJECT`.
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
        let prefix = thread_path("");
        let Some(id) = request.path.strip_prefix(&prefix) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        if request.method != "GET" {
            return Reply::json(405, &json!({ "error": "method_not_allowed" }));
        }
        let later = {
            let mut reads = first_reads.lock().unwrap();
            reads
                .entry(id.to_string())
                .or_insert_with(Instant::now)
                .elapsed()
                >= LATER
        };
        let none = Value::Null;
        let thread = match id {
            QUIET => thread_json(id, vec![phase(1, false, &["run-7/critic-a"])], none),
            SAME_ROLE => thread_json(
                id,
                vec![phase(1, true, &["run-7/critic", "run-8/critic"])],
                none,
            ),
            EARLIER_PHASE => thread_json(
                id,
                vec![
                    phase(
                        1,
                        false,
                        &["run-7/critic-a", "run-7/critic-b", "run-7/scout"],
                    ),
                    phase(2, true, &["run-7/critic-a"]),
                ],
                none,
            ),
            GROWING if later => thread_json(
                id,
                vec![phase(
                    1,
                    true,
                    &["run-7/critic-a", "run-7/critic-b", "run-7/moderator"],
                )],
                none,
            ),
            GROWING => thread_json(id, vec![phase(1, true, &["run-7/critic-a"])], none),
            MOVED_ON if later => thread_json(
                id,
                vec![phase(1, true, &["run-7/critic-a"]), phase(2, false, &[])],
                none,
            ),
            MOVED_ON => thread_json(id, vec![phase(1, true, &["run-7/critic-a"])], none),
            MET_THEN_MOVED if later => thread_json(
                id,
                vec![
                    phase(
                        1,
                        true,
                        &["run-7/critic-a", "run-7/critic-b", "run-7/critic-c"],
                    ),
                    phase(2, false, &[]),
                ],
                none,
            ),
            MET_THEN_MOVED => thread_json(id, vec![phase(1, true, &["run-7/critic-a"])], none),
            APPROVED_LATER => thread_json(
                id,
                vec![phase(1, false, &["run-7/critic-a"])],
                decision(if later { "approved" } else { "proposed" }),
            ),
            REJECTED_LATER => thread_json(
                id,
                vec![phase(1, false, &["run-7/critic-a"])],
                decision(if later { "rejected" } else { "proposed" }),
            ),
            APPROVED => thread_json(
                id,
                vec![phase(1, false, &["run-7/critic-a"])],
                decision("approved"),
            ),
            _ => return Reply::json(404, &json!({ "error": "not_found" })),
        };
        Reply::json(200, &thread)
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
        .args(&["--as", "run-7/moderator"])
        .run()
}

/// `omni wait --thread <thread> --until <until>` with `extra` options.
fn wait_for(home: &Scratch, thread: &str, until: &str, extra: &[&str]) -> Out {
    let mut args = vec!["--thread", thread, "--until", until];
    args.extend_from_slice(extra);
    wait(home, &args)
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

fn assert_met(out: &Out, what: &str) {
    assert_eq!(
        out.code, 0,
        "{what}: the condition is met; stdout: {:?}; stderr: {:?}",
        out.stdout, out.stderr
    );
}

/// The wait gave up at the timeout of one second.
fn assert_timed_out(out: &Out, what: &str) {
    assert_one_line_failure(out, 2, &["time"]);
    assert!(
        out.elapsed >= Duration::from_millis(900),
        "{what}: it waited for the timeout: {:?}",
        out.elapsed
    );
    assert!(
        out.elapsed < Duration::from_secs(4),
        "{what}: it stopped at the timeout: {:?}",
        out.elapsed
    );
}

/// The wait ended as `LATER` came, give or take a read a second.
fn assert_ended_later(out: &Out, what: &str) {
    assert!(
        out.elapsed >= LATER - Duration::from_millis(100),
        "{what}: it waited for the thread to change: {:?}",
        out.elapsed
    );
    assert!(
        out.elapsed < LATER + Duration::from_millis(2500),
        "{what}: it noticed the change late: {:?}",
        out.elapsed
    );
}

fn reads_of(server: &MockServer, id: &str) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.path == thread_path(id))
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
// posts:<n>
// ---------------------------------------------------------------------------

#[test]
fn posts_n_that_is_already_met_exits_0_at_once_and_reads_as_the_agent() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, QUIET, "posts:1", &["--timeout", "5"]);
    assert_met(&out, "posts:1");
    assert!(out.elapsed < Duration::from_secs(3), "{:?}", out.elapsed);
    let reads: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.path == thread_path(QUIET))
        .collect();
    assert!(
        !reads.is_empty(),
        "it reads the thread: {:?}",
        server.requests()
    );
    assert_eq!(
        reads[0].header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(reads[0].header("x-omni-run"), Some("run-7"));
    assert_eq!(reads[0].header("x-omni-role"), Some("moderator"));
    assert_only_reads(&server);
}

#[test]
fn posts_n_counts_identities_so_two_runs_with_one_role_are_two() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, SAME_ROLE, "posts:2", &["--timeout", "5"]);
    assert_met(&out, "posts:2 with run-7/critic and run-8/critic");
}

#[test]
fn posts_n_that_is_not_met_times_out_with_2() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, QUIET, "posts:2", &["--timeout", "1"]);
    assert_timed_out(&out, "posts:2 with one poster");
    assert!(reads_of(&server, QUIET) >= 1, "{:?}", server.requests());
    assert_only_reads(&server);
}

#[test]
fn posts_n_counts_only_the_current_phase() {
    // Phase 1 had three posters; the current phase 2 has one.
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, EARLIER_PHASE, "posts:2", &["--timeout", "1"]);
    assert_timed_out(&out, "posts:2 counts phase 2 only");
}

#[test]
fn posts_n_blocks_until_enough_identities_have_posted() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, GROWING, "posts:3", &["--timeout", "10"]);
    assert_met(&out, "posts:3");
    assert_ended_later(&out, "posts:3");
    assert!(
        reads_of(&server, GROWING) >= 2,
        "it read the thread again: {:?}",
        server.requests()
    );
    assert_only_reads(&server);
}

#[test]
fn posts_n_can_no_longer_be_met_once_a_later_phase_started() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, MOVED_ON, "posts:2", &["--timeout", "10"]);
    assert_one_line_failure(&out, 3, &["phase"]);
    assert_ended_later(&out, "posts:2 after phase 2 started");
}

#[test]
fn posts_n_met_in_the_waited_phase_counts_even_after_the_next_phase_started() {
    // Between two reads, two more critics posted in phase 1 and phase 2
    // started: the phase the wait keeps to reached three identities.
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, MET_THEN_MOVED, "posts:3", &["--timeout", "10"]);
    assert_met(&out, "posts:3 in phase 1");
    assert_ended_later(&out, "posts:3 in phase 1");
}

// ---------------------------------------------------------------------------
// roles:<pattern>,...
// ---------------------------------------------------------------------------

#[test]
fn roles_with_a_wildcard_or_an_exact_role_is_met_by_a_poster_with_that_role() {
    let server = mock();
    let home = logged_in(&server.url);
    for until in [
        "roles:critic-a",
        "roles:critic-*",
        "roles:*-a",
        "roles:*",
        "roles:c*a",
    ] {
        let out = wait_for(&home, QUIET, until, &["--timeout", "5"]);
        assert_met(&out, until);
        assert!(
            out.elapsed < Duration::from_secs(3),
            "{until}: {:?}",
            out.elapsed
        );
    }
}

#[test]
fn roles_match_the_whole_role() {
    let server = mock();
    let home = logged_in(&server.url);
    for until in [
        "roles:critic",
        "roles:critic-",
        "roles:ritic-a",
        "roles:*-b",
    ] {
        let out = wait_for(&home, QUIET, until, &["--timeout", "1"]);
        assert_timed_out(&out, until);
    }
}

#[test]
fn roles_needs_every_listed_pattern() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, QUIET, "roles:critic-*,scout", &["--timeout", "1"]);
    assert_timed_out(&out, "roles:critic-*,scout without a scout");
}

#[test]
fn roles_counts_only_the_current_phase() {
    // The scout posted in phase 1 only.
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, EARLIER_PHASE, "roles:scout", &["--timeout", "1"]);
    assert_timed_out(&out, "roles:scout in phase 2");
}

#[test]
fn roles_blocks_until_every_pattern_has_posted() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(
        &home,
        GROWING,
        "roles:critic-*,moderator,*-b",
        &["--timeout", "10"],
    );
    assert_met(&out, "roles:critic-*,moderator,*-b");
    assert_ended_later(&out, "roles:critic-*,moderator,*-b");
    assert_only_reads(&server);
}

#[test]
fn roles_can_no_longer_be_met_once_a_later_phase_started() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, MOVED_ON, "roles:critic-b", &["--timeout", "10"]);
    assert_one_line_failure(&out, 3, &["phase"]);
    assert_ended_later(&out, "roles:critic-b after phase 2 started");
}

// ---------------------------------------------------------------------------
// phase-change
// ---------------------------------------------------------------------------

#[test]
fn phase_change_blocks_until_the_next_phase_starts() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, MOVED_ON, "phase-change", &["--timeout", "10"]);
    assert_met(&out, "phase-change");
    assert_ended_later(&out, "phase-change");
    assert!(reads_of(&server, MOVED_ON) >= 2, "{:?}", server.requests());
    assert_only_reads(&server);
}

#[test]
fn phase_change_is_not_met_by_phases_that_were_there_at_the_start() {
    // EARLIER_PHASE already has two phases when the wait starts.
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, EARLIER_PHASE, "phase-change", &["--timeout", "1"]);
    assert_timed_out(&out, "phase-change without a new phase");
}

// ---------------------------------------------------------------------------
// decision:approved and decision:rejected
// ---------------------------------------------------------------------------

#[test]
fn decision_approved_blocks_until_the_human_approves() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(
        &home,
        APPROVED_LATER,
        "decision:approved",
        &["--timeout", "10"],
    );
    assert_met(&out, "decision:approved");
    assert_ended_later(&out, "decision:approved");
    assert_only_reads(&server);
}

#[test]
fn decision_rejected_blocks_until_the_human_rejects() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(
        &home,
        REJECTED_LATER,
        "decision:rejected",
        &["--timeout", "10"],
    );
    assert_met(&out, "decision:rejected");
    assert_ended_later(&out, "decision:rejected");
}

#[test]
fn an_approved_decision_meets_decision_approved_at_once() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, APPROVED, "decision:approved", &["--timeout", "5"]);
    assert_met(&out, "decision:approved");
    assert!(out.elapsed < Duration::from_secs(3), "{:?}", out.elapsed);
}

#[test]
fn a_thread_without_a_decision_times_out_waiting_for_one() {
    let server = mock();
    let home = logged_in(&server.url);
    for until in ["decision:approved", "decision:rejected"] {
        let out = wait_for(&home, QUIET, until, &["--timeout", "1"]);
        assert_timed_out(&out, until);
    }
}

#[test]
fn a_rejection_ends_a_wait_for_approval_with_3() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(
        &home,
        REJECTED_LATER,
        "decision:approved",
        &["--timeout", "10"],
    );
    assert_one_line_failure(&out, 3, &["rejected"]);
    assert_ended_later(&out, "decision:approved, then rejected");
}

#[test]
fn an_approval_ends_a_wait_for_rejection_with_3() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, APPROVED, "decision:rejected", &["--timeout", "10"]);
    assert_one_line_failure(&out, 3, &["approved"]);
    assert!(out.elapsed < Duration::from_secs(3), "{:?}", out.elapsed);
}

// ---------------------------------------------------------------------------
// Output, usage and API errors
// ---------------------------------------------------------------------------

#[test]
fn json_output_is_the_thread_as_the_api_answers_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, SAME_ROLE, "posts:2", &["--json", "--timeout", "5"]);
    assert_met(&out, "posts:2 --json");
    let answer = out.json();
    assert_eq!(answer["thread"]["id"], SAME_ROLE, "{answer}");
    assert_eq!(
        answer["thread"]["phases"][0]["posted_by"],
        json!([identity("run-7/critic"), identity("run-8/critic")]),
        "{answer}"
    );
}

#[test]
fn a_thread_wait_with_a_bad_condition_is_a_usage_error_that_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let cases: [(&[&str], &str); 10] = [
        (&["--thread", QUIET], "--until"),
        (&["--thread", QUIET, "--until", "posts:abc"], "posts:abc"),
        (&["--thread", QUIET, "--until", "posts:"], "posts:"),
        (&["--thread", QUIET, "--until", "posts:-2"], "posts:-2"),
        (&["--thread", QUIET, "--until", "roles:"], "roles:"),
        (
            &["--thread", QUIET, "--until", "decision:maybe"],
            "decision:maybe",
        ),
        (&["--thread", QUIET, "--until", "approved"], "approved"),
        (&["--thread", QUIET, "--until", "sometime"], "sometime"),
        (
            &[
                "--thread",
                QUIET,
                "--until",
                "phase-change",
                "--timeout",
                "soon",
            ],
            "--timeout",
        ),
        (
            &[
                "--thread",
                QUIET,
                "--changeset",
                "01900000-0000-7000-8000-0000000000d1",
                "--until",
                "phase-change",
            ],
            "--thread",
        ),
    ];
    for (args, needle) in cases {
        let out = wait(&home, args);
        assert_one_line_failure(&out, 1, &[needle]);
        if needle == "--thread" {
            assert!(
                out.stderr.contains("--changeset"),
                "--thread with --changeset names both: {:?}",
                out.stderr
            );
        }
    }
    let threads: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.path.starts_with(&thread_path("")))
        .collect();
    assert!(
        threads.is_empty(),
        "a usage error reads no thread: {threads:?}"
    );
}

#[test]
fn an_unknown_thread_is_one_line_with_the_error_code() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = wait_for(&home, MISSING, "phase-change", &["--timeout", "5"]);
    assert_one_line_failure(&out, 1, &["not_found"]);
}

#[test]
fn the_help_lists_omni_wait_for_threads_and_its_conditions() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    for word in [
        "omni wait --thread",
        "posts:<n>",
        "roles:",
        "phase-change",
        "decision:approved",
        "decision:rejected",
        // US-025's wait stays listed.
        "omni wait --changeset",
    ] {
        assert!(
            help.stdout.contains(word),
            "omni --help mentions {word}: {}",
            help.stdout
        );
    }
}
