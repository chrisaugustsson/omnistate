//! US-038: `omni comment list`, `omni comment reply`, `omni comment
//! addressed` and `omni version save --remove-commented`, tested as the built
//! binary against a mock HTTP server. None of these tests needs Postgres; the
//! API's contract is in crates/omni-api/tests/comments.rs, and the round trip
//! against the API is in tests/stack_comments.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni comment list` sends `GET /comments` and only reads. `--doc <id>`
//!   asks for one doc's comments (`?doc=<id>`), `--changeset <id>` for one
//!   change set's (`?changeset=<id>`), and without either it lists the whole
//!   project's; `--status open|addressed|resolved` adds `status=<status>`.
//!   `--doc` and `--changeset` together, and a status that is none of the
//!   three, are a one-line error that names the options or the status and
//!   sends nothing. It prints one line per comment on standard output, with
//!   the comment's ID, its status, its node address and its text (the first
//!   line of it); a comment whose node changed in a later version says
//!   `changed` on its line and one whose node was removed says `removed`,
//!   and a comment whose node is current says neither. `--json` prints the
//!   API's JSON.
//! - `omni comment reply <id>` reads the reply's text from `--body-file
//!   <path|->` or, without it, from standard input, and sends `POST
//!   /comments/{id}/replies` with `{"text"}`. It prints a line naming the
//!   comment. A missing ID or a blank text is a one-line error that sends
//!   nothing; an unknown comment (404) is a one-line error with `not_found`.
//! - `omni comment addressed <id> --version <v>` sends `POST
//!   /comments/{id}/addressed` with `{"version_id": <v>}` and prints a line
//!   naming the comment and `addressed`. A missing ID or `--version` is a
//!   one-line error that names what is missing and sends nothing; an answer
//!   of 409 is a one-line error with its code (`invalid_transition`).
//! - Resolving and reopening are the human's (FR-4): `omni comment resolve
//!   <id>` and `omni comment reopen <id>` are a one-line error that sends
//!   nothing.
//! - `omni version save <doc> --changeset <id>` sends a body whose
//!   `remove_commented` is not `true`; with `--remove-commented` it sends
//!   `{"remove_commented": true}`. When the API answers 409
//!   `commented_nodes_removed`, the command exits with a documented non-zero
//!   code, prints nothing on standard output, and lists on standard error
//!   every comment of the answer (its ID, its node and its text) and that
//!   `--remove-commented` saves anyway.
//! - `omni --help` lists the three comment commands and `--remove-commented`.
//! - Every command is an agent request with the agent identity headers, and
//!   errors are one line on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
/// A doc whose save the API refuses: its draft removed commented nodes.
const COMMENTED_DOC: &str = "01900000-0000-7000-8000-0000000000d2";
const V1: &str = "01900000-0000-7000-8000-0000000000f1";
const V2: &str = "01900000-0000-7000-8000-0000000000f2";
const NEW_VERSION: &str = "01900000-0000-7000-8000-0000000000f3";
/// An open comment whose node is current.
const OPEN: &str = "01900000-0000-7000-8000-0000000000a1";
/// An addressed comment whose node changed in a later version.
const CHANGED: &str = "01900000-0000-7000-8000-0000000000a2";
/// An open comment whose node was removed.
const REMOVED: &str = "01900000-0000-7000-8000-0000000000a3";
/// A resolved comment, which the API refuses to mark addressed.
const RESOLVED: &str = "01900000-0000-7000-8000-0000000000a4";
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-0000000000e9";
const NODE_OPEN: &str = "k3f9x2qa";
const NODE_CHANGED: &str = "b7m2c4dd";
const NODE_REMOVED: &str = "r9t5w1ee";
const OPEN_TEXT: &str = "The hero copy runs too long at 320 px";
const CHANGED_TEXT: &str = "Use the brand blue for the button";
const REMOVED_TEXT: &str = "Is this paragraph needed at all?";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn comment_json(id: &str, node: &str, text: &str, status: &str, node_status: &str) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "doc_id": DOC,
        "version_id": V1,
        "node": node,
        "changeset_id": CHANGESET,
        "author": { "kind": "human", "user_id": "01900000-0000-7000-8000-0000000000aa" },
        "text": text,
        "status": status,
        "addressed_in": if status == "addressed" { json!(V2) } else { Value::Null },
        "replies": [],
        "node_status": node_status,
        "compared_to": V2,
        "created_at": "2026-10-21T10:00:00.000000Z"
    })
}

fn all_comments() -> Value {
    json!([
        comment_json(OPEN, NODE_OPEN, OPEN_TEXT, "open", "current"),
        comment_json(CHANGED, NODE_CHANGED, CHANGED_TEXT, "addressed", "changed"),
        comment_json(REMOVED, NODE_REMOVED, REMOVED_TEXT, "open", "removed"),
    ])
}

fn version_json(id: &str) -> Value {
    json!({ "version": {
        "id": id,
        "doc_id": DOC,
        "changeset_id": CHANGESET,
        "draft_id": "01900000-0000-7000-8000-0000000000e2",
        "removed": false,
        "created_at": "2026-10-21T10:00:00.000000Z",
        "agent": { "run": "run-7", "role": "author", "model": null, "harness": null },
        "nodes": []
    } })
}

/// The path without its query, and the query's parameters.
fn split_query(path: &str) -> (String, BTreeMap<String, String>) {
    match path.split_once('?') {
        None => (path.to_string(), BTreeMap::new()),
        Some((path, query)) => (
            path.to_string(),
            query
                .split('&')
                .filter(|pair| !pair.is_empty())
                .map(|pair| {
                    let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                    (name.to_string(), value.to_string())
                })
                .collect(),
        ),
    }
}

/// An API that knows the comments `OPEN`, `CHANGED`, `REMOVED` and
/// `RESOLVED`, saves versions of `DOC` and refuses to save `COMMENTED_DOC`
/// without `remove_commented`.
fn mock() -> MockServer {
    MockServer::start(move |request| {
        let (path, _) = split_query(&request.path);
        let method = request.method.as_str();
        if method == "GET" && path == format!("/api/v1/w/{WORKSPACE}/projects") {
            return Reply::json(
                200,
                &json!({ "projects": [{
                    "id": PROJECT,
                    "name": "brand-site",
                    "created_at": "2026-10-15T10:00:00.000000Z"
                }] }),
            );
        }
        let Some(rest) = path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        match (method, rest) {
            ("GET", "/comments") => Reply::json(
                200,
                &json!({ "project_id": PROJECT, "comments": all_comments() }),
            ),
            ("POST", r) if r == format!("/comments/{OPEN}/replies") => {
                let mut comment = comment_json(OPEN, NODE_OPEN, OPEN_TEXT, "open", "current");
                comment["replies"] = json!([{
                    "id": "01900000-0000-7000-8000-0000000000b9",
                    "author": { "kind": "agent", "run": "run-7", "role": "author", "model": null, "harness": null },
                    "text": body["text"],
                    "created_at": "2026-10-21T10:05:00.000000Z"
                }]);
                Reply::json(201, &json!({ "comment": comment }))
            }
            ("POST", r) if r == format!("/comments/{OPEN}/addressed") => {
                let mut comment = comment_json(OPEN, NODE_OPEN, OPEN_TEXT, "addressed", "current");
                comment["addressed_in"] = body["version_id"].clone();
                Reply::json(200, &json!({ "comment": comment }))
            }
            ("POST", r) if r == format!("/comments/{RESOLVED}/addressed") => {
                Reply::json(409, &json!({ "error": "invalid_transition" }))
            }
            ("POST", r) if r == format!("/changesets/{CHANGESET}/docs/{DOC}/versions") => {
                Reply::json(201, &version_json(NEW_VERSION))
            }
            ("POST", r)
                if r == format!("/changesets/{CHANGESET}/docs/{COMMENTED_DOC}/versions") =>
            {
                if body["remove_commented"] == true {
                    Reply::json(201, &version_json(NEW_VERSION))
                } else {
                    Reply::json(
                        409,
                        &json!({
                            "error": "commented_nodes_removed",
                            "comments": [
                                comment_json(OPEN, NODE_OPEN, OPEN_TEXT, "open", "current"),
                                comment_json(REMOVED, NODE_REMOVED, REMOVED_TEXT, "open", "current"),
                            ]
                        }),
                    )
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

fn run_with_stdin(home: &Scratch, args: &[&str], stdin: &[u8]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--project", PROJECT, "--as", "run-7/author"])
        .stdin(stdin)
        .run()
}

/// The requests other than the project lookup.
fn requests(server: &MockServer) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| {
            !r.path
                .starts_with(&format!("/api/v1/w/{WORKSPACE}/projects?"))
        })
        .filter(|r| r.path != format!("/api/v1/w/{WORKSPACE}/projects"))
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

fn body_of(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body))
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

fn assert_contains_all(text: &str, needles: &[&str], what: &str) {
    for needle in needles {
        assert!(
            text.contains(needle),
            "{what} should contain {needle:?}: {text:?}"
        );
    }
}

/// The one request that the command sent, besides the project lookup.
fn only_request(server: &MockServer) -> Recorded {
    let sent = requests(server);
    assert_eq!(sent.len(), 1, "one request: {sent:?}");
    assert_agent_request(&sent[0]);
    sent[0].clone()
}

// ---------------------------------------------------------------------------
// omni comment list
// ---------------------------------------------------------------------------

#[test]
fn list_asks_for_the_projects_comments_and_prints_one_line_each() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["comment", "list"]);
    assert_eq!(out.code, 0, "comment list: {}", out.stderr);

    let sent = only_request(&server);
    assert_eq!(sent.method, "GET");
    let (path, query) = split_query(&sent.path);
    assert_eq!(path, format!("{}/comments", base()));
    assert!(
        !query.contains_key("doc") && !query.contains_key("changeset"),
        "the whole project: {query:?}"
    );

    let lines = non_empty_lines(&out.stdout);
    assert_eq!(lines.len(), 3, "one line per comment: {:?}", out.stdout);
    let line = |id: &str| -> &str {
        lines
            .iter()
            .copied()
            .find(|l| l.contains(id))
            .unwrap_or_else(|| panic!("a line for {id}: {:?}", out.stdout))
    };
    assert_contains_all(
        line(OPEN),
        &["open", NODE_OPEN, OPEN_TEXT],
        "the open comment",
    );
    assert_contains_all(
        line(CHANGED),
        &["addressed", NODE_CHANGED, CHANGED_TEXT, "changed"],
        "the comment on a changed node",
    );
    assert_contains_all(
        line(REMOVED),
        &["open", NODE_REMOVED, REMOVED_TEXT, "removed"],
        "the comment on a removed node",
    );
    assert!(
        !line(OPEN).contains("changed") && !line(OPEN).contains("removed"),
        "a comment on a current node is not marked: {:?}",
        line(OPEN)
    );
}

#[test]
fn list_narrows_to_a_doc_or_a_change_set_and_a_status() {
    let server = mock();
    let home = logged_in(&server.url);
    for (args, name, value) in [
        (vec!["comment", "list", "--doc", DOC], "doc", DOC),
        (
            vec!["comment", "list", "--changeset", CHANGESET],
            "changeset",
            CHANGESET,
        ),
    ] {
        let out = run(&home, &args);
        assert_eq!(out.code, 0, "{args:?}: {}", out.stderr);
        let sent = requests(&server);
        let (path, query) = split_query(&sent.last().expect("a request").path);
        assert_eq!(path, format!("{}/comments", base()), "{args:?}");
        assert_eq!(query.get(name).map(String::as_str), Some(value), "{args:?}");
    }

    let out = run(
        &home,
        &[
            "comment",
            "list",
            "--changeset",
            CHANGESET,
            "--status",
            "open",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = requests(&server);
    let (_, query) = split_query(&sent.last().expect("a request").path);
    assert_eq!(
        query.get("changeset").map(String::as_str),
        Some(CHANGESET),
        "{query:?}"
    );
    assert_eq!(
        query.get("status").map(String::as_str),
        Some("open"),
        "{query:?}"
    );
    assert!(sent.iter().all(|r| r.method == "GET"), "list only reads");
}

#[test]
fn list_with_json_prints_the_apis_answer() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["comment", "list", "--doc", DOC, "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.json(),
        json!({ "project_id": PROJECT, "comments": all_comments() })
    );
}

#[test]
fn list_refuses_a_doc_and_a_change_set_together_and_a_status_that_is_none() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["comment", "list", "--doc", DOC, "--changeset", CHANGESET],
    );
    assert_one_line_error(&out, &["--doc", "--changeset"]);
    let out = run(&home, &["comment", "list", "--status", "done"]);
    assert_one_line_error(&out, &["done"]);
    assert!(requests(&server).is_empty(), "nothing was sent");
}

// ---------------------------------------------------------------------------
// omni comment reply
// ---------------------------------------------------------------------------

#[test]
fn reply_sends_the_text_from_a_file_or_standard_input() {
    let server = mock();
    let home = logged_in(&server.url);
    let file = home.join("reply.md");
    std::fs::write(&file, "On phones the copy stacks above the shot.\n").expect("write the reply");
    let out = run(
        &home,
        &[
            "comment",
            "reply",
            OPEN,
            "--body-file",
            file.to_str().expect("a UTF-8 path"),
        ],
    );
    assert_eq!(out.code, 0, "comment reply: {}", out.stderr);
    assert!(
        out.stdout.contains(OPEN),
        "names the comment: {:?}",
        out.stdout
    );
    let sent = only_request(&server);
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, format!("{}/comments/{OPEN}/replies", base()));
    assert_eq!(
        body_of(&sent)["text"].as_str().map(str::trim),
        Some("On phones the copy stacks above the shot."),
        "{:?}",
        body_of(&sent)
    );

    let out = run_with_stdin(
        &home,
        &["comment", "reply", OPEN],
        b"Fixed in the next version.",
    );
    assert_eq!(out.code, 0, "comment reply from stdin: {}", out.stderr);
    let sent = requests(&server);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(
        body_of(&sent[1])["text"].as_str().map(str::trim),
        Some("Fixed in the next version.")
    );
}

#[test]
fn reply_needs_an_id_and_a_text() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run_with_stdin(&home, &["comment", "reply"], b"Hello");
    assert_one_line_error(&out, &["omni comment reply"]);
    let out = run_with_stdin(&home, &["comment", "reply", OPEN], b"  \n ");
    assert_one_line_error(&out, &["text"]);
    assert!(requests(&server).is_empty(), "nothing was sent");
}

#[test]
fn reply_to_an_unknown_comment_is_one_line_with_not_found() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run_with_stdin(&home, &["comment", "reply", UNKNOWN_ID], b"Hello");
    assert_one_line_error(&out, &["not_found"]);
}

// ---------------------------------------------------------------------------
// omni comment addressed
// ---------------------------------------------------------------------------

#[test]
fn addressed_names_the_version_that_fixes_the_comment() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["comment", "addressed", OPEN, "--version", V2]);
    assert_eq!(out.code, 0, "comment addressed: {}", out.stderr);
    assert_contains_all(&out.stdout, &[OPEN, "addressed"], "the output");
    let sent = only_request(&server);
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, format!("{}/comments/{OPEN}/addressed", base()));
    assert_eq!(body_of(&sent), json!({ "version_id": V2 }));

    let out = run(
        &home,
        &["comment", "addressed", OPEN, "--version", V2, "--json"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json()["comment"]["addressed_in"], V2, "{}", out.stdout);
}

#[test]
fn addressed_needs_an_id_and_a_version() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["comment", "addressed", OPEN]);
    assert_one_line_error(&out, &["--version"]);
    let out = run(&home, &["comment", "addressed", "--version", V2]);
    assert_one_line_error(&out, &["omni comment addressed"]);
    assert!(requests(&server).is_empty(), "nothing was sent");
}

#[test]
fn addressed_that_the_api_refuses_is_one_line_with_its_error_code() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["comment", "addressed", RESOLVED, "--version", V2]);
    assert_one_line_error(&out, &["invalid_transition"]);
}

#[test]
fn resolving_and_reopening_are_the_humans_and_send_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    for action in ["resolve", "reopen"] {
        let out = run(&home, &["comment", action, OPEN]);
        assert_one_line_error(&out, &["human"]);
    }
    assert!(requests(&server).is_empty(), "nothing was sent");
}

// ---------------------------------------------------------------------------
// omni version save --remove-commented
// ---------------------------------------------------------------------------

#[test]
fn version_save_sends_remove_commented_only_when_asked() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["version", "save", DOC, "--changeset", CHANGESET]);
    assert_eq!(out.code, 0, "version save: {}", out.stderr);
    let out = run(
        &home,
        &[
            "version",
            "save",
            DOC,
            "--changeset",
            CHANGESET,
            "--remove-commented",
        ],
    );
    assert_eq!(
        out.code, 0,
        "version save --remove-commented: {}",
        out.stderr
    );
    assert!(out.stdout.contains(NEW_VERSION), "{:?}", out.stdout);

    let sent = requests(&server);
    assert_eq!(sent.len(), 2, "{sent:?}");
    let plain = body_of(&sent[0]);
    assert!(plain.is_object(), "{plain}");
    assert_ne!(plain["remove_commented"], true, "{plain}");
    assert_eq!(body_of(&sent[1]), json!({ "remove_commented": true }));
}

#[test]
fn version_save_that_would_remove_commented_nodes_lists_those_comments() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["version", "save", COMMENTED_DOC, "--changeset", CHANGESET],
    );
    assert_ne!(out.code, 0, "the save is refused; stdout: {:?}", out.stdout);
    assert!(
        documented_exit_codes().contains(&out.code),
        "exit code {} is not documented under \"Exit codes\" in omni --help",
        out.code
    );
    assert_eq!(out.stdout, "", "nothing on standard output");
    assert_contains_all(
        &out.stderr,
        &[
            "--remove-commented",
            OPEN,
            NODE_OPEN,
            OPEN_TEXT,
            REMOVED,
            NODE_REMOVED,
            REMOVED_TEXT,
        ],
        "standard error",
    );

    let out = run(
        &home,
        &[
            "version",
            "save",
            COMMENTED_DOC,
            "--changeset",
            CHANGESET,
            "--remove-commented",
        ],
    );
    assert_eq!(out.code, 0, "with --remove-commented: {}", out.stderr);
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

#[test]
fn help_lists_the_comment_commands_and_remove_commented() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    assert_contains_all(
        &help.stdout,
        &[
            "omni comment list",
            "omni comment reply",
            "omni comment addressed",
            "--remove-commented",
        ],
        "omni --help",
    );
}
