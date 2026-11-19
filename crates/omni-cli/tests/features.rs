//! US-307: `omni feature`, tested as the built binary against a mock HTTP
//! server. The API's rules for features are in crates/omni-api/tests/features.rs.
//!
//! The contract these tests pin down:
//!
//! - `omni feature create <name> --part <part> [--description <text>]` sends
//!   `POST /api/v1/w/{workspace}/projects/{project_id}/parts/{part_id}/features`
//!   with `{"name", "description"}` and prints the new feature's ID.
//! - `omni feature list --part <part>` shows the features of that part only.
//! - `omni feature show <name>` prints the feature's part, name and
//!   description. A name is looked up across the project's features,
//!   ignoring case; a name that several parts have is `ambiguous_feature`
//!   naming those parts, and `--part <part>` narrows the search to one.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const WISION: &str = "01900000-0000-7000-8000-0000000000c1";
const IOT_SERVICE: &str = "01900000-0000-7000-8000-0000000000c2";
const ANALYZE: &str = "01900000-0000-7000-8000-0000000000d1";
const WISION_ALARMS: &str = "01900000-0000-7000-8000-0000000000d2";
const IOT_ALARMS: &str = "01900000-0000-7000-8000-0000000000d3";
const NEW_FEATURE: &str = "01900000-0000-7000-8000-0000000000d4";
const DESCRIPTION: &str = "Time-series chart per channel";

fn parts_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/parts")
}

fn features_path(part: &str) -> String {
    format!("{}/{part}/features", parts_path())
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

fn feature_json(id: &str, part: &str, name: &str, description: &str) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "part_id": part,
        "name": name,
        "description": description,
        "created_at": "2026-11-16T10:00:00.000000Z"
    })
}

/// An API whose project has the parts Wision (app) and iot-service
/// (service). Wision has the features Analyze and Alarms, iot-service has
/// Alarms. No track is linked to a feature. A new feature is answered with
/// the name and description it was sent.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let path = request.path.as_str();
        match request.method.as_str() {
            "GET" if path == parts_path() => Reply::json(
                200,
                &json!({ "parts": [
                    part_json(WISION, "Wision", "app"),
                    part_json(IOT_SERVICE, "iot-service", "service")
                ] }),
            ),
            "GET" if path == features_path(WISION) => Reply::json(
                200,
                &json!({ "features": [
                    feature_json(ANALYZE, WISION, "Analyze", DESCRIPTION),
                    feature_json(WISION_ALARMS, WISION, "Alarms", "")
                ] }),
            ),
            "GET" if path == features_path(IOT_SERVICE) => Reply::json(
                200,
                &json!({ "features": [feature_json(IOT_ALARMS, IOT_SERVICE, "Alarms", "")] }),
            ),
            "GET" if path.starts_with(&parts_path()) && path.ends_with("/tracks") => {
                Reply::json(200, &json!({ "tracks": [] }))
            }
            "POST" if path == features_path(WISION) || path == features_path(IOT_SERVICE) => {
                let part = if path == features_path(WISION) {
                    WISION
                } else {
                    IOT_SERVICE
                };
                Reply::json(
                    201,
                    &json!({ "feature": feature_json(
                        NEW_FEATURE,
                        part,
                        body["name"].as_str().unwrap_or(""),
                        body["description"].as_str().unwrap_or("")
                    ) }),
                )
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
fn feature_create_sends_the_name_and_description_to_the_part_and_prints_the_new_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "feature",
            "create",
            "Analyze",
            "--part",
            "Wision",
            "--description",
            DESCRIPTION,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let posts: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .collect();
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    assert_eq!(posts[0].path, features_path(WISION));
    let body: Value = serde_json::from_slice(&posts[0].body).expect("a JSON body");
    assert_eq!(body["name"], "Analyze", "{body}");
    assert_eq!(body["description"], DESCRIPTION, "{body}");
    assert!(
        out.stdout.contains(NEW_FEATURE),
        "create prints the new feature's ID: {:?}",
        out.stdout
    );
}

#[test]
fn feature_list_with_a_part_shows_the_features_of_that_part_only() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["feature", "list", "--part", "Wision"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let lines = non_empty_lines(&out.stdout);
    assert_eq!(lines.len(), 2, "one line per feature of Wision: {lines:?}");
    assert!(
        lines.iter().any(|line| line.contains("Analyze")),
        "{lines:?}"
    );
    assert!(
        !out.stdout.contains(IOT_ALARMS),
        "no feature of iot-service: {:?}",
        out.stdout
    );
}

#[test]
fn feature_show_by_name_ignoring_case_prints_its_part_name_and_description() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["feature", "show", "analyze"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    for shown in ["Wision", "Analyze", DESCRIPTION] {
        assert!(
            out.stdout.contains(shown),
            "show prints {shown:?}: {:?}",
            out.stdout
        );
    }
}

#[test]
fn a_feature_name_on_several_parts_is_ambiguous_feature_naming_them_and_part_narrows_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["feature", "show", "Alarms"]);
    assert_ne!(out.code, 0, "expected a failure; stdout: {:?}", out.stdout);
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr_lines().len(), 1, "{:?}", out.stderr);
    for needle in ["ambiguous_feature", "Wision", "iot-service"] {
        assert!(
            out.stderr.contains(needle),
            "the error names {needle:?}: {:?}",
            out.stderr
        );
    }
    let out = run(
        &home,
        &["feature", "show", "Alarms", "--part", "iot-service"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains(IOT_ALARMS), "{:?}", out.stdout);
    assert!(!out.stdout.contains(WISION_ALARMS), "{:?}", out.stdout);
}
