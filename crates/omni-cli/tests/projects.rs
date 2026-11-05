//! US-016: `omni project`, tested as the built binary against a mock HTTP
//! server. None of these tests needs Postgres.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni project create <name>` sends `POST /api/v1/w/{workspace}/projects`
//!   with the JSON body `{"name": "<name>"}`; `omni project list` sends `GET`
//!   on the same path. The API answers `{"project": {"id", "name",
//!   "created_at"}}` and `{"projects": [...]}`.
//! - Short text by default and the API's JSON with `--json`, errors as one line
//!   on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const OTHER_PROJECT: &str = "01900000-0000-7000-8000-0000000000b2";
const NEW_PROJECT: &str = "01900000-0000-7000-8000-0000000000b3";

fn projects_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects")
}

fn project_json(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "created_at": "2026-10-15T10:00:00.000000Z" })
}

/// An API that knows two projects, `brand-site` and `other`.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        if request.path != projects_path() {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        match request.method.as_str() {
            "GET" => Reply::json(
                200,
                &json!({ "projects": [
                    project_json(PROJECT, "brand-site"),
                    project_json(OTHER_PROJECT, "other")
                ] }),
            ),
            "POST" => Reply::json(
                201,
                &json!({ "project": project_json(NEW_PROJECT, body["name"].as_str().unwrap_or("?")) }),
            ),
            _ => Reply::json(405, &json!({ "error": "method_not_allowed" })),
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
        .args(&["--as", "run-7/author"])
        .run()
}

fn requests_to(server: &MockServer, method: &str, path: &str) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == method && r.path == path)
        .collect()
}

fn body_of(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body))
}

#[test]
fn project_create_sends_the_name_as_the_agent_and_prints_the_new_project() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["project", "create", "landing-page"]);
    assert_eq!(out.code, 0, "{}", out.stderr);

    let posts = requests_to(&server, "POST", &projects_path());
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    assert_eq!(body_of(&posts[0])["name"], "landing-page");
    assert_eq!(
        posts[0].header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(posts[0].header("x-omni-run"), Some("run-7"));
    assert_eq!(posts[0].header("x-omni-role"), Some("author"));

    assert!(
        serde_json::from_str::<Value>(&out.stdout).is_err(),
        "the default output is plain text: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains(NEW_PROJECT) && out.stdout.contains("landing-page"),
        "the text has the project's ID and name: {:?}",
        out.stdout
    );
    assert_eq!(
        non_empty_lines(&out.stdout),
        vec![
            format!("created project landing-page ({NEW_PROJECT})"),
            "link a folder to it with: omni link landing-page".to_string()
        ]
    );

    let json = run(&home, &["project", "create", "landing-page", "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "project": project_json(NEW_PROJECT, "landing-page") }),
        "--json prints the whole response"
    );
}

#[test]
fn project_list_prints_one_line_per_project() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["project", "list"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(requests_to(&server, "GET", &projects_path()).len(), 1);
    let lines = non_empty_lines(&out.stdout);
    let line_of = |needle: &str| {
        lines
            .iter()
            .filter(|line| line.contains(needle))
            .collect::<Vec<_>>()
    };
    assert_eq!(line_of("brand-site").len(), 1, "{:?}", out.stdout);
    assert_eq!(line_of("other").len(), 1, "{:?}", out.stdout);
    assert!(
        line_of("brand-site")[0].contains(PROJECT) && line_of("other")[0].contains(OTHER_PROJECT),
        "each line has the project's ID: {:?}",
        out.stdout
    );

    let json = run(&home, &["project", "list", "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json()["projects"].as_array().unwrap().len(), 2);
}
