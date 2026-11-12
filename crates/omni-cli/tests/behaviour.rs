//! US-201 to US-208: `omni behaviour add`, `read`, `edit`, `list`, `move`, `ask`, `answer` and `questions`, tested as the built binary against
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
//! - `answer --change` sends `change: true`. A 409 other than `conflict`
//!   (`not_answered`, `already_answered`, `answered_by_human`) is exit 1 with
//!   the API's code; only a stale version (`conflict`) is exit 3.
//! - `omni behaviour question <id>` prints the question, then the current
//!   answer, then the earlier ones oldest first, each with who gave it
//!   (`person` or `agent <run>/<role>`, an agent's own marked `(default)`)
//!   and when.

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
