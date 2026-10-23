//! US-060: change sets in a feature, tested as the built `omni` against a mock
//! HTTP server. None of these tests needs Postgres.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - The feature a command works in is `--feature <name or id>`, else the
//!   `feature` of the nearest `.omni.toml` (looking in the current directory
//!   and then in each parent), else none. A feature ID (a UUID) is used as it
//!   is; a name is looked up in the project's features (`GET
//!   .../projects/{project_id}/features`). A name the project does not have is
//!   a one-line error that names it, and nothing is created.
//! - `omni changeset create` sends `"feature_id": "<id>"` in its body when
//!   there is a feature. Without one, or with `--global` (which wins over the
//!   `.omni.toml` feature), the body has no `feature_id` (or `null`).
//! - `omni changeset list` prints one line per change set, `<id>  <status>
//!   <scope>  <title>` (two spaces between the fields), where the scope is
//!   `project` or the name of the change set's feature. `--feature <name or
//!   id>` sends `GET .../changesets?feature=<id>` and `--global` sends
//!   `GET .../changesets?feature=global`; without either it is `GET
//!   .../changesets`. `--json` prints the API's answer.
//! - `omni --help` names `--feature` and `--global`.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const OTHER_PROJECT: &str = "01900000-0000-7000-8000-0000000000b2";
const CHECKOUT: &str = "01900000-0000-7000-8000-0000000000f1";
const SEARCH: &str = "01900000-0000-7000-8000-0000000000f2";
const GLOBAL_CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const CHECKOUT_CHANGESET: &str = "01900000-0000-7000-8000-0000000000c2";
const SEARCH_CHANGESET: &str = "01900000-0000-7000-8000-0000000000c3";
const NEW_CHANGESET: &str = "01900000-0000-7000-8000-0000000000c4";

fn projects_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects")
}

fn changesets_path(project: &str) -> String {
    format!("{}/{project}/changesets", projects_path())
}

fn features_path(project: &str) -> String {
    format!("{}/{project}/features", projects_path())
}

fn project_json(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "created_at": "2026-10-23T10:00:00.000000Z" })
}

fn feature_json(id: &str, project: &str, name: &str) -> Value {
    json!({
        "id": id,
        "project_id": project,
        "name": name,
        "description": "",
        "status": "active",
        "created_at": "2026-10-23T10:00:00.000000Z"
    })
}

fn changeset_json(id: &str, project: &str, title: &str, status: &str, feature: Value) -> Value {
    json!({
        "id": id,
        "project_id": project,
        "feature_id": feature,
        "title": title,
        "description": "",
        "status": status,
        "run_tag": null,
        "created_at": "2026-10-23T10:00:00.000000Z",
        "history": [],
        "approved_versions": [],
        "decisions": []
    })
}

fn global_changeset(project: &str) -> Value {
    changeset_json(
        GLOBAL_CHANGESET,
        project,
        "Brand refresh",
        "in_review",
        Value::Null,
    )
}

fn checkout_changeset(project: &str) -> Value {
    changeset_json(
        CHECKOUT_CHANGESET,
        project,
        "Checkout flow",
        "open",
        json!(CHECKOUT),
    )
}

fn search_changeset(project: &str) -> Value {
    changeset_json(
        SEARCH_CHANGESET,
        project,
        "Search page",
        "merged",
        json!(SEARCH),
    )
}

/// An API that knows two projects, `brand-site` and `other`, each with the
/// features "Checkout" and "Search", and three change sets: one at project
/// level, one in Checkout and one in Search. The list answers `?feature=<id>`
/// and `?feature=global` with the matching change sets only; a new change set
/// takes the `feature_id` of the request.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let path = request.path.as_str();
        let method = request.method.as_str();
        if path == projects_path() && method == "GET" {
            return Reply::json(
                200,
                &json!({ "projects": [
                    project_json(PROJECT, "brand-site"),
                    project_json(OTHER_PROJECT, "other")
                ] }),
            );
        }
        for project in [PROJECT, OTHER_PROJECT] {
            if path == features_path(project) && method == "GET" {
                return Reply::json(
                    200,
                    &json!({ "features": [
                        feature_json(CHECKOUT, project, "Checkout"),
                        feature_json(SEARCH, project, "Search")
                    ] }),
                );
            }
            for (feature, name) in [(CHECKOUT, "Checkout"), (SEARCH, "Search")] {
                if path == format!("{}/{feature}", features_path(project)) && method == "GET" {
                    return Reply::json(
                        200,
                        &json!({ "feature": feature_json(feature, project, name) }),
                    );
                }
            }
            let Some(rest) = path.strip_prefix(&changesets_path(project)) else {
                continue;
            };
            let all = [
                global_changeset(project),
                checkout_changeset(project),
                search_changeset(project),
            ];
            let only = |keep: &dyn Fn(&Value) -> bool| {
                let listed: Vec<Value> = all.iter().filter(|c| keep(c)).cloned().collect();
                Reply::json(200, &json!({ "changesets": listed }))
            };
            return match (method, rest) {
                ("POST", "") => {
                    let title = body["title"].as_str().unwrap_or("untitled");
                    let feature = body.get("feature_id").cloned().unwrap_or(Value::Null);
                    Reply::json(
                        201,
                        &json!({ "changeset": changeset_json(NEW_CHANGESET, project, title, "open", feature) }),
                    )
                }
                ("GET", "") => only(&|_| true),
                ("GET", "?feature=global") => only(&|c| c["feature_id"].is_null()),
                ("GET", rest) if rest.starts_with("?feature=") => {
                    let feature = &rest["?feature=".len()..];
                    only(&|c| c["feature_id"] == feature)
                }
                _ => Reply::json(404, &json!({ "error": "not_found" })),
            };
        }
        Reply::json(404, &json!({ "error": "not_found" }))
    })
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
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

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", "run-7/planner"])
        .run()
}

fn body_of(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body)
        .unwrap_or_else(|e| panic!("the request body is JSON ({e}): {:?}", request.body))
}

fn posts_to(server: &MockServer, path: &str) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path == path)
        .collect()
}

/// The `feature_id` of the only change set created since `before` requests.
fn created_feature(server: &MockServer, before: usize) -> Value {
    let posts: Vec<Recorded> = server
        .requests()
        .into_iter()
        .skip(before)
        .filter(|r| r.method == "POST" && r.path == changesets_path(PROJECT))
        .collect();
    assert_eq!(posts.len(), 1, "one change set: {:?}", server.requests());
    let body = body_of(&posts[0]);
    assert_eq!(body["title"], "Pay page", "{body}");
    body.get("feature_id").cloned().unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// omni changeset create
// ---------------------------------------------------------------------------

#[test]
fn changeset_create_opens_the_change_set_in_the_feature_given_by_name_or_id() {
    let server = mock();
    let home = logged_in(&server.url);
    for (feature, expected) in [("Checkout", CHECKOUT), (SEARCH, SEARCH)] {
        let before = server.requests().len();
        let out = run(
            &home,
            &[
                "changeset",
                "create",
                "--title",
                "Pay page",
                "--feature",
                feature,
                "--project",
                PROJECT,
            ],
        );
        assert_eq!(out.code, 0, "--feature {feature}: {}", out.stderr);
        assert_eq!(
            created_feature(&server, before),
            expected,
            "--feature {feature} opens the change set in that feature"
        );
    }
}

#[test]
fn without_a_feature_the_change_set_is_project_level() {
    let server = mock();
    let home = logged_in(&server.url);
    for extra in [&[][..], &["--global"][..]] {
        let before = server.requests().len();
        let out = run(
            &home,
            &[
                &[
                    "changeset",
                    "create",
                    "--title",
                    "Pay page",
                    "--project",
                    PROJECT,
                ][..],
                extra,
            ]
            .concat(),
        );
        assert_eq!(out.code, 0, "{extra:?}: {}", out.stderr);
        assert_eq!(
            created_feature(&server, before),
            Value::Null,
            "{extra:?}: no feature_id is sent"
        );
    }
}

#[test]
fn the_feature_comes_from_the_nearest_omni_toml_and_the_flags_win() {
    let server = mock();
    let home = logged_in(&server.url);
    let project_dir = Scratch::new("project");
    std::fs::write(
        project_dir.join(".omni.toml"),
        "project = \"brand-site\"\nfeature = \"Checkout\"\n",
    )
    .unwrap();
    let nested = project_dir.join("docs").join("checkout");
    std::fs::create_dir_all(&nested).unwrap();
    let in_nested = |extra: &[&str]| {
        omni(&home.path)
            .cwd(&nested)
            .args(&["changeset", "create", "--title", "Pay page"])
            .args(extra)
            .args(&["--as", "run-7/planner"])
            .run()
    };

    let before = server.requests().len();
    let out = in_nested(&[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        created_feature(&server, before),
        CHECKOUT,
        "the feature of the nearest .omni.toml, by name"
    );

    let before = server.requests().len();
    let out = in_nested(&["--feature", "Search"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        created_feature(&server, before),
        SEARCH,
        "--feature wins over .omni.toml"
    );

    let before = server.requests().len();
    let out = in_nested(&["--global"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        created_feature(&server, before),
        Value::Null,
        "--global makes the change set project level whatever .omni.toml says"
    );
}

#[test]
fn an_unknown_feature_is_one_line_and_creates_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let control = run(
        &home,
        &[
            "changeset",
            "create",
            "--title",
            "Pay page",
            "--feature",
            "Checkout",
            "--project",
            PROJECT,
        ],
    );
    assert_eq!(control.code, 0, "a known feature works: {}", control.stderr);
    let before = posts_to(&server, &changesets_path(PROJECT)).len();

    let out = run(
        &home,
        &[
            "changeset",
            "create",
            "--title",
            "Pay page",
            "--feature",
            "Loyalty points",
            "--project",
            PROJECT,
        ],
    );
    assert_one_line_error(&out, &["Loyalty points"]);
    assert_eq!(
        posts_to(&server, &changesets_path(PROJECT)).len(),
        before,
        "no change set is created: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni changeset list
// ---------------------------------------------------------------------------

#[test]
fn changeset_list_prints_the_scope_of_each_change_set() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["changeset", "list", "--project", PROJECT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        server
            .requests()
            .iter()
            .any(|r| r.method == "GET" && r.path == changesets_path(PROJECT)),
        "without a filter the list is GET .../changesets: {:?}",
        server.requests()
    );
    assert_eq!(
        non_empty_lines(&out.stdout),
        [
            format!("{GLOBAL_CHANGESET}  in_review  project  Brand refresh"),
            format!("{CHECKOUT_CHANGESET}  open  Checkout  Checkout flow"),
            format!("{SEARCH_CHANGESET}  merged  Search  Search page"),
        ],
        "one `<id>  <status>  <scope>  <title>` line per change set"
    );

    let json = run(
        &home,
        &["changeset", "list", "--project", PROJECT, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "changesets": [
            global_changeset(PROJECT),
            checkout_changeset(PROJECT),
            search_changeset(PROJECT)
        ] }),
        "--json prints the whole response"
    );
}

#[test]
fn changeset_list_filters_by_feature_or_global() {
    let server = mock();
    let home = logged_in(&server.url);
    let list_path = |query: &str| format!("{}?feature={query}", changesets_path(PROJECT));
    let gets_to = |path: &str| {
        server
            .requests()
            .iter()
            .filter(|r| r.method == "GET" && r.path == path)
            .count()
    };

    for feature in ["Checkout", CHECKOUT] {
        let before = gets_to(&list_path(CHECKOUT));
        let out = run(
            &home,
            &[
                "changeset",
                "list",
                "--feature",
                feature,
                "--project",
                PROJECT,
            ],
        );
        assert_eq!(out.code, 0, "--feature {feature}: {}", out.stderr);
        assert_eq!(
            gets_to(&list_path(CHECKOUT)),
            before + 1,
            "--feature {feature} asks for ?feature=<id>: {:?}",
            server.requests()
        );
        assert_eq!(
            non_empty_lines(&out.stdout),
            [format!(
                "{CHECKOUT_CHANGESET}  open  Checkout  Checkout flow"
            )],
            "--feature {feature} lists that feature's change sets"
        );
    }

    let out = run(
        &home,
        &["changeset", "list", "--global", "--project", PROJECT],
    );
    assert_eq!(out.code, 0, "--global: {}", out.stderr);
    assert_eq!(
        gets_to(&list_path("global")),
        1,
        "--global asks for ?feature=global: {:?}",
        server.requests()
    );
    assert_eq!(
        non_empty_lines(&out.stdout),
        [format!(
            "{GLOBAL_CHANGESET}  in_review  project  Brand refresh"
        )],
        "--global lists the project-level change sets"
    );
}

#[test]
fn help_names_the_feature_and_global_flags() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    for flag in ["--feature", "--global"] {
        assert!(
            help.stdout.contains(flag),
            "omni --help names {flag}: {:?}",
            help.stdout
        );
    }
}
