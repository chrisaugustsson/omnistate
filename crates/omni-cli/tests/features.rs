//! US-059: `omni feature`, tested as the built binary against a mock HTTP
//! server. None of these tests needs Postgres; the real round trip against the
//! API is in tests/stack_features.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni feature create <name> [--description <text>]` sends `POST
//!   /api/v1/w/{workspace}/projects/{project_id}/features` with the JSON body
//!   `{"name", "description"}` (`description` only when it was given) and
//!   prints one line, `<id>  <name>` (two spaces). The API answers 201
//!   `{"feature": Feature}`, where a Feature has `id`, `project_id`, `name`,
//!   `description`, `status` (`active` or `done`) and `created_at`.
//! - `omni feature list` sends `GET .../features` (`{"features": [...]}`, in
//!   creation order) and prints one line per feature, `<id>  <status>
//!   <name>` (two spaces between the fields), in the API's order.
//! - `omni feature show <name or id>` prints the feature: its ID, name, status
//!   and description. An ID (a UUID) is shown with `GET .../features/{id}`; a
//!   name is looked up in the project's features. A name that the project
//!   does not have is an error that names it.
//! - All of them take the project from `--project`, else the `project` of the
//!   nearest `.omni.toml`, as a UUID or a project name (US-016), print the
//!   API's JSON with `--json`, and report errors as one line on standard
//!   error (US-015, tests/common/mod.rs). `omni --help` lists them.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const OTHER_PROJECT: &str = "01900000-0000-7000-8000-0000000000b2";
const FEATURE: &str = "01900000-0000-7000-8000-0000000000f1";
const DONE_FEATURE: &str = "01900000-0000-7000-8000-0000000000f2";
const NEW_FEATURE: &str = "01900000-0000-7000-8000-0000000000f3";

fn projects_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects")
}

fn features_path(project: &str) -> String {
    format!("{}/{project}/features", projects_path())
}

fn project_json(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "created_at": "2026-10-23T10:00:00.000000Z" })
}

fn feature_json(id: &str, project: &str, name: &str, description: &str, status: &str) -> Value {
    json!({
        "id": id,
        "project_id": project,
        "name": name,
        "description": description,
        "status": status,
        "created_at": "2026-10-23T10:00:00.000000Z"
    })
}

fn checkout(project: &str) -> Value {
    feature_json(
        FEATURE,
        project,
        "Checkout",
        "Pay for the basket in one step.",
        "active",
    )
}

fn wish_list(project: &str) -> Value {
    feature_json(
        DONE_FEATURE,
        project,
        "Wish list",
        "Keep products for later.",
        "done",
    )
}

/// An API that knows two projects, `brand-site` and `other`, each with the
/// features "Checkout" (active) and "Wish list" (done), in that order. A new
/// feature named "checkout" in any case is refused as taken.
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
            let Some(rest) = path.strip_prefix(&features_path(project)) else {
                continue;
            };
            return match (method, rest) {
                ("POST", "") => {
                    let name = body["name"].as_str().unwrap_or("");
                    if name.eq_ignore_ascii_case("checkout") {
                        return Reply::json(409, &json!({ "error": "name_taken" }));
                    }
                    let description = body["description"].as_str().unwrap_or("");
                    Reply::json(
                        201,
                        &json!({ "feature": feature_json(NEW_FEATURE, project, name, description, "active") }),
                    )
                }
                ("GET", "") => Reply::json(
                    200,
                    &json!({ "features": [checkout(project), wish_list(project)] }),
                ),
                ("GET", rest) if rest == format!("/{FEATURE}") => {
                    Reply::json(200, &json!({ "feature": checkout(project) }))
                }
                ("GET", rest) if rest == format!("/{DONE_FEATURE}") => {
                    Reply::json(200, &json!({ "feature": wish_list(project) }))
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

fn requests_to(server: &MockServer, method: &str, path: &str) -> Vec<Recorded> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == method && r.path == path)
        .collect()
}

fn assert_nothing_posted(server: &MockServer) {
    let posts: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.method != "GET")
        .collect();
    assert!(posts.is_empty(), "nothing should be sent: {posts:?}");
}

// ---------------------------------------------------------------------------
// omni feature create
// ---------------------------------------------------------------------------

#[test]
fn feature_create_sends_the_name_and_description_and_prints_the_id_and_name() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "feature",
            "create",
            "Search",
            "--description",
            "Find products by name.",
            "--project",
            PROJECT,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);

    let posts = requests_to(&server, "POST", &features_path(PROJECT));
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    let body = body_of(&posts[0]);
    assert_eq!(body["name"], "Search", "{body}");
    assert_eq!(body["description"], "Find products by name.", "{body}");
    assert_eq!(
        posts[0].header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(posts[0].header("x-omni-run"), Some("run-7"));
    assert_eq!(posts[0].header("x-omni-role"), Some("planner"));

    assert_eq!(
        non_empty_lines(&out.stdout),
        [format!("{NEW_FEATURE}  Search")],
        "create prints `<id>  <name>`"
    );

    let json = run(
        &home,
        &[
            "feature",
            "create",
            "Search",
            "--project",
            PROJECT,
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "feature": feature_json(NEW_FEATURE, PROJECT, "Search", "", "active") }),
        "--json prints the whole response"
    );
}

#[test]
fn a_feature_without_a_description_sends_none() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["feature", "create", "Gift cards", "--project", PROJECT],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let posts = requests_to(&server, "POST", &features_path(PROJECT));
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    let body = body_of(&posts[0]);
    assert_eq!(body["name"], "Gift cards", "{body}");
    assert!(
        body.get("description")
            .is_none_or(|d| d.is_null() || d == ""),
        "no description is given: {body}"
    );
    assert_eq!(
        non_empty_lines(&out.stdout),
        [format!("{NEW_FEATURE}  Gift cards")]
    );
}

#[test]
fn feature_create_without_a_name_is_one_line_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["feature", "create", "--project", PROJECT]);
    assert_one_line_error(&out, &["name"]);
    assert_nothing_posted(&server);
}

#[test]
fn a_taken_name_is_one_line_with_the_error_code() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["feature", "create", "checkout", "--project", PROJECT],
    );
    assert_one_line_error(&out, &["name_taken"]);
}

// ---------------------------------------------------------------------------
// omni feature list
// ---------------------------------------------------------------------------

#[test]
fn feature_list_prints_one_line_per_feature_with_its_status() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["feature", "list", "--project", PROJECT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        requests_to(&server, "GET", &features_path(PROJECT)).len(),
        1,
        "{:?}",
        server.requests()
    );
    assert_eq!(
        non_empty_lines(&out.stdout),
        [
            format!("{FEATURE}  active  Checkout"),
            format!("{DONE_FEATURE}  done  Wish list"),
        ],
        "one `<id>  <status>  <name>` line per feature, in the API's order"
    );

    let json = run(&home, &["feature", "list", "--project", PROJECT, "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "features": [checkout(PROJECT), wish_list(PROJECT)] }),
        "--json prints the whole response"
    );
}

// ---------------------------------------------------------------------------
// omni feature show
// ---------------------------------------------------------------------------

#[test]
fn feature_show_prints_the_feature_named_by_its_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["feature", "show", FEATURE, "--project", PROJECT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        requests_to(
            &server,
            "GET",
            &format!("{}/{FEATURE}", features_path(PROJECT))
        )
        .len(),
        1,
        "an ID is shown with GET .../features/{{id}}: {:?}",
        server.requests()
    );
    for needle in [
        FEATURE,
        "Checkout",
        "active",
        "Pay for the basket in one step.",
    ] {
        assert!(
            out.stdout.contains(needle),
            "the text shows {needle:?}: {:?}",
            out.stdout
        );
    }

    let json = run(
        &home,
        &["feature", "show", FEATURE, "--project", PROJECT, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), json!({ "feature": checkout(PROJECT) }));
}

#[test]
fn feature_show_finds_a_feature_by_its_name() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["feature", "show", "Wish list", "--project", PROJECT],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    for needle in [
        DONE_FEATURE,
        "Wish list",
        "done",
        "Keep products for later.",
    ] {
        assert!(
            out.stdout.contains(needle),
            "the text shows {needle:?}: {:?}",
            out.stdout
        );
    }
    assert!(
        !out.stdout.contains(FEATURE),
        "only the named feature is shown: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &[
            "feature",
            "show",
            "Wish list",
            "--project",
            PROJECT,
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), json!({ "feature": wish_list(PROJECT) }));
    assert_nothing_posted(&server);
}

#[test]
fn feature_show_of_a_name_the_project_does_not_have_is_one_line() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["feature", "show", "Loyalty points", "--project", PROJECT],
    );
    assert_one_line_error(&out, &["Loyalty points"]);
    assert_nothing_posted(&server);
}

#[test]
fn feature_show_needs_a_name_or_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let control = run(&home, &["feature", "show", FEATURE, "--project", PROJECT]);
    assert_eq!(control.code, 0, "with an ID it works: {}", control.stderr);
    let before = server.requests().len();
    let out = run(&home, &["feature", "show", "--project", PROJECT]);
    assert_one_line_error(&out, &["feature"]);
    assert_eq!(
        server.requests().len(),
        before,
        "nothing is sent without a name or ID: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// The project
// ---------------------------------------------------------------------------

#[test]
fn the_project_comes_from_omni_toml_by_name_and_project_wins() {
    let server = mock();
    let home = logged_in(&server.url);
    let project_dir = Scratch::new("project");
    std::fs::write(project_dir.join(".omni.toml"), "project = \"brand-site\"\n").unwrap();
    let in_project = |args: &[&str]| {
        omni(&home.path)
            .cwd(&project_dir.path)
            .args(args)
            .args(&["--as", "run-7/planner"])
            .run()
    };

    for args in [
        &["feature", "create", "Search"][..],
        &["feature", "list"][..],
        &["feature", "show", FEATURE][..],
    ] {
        let out = in_project(args);
        assert_eq!(out.code, 0, "{args:?}: {}", out.stderr);
    }
    assert_eq!(
        requests_to(&server, "POST", &features_path(PROJECT)).len(),
        1,
        "the name `brand-site` is looked up and its ID is used: {:?}",
        server.requests()
    );
    assert_eq!(
        requests_to(&server, "GET", &features_path(PROJECT)).len(),
        1,
        "{:?}",
        server.requests()
    );
    assert_eq!(
        requests_to(
            &server,
            "GET",
            &format!("{}/{FEATURE}", features_path(PROJECT))
        )
        .len(),
        1,
        "{:?}",
        server.requests()
    );

    // --project wins over .omni.toml, by name as well as by ID.
    for project in ["other", OTHER_PROJECT] {
        let out = in_project(&["feature", "create", "Search", "--project", project]);
        assert_eq!(out.code, 0, "--project {project}: {}", out.stderr);
        let out = in_project(&["feature", "list", "--project", project]);
        assert_eq!(out.code, 0, "--project {project}: {}", out.stderr);
    }
    assert_eq!(
        requests_to(&server, "POST", &features_path(OTHER_PROJECT)).len(),
        2,
        "{:?}",
        server.requests()
    );
    assert_eq!(
        requests_to(&server, "GET", &features_path(OTHER_PROJECT)).len(),
        2,
        "{:?}",
        server.requests()
    );
}

#[test]
fn an_unknown_or_missing_project_is_one_line_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let unknown = run(
        &home,
        &[
            "feature",
            "create",
            "Search",
            "--project",
            "no-such-project",
        ],
    );
    assert_one_line_error(&unknown, &["no-such-project"]);

    let missing = run(&home, &["feature", "list"]);
    assert_one_line_error(&missing, &["project"]);
    assert_nothing_posted(&server);
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.path.contains("/features")),
        "no feature route is called without a project: {:?}",
        server.requests()
    );
}

#[test]
fn help_lists_the_feature_commands() {
    let home = Scratch::new("help");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    for command in ["feature create", "feature list", "feature show"] {
        assert!(
            help.stdout.contains(command),
            "omni --help lists `omni {command}`: {:?}",
            help.stdout
        );
    }
}
