//! US-015: the `omni` command line, tested as the built binary against a mock
//! HTTP server. None of these tests needs Postgres; the ones that log in or
//! act as the human against the real API are in tests/stack.rs. The contract
//! they pin down is described in tests/common/mod.rs.

mod common;

use common::*;
use serde_json::{Value, json};
use std::time::Duration;

fn whoami_body() -> Value {
    json!({ "workspace_id": WORKSPACE, "actor": { "kind": "agent" }, "extra": [1, 2, 3] })
}

/// A server that answers `whoami` and the echo route, and 404 for the rest.
fn mock() -> MockServer {
    MockServer::start(|request| {
        if request.path == format!("/api/v1/w/{WORKSPACE}/whoami") {
            Reply::json(200, &whoami_body())
        } else if request.path.starts_with("/api/v1/echo") {
            Reply::json(200, &json!({ "ok": true }))
        } else {
            Reply::json(404, &json!({ "error": "not_found" }))
        }
    })
}

/// A home directory with a config file as `omni login` leaves it.
fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

/// An error as the criteria define it: a documented non-zero exit code, nothing
/// on standard output and exactly one line on standard error that has every
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
fn omni_version_returns_in_under_20_ms() {
    let home = Scratch::new("version");
    // The server is not reachable: --version must not look at the network or
    // at any config.
    let mut fastest = Duration::MAX;
    for run in 0..15 {
        let out = omni(&home.path)
            .env("OMNI_SERVER", &closed_port_url())
            .args(&["--version"])
            .run();
        assert_eq!(out.code, 0, "omni --version: {}", out.stderr);
        if run == 0 {
            assert!(
                out.stdout.trim().starts_with("omni ")
                    && out.stdout.contains(env!("CARGO_PKG_VERSION")),
                "omni --version prints the name and version: {:?}",
                out.stdout
            );
        }
        fastest = fastest.min(out.elapsed);
    }
    assert!(
        fastest < Duration::from_millis(20),
        "the fastest of 15 runs of omni --version took {fastest:?}, the limit is 20 ms"
    );
}

#[test]
fn a_command_without_a_login_fails_with_one_line_that_says_to_log_in() {
    let home = Scratch::new("nologin");
    let out = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer"])
        .run();
    assert_one_line_error(&out, &["omni login"]);
}

#[test]
fn an_unreachable_server_is_one_line_that_names_the_server() {
    let url = closed_port_url();
    let home = logged_in(&url);
    let out = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer"])
        .run();
    let address = url.trim_start_matches("http://");
    assert_one_line_error(&out, &[address]);
}

#[test]
fn an_api_error_is_one_line_with_the_error_code_and_how_to_fix_it() {
    let rejecting = MockServer::start(|_| Reply::json(401, &json!({ "error": "unauthorized" })));
    let home = logged_in(&rejecting.url);
    let out = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer"])
        .run();
    assert_one_line_error(&out, &["unauthorized", "omni login"]);

    let missing = MockServer::start(|_| Reply::json(404, &json!({ "error": "not_found" })));
    let home = logged_in(&missing.url);
    let out = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer", "--json"])
        .run();
    assert_one_line_error(&out, &["not_found"]);
}

#[test]
fn a_request_without_an_identity_exits_nonzero_with_a_hint_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = omni(&home.path).args(&["whoami"]).run();
    assert_one_line_error(&out, &["--as", "OMNI_RUN"]);
    assert!(
        server.requests().is_empty(),
        "no request leaves without an identity: {:?}",
        server.requests()
    );
}

#[test]
fn flags_put_the_agent_identity_on_the_request() {
    let server = mock();
    let home = logged_in(&server.url);
    let flags = [
        "--as",
        "run-7/explorer",
        "--model",
        "claude-opus-4-7",
        "--harness",
        "claude-code",
    ];

    // Global flags work before and after the command name.
    let mut before = flags.to_vec();
    before.push("whoami");
    let mut after = vec!["whoami"];
    after.extend(flags);
    for args in [before, after] {
        let out = omni(&home.path).args(&args).run();
        assert_eq!(out.code, 0, "omni {args:?}: {}", out.stderr);
    }

    let requests = server.requests();
    assert_eq!(requests.len(), 2, "one request per run: {requests:?}");
    for request in requests {
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, format!("/api/v1/w/{WORKSPACE}/whoami"));
        assert_eq!(
            request.header("authorization"),
            Some(format!("Bearer {API_KEY}").as_str()),
            "the API key goes in the Authorization header"
        );
        assert_eq!(request.header("x-omni-run"), Some("run-7"));
        assert_eq!(request.header("x-omni-role"), Some("explorer"));
        assert_eq!(request.header("x-omni-model"), Some("claude-opus-4-7"));
        assert_eq!(request.header("x-omni-harness"), Some("claude-code"));
    }
}

#[test]
fn environment_variables_are_the_fallback_identity_and_flags_win() {
    let server = mock();
    let home = logged_in(&server.url);
    let env_only = omni(&home.path)
        .env("OMNI_RUN", "env-run")
        .env("OMNI_ROLE", "env-role")
        .env("OMNI_MODEL", "env-model")
        .env("OMNI_HARNESS", "env-harness")
        .args(&["whoami"])
        .run();
    assert_eq!(env_only.code, 0, "{}", env_only.stderr);
    let mixed = omni(&home.path)
        .env("OMNI_RUN", "env-run")
        .env("OMNI_ROLE", "env-role")
        .env("OMNI_MODEL", "env-model")
        .env("OMNI_HARNESS", "env-harness")
        .args(&[
            "whoami",
            "--as",
            "flag-run/flag-role",
            "--model",
            "flag-model",
        ])
        .run();
    assert_eq!(mixed.code, 0, "{}", mixed.stderr);

    let requests = server.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    let identity = |i: usize| {
        ["run", "role", "model", "harness"].map(|part| {
            requests[i]
                .header(&format!("x-omni-{part}"))
                .unwrap_or("")
                .to_string()
        })
    };
    assert_eq!(
        identity(0),
        ["env-run", "env-role", "env-model", "env-harness"]
    );
    assert_eq!(
        identity(1),
        ["flag-run", "flag-role", "flag-model", "env-harness"],
        "flags win over the environment, and the harness falls back to it"
    );
}

#[test]
fn output_is_short_text_by_default_and_the_full_json_with_json() {
    let server = mock();
    let home = logged_in(&server.url);

    let text = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer"])
        .run();
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert!(
        serde_json::from_str::<Value>(&text.stdout).is_err(),
        "the default output is plain text, not JSON: {:?}",
        text.stdout
    );
    assert!(
        text.stdout.contains(WORKSPACE) && text.stdout.contains("agent"),
        "the text says the workspace and that the caller is an agent: {:?}",
        text.stdout
    );
    assert!(
        non_empty_lines(&text.stdout).len() <= 5,
        "short text: {:?}",
        text.stdout
    );

    let json = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer", "--json"])
        .run();
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        whoami_body(),
        "--json prints the whole response of the API"
    );
}

#[test]
fn notices_go_to_standard_error_and_leave_standard_output_parseable() {
    let notice = "directive from the human: keep the grid at 8 px";
    let server = MockServer::start(move |_| {
        Reply::json(200, &whoami_body()).with_header("X-Omni-Notice", notice)
    });
    let home = logged_in(&server.url);

    let json = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer", "--json"])
        .run();
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), whoami_body(), "standard output is still JSON");
    assert!(
        json.stderr.contains(notice),
        "the notice is on standard error: {:?}",
        json.stderr
    );

    let text = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer"])
        .run();
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert!(
        !text.stdout.contains(notice) && text.stderr.contains(notice),
        "in text mode too: stdout {:?}, stderr {:?}",
        text.stdout,
        text.stderr
    );
}

fn long_text() -> String {
    let paragraph = "Ünïcödé ✓ — a long paragraph of the kind an agent posts.\n\n";
    format!("# Title\n\n{}end without a newline", paragraph.repeat(800))
}

#[test]
fn long_text_is_read_from_a_body_file() {
    let server = mock();
    let home = logged_in(&server.url);
    let body = long_text();
    let file = home.join("body.md");
    std::fs::write(&file, &body).unwrap();

    let out = omni(&home.path)
        .args(&["api", "POST", "/api/v1/echo", "--as", "run-1/author"])
        .args(&["--body-file", file.to_str().unwrap()])
        .run();
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.json(),
        json!({ "ok": true }),
        "the response body is printed"
    );

    let requests = server.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/api/v1/echo");
    assert_eq!(
        requests[0].body,
        body.as_bytes(),
        "the file is sent byte for byte"
    );
    assert_eq!(
        requests[0].header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(requests[0].header("x-omni-run"), Some("run-1"));
}

#[test]
fn long_text_is_read_from_standard_input() {
    let server = mock();
    let home = logged_in(&server.url);
    let body = long_text();

    let out = omni(&home.path)
        .args(&["api", "POST", "/api/v1/echo", "--as", "run-1/author"])
        .args(&["--body-file", "-"])
        .stdin(body.as_bytes())
        .run();
    assert_eq!(out.code, 0, "{}", out.stderr);
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].body, body.as_bytes());
}

#[test]
fn a_missing_body_file_is_one_line_that_names_the_file_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let missing = home.join("no-such-file.md");
    let out = omni(&home.path)
        .args(&["api", "POST", "/api/v1/echo", "--as", "run-1/author"])
        .args(&["--body-file", missing.to_str().unwrap()])
        .run();
    assert_one_line_error(&out, &["no-such-file.md"]);
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

#[test]
fn omni_toml_sets_the_default_project_for_commands_run_in_that_directory() {
    let home = logged_in("http://127.0.0.1:9");
    let project_dir = Scratch::new("project");
    std::fs::write(project_dir.join(".omni.toml"), "project = \"brand-site\"\n").unwrap();
    let elsewhere = Scratch::new("elsewhere");

    let here = omni(&home.path)
        .cwd(&project_dir.path)
        .args(&["config", "show", "--json"])
        .run();
    assert_eq!(here.code, 0, "{}", here.stderr);
    let settings = here.json();
    assert_eq!(settings["project"], "brand-site", "{settings}");
    assert_eq!(settings["workspace_id"], WORKSPACE, "{settings}");
    assert_eq!(settings["server"], "http://127.0.0.1:9", "{settings}");
    assert!(
        !here.stdout.contains(API_KEY) && !here.stderr.contains(API_KEY),
        "the key is never printed"
    );

    let text = omni(&home.path)
        .cwd(&project_dir.path)
        .args(&["config", "show"])
        .run();
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert!(text.stdout.contains("brand-site"), "{:?}", text.stdout);

    let there = omni(&home.path)
        .cwd(&elsewhere.path)
        .args(&["config", "show", "--json"])
        .run();
    assert_eq!(there.code, 0, "{}", there.stderr);
    assert!(
        there.json()["project"].is_null(),
        "no .omni.toml, no default project: {}",
        there.stdout
    );
}

#[test]
fn a_malformed_omni_toml_is_one_line_that_names_the_file() {
    let home = logged_in("http://127.0.0.1:9");
    let project_dir = Scratch::new("badproject");
    std::fs::write(project_dir.join(".omni.toml"), "project = \n").unwrap();
    let out = omni(&home.path)
        .cwd(&project_dir.path)
        .args(&["config", "show"])
        .run();
    assert_one_line_error(&out, &[".omni.toml"]);
}

#[test]
fn the_server_comes_from_the_flag_or_the_environment_before_the_config_file() {
    let server = mock();
    let home = Scratch::new("serverflag");
    write_config(&home.path, &closed_port_url(), WORKSPACE, API_KEY);

    let by_env = omni(&home.path)
        .env("OMNI_SERVER", &server.url)
        .args(&["whoami", "--as", "run-1/explorer"])
        .run();
    assert_eq!(by_env.code, 0, "OMNI_SERVER: {}", by_env.stderr);
    let by_flag = omni(&home.path)
        .args(&["whoami", "--as", "run-1/explorer", "--server", &server.url])
        .run();
    assert_eq!(by_flag.code, 0, "--server: {}", by_flag.stderr);
    assert_eq!(server.requests().len(), 2);
}
