//! US-034: `omni post`, and the directives that the API gives an agent with
//! its requests, tested as the built binary against a mock HTTP server. None
//! of these tests needs Postgres; the API's contract is in
//! crates/omni-api/tests/posts.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni post <thread> --summary <text> [<body>] [--body-file <path>]
//!   [--attach <file>]... [--link <target>]... [--confidence <n>] [--replaces
//!   <post>]` sends `POST /threads/{thread}/posts` with `{"summary", "body"}`
//!   and the options that were given, and no `type` (US-108 removed post
//!   types). The body is the `<body>` argument, else the text of
//!   `--body-file`, or else all of standard input.
//! - Each `--attach <file>` is first uploaded as `omni attach` does it (US-019:
//!   `POST /api/v1/w/{workspace}/attachments` with the file's bytes and
//!   `X-Omni-Filename`), and the post's `attachments` lists the new IDs in the
//!   order the options were given.
//! - `--link` takes `post:<id>`, `attachment:<id>`, `node:<doc_id>/<address>`
//!   (the address may hold a `/` itself) or an http or https address, and
//!   sends `{"kind": "post", "post_id"}`, `{"kind": "attachment",
//!   "attachment_id"}`, `{"kind": "node", "doc_id", "address"}` or `{"kind":
//!   "url", "url"}` in `links`, in order. `--confidence` sends a number and
//!   `--replaces` the ID of the post it replaces.
//! - It prints the new post's ID; `--json` prints the API's JSON.
//! - `--type` is an unknown option. A missing `--summary`, a `--confidence`
//!   that is no number, a
//!   `--link` of another form, or a body or attachment file that cannot be
//!   read is a one-line error (naming the option or the file) that sends
//!   nothing: no upload and no post. A thread that the API does not know (404)
//!   is a one-line error that names the thread.
//! - When the JSON answer of any request has a `directives` list, the CLI
//!   prints each directive on standard error, one line per directive with its
//!   summary, and standard output stays as it is without directives.
//! - Every command is an agent request with the agent identity headers, and
//!   errors are one line on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const THREAD: &str = "01900000-0000-7000-8000-0000000000e1";
const UNKNOWN_THREAD: &str = "01900000-0000-7000-8000-0000000000e9";
const POST: &str = "01900000-0000-7000-8000-0000000000f1";
const EARLIER_POST: &str = "01900000-0000-7000-8000-0000000000f0";
const DIRECTIVE: &str = "01900000-0000-7000-8000-0000000000f7";
const DOC: &str = "01900000-0000-7000-8000-0000000000d7";
const SKETCH: &str = "01900000-0000-7000-8000-0000000000a1";
const NOTES: &str = "01900000-0000-7000-8000-0000000000a2";
const DIRECTIVE_SUMMARY: &str = "Keep the hero under 600 px";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn posts_path(thread: &str) -> String {
    format!("{}/threads/{thread}/posts", base())
}

fn attachments_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/attachments")
}

fn directive_json() -> Value {
    json!({
        "id": DIRECTIVE,
        "thread_id": THREAD,
        "phase": 1,
        "type": "directive",
        "author": { "kind": "human", "user_id": "01900000-0000-7000-8000-0000000000ab" },
        "summary": DIRECTIVE_SUMMARY,
        "body": "It must fit above the fold.",
        "body_html": "<p>It must fit above the fold.</p>",
        "links": [],
        "attachments": [],
        "confidence": null,
        "replaces": null,
        "created_at": "2026-10-21T10:00:00.000000Z"
    })
}

/// The Post that the API answers for a request body.
fn post_json(body: &Value) -> Value {
    json!({
        "id": POST,
        "thread_id": THREAD,
        "phase": 1,
        "type": body["type"],
        "author": {
            "kind": "agent",
            "run": "run-7",
            "role": "moderator",
            "model": null,
            "harness": null
        },
        "summary": body["summary"],
        "body": body["body"],
        "body_html": "<p>rendered</p>",
        "links": body.get("links").cloned().unwrap_or(json!([])),
        "attachments": body.get("attachments").cloned().unwrap_or(json!([])),
        "confidence": body.get("confidence").cloned().unwrap_or(Value::Null),
        "replaces": body.get("replaces").cloned().unwrap_or(Value::Null),
        "created_at": "2026-10-21T10:01:00.000000Z"
    })
}

fn thread_json() -> Value {
    json!({
        "id": THREAD,
        "project_id": PROJECT,
        "changeset_id": CHANGESET,
        "question": "Which hero layout?",
        "criteria": ["Readable"],
        "run_tag": "run-7",
        "created_at": "2026-10-21T10:00:00.000000Z",
        "phases": [
            { "number": 1, "sealed": false, "started_at": "2026-10-21T10:00:00.000000Z" },
            { "number": 2, "sealed": true, "started_at": "2026-10-21T10:02:00.000000Z" }
        ]
    })
}

fn changeset_json() -> Value {
    json!({
        "id": CHANGESET,
        "project_id": PROJECT,
        "title": "Hero section",
        "description": "Landing hero, first pass.",
        "status": "open",
        "run_tag": "run-7",
        "created_at": "2026-10-21T10:00:00.000000Z",
        "history": [
            {
                "from_status": null,
                "to_status": "open",
                "actor": "agent",
                "reason": null,
                "at": "2026-10-21T10:00:00.000000Z"
            }
        ]
    })
}

/// An API that takes posts and phases in `THREAD`, shows `CHANGESET` and
/// takes uploads; it knows no other thread. With `directives`, every answer
/// in the change set carries the directive.
fn mock(directives: bool) -> MockServer {
    MockServer::start(move |request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let with_directives = |mut answer: Value| {
            if directives {
                answer["directives"] = json!([directive_json()]);
            }
            answer
        };
        let path = request.path.as_str();
        let method = request.method.as_str();
        if method == "POST" && path == attachments_path() {
            let name = request.header("x-omni-filename").unwrap_or("");
            let id = if name.contains("sketch") {
                SKETCH
            } else {
                NOTES
            };
            return Reply::json(
                201,
                &json!({ "attachment": {
                    "id": id,
                    "ref": format!("attachment:{id}"),
                    "filename": name,
                    "kind": "file",
                    "content_type": "application/octet-stream",
                    "size": request.body.len(),
                    "created_at": "2026-10-21T10:00:00.000000Z"
                } }),
            );
        }
        let Some(rest) = path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        match (method, rest) {
            ("POST", r) if r == format!("/threads/{THREAD}/posts") => {
                Reply::json(201, &with_directives(json!({ "post": post_json(&body) })))
            }
            ("POST", r) if r == format!("/threads/{THREAD}/phases") => {
                Reply::json(201, &with_directives(json!({ "thread": thread_json() })))
            }
            ("GET", r) if r == format!("/changesets/{CHANGESET}") => Reply::json(
                200,
                &with_directives(json!({ "changeset": changeset_json() })),
            ),
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
    run_with_stdin(home, args, b"")
}

fn run_with_stdin(home: &Scratch, args: &[&str], stdin: &[u8]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--project", PROJECT, "--as", "run-7/moderator"])
        .stdin(stdin)
        .run()
}

/// A file in `home` with `bytes`; returns its path.
fn file(home: &Scratch, name: &str, bytes: &[u8]) -> String {
    let path = home.join(name);
    std::fs::write(&path, bytes).expect("write a file");
    path.to_str().expect("a UTF-8 path").to_string()
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
    assert_eq!(
        request.header("x-omni-role"),
        Some("moderator"),
        "{request:?}"
    );
}

fn body_of(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body))
}

/// The one post the server got; its body.
fn sent_post(server: &MockServer) -> Value {
    let sent = requests_to(server, "POST", &posts_path(THREAD));
    assert_eq!(sent.len(), 1, "one post: {:?}", server.requests());
    assert_agent_request(&sent[0]);
    body_of(&sent[0])
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

// ---------------------------------------------------------------------------
// omni post
// ---------------------------------------------------------------------------

#[test]
fn post_reads_the_body_from_the_body_file() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let text = "## Why\n\nThe *left* layout reads better at 320 px.\n";
    let body_file = file(&home, "body.md", text.as_bytes());
    let out = run(
        &home,
        &[
            "post",
            THREAD,
            "--summary",
            "Pick the left layout",
            "--body-file",
            &body_file,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let body = sent_post(&server);
    assert!(body.get("type").is_none(), "a post has no type: {body}");
    assert_eq!(body["summary"], "Pick the left layout", "{body}");
    assert_eq!(
        body["body"].as_str().map(str::trim_end),
        Some(text.trim_end()),
        "the body is the file's text: {body}"
    );
    assert!(
        out.stdout.contains(POST),
        "post prints the new post's ID: {:?}",
        out.stdout
    );
}

#[test]
fn post_sends_the_summary_and_the_body_argument_and_no_type() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run_with_stdin(
        &home,
        &[
            "post",
            THREAD,
            "--summary",
            "Pick the left layout",
            "The *left* layout reads better at 320 px.",
        ],
        b"Not this: the body argument comes first.",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let body = sent_post(&server);
    assert_eq!(
        body,
        json!({
            "summary": "Pick the left layout",
            "body": "The *left* layout reads better at 320 px."
        }),
        "the summary and the body argument, and nothing else"
    );
    assert!(
        out.stdout.contains(POST),
        "post prints the new post's ID: {:?}",
        out.stdout
    );
}

#[test]
fn post_reads_the_body_from_standard_input_without_a_body_file() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let text = "Found a contrast problem in the **hero**.\n\nSee the sketch.\n";
    let out = run_with_stdin(
        &home,
        &["post", THREAD, "--summary", "Low contrast in the hero"],
        text.as_bytes(),
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let body = sent_post(&server);
    assert!(body.get("type").is_none(), "a post has no type: {body}");
    assert_eq!(
        body["body"].as_str().map(str::trim_end),
        Some(text.trim_end()),
        "the body is standard input: {body}"
    );
    assert!(out.stdout.contains(POST), "{:?}", out.stdout);
}

#[test]
fn post_uploads_each_attach_file_and_sends_the_attachment_ids_in_order() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let notes = file(&home, "notes.txt", b"plain notes");
    let sketch = file(&home, "sketch.png", b"\x89PNG not really");
    let out = run_with_stdin(
        &home,
        &[
            "post",
            THREAD,
            "--summary",
            "With evidence",
            "--attach",
            &notes,
            "--attach",
            &sketch,
        ],
        b"See the files.",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let uploads = requests_to(&server, "POST", &attachments_path());
    assert_eq!(uploads.len(), 2, "two uploads: {:?}", server.requests());
    assert_eq!(uploads[0].body, b"plain notes", "{:?}", uploads[0]);
    assert_eq!(uploads[1].body, b"\x89PNG not really", "{:?}", uploads[1]);
    assert_eq!(
        uploads[0].header("x-omni-filename"),
        Some("notes.txt"),
        "{:?}",
        uploads[0]
    );
    assert_eq!(
        uploads[1].header("x-omni-filename"),
        Some("sketch.png"),
        "{:?}",
        uploads[1]
    );
    for upload in &uploads {
        assert_agent_request(upload);
    }
    let body = sent_post(&server);
    assert_eq!(
        body["attachments"],
        json!([NOTES, SKETCH]),
        "the attachments, in the order given: {body}"
    );
    let order: Vec<&str> = server
        .requests()
        .iter()
        .map(|r| r.path.as_str())
        .filter(|p| *p == attachments_path() || *p == posts_path(THREAD))
        .map(|p| {
            if p == attachments_path() {
                "upload"
            } else {
                "post"
            }
        })
        .collect();
    assert_eq!(
        order,
        vec!["upload", "upload", "post"],
        "the files are uploaded before the post"
    );
}

#[test]
fn post_sends_links_a_confidence_and_the_post_it_replaces() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let node = format!("node:{DOC}/a0000024/b0000031");
    let earlier = format!("post:{EARLIER_POST}");
    let attachment = format!("attachment:{SKETCH}");
    let out = run_with_stdin(
        &home,
        &[
            "post",
            THREAD,
            "--summary",
            "Left layout, smaller hero",
            "--link",
            &earlier,
            "--link",
            "https://example.com/research",
            "--link",
            &node,
            "--link",
            &attachment,
            "--confidence",
            "0.8",
            "--replaces",
            EARLIER_POST,
        ],
        b"Second take.",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let body = sent_post(&server);
    assert_eq!(
        body["links"],
        json!([
            { "kind": "post", "post_id": EARLIER_POST },
            { "kind": "url", "url": "https://example.com/research" },
            { "kind": "node", "doc_id": DOC, "address": "a0000024/b0000031" },
            { "kind": "attachment", "attachment_id": SKETCH }
        ]),
        "{body}"
    );
    assert_eq!(body["confidence"].as_f64(), Some(0.8), "{body}");
    assert_eq!(body["replaces"], EARLIER_POST, "{body}");
}

#[test]
fn post_with_json_prints_the_apis_answer() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run_with_stdin(
        &home,
        &["post", THREAD, "--summary", "Noted", "--json"],
        b"A note.",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let body = sent_post(&server);
    assert_eq!(out.json(), json!({ "post": post_json(&body) }));
}

#[test]
fn post_needs_a_summary_and_takes_no_type() {
    // US-108 removed post types: `--type` is an option `omni` no longer knows.
    let server = mock(false);
    let home = logged_in(&server.url);
    for (args, named) in [
        (vec!["post", THREAD], "--summary"),
        (
            vec!["post", THREAD, "--type", "note", "--summary", "Typed"],
            "--type",
        ),
    ] {
        let out = run_with_stdin(&home, &args, b"A body.");
        assert_one_line_error(&out, &[named]);
    }
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

#[test]
fn post_with_a_confidence_or_link_it_cannot_read_sends_nothing() {
    let server = mock(false);
    let home = logged_in(&server.url);
    for (extra, needle) in [
        (["--confidence", "very"], "--confidence"),
        (["--link", "javascript:alert(1)"], "--link"),
        (["--link", "ftp://example.com/file"], "--link"),
    ] {
        let mut args = vec!["post", THREAD, "--summary", "x"];
        args.extend(extra);
        let out = run_with_stdin(&home, &args, b"A body.");
        assert_one_line_error(&out, &[needle]);
    }
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

#[test]
fn post_with_a_file_that_cannot_be_read_is_one_line_that_names_it_and_sends_nothing() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let missing_body = home.join("no-such-body.md");
    let missing_body = missing_body.to_str().expect("a UTF-8 path");
    let out = run(
        &home,
        &[
            "post",
            THREAD,
            "--summary",
            "x",
            "--body-file",
            missing_body,
        ],
    );
    assert_one_line_error(&out, &["no-such-body.md"]);

    let present = file(&home, "present.png", b"bytes");
    let missing_attachment = home.join("no-such-sketch.png");
    let missing_attachment = missing_attachment.to_str().expect("a UTF-8 path");
    let out = run_with_stdin(
        &home,
        &[
            "post",
            THREAD,
            "--summary",
            "x",
            "--attach",
            &present,
            "--attach",
            missing_attachment,
        ],
        b"A body.",
    );
    assert_one_line_error(&out, &["no-such-sketch.png"]);
    assert!(
        server.requests().is_empty(),
        "nothing should be sent, not even the file that could be read: {:?}",
        server.requests()
    );
}

#[test]
fn post_in_an_unknown_thread_is_one_line_that_names_it() {
    let server = mock(false);
    let home = logged_in(&server.url);
    let out = run_with_stdin(
        &home,
        &["post", UNKNOWN_THREAD, "--summary", "x"],
        b"A body.",
    );
    assert_one_line_error(&out, &[UNKNOWN_THREAD]);
    assert_eq!(
        requests_to(&server, "POST", &posts_path(UNKNOWN_THREAD)).len(),
        1,
        "{:?}",
        server.requests()
    );
}

#[test]
fn help_lists_the_post_command() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    assert!(
        help.stdout.contains("omni post"),
        "omni --help lists `omni post`: {:?}",
        help.stdout
    );
}

// ---------------------------------------------------------------------------
// Directives
// ---------------------------------------------------------------------------

/// Checks that `out` succeeded and printed the directive on standard error
/// and not on standard output.
fn assert_directive_on_stderr(out: &Out, what: &str) {
    assert_eq!(out.code, 0, "{what}: {}", out.stderr);
    assert!(
        out.stderr_lines()
            .iter()
            .any(|line| line.contains(DIRECTIVE_SUMMARY)),
        "{what} prints the directive on standard error: {:?}",
        out.stderr
    );
    assert!(
        !out.stdout.contains(DIRECTIVE_SUMMARY),
        "{what} keeps the directive off standard output: {:?}",
        out.stdout
    );
}

#[test]
fn the_directives_of_an_answer_go_to_standard_error() {
    let server = mock(true);
    let home = logged_in(&server.url);

    let out = run(&home, &["thread", "phase", THREAD, "--sealed"]);
    assert_directive_on_stderr(&out, "omni thread phase");
    assert!(
        non_empty_lines(&out.stdout)
            .iter()
            .any(|l| l.contains('2') && l.contains("sealed")),
        "the phase is still printed: {:?}",
        out.stdout
    );

    let out = run(&home, &["changeset", "show", CHANGESET]);
    assert_directive_on_stderr(&out, "omni changeset show");
    assert!(
        out.stdout.contains("Hero section"),
        "the change set is still printed: {:?}",
        out.stdout
    );

    let out = run_with_stdin(&home, &["post", THREAD, "--summary", "Noted"], b"A note.");
    assert_directive_on_stderr(&out, "omni post");
    assert!(
        out.stdout.contains(POST),
        "the post's ID is still printed: {:?}",
        out.stdout
    );
}
