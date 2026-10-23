//! US-051, US-105: `omni import penpot <file.penpot> --project <id> [--feature
//! <name>]`, tested as the built binary against a mock HTTP server. The import
//! against the real API is in tests/stack_import_penpot.rs, and the rules of
//! the import in crates/omni-penpot/tests/import_synthetic.rs.
//!
//! The contract these tests pin down:
//!
//! - `omni --help` lists `omni import penpot` with its `--project` and its
//!   `--feature` (US-105).
//! - The export is read before anything is sent: a file that is missing, or
//!   that is not a Penpot export, is a one-line error that names the file and
//!   sends nothing, so it leaves no change set behind. A missing file argument
//!   is a one-line error that says a file is needed, and sends nothing.
//! - Errors are one line on standard error, as in US-015
//!   (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::json;

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";

fn mock() -> MockServer {
    MockServer::start(|_| Reply::json(500, &json!({ "error": "unexpected" })))
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--project", PROJECT, "--as", "run-51/importer"])
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

fn assert_nothing_sent(server: &MockServer) {
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

#[test]
fn help_lists_import_penpot() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    let line = help
        .stdout
        .lines()
        .find(|line| line.contains("omni import penpot"))
        .unwrap_or_else(|| panic!("omni --help lists omni import penpot: {}", help.stdout));
    assert!(line.contains("--project"), "{line:?}");
    assert!(line.contains("--feature"), "{line:?}");
}

#[test]
fn a_missing_file_is_a_one_line_error_that_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let missing = home.join("nirgendwo.penpot");
    let out = run(&home, &["import", "penpot", missing.to_str().unwrap()]);
    assert_one_line_error(&out, &["nirgendwo.penpot"]);
    assert_nothing_sent(&server);
}

#[test]
fn a_file_that_is_no_penpot_export_is_a_one_line_error_that_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let file = home.join("anteckning.penpot");
    std::fs::write(&file, "inte en export").unwrap();
    let out = run(&home, &["import", "penpot", file.to_str().unwrap()]);
    assert_one_line_error(&out, &["anteckning.penpot"]);
    assert_nothing_sent(&server);
}

#[test]
fn import_penpot_needs_a_file() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["import", "penpot"]);
    assert_one_line_error(&out, &["file"]);
    assert_nothing_sent(&server);
}
