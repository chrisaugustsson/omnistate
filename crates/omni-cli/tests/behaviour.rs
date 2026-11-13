//! US-201 to US-212: `omni behaviour add`, `read`, `edit`, `list`, `move`, `ask`, `answer`, `questions`, `rule` and `rules`, tested as the built binary against
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
//!   The read prints one block per node, the chain root first, blocks
//!   separated by a blank line and the node's own after `=== this node ===`:
//!   `# <title>`, `path: <path>  level: <feature|story|situation>  status:
//!   <status>` (plus `  needs another pass`, `  same-as: <path>` or `  handoff:
//!   <path>`), `Summary:`, `Situation:`, `Behaviour:` (verbatim), `Decisions:`
//!   (`- <question> → <answer>`, ` (default)` for an agent's own) and `Open
//!   questions:` (`- <question>  [<id>]`), then `Rules that apply here:`,
//!   `Children:` and `Siblings:`; a line or section with nothing is left out.
//! - `behavior` is a second spelling of the command word.
//! - `omni behaviour edit <node>` sends a `PATCH` to the node the list gives
//!   (by uuid or path) with the `version` the list gives as
//!   `expected_version` and the fields given; `--status out-of-scope` is sent
//!   as `out_of_scope`, and `--behavior` is `--behaviour`. It prints
//!   `<path>  <status>` from the answer.
//! - `omni behaviour list [<node>]` lists the feature, or `?under=` the node
//!   the list gives at that path, one line per node: `<slug> — <summary>
//!   (<status>)`, indented two spaces per level below the first line, the
//!   title for an empty summary and the root's title for its slug. A feature
//!   without a tree prints nothing.
//! - `omni behaviour move <node> --to <parent>` posts to the move route of
//!   the node the list gives, with its listed `version` as `expected_version`
//!   and the listed id of `<parent>` as `parent_id`, and prints the new path.
//! - `--same-as <node>` and `--handoff <node>` on `add` and `edit` send the
//!   listed id of that node as `same_as` or `handoff`; `--same-as ""` in
//!   `edit` sends `null`. A uuid the list does not have is exit 1 with
//!   `not_found: no node <id> in this feature`, and nothing is sent. Both
//!   flags together are sent, and the API's 422 is the error. `list` ends
//!   the line of a node with a pointer in ` → same-as <path>` or
//!   ` → handoff <path>`.
//! - `omni behaviour ask <node> <question>` posts `{question}` to the
//!   questions route of the node the list gives, and prints the question's
//!   id. `omni behaviour questions [<node>] [--open]` asks for `state=open`
//!   with `--open` and `under=<id>` with a node, and prints `<node path>
//!   <question id>  <question>` per question, the root's path as `.`.
//! - `omni behaviour answer <question id> <answer> [--by human]` posts
//!   `{expected_version, answer, by}` to the answers route of that question,
//!   with the `version` the question list gives, and prints the question's
//!   id. `ask --answer <text> [--by human]` sends `answer` and `by` with the
//!   question. `questions --default` asks for `state=answered&by=agent` and
//!   ends each line in ` → <answer>`.
//! - `ask --recommend <text>` sends `recommendation` with the question; with
//!   `--answer` both are sent, and the API's 422 is the error. An open
//!   question with a recommendation prints it: `read` adds the line
//!   `  Recommended: <text>` under the question's, `questions` ends the line
//!   in ` (recommended: <text>)`, and `question` adds the line `recommended:
//!   <text>` after the question's. An answered question prints as without one.
//! - `answer --change` sends `change: true`. A 409 other than `conflict`
//!   (`not_answered`, `already_answered`, `answered_by_human`) is exit 1 with
//!   the API's code; only a stale version (`conflict`) is exit 3.
//! - `omni behaviour question <id>` prints the question, then the current
//!   answer, then the earlier ones oldest first, each with who gave it
//!   (`person` or `agent <run>/<role>`, an agent's own marked `(default)`)
//!   and when.
//! - `omni behaviour list --needs-pass` asks for `needs_pass=true` and prints
//!   `<path> — <summary> (<status>)` per marked node, not indented; the plain
//!   list ends a marked node's line in ` [needs another pass]`. `omni
//!   behaviour edit <node> --grilled` sends `needs_pass: false` with the
//!   listed version.
//! - `omni behaviour rule add <node> <text> [--scope <s>]` posts `{text,
//!   scope}` to the rules route of the node the list gives and prints the
//!   rule's id. `omni behaviour rules [<node>] [--scope <s>]` asks for
//!   `scope=` and `under=<id>`, and prints `<scope>  <node path>  <rule id>
//!   <text>` per rule in force, the root's path as `.`. `omni behaviour rule
//!   edit <id>` sends the `version` the rule list gives with `scope`, or
//!   `withdrawn: true` for `--withdraw`.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const FEATURE: &str = "01900000-0000-7000-8000-0000000000f1";
const ROOT: &str = "01900000-0000-7000-8000-000000000200";
const STORY: &str = "01900000-0000-7000-8000-000000000201";
const OTHER: &str = "01900000-0000-7000-8000-000000000202";
const NEW: &str = "01900000-0000-7000-8000-000000000203";

const QUESTION: &str = "01900000-0000-7000-8000-000000000301";
const ROOT_QUESTION: &str = "01900000-0000-7000-8000-000000000302";
const SOLD_OUT: &str = "What if the product sold out between page load and the click?";

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
        "version": 1,
        "same_as": null,
        "handoff": null
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
/// under second, so a resolver that takes the first story is caught. That
/// story was written twice, so its version is 3.
fn listed() -> Value {
    let mut story = summary(STORY, Some(ROOT), "customer-buys-product", 1, "Buys");
    story["version"] = json!(3);
    json!({ "nodes": [
        summary(ROOT, None, "", 0, "Checkout"),
        summary(OTHER, Some(ROOT), "customer-returns-product", 1, "Returns"),
        story
    ] })
}

/// An API that lists `listed()`, reads the root, answers a create with a
/// situation under the story (422 `two_pointers` when it has both), answers an edit of the story with the
/// story at the status the request sent, and answers a move of the story
/// with it under the other story.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let base = tree_path();
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", path) if path == base => Reply::json(200, &listed()),
            ("GET", path) if path == format!("{base}/nodes/{ROOT}") => Reply::json(
                200,
                &json!({ "node": node(ROOT, None, "", 0, "Checkout"), "chain": [] }),
            ),
            ("POST", path) if path == format!("{base}/nodes") => {
                let sent: Value = serde_json::from_slice(&request.body).expect("a JSON body");
                if sent.get("same_as").is_some() && sent.get("handoff").is_some() {
                    return Reply::json(422, &json!({ "error": "two_pointers" }));
                }
                Reply::json(
                    201,
                    &json!({ "node": node(
                        NEW,
                        Some(STORY),
                        "customer-buys-product/user-presses-the-buy-button",
                        2,
                        "User presses the buy button"
                    ) }),
                )
            }
            ("POST", path) if path == format!("{base}/nodes/{STORY}/move") => Reply::json(
                200,
                &json!({ "node": node(
                    STORY,
                    Some(OTHER),
                    "customer-returns-product/customer-buys-product",
                    2,
                    "Buys"
                ) }),
            ),
            ("PATCH", path) if path == format!("{base}/nodes/{STORY}") => {
                let sent: Value = serde_json::from_slice(&request.body).expect("a JSON body");
                let mut story = node(STORY, Some(ROOT), "customer-buys-product", 1, "Buys");
                story["status"] = sent["status"].clone();
                story["version"] = json!(4);
                Reply::json(200, &json!({ "node": story }))
            }
            ("POST", path) if path == format!("{base}/nodes/{STORY}/questions") => Reply::json(
                201,
                &json!({ "question": question(QUESTION, STORY, "customer-buys-product", SOLD_OUT) }),
            ),
            ("GET", path) if path == format!("{base}/questions?state=open") => Reply::json(
                200,
                &json!({ "questions": [
                    question(ROOT_QUESTION, ROOT, "", "Who may buy?"),
                    question(QUESTION, STORY, "customer-buys-product", SOLD_OUT)
                ] }),
            ),
            ("GET", path) if path == format!("{base}/questions?under={STORY}") => Reply::json(
                200,
                &json!({ "questions": [
                    question(QUESTION, STORY, "customer-buys-product", SOLD_OUT)
                ] }),
            ),
            ("GET", path) if path == format!("{base}/questions") => Reply::json(
                200,
                &json!({ "questions": [
                    question(ROOT_QUESTION, ROOT, "", "Who may buy?"),
                    listed_question()
                ] }),
            ),
            ("POST", path) if path == format!("{base}/questions/{QUESTION}/answers") => {
                Reply::json(201, &json!({ "question": answered_question() }))
            }
            ("GET", path) if path == format!("{base}/questions?state=answered&by=agent") => {
                Reply::json(
                    200,
                    &json!({ "questions": [
                        answered_question()
                    ] }),
                )
            }
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

const STOCK: &str = "Same message, and the stock indicator updates.";

/// The question on the story, written twice, so its version is 3.
fn listed_question() -> Value {
    let mut question = question(QUESTION, STORY, "customer-buys-product", SOLD_OUT);
    question["version"] = json!(3);
    question
}

/// The question on the story, answered by the agent on its own.
fn answered_question() -> Value {
    let mut question = listed_question();
    question["state"] = json!("answered");
    question["answered_by"] = json!("agent");
    question["version"] = json!(4);
    question["answer"] = json!({
        "answer": STOCK,
        "given_by": "agent",
        "author": { "kind": "agent", "run": "t1", "role": "grill" },
        "at": "2026-11-12T10:00:00.000000Z"
    });
    question
}

fn question(id: &str, node: &str, path: &str, text: &str) -> Value {
    json!({
        "id": id,
        "node_id": node,
        "node_path": path,
        "question": text,
        "state": "open",
        "answered_by": null,
        "asked_by": { "kind": "agent", "run": "t1", "role": "grill" },
        "version": 1,
        "created_at": "2026-11-12T10:00:00.000000Z"
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
    sent(server, "POST")
}

fn sent(server: &MockServer, method: &str) -> Vec<Value> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == method)
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
    assert_eq!(
        out.stdout,
        "=== this node ===\n# Checkout\npath: .  level: feature  status: open\n"
    );
}

#[test]
fn behavior_is_the_same_command_word() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["behavior", "read", "."]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("Checkout"), "{:?}", out.stdout);
}

#[test]
fn edit_by_id_sends_the_listed_version_with_the_new_text_and_prints_path_and_status() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "edit",
            STORY,
            "--situation",
            "The user is on a product page and presses buy.",
            "--behaviour",
            "The product is added to the cart and a confirmation appears.",
            "--status",
            "done",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "customer-buys-product  done\n");
    assert_eq!(
        sent(&server, "PATCH"),
        vec![json!({
            "expected_version": 3,
            "situation": "The user is on a product page and presses buy.",
            "behaviour": "The product is added to the cart and a confirmation appears.",
            "status": "done",
        })]
    );
}

#[test]
fn edit_status_out_of_scope_sends_and_prints_out_of_scope() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "edit",
            "customer-buys-product",
            "--status",
            "out-of-scope",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "customer-buys-product  out_of_scope\n");
    assert_eq!(
        sent(&server, "PATCH"),
        vec![json!({ "expected_version": 3, "status": "out_of_scope" })]
    );
}

#[test]
fn behavior_is_the_same_flag_as_behaviour() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "edit",
            "customer-buys-product",
            "--behavior",
            "A confirmation appears.",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        sent(&server, "PATCH"),
        vec![json!({ "expected_version": 3, "behaviour": "A confirmation appears." })]
    );
}

#[test]
fn move_sends_the_listed_version_and_the_parents_id_and_prints_the_new_path() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "move",
            "customer-buys-product",
            "--to",
            "customer-returns-product",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "customer-returns-product/customer-buys-product\n"
    );
    let moves: Vec<String> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .map(|r| r.path)
        .collect();
    assert_eq!(moves, vec![format!("{}/nodes/{STORY}/move", tree_path())]);
    assert_eq!(
        posts(&server),
        vec![json!({ "expected_version": 3, "parent_id": OTHER })]
    );
}

const PRESS: &str = "01900000-0000-7000-8000-000000000204";
const OUT: &str = "01900000-0000-7000-8000-000000000205";

fn with(mut node: Value, slug: Option<&str>, summary: &str, status: &str) -> Value {
    node["slug"] = json!(slug);
    node["summary"] = json!(summary);
    node["status"] = json!(status);
    node
}

/// The story `customer-buys-product`, then `user-presses-buy` below it and
/// `product-out-of-stock` (no summary) below that.
fn story_nodes() -> Vec<Value> {
    vec![
        with(
            summary(STORY, Some(ROOT), "customer-buys-product", 1, "Buys"),
            Some("customer-buys-product"),
            "Customer buys a product.",
            "open",
        ),
        with(
            summary(
                PRESS,
                Some(STORY),
                "customer-buys-product/user-presses-buy",
                2,
                "User presses buy",
            ),
            Some("user-presses-buy"),
            "Product goes to the cart, or the user sees why it can't.",
            "done",
        ),
        with(
            summary(
                OUT,
                Some(PRESS),
                "customer-buys-product/user-presses-buy/product-out-of-stock",
                3,
                "Product is out of stock",
            ),
            Some("product-out-of-stock"),
            "",
            "open",
        ),
    ]
}

/// The whole tree: the root (no slug), the story with its nodes, then a
/// second story.
fn whole_tree() -> Value {
    let mut nodes = vec![with(
        summary(ROOT, None, "", 0, "Checkout"),
        None,
        "",
        "open",
    )];
    nodes.extend(story_nodes());
    nodes.push(with(
        summary(OTHER, Some(ROOT), "customer-returns-product", 1, "Returns"),
        Some("customer-returns-product"),
        "Customer sends a product back.",
        "open",
    ));
    json!({ "nodes": nodes })
}

/// An API that lists `nodes` for the whole tree and the story's nodes for
/// `?under=` the story.
fn list_mock(nodes: Value) -> MockServer {
    MockServer::start(move |request| {
        let base = tree_path();
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", path) if path == base => Reply::json(200, &nodes),
            ("GET", path) if path == format!("{base}?under={STORY}") => {
                Reply::json(200, &json!({ "nodes": story_nodes() }))
            }
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

#[test]
fn list_of_a_story_prints_its_nodes_indented_below_it_with_the_title_for_an_empty_summary() {
    let server = list_mock(whole_tree());
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "list", "customer-buys-product"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "customer-buys-product — Customer buys a product. (open)\n\
         \x20 user-presses-buy — Product goes to the cart, or the user sees why it can't. (done)\n\
         \x20   product-out-of-stock — Product is out of stock (open)\n"
    );
}

#[test]
fn list_of_the_feature_names_the_root_by_its_title_and_indents_the_stories_below_it() {
    let server = list_mock(whole_tree());
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "list"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "Checkout — Checkout (open)\n\
         \x20 customer-buys-product — Customer buys a product. (open)\n\
         \x20   user-presses-buy — Product goes to the cart, or the user sees why it can't. (done)\n\
         \x20     product-out-of-stock — Product is out of stock (open)\n\
         \x20 customer-returns-product — Customer sends a product back. (open)\n"
    );
}

#[test]
fn list_of_a_feature_without_a_tree_prints_nothing_and_exits_0() {
    let server = list_mock(json!({ "nodes": [] }));
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "list"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "");
}

#[test]
fn edit_same_as_a_path_sends_its_id_and_an_empty_same_as_sends_null() {
    let server = mock();
    let home = logged_in(&server);

    let set = run(
        &home,
        &[
            "behaviour",
            "edit",
            "customer-buys-product",
            "--same-as",
            "customer-returns-product",
        ],
    );
    let cleared = run(
        &home,
        &[
            "behaviour",
            "edit",
            "customer-buys-product",
            "--same-as",
            "",
        ],
    );

    assert_eq!(set.code, 0, "stderr: {}", set.stderr);
    assert_eq!(cleared.code, 0, "stderr: {}", cleared.stderr);
    assert_eq!(
        sent(&server, "PATCH"),
        vec![
            json!({ "expected_version": 3, "same_as": OTHER }),
            json!({ "expected_version": 3, "same_as": null }),
        ]
    );
}

#[test]
fn a_pointer_to_a_uuid_the_feature_does_not_have_exits_1_with_not_found_and_sends_nothing() {
    // Settled decision: resolved locally rather than sent for the API's 422.
    let server = mock();
    let home = logged_in(&server);
    let foreign = "01900000-0000-7000-8000-0000000009f9";

    let out = run(
        &home,
        &[
            "behaviour",
            "edit",
            "customer-buys-product",
            "--same-as",
            foreign,
        ],
    );

    assert_eq!(out.code, 1, "stdout: {}", out.stdout);
    assert!(
        out.stderr
            .contains(&format!("not_found: no node {foreign} in this feature")),
        "{:?}",
        out.stderr
    );
    assert!(sent(&server, "PATCH").is_empty());
}

#[test]
fn same_as_and_handoff_together_are_sent_and_the_apis_422_is_the_error() {
    // Settled decision: no local usage error, so omni refuses as the API does.
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "add",
            "--title",
            "User goes to checkout",
            "--same-as",
            "customer-buys-product",
            "--handoff",
            "customer-returns-product",
        ],
    );

    assert_eq!(out.code, 1, "stdout: {}", out.stdout);
    assert!(out.stderr.contains("two_pointers"), "{:?}", out.stderr);
    assert_eq!(
        posts(&server),
        vec![json!({
            "title": "User goes to checkout",
            "same_as": STORY,
            "handoff": OTHER
        })]
    );
}

#[test]
fn list_ends_the_line_of_a_node_with_a_pointer_in_the_pointer_and_its_path() {
    let mut nodes = story_nodes();
    nodes[2]["handoff"] = json!({ "id": OTHER, "path": "customer-returns-product" });
    let mut sold_out = with(
        summary(
            NEW,
            Some(STORY),
            "customer-buys-product/product-is-sold-out",
            2,
            "Product is sold out",
        ),
        Some("product-is-sold-out"),
        "",
        "open",
    );
    sold_out["same_as"] = json!({
        "id": OUT,
        "path": "customer-buys-product/user-presses-buy/product-out-of-stock"
    });
    nodes.push(sold_out);
    let server = MockServer::start(move |request| {
        let base = tree_path();
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", path) if path == base || path == format!("{base}?under={STORY}") => {
                Reply::json(200, &json!({ "nodes": nodes }))
            }
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    });
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "list", "customer-buys-product"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "customer-buys-product — Customer buys a product. (open)\n\
         \x20 user-presses-buy — Product goes to the cart, or the user sees why it can't. (done)\n\
         \x20   product-out-of-stock — Product is out of stock (open) → handoff customer-returns-product\n\
         \x20 product-is-sold-out — Product is sold out (open) → same-as customer-buys-product/user-presses-buy/product-out-of-stock\n"
    );
}

#[test]
fn ask_on_a_path_posts_the_question_to_that_nodes_id_and_prints_the_question_id() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &["behaviour", "ask", "customer-buys-product", SOLD_OUT],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, format!("{QUESTION}\n"));
    assert_eq!(posts(&server), vec![json!({ "question": SOLD_OUT })]);
}

#[test]
fn questions_open_prints_path_id_and_question_per_line_with_the_root_as_a_dot() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "questions", "--open"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!(
            ".  {ROOT_QUESTION}  Who may buy?\n\
             customer-buys-product  {QUESTION}  {SOLD_OUT}\n"
        )
    );
}

#[test]
fn questions_of_a_path_asks_for_those_under_that_nodes_id() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "questions", "customer-buys-product"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!("customer-buys-product  {QUESTION}  {SOLD_OUT}\n")
    );
}

#[test]
fn answer_sends_the_listed_version_and_by_to_that_questions_answers_and_prints_its_id() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &["behaviour", "answer", QUESTION, STOCK, "--by", "human"],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, format!("{QUESTION}\n"));
    let posted: Vec<String> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .map(|r| r.path)
        .collect();
    assert_eq!(
        posted,
        vec![format!("{}/questions/{QUESTION}/answers", tree_path())]
    );
    assert_eq!(
        posts(&server),
        vec![json!({ "expected_version": 3, "answer": STOCK, "by": "human" })]
    );
}

#[test]
fn ask_with_an_answer_sends_the_answer_and_by_with_the_question() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "ask",
            "customer-buys-product",
            SOLD_OUT,
            "--answer",
            STOCK,
            "--by",
            "human",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        posts(&server),
        vec![json!({ "question": SOLD_OUT, "answer": STOCK, "by": "human" })]
    );
}

const REFUSED: &str = "The buy is refused with a message that the product is sold out.";

/// The question on the story, open, with a recommendation.
fn recommended_question() -> Value {
    let mut question = listed_question();
    question["recommendation"] = json!(REFUSED);
    question["answer"] = json!(null);
    question["answers"] = json!([]);
    question
}

/// The question on the story, with a recommendation, answered by a person.
fn recommended_and_answered() -> Value {
    let mut question = recommended_question();
    question["state"] = json!("answered");
    question["answered_by"] = json!("human");
    question["answers"] = json!([{
        "answer": STOCK,
        "given_by": "human",
        "author": { "kind": "human", "user_id": "01900000-0000-7000-8000-0000000000a1" },
        "at": "2026-11-12T12:00:00.000000Z"
    }]);
    question["answer"] = question["answers"][0].clone();
    question
}

/// An API with the root and the story, whose question lists (plain and
/// `state=open`) and question read give `asked`, and which refuses a
/// question asked with both a recommendation and an answer.
fn recommending(asked: Value) -> MockServer {
    MockServer::start(move |request| {
        let base = tree_path();
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", path) if path == base => Reply::json(
                200,
                &json!({ "nodes": [
                    summary(ROOT, None, "", 0, "Checkout"),
                    summary(STORY, Some(ROOT), "customer-buys-product", 1, "Buys")
                ] }),
            ),
            ("POST", path) if path == format!("{base}/nodes/{STORY}/questions") => {
                let sent: Value = serde_json::from_slice(&request.body).expect("a JSON body");
                if sent.get("recommendation").is_some() && sent.get("answer").is_some() {
                    return Reply::json(422, &json!({ "error": "recommendation_with_answer" }));
                }
                Reply::json(201, &json!({ "question": asked }))
            }
            ("GET", path)
                if path == format!("{base}/questions")
                    || path == format!("{base}/questions?state=open") =>
            {
                Reply::json(
                    200,
                    &json!({ "questions": [
                        question(ROOT_QUESTION, ROOT, "", "Who may buy?"),
                        asked.clone()
                    ] }),
                )
            }
            ("GET", path) if path == format!("{base}/questions/{QUESTION}") => {
                Reply::json(200, &json!({ "question": asked }))
            }
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

#[test]
fn ask_with_recommend_sends_the_recommendation_with_the_question_and_prints_the_question_id() {
    let server = recommending(recommended_question());
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "ask",
            "customer-buys-product",
            SOLD_OUT,
            "--recommend",
            REFUSED,
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, format!("{QUESTION}\n"));
    assert_eq!(
        posts(&server),
        vec![json!({ "question": SOLD_OUT, "recommendation": REFUSED })]
    );
}

#[test]
fn recommend_and_answer_together_are_sent_and_the_apis_422_is_the_error() {
    // The same policy as `--same-as` with `--handoff`: no local usage error.
    let server = recommending(recommended_question());
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "ask",
            "customer-buys-product",
            SOLD_OUT,
            "--recommend",
            REFUSED,
            "--answer",
            STOCK,
        ],
    );

    assert_eq!(out.code, 1, "stdout: {}", out.stdout);
    assert!(
        out.stderr.contains("recommendation_with_answer"),
        "{:?}",
        out.stderr
    );
    assert_eq!(
        posts(&server),
        vec![json!({ "question": SOLD_OUT, "recommendation": REFUSED, "answer": STOCK })]
    );
}

#[test]
fn questions_end_the_line_of_an_open_question_in_its_recommendation_plain_and_open() {
    let server = recommending(recommended_question());
    let home = logged_in(&server);

    let plain = run(&home, &["behaviour", "questions"]);
    let open = run(&home, &["behaviour", "questions", "--open"]);

    let lines = format!(
        ".  {ROOT_QUESTION}  Who may buy?\n\
         customer-buys-product  {QUESTION}  {SOLD_OUT} (recommended: {REFUSED})\n"
    );
    assert_eq!(plain.code, 0, "stderr: {}", plain.stderr);
    assert_eq!(plain.stdout, lines);
    assert_eq!(open.code, 0, "stderr: {}", open.stderr);
    assert_eq!(open.stdout, lines);
}

#[test]
fn questions_print_an_answered_question_with_a_recommendation_without_it() {
    let server = recommending(recommended_and_answered());
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "questions"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!(
            ".  {ROOT_QUESTION}  Who may buy?\n\
             customer-buys-product  {QUESTION}  {SOLD_OUT}\n"
        )
    );
}

#[test]
fn question_prints_the_recommendation_of_an_open_question_after_the_question_line() {
    let server = recommending(recommended_question());
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "question", QUESTION]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!(
            "customer-buys-product  {QUESTION}  {SOLD_OUT}\n\
             recommended: {REFUSED}\n"
        )
    );
}

#[test]
fn question_prints_an_answered_question_with_a_recommendation_without_it() {
    let server = recommending(recommended_and_answered());
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "question", QUESTION]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!(
            "customer-buys-product  {QUESTION}  {SOLD_OUT}\n\
             answer: {STOCK} — person, 2026-11-12T12:00:00.000000Z\n"
        )
    );
}

#[test]
fn questions_default_asks_for_the_agents_own_answers_and_ends_each_line_in_the_answer() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "questions", "--default"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!("customer-buys-product  {QUESTION}  {SOLD_OUT} → {STOCK}\n")
    );
}

/// An API that lists the questions and refuses every answer with 409 `code`.
fn refusing(code: &'static str) -> MockServer {
    MockServer::start(move |request| {
        let base = tree_path();
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", path) if path == format!("{base}/questions") => {
                Reply::json(200, &json!({ "questions": [listed_question()] }))
            }
            ("POST", path) if path == format!("{base}/questions/{QUESTION}/answers") => {
                if code == "conflict" {
                    Reply::json(409, &json!({ "error": code, "current_version": 4 }))
                } else {
                    Reply::json(409, &json!({ "error": code }))
                }
            }
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

#[test]
fn a_refused_change_exits_1_with_the_apis_code_and_only_a_stale_version_exits_3() {
    // Settled decision: exit 3 stays for 409 `conflict` alone.
    let not_answered = refusing("not_answered");
    let answered_by_human = refusing("answered_by_human");
    let already_answered = refusing("already_answered");
    let stale = refusing("conflict");
    let change = [
        "behaviour",
        "answer",
        QUESTION,
        "Guest cart after all.",
        "--change",
    ];

    let not_answered_out = run(&logged_in(&not_answered), &change);
    let answered_by_human_out = run(&logged_in(&answered_by_human), &change);
    let already_answered_out = run(
        &logged_in(&already_answered),
        &["behaviour", "answer", QUESTION, "Guest cart after all."],
    );
    let stale_out = run(&logged_in(&stale), &change);

    assert_eq!(
        not_answered_out.code, 1,
        "stderr: {}",
        not_answered_out.stderr
    );
    assert!(
        not_answered_out.stderr.contains("not_answered"),
        "{:?}",
        not_answered_out.stderr
    );
    assert_eq!(
        answered_by_human_out.code, 1,
        "stderr: {}",
        answered_by_human_out.stderr
    );
    assert!(
        answered_by_human_out.stderr.contains("answered_by_human"),
        "{:?}",
        answered_by_human_out.stderr
    );
    assert_eq!(
        already_answered_out.code, 1,
        "stderr: {}",
        already_answered_out.stderr
    );
    assert!(
        already_answered_out.stderr.contains("already_answered"),
        "{:?}",
        already_answered_out.stderr
    );
    assert_eq!(stale_out.code, 3, "stderr: {}", stale_out.stderr);
    assert_eq!(
        posts(&answered_by_human),
        vec![json!({ "expected_version": 3, "answer": "Guest cart after all.", "change": true })]
    );
}

#[test]
fn question_prints_the_current_answer_first_then_the_earlier_ones_oldest_first_with_who_gave_each()
{
    let mut answered = listed_question();
    answered["state"] = json!("answered");
    answered["answered_by"] = json!("human");
    answered["answers"] = json!([
        {
            "answer": "Guest cart",
            "given_by": "agent",
            "author": { "kind": "agent", "run": "t1", "role": "grill" },
            "at": "2026-11-12T10:00:00.000000Z"
        },
        {
            "answer": "Account cart.",
            "given_by": "agent",
            "author": { "kind": "agent", "run": "t2", "role": "review" },
            "at": "2026-11-12T11:00:00.000000Z"
        },
        {
            "answer": "Login required before buying.",
            "given_by": "human",
            "author": { "kind": "human", "user_id": "01900000-0000-7000-8000-0000000000a1" },
            "at": "2026-11-12T12:00:00.000000Z"
        }
    ]);
    answered["answer"] = answered["answers"][2].clone();
    let server =
        MockServer::start(
            move |request| match (request.method.as_str(), request.path.as_str()) {
                ("GET", path) if path == format!("{}/questions/{QUESTION}", tree_path()) => {
                    Reply::json(200, &json!({ "question": answered }))
                }
                _ => Reply::json(404, &json!({ "error": "not_found" })),
            },
        );
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "question", QUESTION]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{}", out.stdout);
    assert!(lines[0].contains(SOLD_OUT), "{}", lines[0]);
    assert!(
        lines[1].contains("Login required before buying."),
        "{}",
        lines[1]
    );
    assert!(lines[1].contains("person"), "{}", lines[1]);
    assert!(!lines[1].contains("(default)"), "{}", lines[1]);
    assert!(lines[1].contains("2026-11-12T12:00:00"), "{}", lines[1]);
    assert!(lines[2].contains("Guest cart"), "{}", lines[2]);
    assert!(lines[2].contains("(default)"), "{}", lines[2]);
    assert!(lines[2].contains("agent t1/grill"), "{}", lines[2]);
    assert!(lines[2].contains("2026-11-12T10:00:00"), "{}", lines[2]);
    assert!(lines[3].contains("Account cart."), "{}", lines[3]);
    assert!(lines[3].contains("(default)"), "{}", lines[3]);
    assert!(lines[3].contains("agent t2/review"), "{}", lines[3]);
    assert!(lines[3].contains("2026-11-12T11:00:00"), "{}", lines[3]);
}

/// The whole tree with `user-presses-buy` marked for another pass, and only
/// it and `product-out-of-stock` (given a summary) for `?needs_pass=true`.
fn marked_mock() -> MockServer {
    let mut tree = whole_tree();
    tree["nodes"][2]["needs_pass"] = json!(true);
    let mut marked = story_nodes()[1..].to_vec();
    marked[1]["summary"] = json!("The user sees that it is sold out.");
    MockServer::start(move |request| {
        let base = tree_path();
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", path) if path == base => Reply::json(200, &tree),
            ("GET", path) if path == format!("{base}?needs_pass=true") => {
                Reply::json(200, &json!({ "nodes": marked }))
            }
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

#[test]
fn list_needs_pass_prints_the_marked_nodes_by_path_one_per_line_not_indented() {
    let server = marked_mock();
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "list", "--needs-pass"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "customer-buys-product/user-presses-buy — Product goes to the cart, or the user sees why it can't. (done)\n\
         customer-buys-product/user-presses-buy/product-out-of-stock — The user sees that it is sold out. (open)\n"
    );
}

#[test]
fn list_ends_the_line_of_a_marked_node_in_needs_another_pass() {
    let server = marked_mock();
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "list"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "Checkout — Checkout (open)\n\
         \x20 customer-buys-product — Customer buys a product. (open)\n\
         \x20   user-presses-buy — Product goes to the cart, or the user sees why it can't. (done) [needs another pass]\n\
         \x20     product-out-of-stock — Product is out of stock (open)\n\
         \x20 customer-returns-product — Customer sends a product back. (open)\n"
    );
}

#[test]
fn edit_grilled_sends_needs_pass_false_with_the_listed_version() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &["behaviour", "edit", "customer-buys-product", "--grilled"],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        sent(&server, "PATCH"),
        vec![json!({ "expected_version": 3, "needs_pass": false })]
    );
}

const RULE: &str = "01900000-0000-7000-8000-000000000401";
const ROOT_RULE: &str = "01900000-0000-7000-8000-000000000402";
const WITHDRAWN_RULE: &str = "01900000-0000-7000-8000-000000000403";
const BUY_DISABLED: &str = "Buy button is disabled whenever the product is out of stock.";

fn rule(id: &str, node: &str, path: &str, scope: &str, text: &str) -> Value {
    json!({
        "id": id,
        "node_id": node,
        "node_path": path,
        "text": text,
        "scope": scope,
        "withdrawn": false,
        "author": { "kind": "agent", "run": "t1", "role": "grill" },
        "version": 1,
        "created_at": "2026-11-12T10:00:00.000000Z"
    })
}

/// The story's rule, written twice, so its version is 3.
fn story_rule() -> Value {
    let mut rule = rule(RULE, STORY, "customer-buys-product", "story", BUY_DISABLED);
    rule["version"] = json!(3);
    rule
}

/// A withdrawn rule on the story.
fn withdrawn_rule() -> Value {
    let mut rule = rule(
        WITHDRAWN_RULE,
        STORY,
        "customer-buys-product",
        "node",
        "Only members can buy.",
    );
    rule["withdrawn"] = json!(true);
    rule
}

/// An API that lists `listed()`, gives a feature rule on the root, the
/// story's rule and a withdrawn rule on the story, answers `?scope=feature`
/// with the root's rule and `?under=` the story with the story's two, and
/// answers a new rule on the story and an edit of its rule.
fn rules_mock() -> MockServer {
    MockServer::start(|request| {
        let base = tree_path();
        let root_rule = rule(ROOT_RULE, ROOT, "", "feature", "Prices include VAT.");
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", path) if path == base => Reply::json(200, &listed()),
            ("GET", path) if path == format!("{base}/rules") => Reply::json(
                200,
                &json!({ "rules": [root_rule, story_rule(), withdrawn_rule()] }),
            ),
            ("GET", path) if path == format!("{base}/rules?scope=feature") => {
                Reply::json(200, &json!({ "rules": [root_rule] }))
            }
            ("GET", path) if path == format!("{base}/rules?under={STORY}") => {
                Reply::json(200, &json!({ "rules": [story_rule(), withdrawn_rule()] }))
            }
            ("POST", path) if path == format!("{base}/nodes/{STORY}/rules") => {
                Reply::json(201, &json!({ "rule": story_rule() }))
            }
            ("PATCH", path) if path == format!("{base}/rules/{RULE}") => {
                Reply::json(200, &json!({ "rule": story_rule() }))
            }
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

#[test]
fn rule_add_on_a_path_posts_the_text_and_scope_to_that_nodes_rules_and_prints_the_rule_id() {
    let server = rules_mock();
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "behaviour",
            "rule",
            "add",
            "customer-buys-product",
            BUY_DISABLED,
            "--scope",
            "story",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, format!("{RULE}\n"));
    assert_eq!(
        posts(&server),
        vec![json!({ "text": BUY_DISABLED, "scope": "story" })]
    );
}

#[test]
fn rules_prints_scope_path_id_and_text_per_line_without_the_withdrawn_and_the_root_as_a_dot() {
    let server = rules_mock();
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "rules"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!(
            "feature  .  {ROOT_RULE}  Prices include VAT.\n\
             story  customer-buys-product  {RULE}  {BUY_DISABLED}\n"
        )
    );

    let out = run(&home, &["behaviour", "rules", "--json"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let listed: Value = serde_json::from_str(&out.stdout).expect("JSON on stdout");
    let withdrawn: Vec<&Value> = listed["rules"]
        .as_array()
        .expect("a rules list")
        .iter()
        .filter(|rule| rule["id"] == WITHDRAWN_RULE)
        .collect();
    assert_eq!(withdrawn.len(), 1, "stdout: {}", out.stdout);
    assert_eq!(withdrawn[0]["withdrawn"], json!(true));
}

#[test]
fn rules_scope_feature_asks_for_the_feature_rules() {
    let server = rules_mock();
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "rules", "--scope", "feature"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!("feature  .  {ROOT_RULE}  Prices include VAT.\n")
    );
}

#[test]
fn rules_of_a_path_asks_for_those_under_that_nodes_id() {
    let server = rules_mock();
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "rules", "customer-buys-product"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!("story  customer-buys-product  {RULE}  {BUY_DISABLED}\n")
    );
}

#[test]
fn rule_edit_sends_the_listed_version_with_the_scope_or_withdrawn_true() {
    let server = rules_mock();
    let home = logged_in(&server);

    let scoped = run(
        &home,
        &["behaviour", "rule", "edit", RULE, "--scope", "feature"],
    );
    let withdrawn = run(&home, &["behaviour", "rule", "edit", RULE, "--withdraw"]);

    assert_eq!(scoped.code, 0, "stderr: {}", scoped.stderr);
    assert_eq!(withdrawn.code, 0, "stderr: {}", withdrawn.stderr);
    assert_eq!(
        sent(&server, "PATCH"),
        vec![
            json!({ "expected_version": 3, "scope": "feature" }),
            json!({ "expected_version": 3, "withdrawn": true })
        ]
    );
}

const PRESS_PATH: &str = "customer-buys-product/user-presses-buy";
const OUT_PATH: &str = "customer-buys-product/user-presses-buy/out-of-stock";
const ZERO: &str = "01900000-0000-7000-8000-000000000303";

fn answered(id: &str, node: &str, path: &str, text: &str, answer: &str, by: &str) -> Value {
    let mut question = question(id, node, path, text);
    question["state"] = json!("answered");
    question["answered_by"] = json!(by);
    question["answer"] = json!({
        "answer": answer,
        "given_by": by,
        "author": { "kind": "agent", "run": "t1", "role": "grill" },
        "at": "2026-11-12T10:00:00.000000Z"
    });
    question
}

fn child(id: &str, path: &str, title: &str, summary: &str, status: &str) -> Value {
    json!({
        "id": id,
        "path": path,
        "slug": path.rsplit('/').next(),
        "title": title,
        "summary": summary,
        "status": status,
        "same_as": null,
        "handoff": null
    })
}

/// An API whose tree has the root, the story and `user-presses-buy` above
/// `out-of-stock` (level 3), and whose read of `out-of-stock` is `read`.
fn read_mock(read: Value) -> MockServer {
    MockServer::start(move |request| {
        let base = tree_path();
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", path) if path == base => Reply::json(
                200,
                &json!({ "nodes": [
                    summary(ROOT, None, "", 0, "Checkout"),
                    summary(STORY, Some(ROOT), "customer-buys-product", 1, "Buys"),
                    summary(PRESS, Some(STORY), PRESS_PATH, 2, "User presses buy"),
                    summary(OUT, Some(PRESS), OUT_PATH, 3, "Out of stock")
                ] }),
            ),
            ("GET", path) if path == format!("{base}/nodes/{OUT}") => Reply::json(200, &read),
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

#[test]
fn read_prints_the_branch_root_first_with_each_nodes_decisions_then_rules_children_and_siblings() {
    // The verifier's defect: the node's own questions sit at the top level of
    // the read, not inside `node`, and its block printed none of them.
    let mut root = node(ROOT, None, "", 0, "Checkout");
    root["behaviour"] = json!("Hub screens:\n- Cart\n\n- Orders");
    root["questions"] = json!([]);
    let mut story = node(STORY, Some(ROOT), "customer-buys-product", 1, "Buys");
    story["summary"] = json!("Customer buys a product.");
    story["questions"] = json!([answered(
        QUESTION,
        STORY,
        "customer-buys-product",
        "Who may buy?",
        "Anyone with an account.",
        "agent"
    )]);
    let mut press = node(PRESS, Some(STORY), PRESS_PATH, 2, "User presses buy");
    press["situation"] = json!("The product page is open.");
    press["status"] = json!("done");
    press["questions"] = json!([]);
    let mut oos = node(OUT, Some(PRESS), OUT_PATH, 3, "Out of stock");
    oos["summary"] = json!("Item is gone.");
    oos["behaviour"] = json!("Show a notice.");
    let server = read_mock(json!({
        "node": oos,
        "chain": [root, story, press],
        "questions": [
            question(ZERO, OUT, OUT_PATH, "What if quantity is zero?"),
            answered(ROOT_QUESTION, OUT, OUT_PATH, "Logged out user?", "Guest cart, identical behaviour.", "agent"),
            answered(NEW, OUT, OUT_PATH, "Region delivery check here?", "No, handled at checkout.", "human")
        ],
        "rules": [
            rule(ROOT_RULE, ROOT, "", "feature", "Prices include VAT."),
            rule(RULE, STORY, "customer-buys-product", "story", BUY_DISABLED)
        ],
        "children": [
            child(RULE, &format!("{OUT_PATH}/restocked"), "Restocked", "It comes back.", "open"),
            child(WITHDRAWN_RULE, &format!("{OUT_PATH}/gone-for-good"), "Gone for good", "", "out_of_scope")
        ],
        "siblings": [
            child(OTHER, &format!("{PRESS_PATH}/in-stock"), "In stock", "Goes to the cart.", "done")
        ]
    }));
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "read", OUT_PATH]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!(
            "# Checkout
path: .  level: feature  status: open
Behaviour: Hub screens:
- Cart

- Orders

# Buys
path: customer-buys-product  level: story  status: open
Summary: Customer buys a product.
Decisions:
- Who may buy? → Anyone with an account. (default)

# User presses buy
path: {PRESS_PATH}  level: situation  status: done
Situation: The product page is open.

=== this node ===
# Out of stock
path: {OUT_PATH}  level: situation  status: open
Summary: Item is gone.
Behaviour: Show a notice.
Decisions:
- Logged out user? → Guest cart, identical behaviour. (default)
- Region delivery check here? → No, handled at checkout.
Open questions:
- What if quantity is zero?  [{ZERO}]

Rules that apply here:
- Prices include VAT. (feature, from .)
- {BUY_DISABLED} (story, from customer-buys-product)

Children:
- restocked — It comes back. (open)
- gone-for-good — Gone for good (out_of_scope)

Siblings:
- in-stock — Goes to the cart. (done)
"
        )
    );
}

#[test]
fn read_prints_the_recommendation_under_its_open_question_and_not_under_an_answered_one() {
    let mut root = node(ROOT, None, "", 0, "Checkout");
    root["questions"] = json!([]);
    let mut story = node(STORY, Some(ROOT), "customer-buys-product", 1, "Buys");
    let mut on_story = question(QUESTION, STORY, "customer-buys-product", "Who may buy?");
    on_story["recommendation"] = json!("Anyone with an account.");
    on_story["answer"] = json!(null);
    story["questions"] = json!([on_story]);
    let mut press = node(PRESS, Some(STORY), PRESS_PATH, 2, "User presses buy");
    press["questions"] = json!([]);
    let oos = node(OUT, Some(PRESS), OUT_PATH, 3, "Out of stock");
    let mut open = question(ZERO, OUT, OUT_PATH, "What if quantity is zero?");
    open["recommendation"] = json!("The buy button is disabled.");
    open["answer"] = json!(null);
    let mut decided = answered(
        NEW,
        OUT,
        OUT_PATH,
        "Region delivery check here?",
        "No, handled at checkout.",
        "human",
    );
    decided["recommendation"] = json!("Yes, check it here.");
    let server = read_mock(json!({
        "node": oos,
        "chain": [root, story, press],
        "questions": [
            open,
            question(ROOT_QUESTION, OUT, OUT_PATH, "Logged out user?"),
            decided
        ],
        "rules": [],
        "children": [],
        "siblings": []
    }));
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "read", OUT_PATH]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        format!(
            "# Checkout
path: .  level: feature  status: open

# Buys
path: customer-buys-product  level: story  status: open
Open questions:
- Who may buy?  [{QUESTION}]
  Recommended: Anyone with an account.

# User presses buy
path: {PRESS_PATH}  level: situation  status: open

=== this node ===
# Out of stock
path: {OUT_PATH}  level: situation  status: open
Decisions:
- Region delivery check here? → No, handled at checkout.
Open questions:
- What if quantity is zero?  [{ZERO}]
  Recommended: The buy button is disabled.
- Logged out user?  [{ROOT_QUESTION}]
"
        )
    );
}

#[test]
fn read_of_a_marked_node_with_a_handoff_ends_its_path_line_in_both_and_leaves_out_empty_sections() {
    let mut press = node(PRESS, Some(STORY), PRESS_PATH, 2, "User presses buy");
    press["questions"] = json!([]);
    let mut oos = node(OUT, Some(PRESS), OUT_PATH, 3, "Out of stock");
    oos["needs_pass"] = json!(true);
    oos["needs_pass_since"] = json!("2026-11-12T11:00:00.000000Z");
    oos["handoff"] = json!({
        "id": OTHER,
        "path": "customer-returns-product",
        "title": "Returns",
        "summary": ""
    });
    let mut root = node(ROOT, None, "", 0, "Checkout");
    root["questions"] = json!([]);
    let mut story = node(STORY, Some(ROOT), "customer-buys-product", 1, "Buys");
    story["questions"] = json!([]);
    let server = read_mock(json!({
        "node": oos,
        "chain": [root, story, press],
        "questions": [],
        "rules": [],
        "children": [],
        "siblings": []
    }));
    let home = logged_in(&server);

    let out = run(&home, &["behaviour", "read", OUT_PATH]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stdout.ends_with(&format!(
            "=== this node ===
# Out of stock
path: {OUT_PATH}  level: situation  status: open  needs another pass  handoff: customer-returns-product
"
        )),
        "{:?}",
        out.stdout
    );
}
