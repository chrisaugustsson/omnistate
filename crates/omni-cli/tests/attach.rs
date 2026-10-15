//! US-019: `omni attach`, tested as the built binary against a mock HTTP
//! server. None of these tests needs Postgres or RustFS; the round trip against
//! the real API is in tests/stack_attachments.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni attach <file>` sends one `POST /api/v1/w/{workspace}/attachments`
//!   as the agent. The body is the file's bytes, exactly, and `X-Omni-Filename`
//!   is the file's base name (percent-encoded UTF-8; an unencoded name that
//!   decodes to itself passes too). The API answers 201 `{"attachment": {"id",
//!   "ref", "filename", "kind", "content_type", "size", "created_at"}}`, where
//!   `ref` is `attachment:<id>`.
//! - Standard output is one line with the new attachment's ID, either the bare
//!   ID or `attachment:<id>`; `--json` prints the API's JSON instead. Notices
//!   go to standard error.
//! - A file over 20 MB is refused with the one-line error of US-015 that
//!   says "20 MB"; exactly 20,000,000 bytes goes through. A missing file, a
//!   directory and a missing or extra argument are errors that send nothing.
//! - `omni --help` lists the command.

mod common;

use common::*;
use serde_json::{Value, json};

const ATTACHMENT: &str = "01900000-0000-7000-8000-0000000000d1";
const OVER_TWENTY_MIB: usize = 20 * 1024 * 1024 + 1;

fn attachments_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/attachments")
}

/// The text of a percent-encoded header value.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn attachment_json(filename: &str, size: usize) -> Value {
    json!({
        "id": ATTACHMENT,
        "ref": format!("attachment:{ATTACHMENT}"),
        "filename": filename,
        "kind": "file",
        "content_type": "application/octet-stream",
        "size": size,
        "created_at": "2026-10-15T10:00:00.000000Z"
    })
}

/// An API that takes uploads to the workspace's attachments and refuses a file
/// over 20 MiB, as the real one does.
fn mock() -> MockServer {
    MockServer::start(|request| {
        if request.method != "POST" || request.path != attachments_path() {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        if request.body.len() > 20 * 1024 * 1024 {
            return Reply::json(413, &json!({ "error": "file_too_large" }));
        }
        let name = percent_decode(request.header("x-omni-filename").unwrap_or("attachment"));
        Reply::json(
            201,
            &json!({ "attachment": attachment_json(&name, request.body.len()) }),
        )
    })
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
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

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", "run-7/author"])
        .run()
}

/// The ID in what `omni attach` printed: the ID itself or `attachment:<id>`.
fn printed_id(out: &Out) -> String {
    let lines = non_empty_lines(&out.stdout);
    assert_eq!(lines.len(), 1, "one line with the ID: {:?}", out.stdout);
    lines[0]
        .strip_prefix("attachment:")
        .unwrap_or(lines[0])
        .to_string()
}

fn uploads(server: &MockServer) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path == attachments_path())
        .collect()
}

/// Every byte value, twice, with CR LF pairs and NULs among them: nothing a
/// text-only path would carry whole.
fn every_byte() -> Vec<u8> {
    (0..=255u8).chain((0..=255u8).rev()).collect()
}

#[test]
fn attach_uploads_the_file_byte_for_byte_as_the_agent_and_prints_its_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let bytes = every_byte();
    let file = home.join("diagram.bin");
    std::fs::write(&file, &bytes).unwrap();

    let out = run(&home, &["attach", file.to_str().unwrap()]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(printed_id(&out), ATTACHMENT);
    assert_eq!(out.stderr, "", "nothing to say on standard error");

    let posts = uploads(&server);
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    assert!(posts[0].body == bytes, "the file is sent byte for byte");
    assert_eq!(
        posts[0].header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(posts[0].header("x-omni-run"), Some("run-7"));
    assert_eq!(posts[0].header("x-omni-role"), Some("author"));
    assert_eq!(
        percent_decode(posts[0].header("x-omni-filename").unwrap_or("")),
        "diagram.bin"
    );
}

#[test]
fn attach_names_the_file_by_its_base_name_for_relative_and_odd_paths() {
    let server = mock();
    let home = logged_in(&server.url);
    std::fs::create_dir_all(home.join("assets/brand")).unwrap();
    std::fs::write(home.join("assets/brand/logo.png"), b"png-ish").unwrap();
    std::fs::write(home.join("my logo \u{e9}.png"), b"png-ish too").unwrap();

    // A path relative to the working directory, then a name with a space and
    // a non-ASCII letter.
    let out = run(&home, &["attach", "assets/brand/logo.png"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let out = run(&home, &["attach", "my logo \u{e9}.png"]);
    assert_eq!(out.code, 0, "{}", out.stderr);

    let names: Vec<String> = uploads(&server)
        .iter()
        .map(|r| percent_decode(r.header("x-omni-filename").unwrap_or("")))
        .collect();
    assert_eq!(names, ["logo.png", "my logo \u{e9}.png"]);
    assert_eq!(uploads(&server)[0].body, b"png-ish");
}

#[test]
fn attach_with_json_prints_the_whole_response() {
    let server = mock();
    let home = logged_in(&server.url);
    let file = home.join("notes.txt");
    std::fs::write(&file, b"twelve bytes").unwrap();

    let out = run(&home, &["attach", file.to_str().unwrap(), "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.json(),
        json!({ "attachment": attachment_json("notes.txt", 12) })
    );
}

#[test]
fn a_file_of_exactly_20_million_bytes_is_sent_whole() {
    let server = mock();
    let home = logged_in(&server.url);
    let bytes: Vec<u8> = (0..20_000_000usize).map(|n| (n % 251) as u8).collect();
    let file = home.join("big.bin");
    std::fs::write(&file, &bytes).unwrap();

    let out = run(&home, &["attach", file.to_str().unwrap()]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(printed_id(&out), ATTACHMENT);
    let posts = uploads(&server);
    assert_eq!(posts.len(), 1);
    assert!(posts[0].body == bytes, "the file is sent byte for byte");
}

#[test]
fn a_file_over_20_mib_is_one_line_that_names_the_limit() {
    let server = mock();
    let home = logged_in(&server.url);
    let file = home.join("huge.bin");
    std::fs::write(&file, vec![0x61u8; OVER_TWENTY_MIB]).unwrap();

    let out = run(&home, &["attach", file.to_str().unwrap()]);
    assert_one_line_error(&out, &["20 MB"]);
}

#[test]
fn attach_without_a_usable_file_is_one_line_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    std::fs::create_dir_all(home.join("a-directory")).unwrap();
    std::fs::write(home.join("one.txt"), b"1").unwrap();
    std::fs::write(home.join("two.txt"), b"2").unwrap();

    let missing = home.join("no-such-file.png");
    let out = run(&home, &["attach", missing.to_str().unwrap()]);
    assert_one_line_error(&out, &["no-such-file.png"]);

    let out = run(
        &home,
        &["attach", home.join("a-directory").to_str().unwrap()],
    );
    assert_one_line_error(&out, &["a-directory"]);

    let out = run(&home, &["attach"]);
    assert_one_line_error(&out, &["omni attach <file>"]);

    let out = run(&home, &["attach", "one.txt", "two.txt"]);
    assert_one_line_error(&out, &["omni attach <file>"]);

    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

#[test]
fn attach_needs_a_login_and_an_agent_identity() {
    let server = mock();
    let file_home = logged_in(&server.url);
    let file = file_home.join("logo.png");
    std::fs::write(&file, b"png-ish").unwrap();

    let nobody = Scratch::new("nobody");
    let out = run(&nobody, &["attach", file.to_str().unwrap()]);
    assert_one_line_error(&out, &["omni login"]);

    let out = omni(&file_home.path)
        .args(&["attach", file.to_str().unwrap()])
        .run();
    assert_one_line_error(&out, &["--as", "OMNI_RUN"]);

    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

#[test]
fn an_answer_that_is_an_error_is_one_line_and_prints_nothing() {
    for (status, code, needles) in [
        (401, "unauthorized", vec!["unauthorized", "omni login"]),
        (404, "not_found", vec!["not_found"]),
        (503, "unavailable", vec!["unavailable"]),
    ] {
        let server = MockServer::start(move |_| Reply::json(status, &json!({ "error": code })));
        let home = logged_in(&server.url);
        let file = home.join("logo.png");
        std::fs::write(&file, b"png-ish").unwrap();
        let out = run(&home, &["attach", file.to_str().unwrap()]);
        assert_one_line_error(&out, &needles);
    }
}

#[test]
fn the_help_lists_the_command_and_the_reference() {
    let home = Scratch::new("help");
    let out = omni(&home.path).args(&["--help"]).run();
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains("omni attach <file>"),
        "`omni --help` lists `omni attach <file>`:\n{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("attachment:<id>"),
        "`omni --help` says how an attachment is referred to:\n{}",
        out.stdout
    );
}
