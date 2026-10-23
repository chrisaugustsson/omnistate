//! US-102: `omni doc create` without a change set and `omni doc read` of a
//! doc's head, tested as the built `omni` against a mock HTTP server. None of
//! these tests needs Postgres; the real round trip against the API is in
//! tests/stack_docs.rs, and the API's contract in
//! crates/omni-api/tests/live_docs.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`):
//!
//! - `omni doc create --kind <kind> --pillar <pillar> --title <text>`, without
//!   `--changeset`, sends `POST /docs` with `{"kind", "pillar", "title"}`: a
//!   doc at the app level, with no change set. With `--feature <name or id>`
//!   it sends `POST /features/{feature_id}/docs` (a name is looked up in the
//!   project's features, ignoring case, as for `omni doc list`); a name the
//!   project does not have is a one-line error that names it and creates
//!   nothing. It prints the new doc's ID and title; `--json` prints the API's
//!   answer. `--changeset <id>` still starts the doc in that change set.
//! - `omni doc read <doc>` prints the doc's head from `GET /docs/{doc}/html`:
//!   the HTML of an HTML doc (nothing for a new, empty one, which is no
//!   error), and the `tags` of a design (one page for a new one).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHECKOUT: &str = "01900000-0000-7000-8000-0000000000f1";
const SEARCH: &str = "01900000-0000-7000-8000-0000000000f2";
const NEW_DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const NEW_DESIGN: &str = "01900000-0000-7000-8000-0000000000d2";
const PAGE: &str = "p4ge0001";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn feature_json(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "name": name,
        "description": "",
        "created_at": "2026-10-23T10:00:00.000000Z"
    })
}

fn doc_json(id: &str, feature: Option<&str>, body: &Value) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "kind": body["kind"],
        "pillar": body["pillar"],
        "title": body["title"],
        "created_at": "2026-10-23T10:00:00.000000Z",
        "mainline_version_id": null,
        "feature_id": feature,
        "drafts": [],
        "claims": []
    })
}

fn page_tags() -> String {
    format!("<Page id=\"{PAGE}\" name=\"Page 1\"/>")
}

/// An API with the features "Checkout" and "Search", which creates docs at
/// the app level and in either feature, and reads `NEW_DOC` (an empty HTML
/// doc) and `NEW_DESIGN` (an empty design with one page).
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let Some(rest) = request.path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        let created = |feature: Option<&str>| {
            let id = match body["kind"].as_str() {
                Some("html") => NEW_DOC,
                _ => NEW_DESIGN,
            };
            Reply::json(201, &json!({ "doc": doc_json(id, feature, &body) }))
        };
        match (request.method.as_str(), rest) {
            ("GET", "/features") => Reply::json(
                200,
                &json!({ "features": [
                    feature_json(CHECKOUT, "Checkout"),
                    feature_json(SEARCH, "Search")
                ] }),
            ),
            ("POST", "/docs") => created(None),
            ("POST", r) if r == format!("/features/{CHECKOUT}/docs") => created(Some(CHECKOUT)),
            ("POST", r) if r == format!("/features/{SEARCH}/docs") => created(Some(SEARCH)),
            ("POST", r) if r.starts_with("/features/") => {
                Reply::json(404, &json!({ "error": "not_found" }))
            }
            ("GET", r) if r == format!("/docs/{NEW_DOC}/html") => Reply::json(
                200,
                &json!({ "doc_id": NEW_DOC, "version_id": null, "html": "" }),
            ),
            ("GET", r) if r == format!("/docs/{NEW_DESIGN}/html") => Reply::json(
                200,
                &json!({
                    "doc_id": NEW_DESIGN,
                    "version_id": null,
                    "html": "",
                    "tags": page_tags(),
                    "pages": [{ "id": PAGE, "type": "page", "name": "Page 1", "children": [] }]
                }),
            ),
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
        .args(&["--project", PROJECT, "--as", "run-7/author"])
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

fn posts(server: &MockServer) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .collect()
}

fn body_of(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body))
}

fn assert_agent_request(request: &Recorded) {
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str()),
        "{request:?}"
    );
    assert_eq!(request.header("x-omni-run"), Some("run-7"), "{request:?}");
    assert_eq!(request.header("x-omni-role"), Some("author"), "{request:?}");
}

// ---------------------------------------------------------------------------
// omni doc create without a change set
// ---------------------------------------------------------------------------

#[test]
fn doc_create_without_a_change_set_creates_the_doc_at_the_app_level() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "doc",
            "create",
            "--kind",
            "html",
            "--pillar",
            "spec",
            "--title",
            "Checkout plan",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let sent = posts(&server);
    assert_eq!(sent.len(), 1, "{:?}", server.requests());
    assert_eq!(sent[0].path, format!("{}/docs", base()), "{:?}", sent[0]);
    assert_agent_request(&sent[0]);
    let body = body_of(&sent[0]);
    assert_eq!(
        body,
        json!({ "kind": "html", "pillar": "spec", "title": "Checkout plan" }),
        "{body}"
    );
    assert!(
        out.stdout.contains(NEW_DOC) && out.stdout.contains("Checkout plan"),
        "the text has the new doc's ID and title: {:?}",
        out.stdout
    );
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.path.contains("/changesets")),
        "no change set is involved: {:?}",
        server.requests()
    );

    let json = run(
        &home,
        &[
            "doc", "create", "--kind", "design", "--pillar", "ui", "--title", "Screens", "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    let answer = json.json();
    assert_eq!(answer["doc"]["id"], NEW_DESIGN, "{answer}");
    assert_eq!(answer["doc"]["feature_id"], Value::Null, "{answer}");
}

#[test]
fn doc_create_with_a_feature_creates_the_doc_in_that_feature() {
    let server = mock();
    let home = logged_in(&server.url);
    for given in ["checkout", "Checkout", CHECKOUT] {
        let out = run(
            &home,
            &[
                "doc",
                "create",
                "--feature",
                given,
                "--kind",
                "html",
                "--pillar",
                "architecture",
                "--title",
                "Checkout architecture",
                "--json",
            ],
        );
        assert_eq!(out.code, 0, "--feature {given}: {}", out.stderr);
        assert_eq!(out.json()["doc"]["feature_id"], CHECKOUT, "{}", out.stdout);
    }
    let sent = posts(&server);
    assert_eq!(sent.len(), 3, "{:?}", server.requests());
    for post in &sent {
        assert_eq!(
            post.path,
            format!("{}/features/{CHECKOUT}/docs", base()),
            "{post:?}"
        );
        assert_agent_request(post);
        assert_eq!(
            body_of(post),
            json!({ "kind": "html", "pillar": "architecture", "title": "Checkout architecture" })
        );
    }
}

#[test]
fn doc_create_with_an_unknown_feature_is_one_line_and_creates_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "doc",
            "create",
            "--feature",
            "Payments",
            "--kind",
            "html",
            "--pillar",
            "spec",
            "--title",
            "T",
        ],
    );
    assert_one_line_error(&out, &["Payments"]);
    assert!(
        posts(&server).is_empty(),
        "nothing is created: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni doc read of the head
// ---------------------------------------------------------------------------

#[test]
fn doc_read_of_a_new_html_doc_prints_the_empty_doc() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", NEW_DOC]);
    assert_eq!(out.code, 0, "a new doc is readable at once: {}", out.stderr);
    assert_eq!(out.stdout.trim(), "", "the empty doc: {:?}", out.stdout);
    assert_eq!(out.stderr, "", "{:?}", out.stderr);
}

#[test]
fn doc_read_of_a_new_design_prints_its_one_page() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", NEW_DESIGN]);
    assert_eq!(
        out.code, 0,
        "a new design is readable at once: {}",
        out.stderr
    );
    assert_eq!(
        out.stdout.trim(),
        page_tags(),
        "the design's tags as the API gave them: {:?}",
        out.stdout
    );
}
