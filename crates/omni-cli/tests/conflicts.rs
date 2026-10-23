//! US-026: `omni changeset rebase`, `omni changeset conflicts` and `omni
//! changeset resolve`, tested as the built binary against a mock HTTP server.
//! None of these tests needs Postgres; the API's side of the contract is in
//! crates/omni-api/tests/rebase.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni changeset rebase <id> [--project <p>]` sends `POST
//!   .../projects/{project_id}/changesets/{id}/rebase` with a JSON object
//!   body. The API answers `{"changeset": Changeset, "conflicts": [Conflict,
//!   ...]}`. The text has the change set's ID and its new status (`merged`
//!   after a clean rebase, `open` with conflicts) and the ID of every conflict
//!   the rebase recorded.
//! - `omni changeset conflicts <id> [--project <p>]` sends `GET
//!   .../changesets/{id}/conflicts`, answered with `{"conflicts": [Conflict,
//!   ...]}`. A Conflict is `{"id", "changeset_id", "doc_id", "node",
//!   "mainline": {"version_id", "html"}, "changeset": {"version_id", "html"},
//!   "resolution": null | {"note", "resolved_at"}}`, where `html` is `null`
//!   on a side that removed the node. The text shows, for each conflict, its
//!   ID, its node, both sides (a line marked `mainline` and one marked
//!   `changeset`, a removed side as `removed`) and, once resolved, the note.
//!   Without conflicts it says `no conflicts`.
//! - `omni changeset resolve <conflict-id> --note <text> [--project <p>]`
//!   sends `POST .../projects/{project_id}/conflicts/{conflict-id}/resolve`
//!   with `{"note": "<text>"}`, answered with `{"conflict": Conflict}`; the
//!   text has the conflict's ID and `resolved`. Without a note (or with a
//!   blank one) or with an ID that is not a UUID it fails before sending
//!   anything.
//! - `--json` prints the API's JSON; errors are one line on standard error
//!   with the API's code, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CLEAN: &str = "01900000-0000-7000-8000-0000000000c1";
const CONFLICTED: &str = "01900000-0000-7000-8000-0000000000c2";
const STUCK: &str = "01900000-0000-7000-8000-0000000000c3";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const MAINLINE_VERSION: &str = "01900000-0000-7000-8000-0000000000e1";
const CHANGESET_VERSION: &str = "01900000-0000-7000-8000-0000000000e2";
const EDITED: &str = "01900000-0000-7000-8000-0000000000f1";
const REMOVED: &str = "01900000-0000-7000-8000-0000000000f2";
const RESOLVED: &str = "01900000-0000-7000-8000-0000000000f3";
const EDITED_NODE: &str = "k3f9x2qa";
const REMOVED_NODE: &str = "m7q2r8tb";
const NOTE: &str = "Kept the ahead wording, added the behind detail.";

fn changesets_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/changesets")
}

fn resolve_path(conflict: &str) -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/conflicts/{conflict}/resolve")
}

fn changeset_json(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "title": "Checkout copy",
        "description": "",
        "status": status,
        "run_tag": "run-7",
        "created_at": "2026-10-15T10:00:00.000000Z",
        "history": [],
        "approved_versions": []
    })
}

fn conflict_json(
    id: &str,
    node: &str,
    mainline: Option<&str>,
    changeset: Option<&str>,
    note: Option<&str>,
) -> Value {
    let side = |version: &str, text: Option<&str>| {
        json!({
            "version_id": version,
            "html": text.map(|t| format!("<p data-omni-id=\"{node}\">{t}</p>")),
        })
    };
    json!({
        "id": id,
        "changeset_id": CONFLICTED,
        "doc_id": DOC,
        "node": node,
        "mainline": side(MAINLINE_VERSION, mainline),
        "changeset": side(CHANGESET_VERSION, changeset),
        "resolution": note.map(|n| json!({ "note": n, "resolved_at": "2026-10-15T11:00:00.000000Z" })),
    })
}

/// Both sides rewrote one paragraph; the conflict is open.
fn edited(note: Option<&str>) -> Value {
    conflict_json(
        EDITED,
        EDITED_NODE,
        Some("Alpha by ahead"),
        Some("Alpha by behind"),
        note,
    )
}

/// The mainline removed a paragraph that the change set rewrote.
fn removed() -> Value {
    conflict_json(REMOVED, REMOVED_NODE, None, Some("Beta by behind"), None)
}

fn rebase_answer(id: &str) -> Value {
    if id == CLEAN {
        json!({ "changeset": changeset_json(CLEAN, "merged"), "conflicts": [] })
    } else {
        json!({ "changeset": changeset_json(CONFLICTED, "open"), "conflicts": [edited(None), removed()] })
    }
}

/// An API with a change set that rebases cleanly, one that rebases into two
/// conflicts and one that cannot be rebased.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let path = request.path.as_str();
        let method = request.method.as_str();
        let changesets = changesets_path();
        for id in [CLEAN, CONFLICTED] {
            if method == "POST" && path == format!("{changesets}/{id}/rebase") {
                return Reply::json(200, &rebase_answer(id));
            }
        }
        if method == "POST" && path == format!("{changesets}/{STUCK}/rebase") {
            return Reply::json(409, &json!({ "error": "invalid_transition" }));
        }
        if method == "GET" && path == format!("{changesets}/{CONFLICTED}/conflicts") {
            return Reply::json(
                200,
                &json!({ "conflicts": [edited(Some(NOTE)), removed()] }),
            );
        }
        if method == "GET" && path == format!("{changesets}/{CLEAN}/conflicts") {
            return Reply::json(200, &json!({ "conflicts": [] }));
        }
        if method == "POST" && path == resolve_path(EDITED) {
            let note = body["note"].as_str().unwrap_or("?");
            return Reply::json(200, &json!({ "conflict": edited(Some(note)) }));
        }
        if method == "POST" && path == resolve_path(RESOLVED) {
            return Reply::json(409, &json!({ "error": "already_resolved" }));
        }
        Reply::json(404, &json!({ "error": "not_found" }))
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
        .args(&["--as", "run-7/author"])
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

fn body_of(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body))
}

fn assert_nothing_sent(server: &MockServer) {
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni changeset rebase
// ---------------------------------------------------------------------------

#[test]
fn rebase_is_an_agent_post_that_prints_the_merged_change_set() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["changeset", "rebase", CLEAN, "--project", PROJECT]);
    assert_eq!(out.code, 0, "{}", out.stderr);

    let path = format!("{}/{CLEAN}/rebase", changesets_path());
    let posts = requests_to(&server, "POST", &path);
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    assert!(
        body_of(&posts[0]).is_object(),
        "the body is a JSON object: {:?}",
        posts[0].body
    );
    assert_eq!(
        posts[0].header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(posts[0].header("x-omni-run"), Some("run-7"));
    assert!(
        out.stdout.contains(CLEAN) && out.stdout.contains("merged"),
        "the text has the change set and its new status: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &["changeset", "rebase", CLEAN, "--project", PROJECT, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        rebase_answer(CLEAN),
        "--json prints the whole response"
    );
}

#[test]
fn a_rebase_with_conflicts_prints_the_open_change_set_and_every_conflict() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["changeset", "rebase", CONFLICTED, "--project", PROJECT],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains(CONFLICTED) && out.stdout.contains("open"),
        "the text has the change set and its new status: {:?}",
        out.stdout
    );
    for conflict in [EDITED, REMOVED] {
        assert!(
            out.stdout.contains(conflict),
            "the text names the conflict {conflict}: {:?}",
            out.stdout
        );
    }
    assert!(
        out.stdout.to_lowercase().contains("conflict"),
        "{:?}",
        out.stdout
    );

    let json = run(
        &home,
        &[
            "changeset",
            "rebase",
            CONFLICTED,
            "--project",
            PROJECT,
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), rebase_answer(CONFLICTED));
}

#[test]
fn a_refused_rebase_is_one_line_with_the_error_code() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["changeset", "rebase", STUCK, "--project", PROJECT]);
    assert_one_line_error(&out, &["invalid_transition"]);
}

#[test]
fn rebase_needs_a_change_set_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["changeset", "rebase", "--project", PROJECT]);
    assert_one_line_error(&out, &["changeset"]);
    let out = run(
        &home,
        &["changeset", "rebase", "not-an-id", "--project", PROJECT],
    );
    assert_one_line_error(&out, &["not-an-id"]);
    assert_nothing_sent(&server);
}

// ---------------------------------------------------------------------------
// omni changeset conflicts
// ---------------------------------------------------------------------------

#[test]
fn conflicts_lists_each_conflict_with_both_sides_and_its_resolution() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["changeset", "conflicts", CONFLICTED, "--project", PROJECT],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        requests_to(
            &server,
            "GET",
            &format!("{}/{CONFLICTED}/conflicts", changesets_path())
        )
        .len(),
        1,
        "{:?}",
        server.requests()
    );

    for needle in [
        EDITED,
        REMOVED,
        EDITED_NODE,
        REMOVED_NODE,
        "Alpha by ahead",
        "Alpha by behind",
        "Beta by behind",
        NOTE,
    ] {
        assert!(
            out.stdout.contains(needle),
            "the text shows {needle:?}: {:?}",
            out.stdout
        );
    }
    let lines = non_empty_lines(&out.stdout);
    let has_line = |label: &str, needle: &str| {
        lines
            .iter()
            .any(|l| l.to_lowercase().contains(label) && l.contains(needle))
    };
    assert!(
        has_line("mainline", "Alpha by ahead"),
        "the mainline side is marked as such: {:?}",
        out.stdout
    );
    assert!(
        has_line("changeset", "Alpha by behind") || has_line("change set", "Alpha by behind"),
        "the change set's side is marked as such: {:?}",
        out.stdout
    );
    assert!(
        has_line("mainline", "removed"),
        "a side that removed the node says so: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &[
            "changeset",
            "conflicts",
            CONFLICTED,
            "--project",
            PROJECT,
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "conflicts": [edited(Some(NOTE)), removed()] }),
        "--json prints the whole response"
    );
}

#[test]
fn conflicts_says_so_when_there_are_none() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["changeset", "conflicts", CLEAN, "--project", PROJECT],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.to_lowercase().contains("no conflicts"),
        "{:?}",
        out.stdout
    );
}

#[test]
fn conflicts_needs_a_change_set_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["changeset", "conflicts", "--project", PROJECT]);
    assert_one_line_error(&out, &["changeset"]);
    let out = run(
        &home,
        &["changeset", "conflicts", "not-an-id", "--project", PROJECT],
    );
    assert_one_line_error(&out, &["not-an-id"]);
    assert_nothing_sent(&server);

    // The control: with an ID, the same command reaches the API.
    let out = run(
        &home,
        &["changeset", "conflicts", CLEAN, "--project", PROJECT],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
}

// ---------------------------------------------------------------------------
// omni changeset resolve
// ---------------------------------------------------------------------------

#[test]
fn resolve_sends_the_note_and_prints_the_resolved_conflict() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "changeset",
            "resolve",
            EDITED,
            "--note",
            NOTE,
            "--project",
            PROJECT,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let posts = requests_to(&server, "POST", &resolve_path(EDITED));
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    assert_eq!(body_of(&posts[0])["note"], NOTE);
    assert_eq!(posts[0].header("x-omni-run"), Some("run-7"));
    assert!(
        out.stdout.contains(EDITED) && out.stdout.to_lowercase().contains("resolved"),
        "the text has the conflict and says it is resolved: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &[
            "changeset",
            "resolve",
            EDITED,
            "--note",
            NOTE,
            "--project",
            PROJECT,
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), json!({ "conflict": edited(Some(NOTE)) }));
}

#[test]
fn resolve_needs_a_note_and_a_conflict_id_before_anything_is_sent() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["changeset", "resolve", EDITED, "--project", PROJECT],
    );
    assert_one_line_error(&out, &["note"]);
    let out = run(
        &home,
        &[
            "changeset",
            "resolve",
            EDITED,
            "--note",
            "   ",
            "--project",
            PROJECT,
        ],
    );
    assert_one_line_error(&out, &["note"]);
    let out = run(
        &home,
        &["changeset", "resolve", "--note", NOTE, "--project", PROJECT],
    );
    assert_one_line_error(&out, &["conflict"]);
    let out = run(
        &home,
        &[
            "changeset",
            "resolve",
            "not-an-id",
            "--note",
            NOTE,
            "--project",
            PROJECT,
        ],
    );
    assert_one_line_error(&out, &["not-an-id"]);
    assert_nothing_sent(&server);
}

#[test]
fn resolving_twice_is_one_line_with_the_error_code() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "changeset",
            "resolve",
            RESOLVED,
            "--note",
            NOTE,
            "--project",
            PROJECT,
        ],
    );
    assert_one_line_error(&out, &["already_resolved"]);
}

#[test]
fn help_lists_the_rebase_and_conflict_commands() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    for command in [
        "changeset rebase",
        "changeset conflicts",
        "changeset resolve",
    ] {
        assert!(
            help.stdout.contains(command),
            "omni --help lists `omni {command}`: {:?}",
            help.stdout
        );
    }
    assert!(
        help.stdout.contains("--note"),
        "omni --help documents --note: {:?}",
        help.stdout
    );
}
