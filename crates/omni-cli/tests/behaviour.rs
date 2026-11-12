//! US-201: `omni behaviour add` and `read`, tested as the built binary against
//! a mock HTTP server. The API's own rules (the root, slugs, levels, the
//! chain) are in crates/omni-api/tests/behaviour.rs; these tests pin what the
//! CLI adds on top: naming a node by its path or by `.` for the root, which
//! the CLI resolves through the feature's node list.
//!
//! The contract (all paths under `/api/v1/w/{workspace}/projects/{project}`
//! `/features/{feature}/behaviour`):
//!
//! - `omni behaviour add --parent <node>` posts the `parent_id` of the node
//!   the list gives at that path, and prints the new node's path. A path the
//!   list does not have is exit 1 with a message naming it, and nothing is
//!   posted.
//! - `omni behaviour read .` reads the root and prints its empty path as `.`.
//! - `behavior` is a second spelling of the command word.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const FEATURE: &str = "01900000-0000-7000-8000-0000000000f1";
const ROOT: &str = "01900000-0000-7000-8000-000000000200";
const STORY: &str = "01900000-0000-7000-8000-000000000201";
const OTHER: &str = "01900000-0000-7000-8000-000000000202";
const NEW: &str = "01900000-0000-7000-8000-000000000203";

fn tree_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/features/{FEATURE}/behaviour")
}

fn summary(id: &str, parent: Option<&str>, path: &str, level: i64, title: &str) -> Value {
    json!({
        "id": id,
        "parent_id": parent,
        "path": path,
        "slug": path.rsplit('/').next(),
        "position": 0,
        "level": level,
        "title": title,
        "summary": "",
        "status": "open",
        "version": 1
    })
}

fn node(id: &str, parent: Option<&str>, path: &str, level: i64, title: &str) -> Value {
    let mut node = summary(id, parent, path, level, title);
    node["situation"] = json!("");
    node["behaviour"] = json!("");
    node["created_by"] = json!({ "kind": "agent", "run": "t1", "role": "grill" });
    node["created_at"] = json!("2026-11-12T10:00:00.000000Z");
    node["updated_at"] = json!("2026-11-12T10:00:00.000000Z");
    node
}

/// The feature's tree: the root and two stories, the one the tests add
/// under second, so a resolver that takes the first story is caught.
fn listed() -> Value {
    json!({ "nodes": [
        summary(ROOT, None, "", 0, "Checkout"),
        summary(OTHER, Some(ROOT), "customer-returns-product", 1, "Returns"),
        summary(STORY, Some(ROOT), "customer-buys-product", 1, "Buys")
    ] })
}

/// An API that lists `listed()`, reads the root, and answers a create with a
/// situation under the story.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let base = tree_path();
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", path) if path == base => Reply::json(200, &listed()),
            ("GET", path) if path == format!("{base}/nodes/{ROOT}") => Reply::json(
                200,
                &json!({ "node": node(ROOT, None, "", 0, "Checkout"), "chain": [] }),
            ),
            ("POST", path) if path == format!("{base}/nodes") => Reply::json(
                201,
                &json!({ "node": node(
                    NEW,
                    Some(STORY),
                    "customer-buys-product/user-presses-the-buy-button",
                    2,
                    "User presses the buy button"
                ) }),
            ),
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

fn logged_in(server: &MockServer) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, &server.url, WORKSPACE, API_KEY);
    home
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(&["--as", "t1/grill"])
        .args(args)
        .args(&["--project", PROJECT, "--feature", FEATURE])
        .run()
}

fn posts(server: &MockServer) -> Vec<Value> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .map(|r| serde_json::from_slice(&r.body).expect("the request body is JSON"))
        .collect()
}

#[test]
fn add_under_a_parent_path_posts_that_nodes_id_and_prints_the_new_path() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "add",
            "--parent",
            "customer-buys-product",
            "--title",
            "User presses the buy button",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "customer-buys-product/user-presses-the-buy-button\n"
    );
    assert_eq!(
        posts(&server),
        vec![json!({ "title": "User presses the buy button", "parent_id": STORY })]
    );
}

#[test]
fn add_under_a_path_the_feature_does_not_have_exits_1_names_the_path_and_posts_nothing() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "add",
            "--parent",
            "no-such-node",
            "--title",
            "Press",
        ],
    );

    assert_eq!(out.code, 1, "stdout: {}", out.stdout);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("no-such-node"), "{:?}", out.stderr);
    assert!(posts(&server).is_empty());
}

#[test]
fn read_dot_reads_the_root_and_prints_its_path_as_a_dot() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "read", "."]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    // The layout of the line is US-212's; this story fixes the path's form.
    assert_eq!(
        out.stdout.split_whitespace().next(),
        Some("."),
        "{:?}",
        out.stdout
    );
    assert!(out.stdout.contains("Checkout"), "{:?}", out.stdout);
}

#[test]
fn behavior_is_the_same_command_word() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["behavior", "read", "."]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("Checkout"), "{:?}", out.stdout);
}
