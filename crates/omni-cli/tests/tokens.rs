//! US-028: `omni tokens`, tested as the built binary against a mock HTTP
//! server. None of these tests needs Postgres; the API's contract is in
//! crates/omni-api/tests/tokens.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni tokens import <file> --changeset <id>` reads the DTCG JSON in
//!   `<file>` and sends `PUT /changesets/{changeset}/tokens` with `{"tokens":
//!   <the file's JSON>}`, which replaces the whole token tree of the change
//!   set's draft of the design system in one all-or-nothing batch. It prints a
//!   short plain-text line or two with the design system's modes; `--json`
//!   prints the API's answer. A missing file argument, a file
//!   that cannot be read (the error names it) and a file that is not JSON are
//!   one-line errors that send nothing. A refusal (400 `invalid_tokens`) is a
//!   one-line error that carries the API's message, and a project without a
//!   design system (404 `no_design_system`) one that says "design system".
//! - `omni tokens export` reads `GET /tokens` (the mainline version) and
//!   prints the DTCG JSON of the answer's `tokens` on standard output, and
//!   nothing else. `--changeset <id>` reads the change set's draft, `GET
//!   /changesets/{changeset}/tokens`, and `--version <v>` reads `GET
//!   /tokens?version=<v>`. `--out <file>` writes the JSON to that file
//!   instead. A design system without a mainline version (`"tokens": null`) is
//!   a one-line error that says there is no version.
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
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const DRAFT: &str = "01900000-0000-7000-8000-0000000000e1";
const MAINLINE: &str = "01900000-0000-7000-8000-0000000000f1";
const OLD_VERSION: &str = "01900000-0000-7000-8000-0000000000f0";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn tree(surface_light: &str, surface_dark: &str) -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "$type": "color",
            "ink": {"$value": "#0f172a"},
            "surface": {
                "$value": surface_light,
                "$extensions": {"omnistate": {"modes": {"light": surface_light, "dark": surface_dark}}}
            }
        }
    })
}

fn mainline_tree() -> Value {
    tree("#ffffff", "#0f172a")
}

fn old_tree() -> Value {
    tree("#fafafa", "#111111")
}

fn draft_tree() -> Value {
    tree("#fefefe", "#000000")
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

/// A read's answer: the whole tree, or the tree of the mode in the query.
fn read_answer(
    query: &[(&str, &str)],
    whole: Value,
    surfaces: (&str, &str),
    extra: Value,
) -> Reply {
    let mode = query.iter().find(|(k, _)| *k == "mode").map(|(_, v)| *v);
    let mut answer = json!({ "doc_id": DOC, "modes": ["light", "dark"] });
    for (key, value) in extra.as_object().unwrap() {
        answer[key] = value.clone();
    }
    match mode {
        None => answer["tokens"] = whole,
        Some("light") => {
            answer["mode"] = json!("light");
            answer["tokens"] = mode_tree("light", surfaces.0);
        }
        Some("dark") => {
            answer["mode"] = json!("dark");
            answer["tokens"] = mode_tree("dark", surfaces.1);
        }
        Some(_) => return Reply::json(400, &json!({ "error": "unknown_mode" })),
    }
    Reply::json(200, &answer)
}

/// An API with a design system doc `DOC` in `PROJECT`: its mainline version
/// (unless `has_mainline` is false), an older version and a draft in
/// `CHANGESET`.
fn mock_with(has_mainline: bool) -> MockServer {
    MockServer::start(move |request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let Some(rest) = request.path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        let (path, query) = split_query(rest);
        let version = query.iter().find(|(k, _)| *k == "version").map(|(_, v)| *v);
        match (request.method.as_str(), path) {
            ("PUT", p) if p == format!("/changesets/{CHANGESET}/tokens") => {
                if body["tokens"].is_object() {
                    Reply::json(
                        200,
                        &json!({
                            "doc_id": DOC,
                            "changeset_id": CHANGESET,
                            "draft_id": DRAFT,
                            "modes": body["tokens"]["$extensions"]["omnistate"]["modes"],
                            "tokens": body["tokens"]
                        }),
                    )
                } else {
                    Reply::json(400, &json!({ "error": "invalid_request" }))
                }
            }
            ("GET", p) if p == format!("/changesets/{CHANGESET}/tokens") => read_answer(
                &query,
                draft_tree(),
                ("#fefefe", "#000000"),
                json!({ "changeset_id": CHANGESET, "draft_id": DRAFT }),
            ),
            ("GET", "/tokens") if version == Some(OLD_VERSION) => read_answer(
                &query,
                old_tree(),
                ("#fafafa", "#111111"),
                json!({ "version_id": OLD_VERSION }),
            ),
            ("GET", "/tokens") if version.is_none() && has_mainline => read_answer(
                &query,
                mainline_tree(),
                ("#ffffff", "#0f172a"),
                json!({ "version_id": MAINLINE }),
            ),
            ("GET", "/tokens") if version.is_none() => Reply::json(
                200,
                &json!({ "doc_id": DOC, "version_id": null, "modes": [], "tokens": null }),
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
fn tokens_import_replaces_the_token_tree_of_the_change_set() {
    let server = mock();
    let home = logged_in(&server.url);
    let file = write_file(&home, "tokens.json", &draft_tree().to_string());
    let out = run(
        &home,
        &["tokens", "import", &file, "--changeset", CHANGESET],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let puts = requests_to(
        &server,
        "PUT",
        &format!("{}/changesets/{CHANGESET}/tokens", base()),
    );
    assert_eq!(puts.len(), 1, "{:?}", server.requests());
    assert_agent_request(&puts[0]);
    assert_eq!(
        body_of(&puts[0]),
        json!({ "tokens": draft_tree() }),
        "the whole file goes in one request"
    );
    assert_eq!(server.requests().len(), 1, "{:?}", server.requests());
    assert!(
        out.stdout.contains("light") && out.stdout.contains("dark"),
        "the text names the modes: {:?}",
        out.stdout
    );
    assert!(
        serde_json::from_str::<Value>(&out.stdout).is_err(),
        "the default output is plain text: {:?}",
        out.stdout
    );
    assert!(non_empty_lines(&out.stdout).len() <= 5, "{:?}", out.stdout);

    let json = run(
        &home,
        &[
            "tokens",
            "import",
            &file,
            "--changeset",
            CHANGESET,
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    let answer = json.json();
    assert_eq!(answer["draft_id"], DRAFT, "{answer}");
    assert_eq!(answer["tokens"], draft_tree(), "{answer}");
}

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
fn tokens_export_prints_the_mainline_dtcg_json() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["tokens", "export"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let gets = requests_to(&server, "GET", &format!("{}/tokens", base()));
    assert_eq!(gets.len(), 1, "{:?}", server.requests());
    assert_agent_request(&gets[0]);
    assert_eq!(
        out.json(),
        mainline_tree(),
        "standard output is the DTCG JSON and nothing else"
    );
}

#[test]
fn tokens_export_reads_a_draft_or_a_version() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["tokens", "export", "--changeset", CHANGESET]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        requests_to(
            &server,
            "GET",
            &format!("{}/changesets/{CHANGESET}/tokens", base())
        )
        .len(),
        1,
        "{:?}",
        server.requests()
    );
    assert_eq!(out.json(), draft_tree());

    let out = run(&home, &["tokens", "export", "--version", OLD_VERSION]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        requests_to(
            &server,
            "GET",
            &format!("{}/tokens?version={OLD_VERSION}", base())
        )
        .len(),
        1,
        "{:?}",
        server.requests()
    );
    assert_eq!(out.json(), old_tree());
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
    assert_eq!(read_json(&file), mainline_tree());
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
