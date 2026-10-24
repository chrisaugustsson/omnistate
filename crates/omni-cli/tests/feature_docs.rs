//! US-061 and US-101: `omni doc list`, tested as the built `omni` against a
//! mock HTTP server. None of these tests needs Postgres.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni doc list` sends `GET .../projects/{project_id}/docs` and prints one
//!   line per doc, `<id>  <scope>  <kind>  <pillar>  <title>` (two spaces
//!   between the fields), where the scope is `project` for a global doc or the
//!   name of the doc's feature. `--json` prints the API's answer.
//! - `--feature <name or id>` sends `GET .../docs?feature=<id>` (a name is
//!   looked up in the project's features, as for `omni changeset`) and
//!   `--global` sends `GET .../docs?feature=global`. A feature name the project
//!   does not have is a one-line error that names it, and no list is asked
//!   for.
//! - `--pillar <pillar>` lists only the docs of that pillar, one of `spec`,
//!   `ui`, `architecture`, `db` and `research` (with or without `--feature`);
//!   the API's `GET .../docs?pillar=<pillar>` filters the same way. Any other
//!   pillar is a one-line error that lists the five and asks for no list.
//! - `omni --help` names `omni doc list`.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const CHECKOUT: &str = "01900000-0000-7000-8000-0000000000f1";
const SEARCH: &str = "01900000-0000-7000-8000-0000000000f2";
const GLOBAL_DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const CHECKOUT_DOC: &str = "01900000-0000-7000-8000-0000000000d2";
const SEARCH_DOC: &str = "01900000-0000-7000-8000-0000000000d3";
const CHECKOUT_SPEC_DOC: &str = "01900000-0000-7000-8000-0000000000d4";
const SEARCH_RESEARCH_DOC: &str = "01900000-0000-7000-8000-0000000000d5";

fn projects_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects")
}

fn docs_path() -> String {
    format!("{}/{PROJECT}/docs", projects_path())
}

fn features_path() -> String {
    format!("{}/{PROJECT}/features", projects_path())
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

fn doc_json(id: &str, feature: Value, kind: &str, pillar: &str, title: &str) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "feature_id": feature,
        "kind": kind,
        "pillar": pillar,
        "title": title,
        "created_at": "2026-10-23T10:00:00.000000Z",
        "mainline_version_id": null,
        "drafts": [],
        "claims": []
    })
}

fn all_docs() -> Vec<Value> {
    vec![
        doc_json(GLOBAL_DOC, Value::Null, "html", "spec", "Brand PRD"),
        doc_json(
            CHECKOUT_DOC,
            json!(CHECKOUT),
            "html",
            "architecture",
            "Checkout architecture",
        ),
        doc_json(SEARCH_DOC, json!(SEARCH), "design", "ui", "Search screens"),
        doc_json(
            CHECKOUT_SPEC_DOC,
            json!(CHECKOUT),
            "html",
            "spec",
            "Checkout spec",
        ),
        doc_json(
            SEARCH_RESEARCH_DOC,
            json!(SEARCH),
            "html",
            "research",
            "Search interviews",
        ),
    ]
}

/// An API that knows the project `brand-site` with the features "Checkout"
/// and "Search", and five docs: a global spec, Checkout's architecture,
/// Search's design, Checkout's spec and Search's research. The list answers
/// `?feature=<id>`, `?feature=global` and `?pillar=<pillar>` (alone or
/// together) with the matching docs only.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let path = request.path.as_str();
        if request.method != "GET" {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        if path == projects_path() {
            return Reply::json(
                200,
                &json!({ "projects": [{
                    "id": PROJECT,
                    "name": "brand-site",
                    "created_at": "2026-10-23T10:00:00.000000Z"
                }] }),
            );
        }
        if path == features_path() {
            return Reply::json(
                200,
                &json!({ "features": [
                    feature_json(CHECKOUT, "Checkout"),
                    feature_json(SEARCH, "Search")
                ] }),
            );
        }
        for (feature, name) in [(CHECKOUT, "Checkout"), (SEARCH, "Search")] {
            if path == format!("{}/{feature}", features_path()) {
                return Reply::json(200, &json!({ "feature": feature_json(feature, name) }));
            }
        }
        let Some(rest) = path.strip_prefix(&docs_path()) else {
            return Reply::json(404, &json!({ "error": "not_found" }));
        };
        let only = |keep: &dyn Fn(&Value) -> bool| {
            let listed: Vec<Value> = all_docs().into_iter().filter(|d| keep(d)).collect();
            Reply::json(200, &json!({ "docs": listed }))
        };
        let query = match rest {
            "" => "",
            rest if rest.starts_with('?') => &rest[1..],
            _ => return Reply::json(404, &json!({ "error": "not_found" })),
        };
        let mut feature = None;
        let mut pillar = None;
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            match pair.split_once('=') {
                Some(("feature", value)) => feature = Some(value.to_string()),
                Some(("pillar", value)) => pillar = Some(value.to_string()),
                _ => return Reply::json(400, &json!({ "error": "invalid_request" })),
            }
        }
        if let Some(pillar) = &pillar
            && !["spec", "ui", "architecture", "db", "research"].contains(&pillar.as_str())
        {
            return Reply::json(400, &json!({ "error": "invalid_request" }));
        }
        only(&|d| {
            let in_scope = match feature.as_deref() {
                None => true,
                Some("global") => d["feature_id"].is_null(),
                Some(id) => d["feature_id"] == id,
            };
            in_scope && pillar.as_deref().is_none_or(|p| d["pillar"] == p)
        })
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
        .args(&["--as", "run-7/planner"])
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

/// How many times the mock was asked for `path`.
fn gets_to(server: &MockServer, path: &str) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.path == path)
        .count()
}

/// How many times the mock was asked for the doc list, with any filter.
fn doc_lists(server: &MockServer) -> usize {
    server
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.path.starts_with(&docs_path()))
        .count()
}

fn global_line() -> String {
    format!("{GLOBAL_DOC}  project  html  spec  Brand PRD")
}

fn checkout_line() -> String {
    format!("{CHECKOUT_DOC}  Checkout  html  architecture  Checkout architecture")
}

fn search_line() -> String {
    format!("{SEARCH_DOC}  Search  design  ui  Search screens")
}

fn checkout_spec_line() -> String {
    format!("{CHECKOUT_SPEC_DOC}  Checkout  html  spec  Checkout spec")
}

fn search_research_line() -> String {
    format!("{SEARCH_RESEARCH_DOC}  Search  html  research  Search interviews")
}

#[test]
fn doc_list_prints_the_scope_kind_pillar_and_title_of_each_doc() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["doc", "list", "--project", PROJECT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        gets_to(&server, &docs_path()),
        1,
        "without a filter the list is GET .../docs: {:?}",
        server.requests()
    );
    assert_eq!(
        non_empty_lines(&out.stdout),
        [
            global_line(),
            checkout_line(),
            search_line(),
            checkout_spec_line(),
            search_research_line()
        ],
        "one `<id>  <scope>  <kind>  <pillar>  <title>` line per doc"
    );

    let json = run(&home, &["doc", "list", "--project", PROJECT, "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "docs": all_docs() }),
        "--json prints the whole response"
    );
}

#[test]
fn doc_list_filters_by_feature_or_global() {
    let server = mock();
    let home = logged_in(&server.url);
    let list_path = |query: &str| format!("{}?feature={query}", docs_path());

    for feature in ["Checkout", CHECKOUT] {
        let before = gets_to(&server, &list_path(CHECKOUT));
        let out = run(
            &home,
            &["doc", "list", "--feature", feature, "--project", PROJECT],
        );
        assert_eq!(out.code, 0, "--feature {feature}: {}", out.stderr);
        assert_eq!(
            gets_to(&server, &list_path(CHECKOUT)),
            before + 1,
            "--feature {feature} asks for ?feature=<id>: {:?}",
            server.requests()
        );
        assert_eq!(
            non_empty_lines(&out.stdout),
            [checkout_line(), checkout_spec_line()],
            "--feature {feature} lists that feature's docs"
        );
    }

    let out = run(&home, &["doc", "list", "--global", "--project", PROJECT]);
    assert_eq!(out.code, 0, "--global: {}", out.stderr);
    assert_eq!(
        gets_to(&server, &list_path("global")),
        1,
        "--global asks for ?feature=global: {:?}",
        server.requests()
    );
    assert_eq!(
        non_empty_lines(&out.stdout),
        [global_line()],
        "--global lists the global docs"
    );
}

#[test]
fn doc_list_of_an_unknown_feature_is_one_line_and_lists_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let control = run(
        &home,
        &["doc", "list", "--feature", "Search", "--project", PROJECT],
    );
    assert_eq!(control.code, 0, "a known feature works: {}", control.stderr);
    let before = doc_lists(&server);

    let out = run(
        &home,
        &[
            "doc",
            "list",
            "--feature",
            "Loyalty points",
            "--project",
            PROJECT,
        ],
    );
    assert_one_line_error(&out, &["Loyalty points"]);
    assert_eq!(
        doc_lists(&server),
        before,
        "no list is asked for: {:?}",
        server.requests()
    );
}

#[test]
fn doc_list_with_a_pillar_lists_only_that_pillars_docs() {
    let server = mock();
    let home = logged_in(&server.url);
    for (pillar, lines) in [
        ("spec", vec![global_line(), checkout_spec_line()]),
        ("research", vec![search_research_line()]),
        ("ui", vec![search_line()]),
        ("db", vec![]),
    ] {
        let out = run(
            &home,
            &["doc", "list", "--pillar", pillar, "--project", PROJECT],
        );
        assert_eq!(out.code, 0, "--pillar {pillar}: {}", out.stderr);
        assert_eq!(
            non_empty_lines(&out.stdout)
                .into_iter()
                .filter(|line| line.starts_with("01900000-"))
                .collect::<Vec<_>>(),
            lines,
            "--pillar {pillar} lists only that pillar's docs: {:?}",
            out.stdout
        );
    }

    let out = run(
        &home,
        &[
            "doc",
            "list",
            "--pillar",
            "spec",
            "--feature",
            "Checkout",
            "--project",
            PROJECT,
        ],
    );
    assert_eq!(out.code, 0, "--pillar with --feature: {}", out.stderr);
    assert_eq!(
        non_empty_lines(&out.stdout),
        [checkout_spec_line()],
        "--pillar spec --feature Checkout lists Checkout's spec docs"
    );
}

#[test]
fn doc_list_refuses_a_pillar_that_is_not_one_of_the_five() {
    let server = mock();
    let home = logged_in(&server.url);
    for pillar in ["other", "prd"] {
        let out = run(
            &home,
            &["doc", "list", "--pillar", pillar, "--project", PROJECT],
        );
        assert_one_line_error(
            &out,
            &[pillar, "spec", "ui", "architecture", "db", "research"],
        );
    }
    assert_eq!(
        doc_lists(&server),
        0,
        "no list is asked for: {:?}",
        server.requests()
    );
}

#[test]
fn help_names_doc_list() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    assert!(
        help.stdout.contains("omni doc list"),
        "omni --help names omni doc list: {:?}",
        help.stdout
    );
}
