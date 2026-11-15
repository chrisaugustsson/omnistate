//! US-304: `omni part`, tested as the built binary against a mock HTTP
//! server. The API's rules for parts are in crates/omni-api/tests/parts.rs.
//!
//! The contract these tests pin down:
//!
//! - `omni part create <name> --kind app|service` sends `POST
//!   /api/v1/w/{workspace}/projects/{project_id}/parts` with `{"name",
//!   "kind"}` and prints the new part's ID, name and kind. A kind other than
//!   `app` or `service` is a one-line error, `kind must be app or service`,
//!   and nothing is sent.
//! - `omni part list` prints one line per part, `<name>  <kind>`.
//! - `omni part delete <name>` looks the name up among the parts that
//!   `GET .../parts` lists and sends `DELETE .../parts/{id}`.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const WISION: &str = "01900000-0000-7000-8000-0000000000c1";
const IOT_SERVICE: &str = "01900000-0000-7000-8000-0000000000c2";
const NEW_PART: &str = "01900000-0000-7000-8000-0000000000c3";

fn parts_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/parts")
}

fn part_json(id: &str, name: &str, kind: &str) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "name": name,
        "kind": kind,
        "description": "",
        "created_at": "2026-11-15T10:00:00.000000Z"
    })
}

/// An API whose project has the parts Wision (app) and iot-service
/// (service), in that order. A new part is answered with the name and kind
/// it was sent, and a delete with 204.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let path = request.path.as_str();
        let Some(rest) = path.strip_prefix(&parts_path()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        match (request.method.as_str(), rest) {
            ("POST", "") => Reply::json(
                201,
                &json!({ "part": part_json(
                    NEW_PART,
                    body["name"].as_str().unwrap_or(""),
                    body["kind"].as_str().unwrap_or("")
                ) }),
            ),
            ("GET", "") => Reply::json(
                200,
                &json!({ "parts": [
                    part_json(WISION, "Wision", "app"),
                    part_json(IOT_SERVICE, "iot-service", "service")
                ] }),
            ),
            ("DELETE", rest)
                if rest == format!("/{WISION}") || rest == format!("/{IOT_SERVICE}") =>
            {
                Reply {
                    status: 204,
                    headers: Vec::new(),
                    body: String::new(),
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
        .args(&["--project", PROJECT, "--as", "run-7/planner"])
        .run()
}

#[test]
fn part_create_with_a_kind_other_than_app_or_service_fails_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["part", "create", "X", "--kind", "device"]);
    assert_ne!(out.code, 0, "expected a failure; stdout: {:?}", out.stdout);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr_lines().len(), 1, "{:?}", out.stderr);
    assert!(
        out.stderr.contains("kind must be app or service"),
        "{:?}",
        out.stderr
    );
    assert!(
        server.requests().is_empty(),
        "nothing is sent: {:?}",
        server.requests()
    );
}

#[test]
fn part_create_sends_the_name_and_kind_and_prints_the_id_name_and_kind() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["part", "create", "Wision", "--kind", "app"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let posts: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path == parts_path())
        .collect();
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    let body: Value = serde_json::from_slice(&posts[0].body).expect("a JSON body");
    assert_eq!(body["name"], "Wision", "{body}");
    assert_eq!(body["kind"], "app", "{body}");
    assert_eq!(
        non_empty_lines(&out.stdout),
        [format!("{NEW_PART}  Wision  app")]
    );
}

#[test]
fn part_list_prints_the_name_and_kind_of_each_part() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["part", "list"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        non_empty_lines(&out.stdout),
        ["Wision  app", "iot-service  service"]
    );
}

#[test]
fn part_delete_by_name_deletes_the_part_of_that_name() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["part", "delete", "iot-service"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let deletes: Vec<String> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "DELETE")
        .map(|r| r.path)
        .collect();
    assert_eq!(deletes, [format!("{}/{IOT_SERVICE}", parts_path())]);
}
