//! US-028: `omni tokens`, tested as the built binary against a mock HTTP
//! server. None of these tests needs Postgres; the API's contract is in
//! crates/omni-api/tests/tokens.rs, and the batch that `omni tokens import`
//! sends in tests/live_designs.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found from `--project` or `.omni.toml`):
//!
//! - `omni tokens import <file>`: a missing file argument, a file that cannot
//!   be read (the error names it) and a file that is not JSON are one-line
//!   errors that send nothing.
//! - `omni tokens export` reads `GET /tokens` (the head of the design system)
//!   and prints the DTCG JSON of the answer's `tokens` on standard output, and
//!   nothing else. `--out <file>` writes the JSON to that file instead. A
//!   design system without tokens (`"tokens": null`) is a one-line error that
//!   says so.
//! - `omni tokens export --per-mode --out <dir>` writes one file per mode of
//!   the answer's `modes`, `<dir>/<mode>.tokens.json`, each the `tokens` of
//!   the same read with `?mode=<mode>` added. `--per-mode` without `--out` is
//!   a one-line error that sends nothing.
//! - Every command is an agent request with the agent identity headers, and
//!   errors are one line on standard error, as in US-015
//!   (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn tree() -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "$type": "color",
            "ink": {"$value": "#0f172a"},
            "surface": {
                "$value": "#ffffff",
                "$extensions": {"omnistate": {"modes": {"light": "#ffffff", "dark": "#0f172a"}}}
            }
        }
    })
}

/// The tree of one mode, as the API answers `?mode=<mode>`.
fn mode_tree(mode: &str, surface: &str) -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": [mode]}},
        "color": {
            "$type": "color",
            "ink": {"$value": "#0f172a"},
            "surface": {"$value": surface}
        }
    })
}

fn split_query(path: &str) -> (&str, Vec<(&str, &str)>) {
    match path.split_once('?') {
        None => (path, Vec::new()),
        Some((path, query)) => (
            path,
            query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .collect(),
        ),
    }
}

/// The answer of `GET /tokens`: the whole tree, or the tree of the mode in the
/// query.
fn read_answer(query: &[(&str, &str)]) -> Reply {
    let mode = query.iter().find(|(k, _)| *k == "mode").map(|(_, v)| *v);
    let mut answer = json!({ "project_id": PROJECT, "doc_id": DOC, "modes": ["light", "dark"] });
    match mode {
        None => answer["tokens"] = tree(),
        Some("light") => {
            answer["mode"] = json!("light");
            answer["tokens"] = mode_tree("light", "#ffffff");
        }
        Some("dark") => {
            answer["mode"] = json!("dark");
            answer["tokens"] = mode_tree("dark", "#0f172a");
        }
        Some(_) => return Reply::json(400, &json!({ "error": "unknown_mode" })),
    }
    Reply::json(200, &answer)
}

/// An API with a design system doc `DOC` in `PROJECT`, whose head has tokens
/// unless `has_tokens` is false.
fn mock_with(has_tokens: bool) -> MockServer {
    MockServer::start(move |request| {
        let Some(rest) = request.path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        let (path, query) = split_query(rest);
        match (request.method.as_str(), path) {
            ("GET", "/tokens") if has_tokens => read_answer(&query),
            ("GET", "/tokens") => Reply::json(
                200,
                &json!({ "project_id": PROJECT, "doc_id": DOC, "modes": [], "tokens": null }),
            ),
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

fn mock() -> MockServer {
    mock_with(true)
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--project", PROJECT, "--as", "run-28/designer"])
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

fn assert_agent_request(request: &Recorded) {
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str()),
        "{request:?}"
    );
    assert_eq!(request.header("x-omni-run"), Some("run-28"), "{request:?}");
    assert_eq!(
        request.header("x-omni-role"),
        Some("designer"),
        "{request:?}"
    );
}

fn assert_nothing_sent(server: &MockServer) {
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

fn write_file(home: &Scratch, name: &str, text: &str) -> String {
    let path = home.join(name);
    std::fs::write(&path, text).expect("write the token file");
    path.display().to_string()
}

fn read_json(path: &std::path::Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} was written ({e})", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} holds JSON ({e}): {text}", path.display()))
}

// ---------------------------------------------------------------------------
// omni tokens import
// ---------------------------------------------------------------------------

#[test]
fn tokens_import_refuses_bad_input_before_sending_anything() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(&run(&home, &["tokens", "import"]), &["file"]);
    let missing = home.join("missing.tokens.json").display().to_string();
    assert_one_line_error(
        &run(&home, &["tokens", "import", &missing]),
        &["missing.tokens.json"],
    );
    let not_json = write_file(&home, "not-json.tokens.json", "color: { surface: #fff }");
    assert_one_line_error(&run(&home, &["tokens", "import", &not_json]), &[]);
    assert_nothing_sent(&server);
}

// ---------------------------------------------------------------------------
// omni tokens export
// ---------------------------------------------------------------------------

#[test]
fn tokens_export_prints_the_dtcg_json_of_the_head() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["tokens", "export"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let gets = requests_to(&server, "GET", &format!("{}/tokens", base()));
    assert_eq!(gets.len(), 1, "{:?}", server.requests());
    assert_agent_request(&gets[0]);
    assert_eq!(
        out.json(),
        tree(),
        "standard output is the DTCG JSON and nothing else"
    );
}

#[test]
fn tokens_export_writes_a_file() {
    let server = mock();
    let home = logged_in(&server.url);
    let file = home.join("design-system.tokens.json");
    let out = run(
        &home,
        &["tokens", "export", "--out", &file.display().to_string()],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(read_json(&file), tree());
    assert!(
        serde_json::from_str::<Value>(&out.stdout).is_err(),
        "the tokens go to the file, not to standard output: {:?}",
        out.stdout
    );
}

#[test]
fn tokens_export_writes_one_file_per_mode() {
    let server = mock();
    let home = logged_in(&server.url);
    let dir = home.join("modes");
    let out = run(
        &home,
        &[
            "tokens",
            "export",
            "--per-mode",
            "--out",
            &dir.display().to_string(),
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        read_json(&dir.join("light.tokens.json")),
        mode_tree("light", "#ffffff")
    );
    assert_eq!(
        read_json(&dir.join("dark.tokens.json")),
        mode_tree("dark", "#0f172a")
    );
    let files = std::fs::read_dir(&dir)
        .expect("the directory was made")
        .count();
    assert_eq!(files, 2, "one file per mode and nothing else");
    assert_eq!(
        requests_to(&server, "GET", &format!("{}/tokens?mode=light", base())).len(),
        1,
        "{:?}",
        server.requests()
    );
    assert_eq!(
        requests_to(&server, "GET", &format!("{}/tokens?mode=dark", base())).len(),
        1,
        "{:?}",
        server.requests()
    );
}

#[test]
fn tokens_export_per_mode_needs_a_directory() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(&run(&home, &["tokens", "export", "--per-mode"]), &["--out"]);
    assert_nothing_sent(&server);
}

#[test]
fn tokens_export_of_a_design_system_without_tokens_is_a_one_line_error() {
    let server = mock_with(false);
    let home = logged_in(&server.url);
    assert_one_line_error(&run(&home, &["tokens", "export"]), &["no tokens"]);
}
