//! US-024: `omni version save`, `omni version diff` and `omni doc remove`,
//! tested as the built binary against a mock HTTP server. None of these tests
//! needs Postgres; the real round trip against the API is in
//! tests/stack_docs.rs, and the API's contract in
//! crates/omni-api/tests/versions.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni version save <doc> --changeset <id>` sends `POST
//!   /changesets/{changeset}/docs/{doc}/versions` with a JSON object body and
//!   prints the new version's ID. A change set without a draft of the doc
//!   (404) is a one-line error that names the doc.
//! - `omni version diff <a> <b>` sends `GET /versions/{a}/diff/{b}` and prints
//!   each listed node on a line of its own, as plain text: the word `added`,
//!   `removed`, `changed` or `moved`, the node's ID and its type; a moved
//!   node's line also has the parent it moved to. An unknown version (404) is
//!   a one-line error that mentions the version.
//! - `omni doc remove <doc> --changeset <id>` sends `POST
//!   /changesets/{changeset}/docs/{doc}/remove` with a JSON object body and
//!   prints the ID of the version that marks the doc removed, and that it is
//!   removed.
//! - A missing doc, change set or version is a one-line error that sends
//!   nothing. Every command is an agent request with the agent identity
//!   headers, `--json` prints the API's JSON, and errors are one line on
//!   standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
/// A change set that has no draft of `DOC`.
const OTHER_CHANGESET: &str = "01900000-0000-7000-8000-0000000000c2";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const DRAFT: &str = "01900000-0000-7000-8000-0000000000e2";
const V1: &str = "01900000-0000-7000-8000-0000000000f1";
const V2: &str = "01900000-0000-7000-8000-0000000000f2";
const REMOVAL: &str = "01900000-0000-7000-8000-0000000000f3";
const UNKNOWN_VERSION: &str = "01900000-0000-7000-8000-000000000000";
const SECTION: &str = "s3ct10na";
const ADDED: &str = "add3d0n3";
const REMOVED: &str = "r3m0v3d0";
const CHANGED: &str = "chang3d0";
const MOVED: &str = "m0v3d000";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn node(id: &str, kind: &str, parent: Option<&str>) -> Value {
    json!({ "id": id, "type": kind, "parent": parent, "fingerprint": format!("fp-{id}") })
}

fn version_json(id: &str, removed: bool) -> Value {
    let nodes = if removed {
        json!([])
    } else {
        json!([
            node(SECTION, "section", None),
            node(CHANGED, "paragraph", Some(SECTION)),
            node(MOVED, "paragraph", Some(SECTION)),
        ])
    };
    json!({
        "id": id,
        "doc_id": DOC,
        "changeset_id": CHANGESET,
        "draft_id": DRAFT,
        "removed": removed,
        "created_at": "2026-10-15T10:00:00.000000Z",
        "agent": { "run": "run-7", "role": "author", "model": "test-model", "harness": "test-harness" },
        "nodes": nodes
    })
}

fn diff_json() -> Value {
    json!({ "diff": {
        "doc_id": DOC,
        "from": V1,
        "to": V2,
        "added": [node(ADDED, "paragraph", None)],
        "removed": [node(REMOVED, "heading", Some(SECTION))],
        "changed": [node(CHANGED, "paragraph", Some(SECTION))],
        "moved": [{ "id": MOVED, "type": "list", "from_parent": null, "to_parent": SECTION }]
    } })
}

/// An API where `CHANGESET` has a draft of `DOC` and `OTHER_CHANGESET` has
/// none; `V1` and `V2` are versions of `DOC`, and no other version exists.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let path = request.path.as_str();
        let method = request.method.as_str();
        let Some(rest) = path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        let draft = format!("/changesets/{CHANGESET}/docs/{DOC}");
        match (method, rest) {
            ("POST", r) if r == format!("{draft}/versions") => {
                Reply::json(201, &json!({ "version": version_json(V2, false) }))
            }
            ("POST", r) if r == format!("{draft}/remove") => {
                Reply::json(201, &json!({ "version": version_json(REMOVAL, true) }))
            }
            ("GET", r) if r == format!("/versions/{V1}/diff/{V2}") => {
                Reply::json(200, &diff_json())
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

fn assert_object_body(request: &Recorded) {
    let body: Value = serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body));
    assert!(body.is_object(), "the body is a JSON object: {body}");
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
    for needle in needles {
        assert!(
            lines[0].contains(needle),
            "the error line should contain {needle:?}: {:?}",
            lines[0]
        );
    }
}

fn draft_path() -> String {
    format!("{}/changesets/{CHANGESET}/docs/{DOC}", base())
}

// ---------------------------------------------------------------------------
// omni version save
// ---------------------------------------------------------------------------

#[test]
fn version_save_saves_the_change_sets_draft_and_prints_the_version() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["version", "save", DOC, "--changeset", CHANGESET]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests_to(&server, "POST", &format!("{}/versions", draft_path()));
    assert_eq!(sent.len(), 1, "{:?}", server.requests());
    assert_agent_request(&sent[0]);
    assert_object_body(&sent[0]);
    assert!(
        out.stdout.contains(V2),
        "save prints the new version's ID: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &["version", "save", DOC, "--changeset", CHANGESET, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), json!({ "version": version_json(V2, false) }));
}

#[test]
fn version_save_without_a_draft_in_the_change_set_is_one_line_that_names_the_doc() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["version", "save", DOC, "--changeset", OTHER_CHANGESET],
    );
    assert_one_line_error(&out, &[DOC]);
    let sent = requests_to(
        &server,
        "POST",
        &format!(
            "{}/changesets/{OTHER_CHANGESET}/docs/{DOC}/versions",
            base()
        ),
    );
    assert_eq!(sent.len(), 1, "the API was asked: {:?}", server.requests());
}

// ---------------------------------------------------------------------------
// omni version diff
// ---------------------------------------------------------------------------

#[test]
fn version_diff_prints_each_added_removed_changed_and_moved_node_on_its_own_line() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["version", "diff", V1, V2]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests_to(
        &server,
        "GET",
        &format!("{}/versions/{V1}/diff/{V2}", base()),
    );
    assert_eq!(sent.len(), 1, "{:?}", server.requests());
    assert_agent_request(&sent[0]);

    let lines = non_empty_lines(&out.stdout);
    for (word, id, kind) in [
        ("added", ADDED, "paragraph"),
        ("removed", REMOVED, "heading"),
        ("changed", CHANGED, "paragraph"),
        ("moved", MOVED, "list"),
    ] {
        let found: Vec<&&str> = lines.iter().filter(|l| l.contains(id)).collect();
        assert_eq!(
            found.len(),
            1,
            "{id} is on exactly one line: {:?}",
            out.stdout
        );
        let line = found[0];
        assert!(
            line.contains(word) && line.contains(kind),
            "{id}'s line says it is {word} and a {kind}: {line:?}"
        );
        assert!(
            !line.contains('{') && !line.contains("\"id\""),
            "the line is plain text, not JSON: {line:?}"
        );
    }
    let moved = lines.iter().find(|l| l.contains(MOVED)).unwrap();
    assert!(
        moved.contains(SECTION),
        "a moved node's line has the parent it moved to: {moved:?}"
    );

    let json = run(&home, &["version", "diff", V1, V2, "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), diff_json());
}

#[test]
fn version_diff_of_an_unknown_version_is_one_line() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["version", "diff", V1, UNKNOWN_VERSION]);
    assert_one_line_error(&out, &["version"]);
    let sent = requests_to(
        &server,
        "GET",
        &format!("{}/versions/{V1}/diff/{UNKNOWN_VERSION}", base()),
    );
    assert_eq!(sent.len(), 1, "the API was asked: {:?}", server.requests());
}

// ---------------------------------------------------------------------------
// omni doc remove
// ---------------------------------------------------------------------------

#[test]
fn doc_remove_saves_a_removal_and_prints_its_version() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "remove", DOC, "--changeset", CHANGESET]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests_to(&server, "POST", &format!("{}/remove", draft_path()));
    assert_eq!(sent.len(), 1, "{:?}", server.requests());
    assert_agent_request(&sent[0]);
    assert_object_body(&sent[0]);
    assert!(
        out.stdout.contains(REMOVAL) && out.stdout.contains("removed"),
        "remove prints the removal's version and that the doc is removed: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &["doc", "remove", DOC, "--changeset", CHANGESET, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "version": version_json(REMOVAL, true) })
    );
}

// ---------------------------------------------------------------------------
// Arguments and help
// ---------------------------------------------------------------------------

#[test]
fn the_version_commands_need_their_doc_change_set_and_versions() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        vec!["version", "save", DOC],
        vec!["version", "save", "--changeset", CHANGESET],
        vec!["version", "diff", V1],
        vec!["version", "diff"],
        vec!["doc", "remove", DOC],
        vec!["doc", "remove", "--changeset", CHANGESET],
    ] {
        let out = run(&home, &args);
        assert_one_line_error(&out, &[]);
    }
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );

    // The control: with every argument, each command does send its request.
    for args in [
        vec!["version", "save", DOC, "--changeset", CHANGESET],
        vec!["version", "diff", V1, V2],
        vec!["doc", "remove", DOC, "--changeset", CHANGESET],
    ] {
        let out = run(&home, &args);
        assert_eq!(out.code, 0, "{args:?}: {}", out.stderr);
    }
    assert_eq!(server.requests().len(), 3, "{:?}", server.requests());
}
