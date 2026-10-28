//! US-016: `omni project` and `omni changeset`, tested as the built binary
//! against a mock HTTP server. None of these tests needs Postgres; the real
//! round trip against the API is in tests/stack_changesets.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni project create <name>` sends `POST /api/v1/w/{workspace}/projects`
//!   with the JSON body `{"name": "<name>"}`; `omni project list` sends `GET`
//!   on the same path. The API answers `{"project": {"id", "name",
//!   "created_at"}}` and `{"projects": [...]}`.
//! - `omni changeset create --title <t> [--description <d>] [--run <tag>]
//!   [--project <p>]` sends `POST .../projects/{project_id}/changesets` with
//!   the JSON body `{"title", "description", "run_tag"}` (only the options
//!   that were given need be in it). `omni changeset list [--project <p>]` and
//!   `omni changeset show <id> [--project <p>]` are `GET`s on
//!   `.../changesets` and `.../changesets/{id}`; `omni changeset
//!   submit|reopen|abandon <id> [--project <p>]` are `POST`s on
//!   `.../changesets/{id}/submit|reopen|abandon`. The API answers
//!   `{"changeset": {...}}` (`{"changesets": [...]}` for the list), where a
//!   change set has `id`, `project_id`, `title`, `description`, `status`,
//!   `run_tag` and `history`, a list of `{"from_status", "to_status", "actor",
//!   "reason", "at"}`.
//! - The project is `--project`, else the `project` of the nearest `.omni.toml`
//!   (US-015). Either is a project ID (a UUID) or a project name; a name is
//!   looked up with `GET .../projects`.
//! - Rejecting is a human action: there is no `omni changeset reject`.
//! - Short text by default and the API's JSON with `--json`, errors as one line
//!   on standard error, as in US-015 (tests/common/mod.rs).

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const OTHER_PROJECT: &str = "01900000-0000-7000-8000-0000000000b2";
const NEW_PROJECT: &str = "01900000-0000-7000-8000-0000000000b3";
const CHANGESET: &str = "01900000-0000-7000-8000-0000000000c1";
const STUCK_CHANGESET: &str = "01900000-0000-7000-8000-0000000000c2";
const NEW_CHANGESET: &str = "01900000-0000-7000-8000-0000000000c3";
const REJECTION: &str = "The hero contrast is too low; rework the headline.";

fn projects_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects")
}

fn changesets_path(project: &str) -> String {
    format!("{}/{project}/changesets", projects_path())
}

fn project_json(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "created_at": "2026-10-15T10:00:00.000000Z" })
}

fn changeset_json(id: &str, project: &str, title: &str, status: &str) -> Value {
    json!({
        "id": id,
        "project_id": project,
        "title": title,
        "description": "Landing hero, first pass.",
        "status": status,
        "run_tag": "ui-research-7",
        "created_at": "2026-10-15T10:00:00.000000Z",
        "history": [
            {
                "from_status": null,
                "to_status": "open",
                "actor": "agent",
                "reason": null,
                "at": "2026-10-15T10:00:00.000000Z"
            }
        ]
    })
}

/// The change set as `show` answers it after one round of review: submitted,
/// rejected by the human with a reason, reopened and submitted again.
fn reviewed_changeset() -> Value {
    let mut changeset = changeset_json(CHANGESET, PROJECT, "Hero section", "in_review");
    let entry = |from: Option<&str>, to: &str, actor: &str, reason: Option<&str>| {
        json!({
            "from_status": from,
            "to_status": to,
            "actor": actor,
            "reason": reason,
            "at": "2026-10-15T10:05:00.000000Z"
        })
    };
    changeset["history"] = json!([
        entry(None, "open", "agent", None),
        entry(Some("open"), "in_review", "agent", None),
        entry(Some("in_review"), "rejected", "human", Some(REJECTION)),
        entry(Some("rejected"), "open", "agent", None),
        entry(Some("open"), "in_review", "agent", None),
    ]);
    changeset
}

/// An API that knows two projects, `brand-site` and `other`, and one change
/// set.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let path = request.path.as_str();
        let method = request.method.as_str();
        let projects = projects_path();
        if path == projects {
            return match method {
                "GET" => Reply::json(
                    200,
                    &json!({ "projects": [
                        project_json(PROJECT, "brand-site"),
                        project_json(OTHER_PROJECT, "other")
                    ] }),
                ),
                "POST" => Reply::json(
                    201,
                    &json!({ "project": project_json(NEW_PROJECT, body["name"].as_str().unwrap_or("?")) }),
                ),
                _ => Reply::json(405, &json!({ "error": "method_not_allowed" })),
            };
        }
        for project in [PROJECT, OTHER_PROJECT] {
            let base = changesets_path(project);
            let Some(rest) = path.strip_prefix(&base) else {
                continue;
            };
            let title = body["title"].as_str().unwrap_or("untitled");
            return match (method, rest) {
                ("POST", "") => Reply::json(
                    201,
                    &json!({ "changeset": changeset_json(NEW_CHANGESET, project, title, "open") }),
                ),
                ("GET", "") => Reply::json(
                    200,
                    &json!({ "changesets": [
                        changeset_json(CHANGESET, project, "Hero section", "in_review"),
                        changeset_json(STUCK_CHANGESET, project, "Pricing table", "abandoned")
                    ] }),
                ),
                ("GET", rest) if rest == format!("/{CHANGESET}") => {
                    Reply::json(200, &json!({ "changeset": reviewed_changeset() }))
                }
                ("POST", rest) if rest.starts_with(&format!("/{CHANGESET}/")) => {
                    let status = match rest.rsplit('/').next() {
                        Some("submit") => "in_review",
                        Some("reopen") => "open",
                        Some("abandon") => "abandoned",
                        _ => return Reply::json(404, &json!({ "error": "not_found" })),
                    };
                    Reply::json(
                        200,
                        &json!({ "changeset": changeset_json(CHANGESET, project, "Hero section", status) }),
                    )
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
        .args(&["--as", "run-7/author"])
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

// ---------------------------------------------------------------------------
// omni project
// ---------------------------------------------------------------------------

#[test]
fn project_create_sends_the_name_as_the_agent_and_prints_the_new_project() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["project", "create", "landing-page"]);
    assert_eq!(out.code, 0, "{}", out.stderr);

    let posts = requests_to(&server, "POST", &projects_path());
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    assert_eq!(body_of(&posts[0])["name"], "landing-page");
    assert_eq!(
        posts[0].header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(posts[0].header("x-omni-run"), Some("run-7"));
    assert_eq!(posts[0].header("x-omni-role"), Some("author"));

    assert!(
        serde_json::from_str::<Value>(&out.stdout).is_err(),
        "the default output is plain text: {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains(NEW_PROJECT) && out.stdout.contains("landing-page"),
        "the text has the project's ID and name: {:?}",
        out.stdout
    );
    assert!(non_empty_lines(&out.stdout).len() <= 5, "{:?}", out.stdout);

    let json = run(&home, &["project", "create", "landing-page", "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "project": project_json(NEW_PROJECT, "landing-page") }),
        "--json prints the whole response"
    );
}

#[test]
fn project_list_prints_one_line_per_project() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["project", "list"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(requests_to(&server, "GET", &projects_path()).len(), 1);
    let lines = non_empty_lines(&out.stdout);
    let line_of = |needle: &str| {
        lines
            .iter()
            .filter(|line| line.contains(needle))
            .collect::<Vec<_>>()
    };
    assert_eq!(line_of("brand-site").len(), 1, "{:?}", out.stdout);
    assert_eq!(line_of("other").len(), 1, "{:?}", out.stdout);
    assert!(
        line_of("brand-site")[0].contains(PROJECT) && line_of("other")[0].contains(OTHER_PROJECT),
        "each line has the project's ID: {:?}",
        out.stdout
    );

    let json = run(&home, &["project", "list", "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json()["projects"].as_array().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// omni changeset create and list
// ---------------------------------------------------------------------------

#[test]
fn changeset_create_sends_the_title_description_and_run_tag() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "changeset",
            "create",
            "--project",
            PROJECT,
            "--title",
            "Hero section",
            "--description",
            "Landing hero, first pass.",
            "--run",
            "ui-research-7",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);

    let posts = requests_to(&server, "POST", &changesets_path(PROJECT));
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    let body = body_of(&posts[0]);
    assert_eq!(body["title"], "Hero section", "{body}");
    assert_eq!(body["description"], "Landing hero, first pass.", "{body}");
    assert_eq!(body["run_tag"], "ui-research-7", "{body}");
    assert_eq!(posts[0].header("x-omni-run"), Some("run-7"));

    assert!(
        out.stdout.contains(NEW_CHANGESET)
            && out.stdout.contains("Hero section")
            && out.stdout.contains("open"),
        "the text has the new change set's ID, title and status: {:?}",
        out.stdout
    );
    assert!(non_empty_lines(&out.stdout).len() <= 6, "{:?}", out.stdout);

    let json = run(
        &home,
        &[
            "changeset",
            "create",
            "--project",
            PROJECT,
            "--title",
            "T",
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json()["changeset"]["id"], NEW_CHANGESET);
}

// ---------------------------------------------------------------------------
// omni changeset show
// ---------------------------------------------------------------------------

#[test]
fn changeset_show_prints_the_status_history_with_the_rejection_reason() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["changeset", "show", CHANGESET, "--project", PROJECT],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        requests_to(
            &server,
            "GET",
            &format!("{}/{CHANGESET}", changesets_path(PROJECT))
        )
        .len(),
        1,
        "{:?}",
        server.requests()
    );

    for needle in [
        CHANGESET,
        "Hero section",
        "Landing hero, first pass.",
        "ui-research-7",
        "in_review",
    ] {
        assert!(
            out.stdout.contains(needle),
            "the text shows {needle:?}: {:?}",
            out.stdout
        );
    }
    assert!(
        out.stdout.contains(REJECTION),
        "the rejection reason is in the history: {:?}",
        out.stdout
    );

    // The reason is on the line of the rejection.
    let lines = non_empty_lines(&out.stdout);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("rejected") && l.contains(REJECTION)),
        "the rejection and its reason are on one line: {:?}",
        out.stdout
    );
    let human_lines: Vec<_> = lines.iter().filter(|l| l.contains("human")).collect();
    assert_eq!(
        human_lines.len(),
        1,
        "the human acted once: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &[
            "changeset",
            "show",
            CHANGESET,
            "--project",
            PROJECT,
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "changeset": reviewed_changeset() }),
        "--json prints the whole response, history included"
    );
}

// ---------------------------------------------------------------------------
// The lifecycle commands
// ---------------------------------------------------------------------------

#[test]
fn submit_reopen_and_abandon_are_agent_posts_that_print_the_new_status() {
    let server = mock();
    let home = logged_in(&server.url);
    for (command, status) in [
        ("submit", "in_review"),
        ("reopen", "open"),
        ("abandon", "abandoned"),
    ] {
        let out = run(
            &home,
            &["changeset", command, CHANGESET, "--project", PROJECT],
        );
        assert_eq!(out.code, 0, "{command}: {}", out.stderr);
        let path = format!("{}/{CHANGESET}/{command}", changesets_path(PROJECT));
        let posts = requests_to(&server, "POST", &path);
        assert_eq!(posts.len(), 1, "{command}: {:?}", server.requests());
        assert_eq!(
            posts[0].header("authorization"),
            Some(format!("Bearer {API_KEY}").as_str())
        );
        assert_eq!(posts[0].header("x-omni-run"), Some("run-7"));
        assert!(
            out.stdout.contains(CHANGESET) && out.stdout.contains(status),
            "{command} prints the change set and its new status {status}: {:?}",
            out.stdout
        );

        let json = run(
            &home,
            &[
                "changeset",
                command,
                CHANGESET,
                "--project",
                PROJECT,
                "--json",
            ],
        );
        assert_eq!(json.code, 0, "{command} --json: {}", json.stderr);
        assert_eq!(json.json()["changeset"]["status"], status);
    }
}

#[test]
fn rejecting_is_left_to_the_human_in_the_web_app() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "changeset",
            "reject",
            CHANGESET,
            "--project",
            PROJECT,
            "--reason",
            "no",
        ],
    );
    assert_one_line_error(&out, &["web app"]);
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}
