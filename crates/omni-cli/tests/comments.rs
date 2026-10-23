//! US-106: `omni comment list` and `omni comment reply` on comments that the
//! human pins to nodes of live docs, tested as the built binary against a
//! mock HTTP server. None of these tests needs Postgres; the API's contract is
//! in crates/omni-api/tests/comments.rs, and the round trip against the API is
//! in tests/stack_comments.rs. (`omni version save --remove-commented` keeps
//! its US-038 tests here until US-107 removes it.)
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni comment list [--doc <id>] [--feature <name or id>]` sends `GET
//!   /comments` and only reads. `--doc <id>` adds `doc=<id>`; `--feature`
//!   adds `feature=<id>`, a name being looked up (ignoring case) in `GET
//!   /features` as for `omni doc list`, and a name that no feature has is a
//!   one-line error that sends no `GET /comments`. It prints every comment in
//!   the order of the answer (oldest first), each with its ID, its doc's ID,
//!   its node address and its text, followed by the text of each of its
//!   replies, oldest first, before the next comment starts. A comment whose
//!   `waiting_on_you` is `true` says `waiting on you`, and no other does.
//!   `--json` prints the API's JSON.
//! - `omni comment reply <id> <text>` sends `POST /comments/{id}/replies` with
//!   `{"text": <text>}` and prints a line naming the comment; `--json` prints
//!   the API's answer. A missing ID or a blank text is a one-line error that
//!   sends nothing; an unknown comment (404) is a one-line error with
//!   `not_found`.
//! - Comments have no status: `omni comment addressed`, `omni comment
//!   resolve` and `omni comment reopen` are a one-line error that sends
//!   nothing, and `omni --help` lists `omni comment list` and `omni comment
//!   reply` but none of them.
//! - Every command is an agent request with the agent identity headers, and
//!   errors are one line on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const OTHER_DOC: &str = "01900000-0000-7000-8000-0000000000d3";
/// A doc whose save the API refuses: its draft removed commented nodes.
const COMMENTED_DOC: &str = "01900000-0000-7000-8000-0000000000d2";
const NEW_VERSION: &str = "01900000-0000-7000-8000-0000000000f3";
const CHECKOUT: &str = "01900000-0000-7000-8000-0000000000e1";
/// The oldest comment: an agent replied, then the human.
const FIRST: &str = "01900000-0000-7000-8000-0000000000a1";
/// An agent replied last: it waits on the human.
const SECOND: &str = "01900000-0000-7000-8000-0000000000a2";
/// The newest comment, without replies.
const THIRD: &str = "01900000-0000-7000-8000-0000000000a3";
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-0000000000e9";
const NODE_FIRST: &str = "k3f9x2qa";
const NODE_SECOND: &str = "b7m2c4dd/r9t5w1ee";
const NODE_THIRD: &str = "h4p8n6zz";
const FIRST_TEXT: &str = "The hero copy runs too long at 320 px";
const SECOND_TEXT: &str = "Use the brand blue for this button label";
const THIRD_TEXT: &str = "Is this paragraph needed at all?";
const FIRST_AGENT_REPLY: &str = "Shortened it to one line.";
const FIRST_HUMAN_REPLY: &str = "Shorter still, please.";
const SECOND_AGENT_REPLY: &str = "The label now uses color.brand.";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn agent_author() -> Value {
    json!({ "kind": "agent", "run": "run-7", "role": "author", "model": null, "harness": null })
}

fn human_author() -> Value {
    json!({ "kind": "human", "user_id": "01900000-0000-7000-8000-0000000000aa" })
}

fn reply_json(id: &str, author: Value, text: &str) -> Value {
    json!({
        "id": id,
        "author": author,
        "text": text,
        "created_at": "2026-10-23T10:05:00.000000Z"
    })
}

fn comment_json(
    id: &str,
    doc: &str,
    node: &str,
    text: &str,
    replies: Vec<Value>,
    waiting: bool,
) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "doc_id": doc,
        "node": node,
        "feature_id": null,
        "author": human_author(),
        "text": text,
        "replies": replies,
        "waiting_on_you": waiting,
        "created_at": "2026-10-23T10:00:00.000000Z"
    })
}

fn all_comments() -> Value {
    json!([
        comment_json(
            FIRST,
            DOC,
            NODE_FIRST,
            FIRST_TEXT,
            vec![
                reply_json(
                    "01900000-0000-7000-8000-0000000000b5",
                    agent_author(),
                    FIRST_AGENT_REPLY
                ),
                reply_json(
                    "01900000-0000-7000-8000-0000000000b6",
                    human_author(),
                    FIRST_HUMAN_REPLY
                ),
            ],
            false,
        ),
        comment_json(
            SECOND,
            OTHER_DOC,
            NODE_SECOND,
            SECOND_TEXT,
            vec![reply_json(
                "01900000-0000-7000-8000-0000000000b7",
                agent_author(),
                SECOND_AGENT_REPLY
            )],
            true,
        ),
        comment_json(THIRD, DOC, NODE_THIRD, THIRD_TEXT, vec![], false),
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

/// An API that knows the comments `FIRST`, `SECOND` and `THIRD` and the
/// feature Checkout, saves versions of `DOC` and refuses to save
/// `COMMENTED_DOC` without `remove_commented`.
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
            ("GET", "/features") => Reply::json(
                200,
                &json!({ "features": [{
                    "id": CHECKOUT,
                    "project_id": PROJECT,
                    "name": "Checkout",
                    "description": "Pay on one page",
                    "created_at": "2026-10-20T10:00:00.000000Z"
                }] }),
            ),
            ("GET", "/comments") => Reply::json(
                200,
                &json!({ "project_id": PROJECT, "comments": all_comments() }),
            ),
            ("POST", r) if r == format!("/comments/{THIRD}/replies") => {
                let comment = comment_json(
                    THIRD,
                    DOC,
                    NODE_THIRD,
                    THIRD_TEXT,
                    vec![reply_json(
                        "01900000-0000-7000-8000-0000000000b9",
                        agent_author(),
                        body["text"].as_str().unwrap_or_default(),
                    )],
                    true,
                );
                Reply::json(201, &json!({ "comment": comment }))
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
                                comment_json(FIRST, COMMENTED_DOC, NODE_FIRST, FIRST_TEXT, vec![], false),
                                comment_json(THIRD, COMMENTED_DOC, NODE_THIRD, THIRD_TEXT, vec![], false),
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

/// The requests for the comments, leaving out the project and feature
/// lookups.
fn comment_requests(server: &MockServer) -> Vec<Recorded> {
    requests(server)
        .into_iter()
        .filter(|r| split_query(&r.path).0 != format!("{}/features", base()))
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

/// Where `needle` starts in `text`.
fn position(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("the output has {needle:?}: {text:?}"))
}

// ---------------------------------------------------------------------------
// omni comment list
// ---------------------------------------------------------------------------

#[test]
fn list_shows_each_comments_doc_node_text_and_replies_oldest_first() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["comment", "list"]);
    assert_eq!(out.code, 0, "comment list: {}", out.stderr);

    let sent = only_request(&server);
    assert_eq!(sent.method, "GET");
    let (path, query) = split_query(&sent.path);
    assert_eq!(path, format!("{}/comments", base()));
    assert!(query.is_empty(), "the whole project: {query:?}");

    let text = &out.stdout;
    assert_contains_all(
        text,
        &[DOC, OTHER_DOC, NODE_FIRST, NODE_SECOND, NODE_THIRD],
        "the list",
    );
    // Each comment, then its replies, oldest first, then the next comment.
    let order = [
        FIRST,
        FIRST_TEXT,
        FIRST_AGENT_REPLY,
        FIRST_HUMAN_REPLY,
        SECOND,
        SECOND_TEXT,
        SECOND_AGENT_REPLY,
        THIRD,
        THIRD_TEXT,
    ];
    let positions: Vec<usize> = order.iter().map(|needle| position(text, needle)).collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "every comment with its replies, oldest first: {text:?}"
    );
    // Each comment's part of the output names its own doc and node.
    let first = &text[positions[0]..positions[4]];
    let second = &text[positions[4]..positions[7]];
    let third = &text[positions[7]..];
    assert_contains_all(first, &[DOC, NODE_FIRST], "the first comment");
    assert_contains_all(second, &[OTHER_DOC, NODE_SECOND], "the second comment");
    assert_contains_all(third, &[DOC, NODE_THIRD], "the third comment");
}

#[test]
fn list_says_waiting_on_you_only_for_a_comment_an_agent_replied_to_last() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["comment", "list"]);
    assert_eq!(out.code, 0, "comment list: {}", out.stderr);
    let text = &out.stdout;
    let (first, second, third) = (
        position(text, FIRST),
        position(text, SECOND),
        position(text, THIRD),
    );
    assert!(
        text[second..third].contains("waiting on you"),
        "the comment an agent replied to last waits: {text:?}"
    );
    assert!(
        !text[first..second].contains("waiting on you")
            && !text[third..].contains("waiting on you"),
        "no other comment says so: {text:?}"
    );
}

#[test]
fn list_narrows_to_a_doc_or_a_feature_by_name_or_id() {
    let server = mock();
    let home = logged_in(&server.url);
    for (args, name, value) in [
        (vec!["comment", "list", "--doc", DOC], "doc", DOC),
        (
            vec!["comment", "list", "--feature", "checkout"],
            "feature",
            CHECKOUT,
        ),
        (
            vec!["comment", "list", "--feature", CHECKOUT],
            "feature",
            CHECKOUT,
        ),
    ] {
        let out = run(&home, &args);
        assert_eq!(out.code, 0, "{args:?}: {}", out.stderr);
        let sent = comment_requests(&server);
        let last = sent.last().expect("a request");
        assert_agent_request(last);
        let (path, query) = split_query(&last.path);
        assert_eq!(path, format!("{}/comments", base()), "{args:?}");
        assert_eq!(query.get(name).map(String::as_str), Some(value), "{args:?}");
    }
    assert!(
        requests(&server).iter().all(|r| r.method == "GET"),
        "list only reads"
    );
}

#[test]
fn list_of_a_feature_that_does_not_exist_is_one_line_and_asks_for_no_comments() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["comment", "list", "--feature", "Search"]);
    assert_one_line_error(&out, &["Search"]);
    assert!(
        comment_requests(&server).is_empty(),
        "no comments were asked for: {:?}",
        requests(&server)
    );
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

// ---------------------------------------------------------------------------
// omni comment reply
// ---------------------------------------------------------------------------

#[test]
fn reply_sends_the_text_given_after_the_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "comment",
            "reply",
            THIRD,
            "It is: it says who the hero is for.",
        ],
    );
    assert_eq!(out.code, 0, "comment reply: {}", out.stderr);
    assert!(
        out.stdout.contains(THIRD),
        "names the comment: {:?}",
        out.stdout
    );
    let sent = only_request(&server);
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, format!("{}/comments/{THIRD}/replies", base()));
    assert_eq!(
        body_of(&sent),
        json!({ "text": "It is: it says who the hero is for." })
    );

    let out = run(&home, &["comment", "reply", THIRD, "Done.", "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.json()["comment"]["replies"][0]["text"],
        "Done.",
        "{}",
        out.stdout
    );
}

#[test]
fn reply_needs_an_id_and_a_text() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["comment", "reply"]);
    assert_one_line_error(&out, &["omni comment reply"]);
    let out = run(&home, &["comment", "reply", THIRD, "  \n "]);
    assert_one_line_error(&out, &["text"]);
    assert!(requests(&server).is_empty(), "nothing was sent");
}

#[test]
fn reply_to_an_unknown_comment_is_one_line_with_not_found() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["comment", "reply", UNKNOWN_ID, "Hello"]);
    assert_one_line_error(&out, &["not_found"]);
}

// ---------------------------------------------------------------------------
// No statuses
// ---------------------------------------------------------------------------

#[test]
fn the_status_commands_are_gone_and_send_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        vec!["comment", "addressed", FIRST, "--version", NEW_VERSION],
        vec!["comment", "resolve", FIRST],
        vec!["comment", "reopen", FIRST],
    ] {
        let out = run(&home, &args);
        assert_one_line_error(&out, &[]);
    }
    assert!(requests(&server).is_empty(), "nothing was sent");
}

#[test]
fn help_lists_list_and_reply_and_no_status_command() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    assert_contains_all(
        &help.stdout,
        &["omni comment list", "omni comment reply"],
        "omni --help",
    );
    for gone in [
        "omni comment addressed",
        "omni comment resolve",
        "omni comment reopen",
    ] {
        assert!(
            !help.stdout.contains(gone),
            "omni --help no longer lists {gone:?}: {:?}",
            help.stdout
        );
    }
}

// ---------------------------------------------------------------------------
// omni version save --remove-commented (US-038, until US-107)
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
            FIRST,
            NODE_FIRST,
            FIRST_TEXT,
            THIRD,
            NODE_THIRD,
            THIRD_TEXT,
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
