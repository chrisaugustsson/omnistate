//! US-022: `omni doc`, tested as the built binary against a mock HTTP server.
//! None of these tests needs Postgres; the real round trip against the API is
//! in tests/stack_docs.rs, and the API's contract in
//! crates/omni-api/tests/docs.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice; all
//! paths are under `/api/v1/w/{workspace}/projects/{project_id}`, the project
//! found as for `omni changeset`, from `--project` or `.omni.toml`):
//!
//! - `omni doc create --changeset <id> --kind html|design|design-system
//!   --pillar spec|ui|architecture|db|research --title <text>` sends `POST
//!   /changesets/{changeset}/docs` with `{"kind", "pillar", "title"}` and prints
//!   the new doc's ID and title; without `--changeset` it creates the doc live
//!   (US-102, tests/live_docs.rs). A missing kind, pillar or title, or a
//!   kind outside that list, is a one-line error that sends nothing; so is a
//!   pillar that is not one of the five (the error lists the five pillar
//!   names) and the old `--module` option (the error names `--pillar`, US-101).
//! - `omni doc open <doc> --changeset <id>` sends `POST
//!   /changesets/{changeset}/docs/{doc}/open` and prints the draft's ID.
//! - `omni doc read <doc>` prints the mainline version's HTML, from `GET
//!   /docs/{doc}/html`; `--version <v>` reads `GET /docs/{doc}/html?version=<v>`
//!   and `--changeset <id>` the change set's draft, `GET
//!   /changesets/{changeset}/docs/{doc}`, whose HTML has `data-rev`. The HTML
//!   is printed as the API gave it, and nothing else is on standard output.
//!   Both options at once are an error that sends nothing. Since US-102 the
//!   read without options is of the doc's head, so a doc without a mainline
//!   version (`"version_id": null`) prints its empty head and is no error.
//! - `omni doc show <doc>` sends `GET /docs/{doc}` and prints the doc's title,
//!   kind, pillar, mainline version (or that there is none), its drafts with
//!   their change sets, and its claims.
//! - `omni doc edit` is in tests/live_edits.rs (US-103).
//! - `<doc>` is a doc ID (a UUID). Every command is an agent request with the
//!   agent identity headers, `--json` prints the API's JSON, and errors are one
//!   line on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
/// A doc that has no mainline version yet.
const NEW_DOC: &str = "01900000-0000-7000-8000-0000000000d2";
const NEW_DRAFT: &str = "01900000-0000-7000-8000-0000000000e1";
const DRAFT: &str = "01900000-0000-7000-8000-0000000000e2";
const MAINLINE: &str = "01900000-0000-7000-8000-0000000000f1";
const OLD_VERSION: &str = "01900000-0000-7000-8000-0000000000f0";
const MAINLINE_HTML: &str =
    "<h1 data-omni-id=\"a0000001\">Checkout</h1><p data-omni-id=\"a0000002\">Mainline text</p>";
const OLD_HTML: &str =
    "<h1 data-omni-id=\"a0000001\">Checkout</h1><p data-omni-id=\"a0000002\">Old text</p>";
const DRAFT_HTML: &str = "<h1 data-omni-id=\"a0000001\" data-rev=\"1\">Checkout</h1><p data-omni-id=\"a0000002\" data-rev=\"3\">Draft text</p>";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn draft_json(id: &str, doc: &str, base_version: Option<&str>) -> Value {
    json!({ "id": id, "doc_id": doc, "changeset_id": CHANGESET, "base_version_id": base_version })
}

fn doc_json(id: &str, title: &str, kind: &str, pillar: &str, mainline: Option<&str>) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "kind": kind,
        "pillar": pillar,
        "title": title,
        "created_at": "2026-10-15T10:00:00.000000Z",
        "mainline_version_id": mainline,
        "drafts": [draft_json(DRAFT, id, mainline)],
        "claims": []
    })
}

/// An API with the doc `DOC` (mainline version `MAINLINE`, an older version
/// `OLD_VERSION` and a draft in `CHANGESET`) and `NEW_DOC`, which has no
/// mainline version.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let path = request.path.as_str();
        let method = request.method.as_str();
        let Some(rest) = path.strip_prefix(&base()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        let changeset = format!("/changesets/{CHANGESET}");
        match (method, rest) {
            ("POST", r) if r == format!("{changeset}/docs") => Reply::json(
                201,
                &json!({
                    "doc": doc_json(
                        NEW_DOC,
                        body["title"].as_str().unwrap_or("?"),
                        body["kind"].as_str().unwrap_or("?"),
                        body["pillar"].as_str().unwrap_or("?"),
                        None
                    ),
                    "draft": draft_json(NEW_DRAFT, NEW_DOC, None)
                }),
            ),
            ("POST", r) if r == format!("{changeset}/docs/{DOC}/open") => Reply::json(
                200,
                &json!({ "draft": draft_json(DRAFT, DOC, Some(MAINLINE)) }),
            ),
            ("GET", r) if r == format!("{changeset}/docs/{DOC}") => {
                let mut draft = draft_json(DRAFT, DOC, Some(MAINLINE));
                draft["html"] = json!(DRAFT_HTML);
                Reply::json(200, &json!({ "draft": draft }))
            }
            ("GET", r) if r == format!("/docs/{DOC}") => Reply::json(
                200,
                &json!({ "doc": doc_json(DOC, "Checkout PRD", "html", "research", Some(MAINLINE)) }),
            ),
            ("GET", r) if r == format!("/docs/{NEW_DOC}") => Reply::json(
                200,
                &json!({ "doc": doc_json(NEW_DOC, "New PRD", "html", "spec", None) }),
            ),
            ("GET", r) if r == format!("/docs/{DOC}/html") => Reply::json(
                200,
                &json!({ "doc_id": DOC, "version_id": MAINLINE, "html": MAINLINE_HTML }),
            ),
            ("GET", r) if r == format!("/docs/{DOC}/html?version={OLD_VERSION}") => Reply::json(
                200,
                &json!({ "doc_id": DOC, "version_id": OLD_VERSION, "html": OLD_HTML }),
            ),
            ("GET", r) if r == format!("/docs/{NEW_DOC}/html") => Reply::json(
                200,
                &json!({ "doc_id": NEW_DOC, "version_id": null, "html": "" }),
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

fn body_of(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body))
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
fn doc_create_starts_a_doc_in_the_change_set() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "doc",
            "create",
            "--changeset",
            CHANGESET,
            "--kind",
            "html",
            "--pillar",
            "spec",
            "--title",
            "Checkout PRD",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let posts = requests_to(
        &server,
        "POST",
        &format!("{}/changesets/{CHANGESET}/docs", base()),
    );
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    assert_agent_request(&posts[0]);
    let body = body_of(&posts[0]);
    assert_eq!(body["kind"], "html", "{body}");
    assert_eq!(body["pillar"], "spec", "{body}");
    assert!(
        body.get("module").is_none(),
        "the pillar replaces the old module: {body}"
    );
    assert_eq!(body["title"], "Checkout PRD", "{body}");
    assert!(
        out.stdout.contains(NEW_DOC) && out.stdout.contains("Checkout PRD"),
        "the text has the new doc's ID and title: {:?}",
        out.stdout
    );
    assert!(
        serde_json::from_str::<Value>(&out.stdout).is_err(),
        "the default output is plain text: {:?}",
        out.stdout
    );
    assert!(non_empty_lines(&out.stdout).len() <= 5, "{:?}", out.stdout);

    let json = run(
        &home,
        &[
            "doc",
            "create",
            "--changeset",
            CHANGESET,
            "--kind",
            "design-system",
            "--pillar",
            "ui",
            "--title",
            "Tokens",
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    let answer = json.json();
    assert_eq!(answer["doc"]["id"], NEW_DOC, "{answer}");
    assert_eq!(answer["doc"]["kind"], "design-system", "{answer}");
    assert_eq!(answer["draft"]["id"], NEW_DRAFT, "{answer}");
}

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
// omni doc open
// ---------------------------------------------------------------------------

#[test]
fn doc_open_creates_or_reuses_the_change_set_draft() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "open", DOC, "--changeset", CHANGESET]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let posts = requests_to(
        &server,
        "POST",
        &format!("{}/changesets/{CHANGESET}/docs/{DOC}/open", base()),
    );
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    assert_agent_request(&posts[0]);
    assert!(
        out.stdout.contains(DRAFT),
        "the draft's ID: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &["doc", "open", DOC, "--changeset", CHANGESET, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "draft": draft_json(DRAFT, DOC, Some(MAINLINE)) })
    );
}

#[test]
fn doc_open_needs_a_doc_and_a_change_set() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_one_line_error(&run(&home, &["doc", "open", DOC]), &["changeset"]);
    assert_one_line_error(
        &run(&home, &["doc", "open", "--changeset", CHANGESET]),
        &["doc"],
    );
    assert_one_line_error(
        &run(
            &home,
            &["doc", "open", "not-a-doc", "--changeset", CHANGESET],
        ),
        &["not-a-doc"],
    );
    assert_nothing_sent(&server);
}

// ---------------------------------------------------------------------------
// omni doc read
// ---------------------------------------------------------------------------

#[test]
fn doc_read_prints_the_mainline_version_by_default() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let gets = requests_to(&server, "GET", &format!("{}/docs/{DOC}/html", base()));
    assert_eq!(gets.len(), 1, "{:?}", server.requests());
    assert_agent_request(&gets[0]);
    assert_eq!(
        out.stdout.trim_end(),
        MAINLINE_HTML,
        "the HTML with data-omni-id, and nothing else"
    );
}

#[test]
fn doc_read_with_a_version_prints_that_version() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", DOC, "--version", OLD_VERSION]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        requests_to(
            &server,
            "GET",
            &format!("{}/docs/{DOC}/html?version={OLD_VERSION}", base())
        )
        .len(),
        1,
        "{:?}",
        server.requests()
    );
    assert_eq!(out.stdout.trim_end(), OLD_HTML);

    // `--version` alone still prints the version of omni.
    let version = omni(&home.path).args(&["--version"]).run();
    assert_eq!(version.code, 0, "{}", version.stderr);
    assert!(version.stdout.starts_with("omni "), "{:?}", version.stdout);
}

#[test]
fn doc_read_with_a_change_set_prints_the_draft_with_revisions() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", DOC, "--changeset", CHANGESET]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        requests_to(
            &server,
            "GET",
            &format!("{}/changesets/{CHANGESET}/docs/{DOC}", base())
        )
        .len(),
        1,
        "{:?}",
        server.requests()
    );
    assert_eq!(
        out.stdout.trim_end(),
        DRAFT_HTML,
        "the draft's HTML with data-omni-id and data-rev"
    );

    let json = run(
        &home,
        &["doc", "read", DOC, "--changeset", CHANGESET, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json()["draft"]["html"], DRAFT_HTML);
}

#[test]
fn doc_read_takes_a_change_set_or_a_version_not_both() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "doc",
            "read",
            DOC,
            "--changeset",
            CHANGESET,
            "--version",
            OLD_VERSION,
        ],
    );
    assert_one_line_error(&out, &["--changeset", "--version"]);
    assert_one_line_error(&run(&home, &["doc", "read"]), &["doc"]);
    assert_nothing_sent(&server);
}

#[test]
fn doc_read_of_a_doc_without_a_mainline_version_prints_its_empty_head() {
    // US-102: a doc is readable at once, from its head; one without a
    // mainline version is no error.
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "read", NEW_DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.trim(), "", "{:?}", out.stdout);
}

// ---------------------------------------------------------------------------
// omni doc show
// ---------------------------------------------------------------------------

#[test]
fn doc_show_prints_the_metadata_mainline_version_drafts_and_claims() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "show", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let gets = requests_to(&server, "GET", &format!("{}/docs/{DOC}", base()));
    assert_eq!(gets.len(), 1, "{:?}", server.requests());
    assert_agent_request(&gets[0]);
    for needle in [
        DOC,
        "Checkout PRD",
        "html",
        "research",
        MAINLINE,
        DRAFT,
        CHANGESET,
    ] {
        assert!(
            out.stdout.contains(needle),
            "show prints {needle:?}: {:?}",
            out.stdout
        );
    }
    let lines = non_empty_lines(&out.stdout);
    assert!(
        lines
            .iter()
            .any(|l| l.contains(DRAFT) && l.contains(CHANGESET)),
        "a draft and its change set are on one line: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.to_lowercase().contains("claims"),
        "show has the claims: {:?}",
        out.stdout
    );

    let new = run(&home, &["doc", "show", NEW_DOC]);
    assert_eq!(new.code, 0, "{}", new.stderr);
    assert!(
        non_empty_lines(&new.stdout)
            .iter()
            .any(|l| l.to_lowercase().contains("mainline") && l.to_lowercase().contains("none")),
        "a doc without a mainline version says so: {:?}",
        new.stdout
    );

    let json = run(&home, &["doc", "show", DOC, "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "doc": doc_json(DOC, "Checkout PRD", "html", "research", Some(MAINLINE)) })
    );
}
