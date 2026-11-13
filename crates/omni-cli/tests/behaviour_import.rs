//! US-214: `omni behaviour import <folder> [--check]`, tested as the built
//! binary against a mock HTTP server, with the grill-tree folder in
//! tests/fixtures/grill-tree. What the API does with the body is in
//! crates/omni-api/tests/behaviour.rs (US-213); these tests pin how the CLI
//! reads the folder into that body.
//!
//! The contract:
//!
//! - Every folder with a `node.md` is a node, its folder name the slug; the
//!   generated files at the root and dot files and folders are skipped. The
//!   `# ` line is the title, `Summary` gives its first line, `Situation` and
//!   `Behavior` are sent trimmed with their inner lines verbatim (as
//!   `situation` and `behaviour`).
//! - Siblings come in the order of the parent's `Children` lines, then the
//!   folders the list leaves out by name; a line without a folder is ignored.
//! - `same-as` and `handoff` are relative paths, sent as the key of the node
//!   they name. `out-of-scope` is sent as `out_of_scope`, `stale` as `open`.
//! - A node longer than 60 lines, a handoff into a story and a `Children`
//!   list that disagrees with the folders are not problems.
//! - Everything is checked before any request. Each problem is one line on
//!   stderr naming the node by its path below the root, exit 1, and nothing
//!   is sent; a missing pointer target names the field and the path as
//!   written. `--check` sends no request at all and prints `would import
//!   <n> nodes (<n> stories), <n> same-as, <n> handoffs, <n> questions (<n> by
//!   default, <n> open), <n> rules`; the import sends one `POST
//!   .../behaviour/import` and prints the same counts after `imported`. A
//!   refused import (`tree_not_empty`) is exit 1 with the API's code.
//!
//! US-215 adds the three list sections of a node:
//!
//! - `Decisions`: `- <question> → <old> → <new>` is one question with its
//!   answers oldest first; a line ending in `(default)` loses the marker and
//!   its answers are `by: "agent"`, any other line's `by: "human"`.
//! - `Open questions`: one open question per line; the node keeps its status.
//! - `Constraints`: one rule per line, scoped by the level it stands in (root
//!   `feature`, story `story`, situation `node`), its text verbatim with a
//!   `(default)` kept. `(from <path>)` moves the rule to the node at that path
//!   with the scope of where the line stood, the marker stripped.
//! - A section line not starting with `- `, a decision without ` → ` and a
//!   `(from …)` path that names no node are problems: one line naming the
//!   node and the line, exit 1, nothing sent.

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const FEATURE: &str = "01900000-0000-7000-8000-0000000000f1";

const COUNTS: &str =
    "10 nodes (3 stories), 1 same-as, 1 handoffs, 4 questions (1 by default, 1 open), 4 rules";

fn import_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/features/{FEATURE}/behaviour/import")
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grill-tree")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A copy of the fixture tree that a test may change.
fn copy_of_fixture() -> Scratch {
    let copy = Scratch::new("tree");
    copy_dir(&fixture(), &copy.path);
    copy
}

fn rewrite(file: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(file).unwrap();
    assert!(text.contains(from), "{} holds {from:?}", file.display());
    std::fs::write(file, text.replacen(from, to, 1)).unwrap();
}

/// An API whose import route answers `status` with `body`.
fn import_mock(status: u16, body: Value) -> MockServer {
    MockServer::start(move |request| {
        if request.method == "POST" && request.path == import_path() {
            Reply::json(status, &body)
        } else {
            Reply::json(404, &json!({ "error": "not_found" }))
        }
    })
}

fn accepting() -> MockServer {
    import_mock(
        201,
        json!({ "nodes": 10, "questions": 0, "answers": 0, "rules": 0 }),
    )
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

fn import(server: &MockServer, folder: &Path) -> Out {
    let home = logged_in(server);
    run(&home, &["behaviour", "import", folder.to_str().unwrap()])
}

/// The nodes of the one import request the server got.
fn sent_nodes(server: &MockServer) -> Vec<Value> {
    let posts: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .collect();
    assert_eq!(posts.len(), 1, "one POST");
    assert_eq!(posts[0].path, import_path());
    let body: Value = serde_json::from_slice(&posts[0].body).expect("a JSON body");
    body["nodes"].as_array().expect("nodes").clone()
}

/// The path below the root of every node in the body, by key: the slugs
/// from the story down along `parent`, the root's empty.
fn paths_by_key(nodes: &[Value]) -> HashMap<String, String> {
    let by_key: HashMap<String, &Value> = nodes
        .iter()
        .map(|node| (node["key"].as_str().unwrap().to_string(), node))
        .collect();
    nodes
        .iter()
        .map(|node| {
            let mut slugs = Vec::new();
            let mut at = node;
            while let Some(parent) = at["parent"].as_str() {
                slugs.push(at["slug"].as_str().unwrap());
                at = by_key[parent];
            }
            slugs.reverse();
            (node["key"].as_str().unwrap().to_string(), slugs.join("/"))
        })
        .collect()
}

fn at<'a>(nodes: &'a [Value], path: &str) -> &'a Value {
    let paths = paths_by_key(nodes);
    nodes
        .iter()
        .find(|node| paths[node["key"].as_str().unwrap()] == path)
        .unwrap_or_else(|| panic!("no node at {path:?}"))
}

/// The slugs of the children of the node at `parent`, in body order.
fn children(nodes: &[Value], parent: &str) -> Vec<String> {
    let key = at(nodes, parent)["key"].clone();
    nodes
        .iter()
        .filter(|node| node["parent"] == key)
        .map(|node| node["slug"].as_str().unwrap().to_string())
        .collect()
}

/// The path of the node that the pointer `field` of the node at `path`
/// names, or `None` when it has none.
fn pointer(nodes: &[Value], path: &str, field: &str) -> Option<String> {
    let paths = paths_by_key(nodes);
    at(nodes, path)[field]
        .as_str()
        .map(|key| paths[key].clone())
}

fn lines_naming<'a>(stderr: &'a str, path: &str) -> Vec<&'a str> {
    stderr.lines().filter(|line| line.contains(path)).collect()
}

#[test]
fn import_sends_every_node_folder_in_one_request_and_prints_the_counts() {
    let server = accepting();
    let out = import(&server, &fixture());
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, format!("imported {COUNTS}\n"));
    let nodes = sent_nodes(&server);
    let mut paths: Vec<String> = paths_by_key(&nodes).into_values().collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            "",
            "player-edits-history",
            "player-edits-history/deletes-practice",
            "player-picks-program",
            "player-picks-program/browses-programs",
            "player-picks-program/picks-short-program",
            "player-runs-session",
            "player-runs-session/logs-last-drill",
            "player-runs-session/logs-last-drill/deletes-on-summary",
            "player-runs-session/starts-from-dashboard",
        ]
    );
    let roots: Vec<&Value> = nodes.iter().filter(|n| n["parent"].is_null()).collect();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0]["title"], "Putting trainer");
    assert_eq!(
        at(&nodes, "player-runs-session/logs-last-drill")["title"],
        "Logs the last drill"
    );
}

#[test]
fn siblings_follow_the_children_lines_then_the_unlisted_folders_by_name() {
    let server = accepting();
    let out = import(&server, &fixture());
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let nodes = sent_nodes(&server);
    assert_eq!(
        children(&nodes, ""),
        [
            "player-runs-session",
            "player-picks-program",
            "player-edits-history"
        ]
    );
    assert_eq!(
        children(&nodes, "player-runs-session"),
        ["starts-from-dashboard", "logs-last-drill"]
    );
    assert_eq!(
        children(&nodes, "player-picks-program"),
        ["browses-programs", "picks-short-program"]
    );
}

#[test]
fn a_pointer_is_sent_as_the_key_of_the_folder_its_relative_path_names() {
    let server = accepting();
    let out = import(&server, &fixture());
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let nodes = sent_nodes(&server);
    assert_eq!(
        pointer(
            &nodes,
            "player-picks-program/picks-short-program",
            "same_as"
        )
        .as_deref(),
        Some("player-runs-session/starts-from-dashboard")
    );
    assert_eq!(
        pointer(
            &nodes,
            "player-runs-session/logs-last-drill/deletes-on-summary",
            "handoff"
        )
        .as_deref(),
        Some("player-edits-history/deletes-practice")
    );
    let with_pointer: Vec<&Value> = nodes
        .iter()
        .filter(|n| n["same_as"].is_string() || n["handoff"].is_string())
        .collect();
    assert_eq!(with_pointer.len(), 2);
}

#[test]
fn out_of_scope_is_sent_as_out_of_scope_and_stale_as_open() {
    let server = accepting();
    let out = import(&server, &fixture());
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let nodes = sent_nodes(&server);
    assert_eq!(at(&nodes, "player-picks-program")["status"], "out_of_scope");
    assert_eq!(
        at(&nodes, "player-runs-session/starts-from-dashboard")["status"],
        "open"
    );
    assert_eq!(at(&nodes, "player-runs-session")["status"], "done");
}

#[test]
fn sections_are_sent_trimmed_with_their_inner_lines_verbatim_and_the_summarys_first_line() {
    let server = accepting();
    let out = import(&server, &fixture());
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let nodes = sent_nodes(&server);
    assert_eq!(
        at(&nodes, "")["behaviour"],
        "The trainer opens on the dashboard.\n\
         \n\
         Hub screens:\n\
         - [Dashboard](./player-runs-session/starts-from-dashboard/) — where a session starts\n\
         - [History](./player-edits-history/deletes-practice/) — past practices"
    );
    let practice = at(&nodes, "player-edits-history/deletes-practice");
    assert_eq!(
        practice["situation"],
        "Player opens a practice in History\nand presses Delete.\n\nThe practice was logged today."
    );
    assert_eq!(practice["summary"], "Player deletes a logged practice.");
    assert_eq!(
        at(&nodes, "player-runs-session/starts-from-dashboard")["summary"],
        "Player opens a session from the dashboard."
    );
}

#[test]
fn check_prints_would_import_with_the_counts_and_sends_no_request() {
    let server = accepting();
    let home = logged_in(&server);
    let out = run(
        &home,
        &[
            "behaviour",
            "import",
            fixture().to_str().unwrap(),
            "--check",
        ],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, format!("would import {COUNTS}\n"));
    assert!(server.requests().is_empty());
}

#[test]
fn a_same_as_to_a_folder_that_does_not_exist_names_the_node_and_the_path_as_written_and_sends_nothing()
 {
    let tree = copy_of_fixture();
    rewrite(
        &tree
            .path
            .join("player-picks-program/picks-short-program/node.md"),
        "same-as: ../../player-runs-session/starts-from-dashboard",
        "same-as: ../../player-runs-session/nowhere",
    );
    let server = accepting();
    let out = import(&server, &tree.path);
    assert_eq!(out.code, 1, "stdout: {}", out.stdout);
    let named = lines_naming(&out.stderr, "player-picks-program/picks-short-program");
    assert_eq!(named.len(), 1, "stderr: {}", out.stderr);
    assert!(named[0].contains("same-as"), "{}", named[0]);
    assert!(
        named[0].contains("../../player-runs-session/nowhere"),
        "{}",
        named[0]
    );
    assert!(server.requests().is_empty());
}

#[test]
fn every_problem_in_the_folder_is_one_line_naming_its_node_and_nothing_is_sent() {
    let tree = copy_of_fixture();
    let root = &tree.path;
    std::fs::create_dir(root.join("player-edits-history/empty-folder")).unwrap();
    std::fs::create_dir(root.join("player-picks-program/Bad_Folder")).unwrap();
    std::fs::copy(
        root.join("player-picks-program/browses-programs/node.md"),
        root.join("player-picks-program/Bad_Folder/node.md"),
    )
    .unwrap();
    rewrite(
        &root.join("player-picks-program/browses-programs/node.md"),
        "# Browses programs\n",
        "",
    );
    rewrite(
        &root.join("player-runs-session/starts-from-dashboard/node.md"),
        "status: stale",
        "status: blocked",
    );
    rewrite(
        &root.join("player-edits-history/deletes-practice/node.md"),
        "same-as:\nhandoff:\n",
        "same-as: ../../player-runs-session\nhandoff: ../../player-runs-session\n",
    );
    rewrite(
        &root.join("player-runs-session/logs-last-drill/node.md"),
        "handoff:\n",
        "handoff: ../starts-from-dashboard\n",
    );
    rewrite(&root.join("node.md"), "status: done", "status: blocked");
    let problem_nodes = [
        "player-edits-history/empty-folder",
        "player-picks-program/Bad_Folder",
        "player-picks-program/browses-programs",
        "player-runs-session/starts-from-dashboard",
        "player-edits-history/deletes-practice",
        "player-runs-session/logs-last-drill",
    ];
    let server = accepting();
    let home = logged_in(&server);
    let folder = root.to_str().unwrap();
    let out = run(&home, &["behaviour", "import", folder]);
    assert_eq!(out.code, 1, "stdout: {}", out.stdout);
    assert_eq!(out.stdout, "");
    for path in problem_nodes {
        let named = lines_naming(&out.stderr, path);
        assert_eq!(named.len(), 1, "{path} in stderr: {}", out.stderr);
    }
    let root_lines = out
        .stderr
        .lines()
        .filter(|line| line.starts_with(".:") || line.starts_with(". "))
        .count();
    assert_eq!(root_lines, 1, "the root as . in stderr: {}", out.stderr);
    let check = run(&home, &["behaviour", "import", folder, "--check"]);
    assert_eq!(check.code, 1, "stdout: {}", check.stdout);
    assert_eq!(check.stdout, "");
    assert_eq!(check.stderr, out.stderr);
    assert!(server.requests().is_empty());
}

#[test]
fn an_import_into_a_feature_with_a_tree_exits_1_with_tree_not_empty() {
    let server = import_mock(409, json!({ "error": "tree_not_empty" }));
    let out = import(&server, &fixture());
    assert_eq!(out.code, 1, "stdout: {}", out.stdout);
    assert!(out.stderr.contains("tree_not_empty"), "{}", out.stderr);
    assert_eq!(out.stdout, "");
}

/// The rule with `text` among the rules sent on the node at `path`.
fn rule<'a>(nodes: &'a [Value], path: &str, text: &str) -> &'a Value {
    at(nodes, path)["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .find(|rule| rule["text"] == text)
        .unwrap_or_else(|| panic!("no rule {text:?} on {path:?}"))
}

#[test]
fn a_decision_is_one_question_with_its_answers_oldest_first_and_default_gives_them_to_the_agent() {
    let server = accepting();
    let out = import(&server, &fixture());
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let nodes = sent_nodes(&server);
    assert_eq!(
        at(&nodes, "player-runs-session/logs-last-drill")["questions"],
        json!([
            {
                "question": "Guest checkout?",
                "answers": [
                    { "answer": "Yes", "by": "human" },
                    { "answer": "No, login required", "by": "human" }
                ]
            },
            {
                "question": "Is the score shown at once?",
                "answers": [{ "answer": "Yes, on the summary.", "by": "agent" }]
            }
        ])
    );
    assert_eq!(
        at(&nodes, "")["questions"],
        json!([{
            "question": "Is a friend a separate role?",
            "answers": [{ "answer": "No, friends are other players.", "by": "human" }]
        }])
    );
}

#[test]
fn an_open_question_is_sent_without_answers() {
    let server = accepting();
    let out = import(&server, &fixture());
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let nodes = sent_nodes(&server);
    let story = at(&nodes, "player-runs-session");
    assert_eq!(
        story["questions"],
        json!([{ "question": "Should a session pause on a phone call?", "answers": [] }])
    );
}

#[test]
fn a_constraint_is_scoped_by_the_level_it_stands_in_and_from_moves_it_to_that_node() {
    let server = accepting();
    let out = import(&server, &fixture());
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let nodes = sent_nodes(&server);
    assert_eq!(
        at(&nodes, "")["rules"],
        json!([{ "text": "Scores are kept per player.", "scope": "feature" }])
    );
    assert_eq!(
        at(&nodes, "player-runs-session")["rules"],
        json!([{ "text": "A session has at least one drill.", "scope": "story" }])
    );
    let drill = "player-runs-session/logs-last-drill";
    assert_eq!(at(&nodes, drill)["rules"].as_array().unwrap().len(), 2);
    assert_eq!(
        rule(&nodes, drill, "Prices include VAT.")["scope"],
        "feature"
    );
    assert_eq!(
        rule(
            &nodes,
            drill,
            "The summary lists drills in the order played. (default)"
        )["scope"],
        "node"
    );
}

#[test]
fn a_section_line_without_a_dash_a_decision_without_an_arrow_and_an_unknown_from_path_are_each_one_line_and_nothing_is_sent()
 {
    let tree = copy_of_fixture();
    let root = &tree.path;
    rewrite(
        &root.join("player-runs-session/logs-last-drill/node.md"),
        "- Guest checkout? → Yes → No, login required",
        "- Guest checkout?",
    );
    rewrite(
        &root.join("player-runs-session/node.md"),
        "- Should a session pause on a phone call?",
        "Should a session pause on a phone call?",
    );
    rewrite(
        &root.join("node.md"),
        "(from player-runs-session/logs-last-drill)",
        "(from player-runs-session/nowhere)",
    );
    let server = accepting();
    let out = import(&server, root);
    assert_eq!(out.code, 1, "stdout: {}", out.stdout);
    assert_eq!(out.stdout, "");
    for (node, line) in [
        ("player-runs-session/logs-last-drill", "Guest checkout?"),
        (
            "player-runs-session",
            "Should a session pause on a phone call?",
        ),
        (".", "Prices include VAT."),
    ] {
        let named: Vec<&str> = out.stderr.lines().filter(|l| l.contains(line)).collect();
        assert_eq!(named.len(), 1, "{line} in stderr: {}", out.stderr);
        assert!(named[0].starts_with(node), "{node} in {}", named[0]);
    }
    assert!(server.requests().is_empty());
}
