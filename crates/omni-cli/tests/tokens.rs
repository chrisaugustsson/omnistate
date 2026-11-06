//! US-028: `omni tokens`, tested as the built binary against a mock HTTP
//! server. None of these tests needs Postgres; the API's contract is in
//! crates/omni-api/tests/tokens.rs, and the batch that `omni tokens import`
//! sends in tests/live_designs.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found from `--project`):
//!
//! - `omni tokens import <file>`: a missing file argument, a file that cannot
//!   be read (the error names it) and a file that is not JSON are one-line
//!   errors that send nothing.
//! - `omni tokens export` reads `GET /tokens` (the head of the design system)
//!   and prints the DTCG JSON of the answer's `tokens` on standard output, and
//!   nothing else. `--out <file>` writes the JSON to that file instead. A
//!   design system without tokens (`"tokens": null`) is a one-line error that
//!   says so.
//! - `omni tokens export --per-mode --out <dir>` writes `<dir>/tokens.json`
//!   (the answer's `tokens`) and, for each collection of the answer's
//!   `collections` and each of its modes, `<dir>/tokens.<collection>.<mode>.json`:
//!   that collection's group of the `tokens` of the same read with
//!   `?mode=<collection>:<mode>` added. A collection without modes has no
//!   file. `--per-mode` without `--out` is a one-line error that sends nothing.
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
        },
        "density": {
            "$extensions": {"omnistate": {"modes": ["comfortable", "compact"]}},
            "space": {
                "$type": "dimension",
                "$value": "16px",
                "$extensions": {"omnistate": {"modes": {"comfortable": "16px", "compact": "12px"}}}
            }
        },
        "state": {
            "$extensions": {"omnistate": {"modes": []}},
            "count": {"$type": "number", "$value": 1}
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
        },
        "density": {
            "$extensions": {"omnistate": {"modes": ["compact"]}},
            "space": {"$type": "dimension", "$value": "12px"}
        },
        "state": {"count": {"$type": "number", "$value": 1}}
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
    let mut answer = json!({
        "project_id": PROJECT,
        "doc_id": DOC,
        "modes": ["light", "dark"],
        "collections": [
            {"name": "color", "modes": ["light", "dark"]},
            {"name": "density", "modes": ["comfortable", "compact"]},
            {"name": "state", "modes": []}
        ]
    });
    match mode {
        None => answer["tokens"] = tree(),
        Some(pair @ ("color:light" | "color:dark" | "density:comfortable" | "density:compact")) => {
            let (collection, mode) = pair.split_once(':').expect("a pair");
            let surface = if mode == "dark" { "#0f172a" } else { "#ffffff" };
            answer["mode"] = json!({ collection: mode });
            answer["tokens"] = mode_tree(mode, surface);
        }
        Some(_) => return Reply::json(400, &json!({ "error": "invalid_request" })),
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
fn tokens_export_writes_one_file_per_collection_and_mode() {
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
    assert_eq!(read_json(&dir.join("tokens.json")), tree());
    let group = |mode: &str, surface: &str, name: &str| {
        let tree = mode_tree(mode, surface);
        json!({ name: tree[name].clone() })
    };
    assert_eq!(
        read_json(&dir.join("tokens.color.light.json")),
        group("light", "#ffffff", "color")
    );
    assert_eq!(
        read_json(&dir.join("tokens.color.dark.json")),
        group("dark", "#0f172a", "color")
    );
    assert_eq!(
        read_json(&dir.join("tokens.density.compact.json")),
        group("compact", "#ffffff", "density")
    );
    assert!(dir.join("tokens.density.comfortable.json").exists());
    let files = std::fs::read_dir(&dir)
        .expect("the directory was made")
        .count();
    assert_eq!(
        files, 5,
        "tokens.json, two colour files, two density files and none for state"
    );
    for pair in [
        "color:light",
        "color:dark",
        "density:comfortable",
        "density:compact",
    ] {
        assert_eq!(
            requests_to(&server, "GET", &format!("{}/tokens?mode={pair}", base())).len(),
            1,
            "{:?}",
            server.requests()
        );
    }
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

// ---------------------------------------------------------------------------
// omni tokens set and remove
// ---------------------------------------------------------------------------

/// An API whose head holds `tree` (none: no tokens) and that answers a PUT
/// with `put_status`.
fn mock_tree(tree: Option<Value>, put_status: u16) -> MockServer {
    MockServer::start(move |request| {
        let Some(path) = request.path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        match (request.method.as_str(), path) {
            ("GET", "/tokens") => Reply::json(
                200,
                &json!({ "project_id": PROJECT, "doc_id": DOC, "modes": ["light", "dark"], "tokens": tree }),
            ),
            ("PUT", "/tokens") if put_status == 200 => Reply::json(
                200,
                &json!({ "project_id": PROJECT, "doc_id": DOC, "modes": ["light", "dark"] }),
            ),
            ("PUT", "/tokens") => Reply::json(
                put_status,
                &json!({ "error": "invalid_tokens", "message": "state.count is refused by the server" }),
            ),
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

fn tree_with_alias() -> Value {
    let mut tree = tree();
    tree["state"]["total"] = json!({"$type": "number", "$value": "{state.count}"});
    tree
}

/// The tree of the one PUT the server got.
fn put_tree(server: &MockServer) -> Value {
    let puts = requests_to(server, "PUT", &format!("{}/tokens", base()));
    assert_eq!(puts.len(), 1, "{:?}", server.requests());
    assert_agent_request(&puts[0]);
    let body: Value = serde_json::from_slice(&puts[0].body).expect("a JSON body");
    body["tokens"].clone()
}

#[test]
fn tokens_set_changes_one_value_and_puts_the_whole_tree() {
    let server = mock_tree(Some(tree()), 200);
    let home = logged_in(&server.url);
    let out = run(&home, &["tokens", "set", "state.count", "3"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.trim(), "state.count = 3");
    let mut expected = tree();
    expected["state"]["count"]["$value"] = json!(3);
    assert_eq!(put_tree(&server), expected);
}

#[test]
fn tokens_set_takes_json_when_it_parses_and_text_otherwise() {
    for (given, ty, written, shown) in [
        ("true", "boolean", json!(true), "true"),
        ("\"Pay now\"", "string", json!("Pay now"), "\"Pay now\""),
        ("Pay now", "string", json!("Pay now"), "\"Pay now\""),
    ] {
        let server = mock_tree(Some(tree()), 200);
        let home = logged_in(&server.url);
        let out = run(
            &home,
            &["tokens", "set", "state.label", given, "--type", ty],
        );
        assert_eq!(out.code, 0, "{given}: {}", out.stderr);
        assert_eq!(out.stdout.trim(), format!("state.label = {shown}"));
        assert_eq!(
            put_tree(&server)["state"]["label"],
            json!({"$type": ty, "$value": written})
        );
    }
}

#[test]
fn tokens_set_with_a_mode_writes_that_mode_and_fills_the_others() {
    let server = mock_tree(Some(tree()), 200);
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["tokens", "set", "color.ink", "#111111", "--mode", "dark"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.trim(), "color.ink (dark) = \"#111111\"");
    let ink = put_tree(&server)["color"]["ink"].clone();
    assert_eq!(ink["$value"], json!("#0f172a"));
    assert_eq!(
        ink["$extensions"]["omnistate"]["modes"],
        json!({"light": "#0f172a", "dark": "#111111"})
    );

    let server = mock_tree(Some(tree()), 200);
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "tokens",
            "set",
            "color.surface",
            "#222222",
            "--mode",
            "dark",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        put_tree(&server)["color"]["surface"]["$extensions"]["omnistate"]["modes"],
        json!({"light": "#ffffff", "dark": "#222222"})
    );
}

#[test]
fn tokens_set_refuses_before_sending_anything() {
    let server = mock_tree(Some(tree()), 200);
    let home = logged_in(&server.url);
    for (args, needle) in [
        (
            vec!["tokens", "set", "state.count", "3", "--mode", "dark"],
            "`dark` is not a mode of `state`",
        ),
        (
            vec!["tokens", "set", "state.nope", "3"],
            "`state.nope` is not a token; pass --type to create it",
        ),
        (
            vec!["tokens", "set", "state.nope", "abc", "--type", "number"],
            "state.nope",
        ),
        (
            vec!["tokens", "set", "state.count", "3", "--type", "string"],
            "`state.count` is a number token",
        ),
        (
            vec!["tokens", "set", "state", "3", "--type", "number"],
            "collection",
        ),
        (
            vec!["tokens", "set", "state.count.x", "3", "--type", "number"],
            "is a token",
        ),
        (
            vec!["tokens", "set", "state.a..b", "3", "--type", "number"],
            "not a token path",
        ),
        (
            vec!["tokens", "set", "state.x", "3", "--type", "pixels"],
            "not a type",
        ),
    ] {
        let out = run(&home, &args);
        assert_eq!(out.code, 2, "{args:?}: {}", out.stderr);
        assert_one_line_error(&out, &[needle]);
    }
    assert!(
        requests_to(&server, "PUT", &format!("{}/tokens", base())).is_empty(),
        "{:?}",
        server.requests()
    );
}

#[test]
fn tokens_set_with_a_type_creates_the_groups_and_the_token() {
    let server = mock_tree(Some(tree()), 200);
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["tokens", "set", "state.cart.items", "0", "--type", "number"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.trim(), "state.cart.items = 0");
    assert_eq!(
        put_tree(&server)["state"]["cart"],
        json!({"items": {"$type": "number", "$value": 0}})
    );
}

#[test]
fn tokens_set_with_a_type_starts_a_design_system_without_tokens() {
    let server = mock_tree(None, 200);
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["tokens", "set", "state.count", "3", "--type", "number"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        put_tree(&server),
        json!({
            "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
            "state": {"count": {"$type": "number", "$value": 3}}
        })
    );
}

#[test]
fn tokens_set_prints_the_servers_detail_when_the_put_is_refused() {
    let server = mock_tree(Some(tree()), 422);
    let home = logged_in(&server.url);
    let out = run(&home, &["tokens", "set", "state.count", "3"]);
    assert_one_line_error(&out, &["state.count is refused by the server"]);
    assert_eq!(out.code, 1);
}

#[test]
fn tokens_remove_deletes_a_token_and_the_groups_it_empties() {
    let server = mock_tree(Some(tree()), 200);
    let home = logged_in(&server.url);
    let out = run(&home, &["tokens", "remove", "state.count"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.trim(), "removed state.count");
    let mut expected = tree();
    expected.as_object_mut().unwrap().remove("state");
    assert_eq!(put_tree(&server), expected);

    let server = mock_tree(Some(tree()), 200);
    let home = logged_in(&server.url);
    let out = run(&home, &["tokens", "remove", "color.ink"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(put_tree(&server)["color"].get("ink").is_none());
}

#[test]
fn tokens_remove_refuses_a_missing_token_a_group_and_a_token_that_is_aliased() {
    let server = mock_tree(Some(tree_with_alias()), 200);
    let home = logged_in(&server.url);
    for (path, needle) in [
        (
            "state.count",
            "`state.total` refers to `state.count`: change it first",
        ),
        ("state.nope", "`state.nope` is not a token"),
        ("state", "`state` is not a token"),
    ] {
        let out = run(&home, &["tokens", "remove", path]);
        assert_eq!(out.code, 2, "{path}: {}", out.stderr);
        assert_one_line_error(&out, &[needle]);
    }
    assert!(
        requests_to(&server, "PUT", &format!("{}/tokens", base())).is_empty(),
        "{:?}",
        server.requests()
    );
}
