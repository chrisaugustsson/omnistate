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
//!   `description` and `created_at`, and no status (US-110).
//! - `omni feature list` sends `GET .../features` (`{"features": [...]}`, in
//!   creation order) and prints one line per feature, `<id>  <name>` (two
//!   spaces between the fields), in the API's order.
//! - `omni feature show <name or id>` prints the feature: its ID, name and
//!   description, and no status. An ID (a UUID) is shown with `GET
//!   .../features/{id}`; a name is looked up in the project's features. A
//!   name that the project does not have is an error that names it.
//! - All of them take the project from `--project`, else the `project` of the
//!   nearest `.omni.toml`, as a UUID or a project name (US-016), print the
//!   API's JSON with `--json`, and report errors as one line on standard
//!   error (US-015, tests/common/mod.rs). `omni --help` lists them.
//! - US-110: a feature has no status, so `omni feature done` and `omni feature
//!   reopen` are gone: each is the one-line usage error of `omni feature`,
//!   which names the feature commands (rename and delete among them), and
//!   nothing is sent.
//! - US-110: `omni feature rename <name or id> <new name>` sends `PATCH
//!   .../features/{id}` with `{"name": <new name>}` and prints `<id>  <new
//!   name>`; `omni feature delete <name or id>` sends `DELETE
//!   .../features/{id}` (204) and prints a line with the feature's ID. A name
//!   is looked up as for `show`, ignoring case. Without the feature (or the
//!   new name) each is a one-line error that sends nothing; an API error is
//!   one line with its code.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const OTHER_PROJECT: &str = "01900000-0000-7000-8000-0000000000b2";
const FEATURE: &str = "01900000-0000-7000-8000-0000000000f1";
const WISH_LIST: &str = "01900000-0000-7000-8000-0000000000f2";
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

fn feature_json(id: &str, project: &str, name: &str, description: &str) -> Value {
    json!({
        "id": id,
        "project_id": project,
        "name": name,
        "description": description,
        "created_at": "2026-10-23T10:00:00.000000Z"
    })
}

fn checkout(project: &str) -> Value {
    feature_json(
        FEATURE,
        project,
        "Checkout",
        "Pay for the basket in one step.",
    )
}

fn wish_list(project: &str) -> Value {
    feature_json(WISH_LIST, project, "Wish list", "Keep products for later.")
}

/// An API that knows two projects, `brand-site` and `other`, each with the
/// features "Checkout" and "Wish list", in that order. A new feature named
/// "checkout" in any case is refused as taken, and so is a rename to it. A
/// rename answers the feature with its new name, and a delete answers 204.
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
                        &json!({ "feature": feature_json(NEW_FEATURE, project, name, description) }),
                    )
                }
                ("GET", "") => Reply::json(
                    200,
                    &json!({ "features": [checkout(project), wish_list(project)] }),
                ),
                ("GET", rest) if rest == format!("/{FEATURE}") => {
                    Reply::json(200, &json!({ "feature": checkout(project) }))
                }
                ("GET", rest) if rest == format!("/{WISH_LIST}") => {
                    Reply::json(200, &json!({ "feature": wish_list(project) }))
                }
                ("PATCH", rest)
                    if rest == format!("/{FEATURE}") || rest == format!("/{WISH_LIST}") =>
                {
                    let name = body["name"].as_str().unwrap_or("");
                    if name.eq_ignore_ascii_case("checkout") {
                        return Reply::json(409, &json!({ "error": "name_taken" }));
                    }
                    let id = &rest[1..];
                    Reply::json(
                        200,
                        &json!({ "feature": feature_json(id, project, name, "") }),
                    )
                }
                ("DELETE", rest)
                    if rest == format!("/{FEATURE}") || rest == format!("/{WISH_LIST}") =>
                {
                    Reply {
                        status: 204,
                        headers: Vec::new(),
                        body: String::new(),
                    }
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
        json!({ "feature": feature_json(NEW_FEATURE, PROJECT, "Search", "") }),
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
fn feature_list_prints_one_line_per_feature_without_a_status() {
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
            format!("{FEATURE}  Checkout"),
            format!("{WISH_LIST}  Wish list"),
        ],
        "one `<id>  <name>` line per feature, in the API's order"
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
    for needle in [FEATURE, "Checkout", "Pay for the basket in one step."] {
        assert!(
            out.stdout.contains(needle),
            "the text shows {needle:?}: {:?}",
            out.stdout
        );
    }
    assert_no_status(&out.stdout);

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
    for needle in [WISH_LIST, "Wish list", "Keep products for later."] {
        assert!(
            out.stdout.contains(needle),
            "the text shows {needle:?}: {:?}",
            out.stdout
        );
    }
    assert_no_status(&out.stdout);
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
    for gone in ["feature done", "feature reopen"] {
        assert!(
            !help.stdout.contains(gone),
            "omni --help no longer lists `omni {gone}`: {:?}",
            help.stdout
        );
    }
    for command in [
        "feature create",
        "feature list",
        "feature show",
        "feature rename",
        "feature delete",
    ] {
        assert!(
            help.stdout.contains(command),
            "omni --help lists `omni {command}`: {:?}",
            help.stdout
        );
    }
}

/// What `omni feature show` prints has no status: no line names one, and no
/// field is left as an unknown `?`.
fn assert_no_status(stdout: &str) {
    for line in non_empty_lines(stdout) {
        let lower = line.to_lowercase();
        assert!(
            !lower.contains("status") && !lower.contains("active") && !lower.contains("  ?  "),
            "a feature has no status: {line:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// omni feature done / reopen are gone (US-110)
// ---------------------------------------------------------------------------

#[test]
fn feature_done_and_reopen_are_gone() {
    let server = mock();
    let home = logged_in(&server.url);
    for command in ["done", "reopen"] {
        for args in [
            &["feature", command, "Checkout", "--project", PROJECT][..],
            &["feature", command, FEATURE, "--project", PROJECT][..],
            &["feature", command][..],
        ] {
            let out = run(&home, args);
            assert_one_line_error(&out, &["usage", "rename", "delete"]);
            assert!(
                !out.stderr.contains("reviewer's call"),
                "no feature can be marked done, by anyone: {:?}",
                out.stderr
            );
        }
    }
    assert!(
        server.requests().is_empty(),
        "nothing is sent: {:?}",
        server.requests()
    );
}

// ---------------------------------------------------------------------------
// omni feature rename / delete (US-110)
// ---------------------------------------------------------------------------

fn feature_path(project: &str, feature: &str) -> String {
    format!("{}/{feature}", features_path(project))
}

#[test]
fn feature_rename_sends_the_new_name_and_prints_the_id_and_name() {
    let server = mock();
    let home = logged_in(&server.url);
    for given in ["Checkout", "checkout", FEATURE] {
        let out = run(
            &home,
            &[
                "feature",
                "rename",
                given,
                "Express checkout",
                "--project",
                PROJECT,
            ],
        );
        assert_eq!(out.code, 0, "rename {given}: {}", out.stderr);
        assert_eq!(
            non_empty_lines(&out.stdout),
            [format!("{FEATURE}  Express checkout")],
            "rename prints `<id>  <new name>`"
        );
    }
    let patches = requests_to(&server, "PATCH", &feature_path(PROJECT, FEATURE));
    assert_eq!(patches.len(), 3, "{:?}", server.requests());
    for patch in &patches {
        assert_eq!(body_of(patch), json!({ "name": "Express checkout" }));
        assert_eq!(
            patch.header("authorization"),
            Some(format!("Bearer {API_KEY}").as_str())
        );
        assert_eq!(patch.header("x-omni-run"), Some("run-7"));
    }

    let json = run(
        &home,
        &[
            "feature",
            "rename",
            "Wish list",
            "Saved items",
            "--project",
            PROJECT,
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "feature": feature_json(WISH_LIST, PROJECT, "Saved items", "") }),
        "--json prints the whole response"
    );
}

#[test]
fn feature_rename_without_a_feature_or_a_new_name_is_one_line_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        &["feature", "rename", "--project", PROJECT][..],
        &["feature", "rename", "Checkout", "--project", PROJECT][..],
        &["feature", "rename", "Checkout", "  ", "--project", PROJECT][..],
    ] {
        let out = run(&home, args);
        assert_one_line_error(&out, &["name"]);
    }
    assert_nothing_posted(&server);
}

#[test]
fn feature_rename_of_an_unknown_name_or_to_a_taken_one_is_one_line() {
    let server = mock();
    let home = logged_in(&server.url);
    let unknown = run(
        &home,
        &[
            "feature",
            "rename",
            "Loyalty points",
            "Points",
            "--project",
            PROJECT,
        ],
    );
    assert_one_line_error(&unknown, &["Loyalty points"]);
    assert_nothing_posted(&server);

    let taken = run(
        &home,
        &[
            "feature",
            "rename",
            "Wish list",
            "CHECKOUT",
            "--project",
            PROJECT,
        ],
    );
    assert_one_line_error(&taken, &["name_taken"]);
}

#[test]
fn feature_delete_sends_a_delete_and_prints_the_id() {
    let server = mock();
    let home = logged_in(&server.url);
    for (given, id) in [("Checkout", FEATURE), (WISH_LIST, WISH_LIST)] {
        let out = run(&home, &["feature", "delete", given, "--project", PROJECT]);
        assert_eq!(out.code, 0, "delete {given}: {}", out.stderr);
        assert!(
            out.stdout.contains(id),
            "delete prints the feature's ID: {:?}",
            out.stdout
        );
        let deletes = requests_to(&server, "DELETE", &feature_path(PROJECT, id));
        assert_eq!(deletes.len(), 1, "{:?}", server.requests());
        assert_eq!(deletes[0].header("x-omni-role"), Some("planner"));
    }
}

#[test]
fn feature_delete_without_a_feature_or_of_an_unknown_one_is_one_line_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let missing = run(&home, &["feature", "delete", "--project", PROJECT]);
    assert_one_line_error(&missing, &["feature"]);
    let unknown = run(
        &home,
        &["feature", "delete", "Loyalty points", "--project", PROJECT],
    );
    assert_one_line_error(&unknown, &["Loyalty points"]);
    assert_nothing_posted(&server);
}
