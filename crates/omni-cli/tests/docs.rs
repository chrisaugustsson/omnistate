//! US-022: `omni doc`, tested as the built binary against a mock HTTP server.
//! None of these tests needs Postgres; the real round trip against the API is
//! in tests/stack_docs.rs, and the API's contract in
//! crates/omni-api/tests/docs.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found from `--project`):
//!
//! - `omni doc create` with a missing kind, pillar or title, or a kind outside
//!   `html`, `design` and `design-system`, is a one-line error that sends
//!   nothing; so is a pillar that is not one of the five (the error lists the
//!   five pillar names) and the old `--module` option (the error names
//!   `--pillar`, US-101). The doc it creates is in tests/live_docs.rs.
//! - `omni doc read <doc>` prints the doc's head from `GET /docs/{doc}/html`:
//!   the HTML as the API gave it, and nothing else is on standard output.
//! - `omni doc read <doc> --node <id>` prints only the node with that
//!   `data-omni-id`, with what is inside it, as it stands in the head;
//!   `--node` given several times prints the nodes in the doc's order, each
//!   from a new line. In a design the node is the tag with that `id`. A node
//!   the doc lacks is an error that names every missing one and prints
//!   nothing on standard output.
//! - `omni doc show <doc>` sends `GET /docs/{doc}` and prints the doc's title,
//!   kind and pillar.
//! - `omni doc edit` is in tests/live_edits.rs (US-103).
//! - `<doc>` is a doc ID (a UUID). Every command is an agent request with the
//!   agent identity headers, `--json` prints the API's JSON, and errors are one
//!   line on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const HEAD_HTML: &str = "<h1 data-omni-id=\"a0000001\" data-rev=\"1\">Checkout</h1><p data-omni-id=\"a0000002\" data-rev=\"3\">Head text</p>";

/// A doc whose nodes nest, as the server prints a head.
const NESTED_DOC: &str = "01900000-0000-7000-8000-0000000000d2";
const SECTION: &str = "<section data-omni-id=\"b0000001\" data-rev=\"1\" class=\"intro\"><h1 data-omni-id=\"b0000002\" data-rev=\"2\">Checkout</h1><p data-omni-id=\"b0000003\" data-rev=\"1\">Pay &amp; <em>go</em></p></section>";
const HEADING: &str = "<h1 data-omni-id=\"b0000002\" data-rev=\"2\">Checkout</h1>";
const PARAGRAPH: &str = "<p data-omni-id=\"b0000003\" data-rev=\"1\">Pay &amp; <em>go</em></p>";
const LIST: &str = "<ul data-omni-id=\"b0000004\" data-rev=\"4\"><li data-omni-id=\"b0000005\" data-rev=\"1\">One</li><li data-omni-id=\"b0000006\" data-rev=\"3\">Two</li></ul>";
const SECOND_ITEM: &str = "<li data-omni-id=\"b0000006\" data-rev=\"3\">Two</li>";

fn nested_html() -> String {
    format!("{SECTION}{LIST}")
}

/// A design doc: a page with a board, a frame in it and a text in the frame,
/// and an empty page after it.
const DESIGN: &str = "01900000-0000-7000-8000-0000000000d3";
const DESIGN_TAGS: &str = r#"<Page id="page0001" name="Page 1">
  <Board id="brd00001" name="Cart" x={0} y={0} width={360} height={640} dir="col">
    <Frame id="frm00001" name="Row" width="fill" dir="row">
      <Text id="txt00001" name="Total" content="Summa 12 kr" fontFamily="Inter" fontSize={16}/>
    </Frame>
  </Board>
</Page>
<Page id="page0002" name="Page 2"/>"#;
const FRAME_TAGS: &str = r#"<Frame id="frm00001" name="Row" width="fill" dir="row">
  <Text id="txt00001" name="Total" content="Summa 12 kr" fontFamily="Inter" fontSize={16}/>
</Frame>"#;
const TEXT_TAGS: &str =
    r#"<Text id="txt00001" name="Total" content="Summa 12 kr" fontFamily="Inter" fontSize={16}/>"#;

fn design_json() -> Value {
    json!({
        "doc_id": DESIGN,
        "html": "",
        "tags": DESIGN_TAGS,
        "pages": [{
            "id": "page0001", "type": "page", "name": "Page 1",
            "children": [{
                "id": "brd00001", "type": "board", "name": "Cart",
                "x": 0, "y": 0, "width": {"fixed": 360}, "height": {"fixed": 640},
                "layout": {"mode": "flex", "direction": "column"},
                "children": [{
                    "id": "frm00001", "type": "frame", "name": "Row",
                    "width": "fill", "height": "hug", "layout": {"mode": "flex", "direction": "row"},
                    "children": [{
                        "id": "txt00001", "type": "text", "name": "Total",
                        "width": "hug", "height": "hug",
                        "content": "Summa 12 kr", "fontFamily": "Inter", "fontSize": 16
                    }]
                }]
            }]
        }, { "id": "page0002", "type": "page", "name": "Page 2", "children": [] }]
    })
}

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn doc_json() -> Value {
    json!({
        "id": DOC,
        "project_id": PROJECT,
        "kind": "html",
        "pillar": "research",
        "title": "Checkout PRD",
        "created_at": "2026-10-15T10:00:00.000000Z",
        "feature_id": null
    })
}

fn head_json() -> Value {
    json!({ "doc_id": DOC, "html": HEAD_HTML })
}

/// An API with the doc `DOC`.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let Some(rest) = request.path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        match (request.method.as_str(), rest) {
            ("GET", r) if r == format!("/docs/{DOC}") => {
                Reply::json(200, &json!({ "doc": doc_json() }))
            }
            ("GET", r) if r == format!("/docs/{DOC}/html") => Reply::json(200, &head_json()),
            ("GET", r) if r == format!("/docs/{NESTED_DOC}/html") => {
                Reply::json(200, &json!({ "doc_id": NESTED_DOC, "html": nested_html() }))
            }
            ("GET", r) if r == format!("/docs/{DESIGN}/html") => Reply::json(200, &design_json()),
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
    assert_eq!(request.header("x-omni-run"), Some("run-7"), "{request:?}");
    assert_eq!(request.header("x-omni-role"), Some("author"), "{request:?}");
}

fn assert_nothing_sent(server: &MockServer) {
    assert!(
        server.requests().is_empty(),
        "nothing should be sent: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni doc create
// ---------------------------------------------------------------------------

#[test]
fn doc_create_refuses_bad_options_before_sending_anything() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(
        &run(
            &home,
            &["doc", "create", "--pillar", "spec", "--title", "T"],
        ),
        &["kind"],
    );
    assert_one_line_error(
        &run(&home, &["doc", "create", "--kind", "html", "--title", "T"]),
        &["pillar"],
    );
    assert_one_line_error(
        &run(
            &home,
            &["doc", "create", "--kind", "html", "--pillar", "spec"],
        ),
        &["title"],
    );
    assert_one_line_error(
        &run(
            &home,
            &[
                "doc", "create", "--kind", "pdf", "--pillar", "spec", "--title", "T",
            ],
        ),
        &["pdf"],
    );
    assert_nothing_sent(&server);
}

#[test]
fn doc_create_refuses_a_pillar_that_is_not_one_of_the_five() {
    let server = mock();
    let home = logged_in(&server.url);
    for pillar in ["other", "prd"] {
        let out = run(
            &home,
            &[
                "doc", "create", "--kind", "html", "--pillar", pillar, "--title", "T",
            ],
        );
        assert_one_line_error(
            &out,
            &[pillar, "spec", "ui", "architecture", "db", "research"],
        );
    }
    assert_nothing_sent(&server);
}

#[test]
fn doc_create_refuses_the_old_module_option_and_names_pillar() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "doc", "create", "--kind", "html", "--module", "prd", "--title", "T",
        ],
    );
    assert_one_line_error(&out, &["--module", "--pillar"]);
    assert_nothing_sent(&server);
}

// ---------------------------------------------------------------------------
// omni doc read
// ---------------------------------------------------------------------------

#[test]
fn doc_read_prints_the_head() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let gets = requests_to(&server, "GET", &format!("{}/docs/{DOC}/html", base()));
    assert_eq!(gets.len(), 1, "{:?}", server.requests());
    assert_agent_request(&gets[0]);
    assert_eq!(
        out.stdout.trim_end(),
        HEAD_HTML,
        "the HTML with data-omni-id and data-rev, and nothing else"
    );
}

#[test]
fn doc_read_with_json_prints_the_api_answer() {
    let server = mock();
    let home = logged_in(&server.url);
    let json = run(&home, &["doc", "read", DOC, "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), head_json());
}

#[test]
fn doc_read_needs_a_doc() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(&run(&home, &["doc", "read"]), &["doc"]);
    assert_nothing_sent(&server);
}

// ---------------------------------------------------------------------------
// omni doc read --node
// ---------------------------------------------------------------------------

#[test]
fn doc_read_node_prints_that_node_only() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", NESTED_DOC, "--node", "b0000003"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, format!("{PARAGRAPH}\n"));
    let gets = requests_to(
        &server,
        "GET",
        &format!("{}/docs/{NESTED_DOC}/html", base()),
    );
    assert_eq!(gets.len(), 1, "{:?}", server.requests());
    assert_agent_request(&gets[0]);
}

#[test]
fn doc_read_nodes_come_in_the_docs_order_each_from_a_new_line() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "doc",
            "read",
            NESTED_DOC,
            "--node",
            "b0000006",
            "--node=b0000002",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, format!("{HEADING}\n{SECOND_ITEM}\n"));

    // A node named twice is printed once.
    let twice = run(
        &home,
        &[
            "doc", "read", NESTED_DOC, "--node", "b0000006", "--node", "b0000006",
        ],
    );
    assert_eq!(twice.code, 0, "{}", twice.stderr);
    assert_eq!(twice.stdout, format!("{SECOND_ITEM}\n"));
}

#[test]
fn doc_read_node_prints_a_node_with_the_nodes_inside_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", NESTED_DOC, "--node", "b0000001"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, format!("{SECTION}\n"));

    // A node and a node inside it are each printed, the outer one first.
    let both = run(
        &home,
        &[
            "doc", "read", NESTED_DOC, "--node", "b0000006", "--node", "b0000004",
        ],
    );
    assert_eq!(both.code, 0, "{}", both.stderr);
    assert_eq!(both.stdout, format!("{LIST}\n{SECOND_ITEM}\n"));
}

#[test]
fn doc_read_node_fails_and_names_every_node_the_doc_lacks() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "doc", "read", NESTED_DOC, "--node", "b0000009", "--node", "b0000003", "--node",
            "nope", "--node", "b0000009",
        ],
    );
    assert_eq!(out.code, 1);
    assert_eq!(out.stdout, "", "not even the node the doc has");
    assert_eq!(
        out.stderr,
        format!(
            "omni: doc {NESTED_DOC} has no node b0000009, nope; `omni doc read {NESTED_DOC}` prints every node it has\n"
        )
    );
}

#[test]
fn doc_read_without_node_prints_the_whole_head_as_before() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", NESTED_DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, format!("{}\n", nested_html()));

    let design = run(&home, &["doc", "read", DESIGN]);
    assert_eq!(design.code, 0, "{}", design.stderr);
    assert_eq!(design.stdout, format!("{DESIGN_TAGS}\n"));
}

#[test]
fn doc_read_node_prints_the_tags_of_a_designs_nodes() {
    let server = mock();
    let home = logged_in(&server.url);
    // The two pages are the whole head: the tags the API gave.
    let pages = run(
        &home,
        &[
            "doc", "read", DESIGN, "--node", "page0002", "--node", "page0001",
        ],
    );
    assert_eq!(pages.code, 0, "{}", pages.stderr);
    assert_eq!(pages.stdout, format!("{DESIGN_TAGS}\n"));

    // A node inside starts at the left margin, as a board of `omni design
    // read` does.
    let out = run(
        &home,
        &[
            "doc", "read", DESIGN, "--node", "txt00001", "--node", "frm00001",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, format!("{FRAME_TAGS}\n{TEXT_TAGS}\n"));

    let missing = run(&home, &["doc", "read", DESIGN, "--node", "b0000003"]);
    assert_eq!(missing.code, 1);
    assert_eq!(missing.stdout, "");
    assert_eq!(
        missing.stderr,
        format!(
            "omni: doc {DESIGN} has no node b0000003; `omni doc read {DESIGN}` prints every node it has\n"
        )
    );
}

#[test]
fn doc_read_node_with_json_is_refused_before_sending_anything() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["doc", "read", NESTED_DOC, "--node", "b0000003", "--json"],
    );
    assert_one_line_error(&out, &["--node", "--json"]);
    assert_nothing_sent(&server);
}

// ---------------------------------------------------------------------------
// omni doc show
// ---------------------------------------------------------------------------

#[test]
fn doc_show_prints_the_metadata() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "show", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let gets = requests_to(&server, "GET", &format!("{}/docs/{DOC}", base()));
    assert_eq!(gets.len(), 1, "{:?}", server.requests());
    assert_agent_request(&gets[0]);
    for needle in [DOC, "Checkout PRD", "html", "research"] {
        assert!(
            out.stdout.contains(needle),
            "show prints {needle:?}: {:?}",
            out.stdout
        );
    }

    let json = run(&home, &["doc", "show", DOC, "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), json!({ "doc": doc_json() }));
}
