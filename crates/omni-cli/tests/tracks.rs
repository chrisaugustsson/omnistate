//! US-059: `omni track`, tested as the built binary against a mock HTTP
//! server. None of these tests needs Postgres; the real round trip against the
//! API is in tests/stack_tracks.rs.
//!
//! The contract these tests pin down (the rest is the green step's choice):
//!
//! - `omni track create <name> [--description <text>]` sends `POST
//!   /api/v1/w/{workspace}/projects/{project_id}/tracks` with the JSON body
//!   `{"name", "description"}` (`description` only when it was given) and
//!   prints one line, `<id>  <name>` (two spaces). The API answers 201
//!   `{"track": Track}`, where a Track has `id`, `project_id`, `name`,
//!   `description` and `created_at`, and no status (US-110).
//! - `omni track list` sends `GET .../tracks` (`{"tracks": [...]}`, in
//!   creation order) and prints one line per track, `<id>  <name>` (two
//!   spaces between the fields), in the API's order.
//! - `omni track show <name or id>` prints the track: its ID, name,
//!   project and description, and no status. An ID (a UUID) is shown with
//!   `GET .../tracks/{id}`; a name is looked up in the project's tracks,
//!   and its `--json` is `{"track": Track}` as for an ID (US-301). A name
//!   that the project does not have is an error that names it.
//! - All of them take the project from `--project`, as a UUID or a project name (US-016), print the
//!   API's JSON with `--json`, and report errors as one line on standard
//!   error (US-015, tests/common/mod.rs). `omni --help` lists them.
//! - US-110: a track has no status, so `omni track done` and `omni track
//!   reopen` are gone: each is the one-line usage error of `omni track`,
//!   which names the track commands (rename and delete among them), and
//!   nothing is sent.
//! - US-110: `omni track rename <name or id> <new name>` sends `PATCH
//!   .../tracks/{id}` with `{"name": <new name>}` and prints `<id>  <new
//!   name>`; `omni track delete <name or id>` sends `DELETE
//!   .../tracks/{id}` (204) and prints a line with the track's ID. A name
//!   is looked up as for `show`, ignoring case. Without the track (or the
//!   new name) each is a one-line error that sends nothing; an API error is
//!   one line with its code.
//! - US-302: `omni track` replaces `omni feature` for work, with no aliases
//!   (D12): `omni feature create` makes no track (since US-307 it makes a
//!   feature and, without `--part`, is `part_required` and sends nothing),
//!   and `--feature` picks no track. `OMNI_TRACK` replaces `OMNI_FEATURE`:
//!   a non-empty `OMNI_FEATURE` picks no track, and omni says it was renamed
//!   in one line on standard error, leaving standard output as it is without
//!   it. An empty `OMNI_FEATURE`, `--help` and `--version` print no notice.
//! - US-305: every track sits on one part. `omni track create <name> --part
//!   <name|id>` sends the part's ID as `part_id`; without `--part` it fails
//!   with `part_required` and sends nothing. `omni track list` prints
//!   `<id>  <name>  <part name>`, and `--part` keeps it to that part's
//!   tracks. `omni track move <name|id> --part <name|id>` sends `POST
//!   .../tracks/{id}/move` with `{"part_id"}`. A `--part` name is looked up
//!   among the project's parts that are not deleted (`GET .../parts`); one
//!   that is not there is `unknown_part` and sends nothing. A `--part` that
//!   is an ID is sent as it is, and the API's 422 `invalid_part` is shown.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const OTHER_PROJECT: &str = "01900000-0000-7000-8000-0000000000b2";
const TRACK: &str = "01900000-0000-7000-8000-0000000000f1";
const WISH_LIST: &str = "01900000-0000-7000-8000-0000000000f2";
const NEW_TRACK: &str = "01900000-0000-7000-8000-0000000000f3";
const WISION: &str = "01900000-0000-7000-8000-0000000000a1";
const IOT_SERVICE: &str = "01900000-0000-7000-8000-0000000000a2";
/// A part ID that the API refuses with 422 `invalid_part`, as it does for a
/// deleted part or a part of another project.
const GONE_PART: &str = "01900000-0000-7000-8000-0000000000a9";

fn projects_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects")
}

fn tracks_path(project: &str) -> String {
    format!("{}/{project}/tracks", projects_path())
}

fn parts_path(project: &str) -> String {
    format!("{}/{project}/parts", projects_path())
}

fn project_json(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "created_at": "2026-10-23T10:00:00.000000Z" })
}

fn track_json(id: &str, project: &str, name: &str, description: &str, part: &str) -> Value {
    json!({
        "id": id,
        "project_id": project,
        "name": name,
        "description": description,
        "part_id": part,
        "state": "planning",
        "created_at": "2026-10-23T10:00:00.000000Z"
    })
}

fn checkout(project: &str) -> Value {
    track_json(
        TRACK,
        project,
        "Checkout",
        "Pay for the basket in one step.",
        WISION,
    )
}

fn wish_list(project: &str) -> Value {
    let mut track = track_json(
        WISH_LIST,
        project,
        "Wish list",
        "Keep products for later.",
        IOT_SERVICE,
    );
    track["state"] = json!("building");
    track
}

fn part_json(id: &str, name: &str, kind: &str) -> Value {
    json!({ "id": id, "name": name, "kind": kind, "description": "" })
}

/// An API that knows two projects, `brand-site` and `other`, each with the
/// parts "Wision" and "iot-service" and the tracks "Checkout" (on Wision) and
/// "Wish list" (on iot-service), in that order; Checkout is planning and Wish
/// list is building. A new track named "checkout"
/// in any case is refused as taken, and so is a rename to it. A track created
/// or moved to `GONE_PART` is 422 `invalid_part`. A rename answers the track
/// with its new name, a move answers it on its new part, and a delete answers
/// 204. A state change of Checkout to building or dropped answers it in that
/// state; any other is 422 `invalid_transition` with the state `planning`.
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
            if path == parts_path(project) && method == "GET" {
                return Reply::json(
                    200,
                    &json!({ "parts": [
                        part_json(WISION, "Wision", "app"),
                        part_json(IOT_SERVICE, "iot-service", "service")
                    ] }),
                );
            }
        }
        for project in [PROJECT, OTHER_PROJECT] {
            let Some(rest) = path.strip_prefix(&tracks_path(project)) else {
                continue;
            };
            let part = body["part_id"].as_str().unwrap_or("");
            if method == "POST" && part == GONE_PART {
                return Reply::json(422, &json!({ "error": "invalid_part" }));
            }
            return match (method, rest) {
                ("POST", "") => {
                    let name = body["name"].as_str().unwrap_or("");
                    if name.eq_ignore_ascii_case("checkout") {
                        return Reply::json(409, &json!({ "error": "name_taken" }));
                    }
                    let description = body["description"].as_str().unwrap_or("");
                    Reply::json(
                        201,
                        &json!({ "track": track_json(NEW_TRACK, project, name, description, part) }),
                    )
                }
                ("POST", rest) if rest == format!("/{TRACK}/move") => {
                    let mut moved = checkout(project);
                    moved["part_id"] = json!(part);
                    Reply::json(200, &json!({ "track": moved }))
                }
                ("POST", rest) if rest == format!("/{TRACK}/state") => {
                    let to = body["state"].as_str().unwrap_or("");
                    if to != "building" && to != "dropped" {
                        return Reply::json(
                            422,
                            &json!({
                                "error": "invalid_transition",
                                "state": "planning",
                                "message": format!("a track in planning cannot move to {to}")
                            }),
                        );
                    }
                    let mut moved = checkout(project);
                    moved["state"] = json!(to);
                    Reply::json(200, &json!({ "track": moved }))
                }
                ("GET", "") => Reply::json(
                    200,
                    &json!({ "tracks": [checkout(project), wish_list(project)] }),
                ),
                ("GET", rest) if rest == format!("/{TRACK}") => {
                    Reply::json(200, &json!({ "track": checkout(project) }))
                }
                ("GET", rest) if rest == format!("/{WISH_LIST}") => {
                    Reply::json(200, &json!({ "track": wish_list(project) }))
                }
                ("PATCH", rest)
                    if rest == format!("/{TRACK}") || rest == format!("/{WISH_LIST}") =>
                {
                    let name = body["name"].as_str().unwrap_or("");
                    if name.eq_ignore_ascii_case("checkout") {
                        return Reply::json(409, &json!({ "error": "name_taken" }));
                    }
                    let id = &rest[1..];
                    let part = if id == TRACK { WISION } else { IOT_SERVICE };
                    Reply::json(
                        200,
                        &json!({ "track": track_json(id, project, name, "", part) }),
                    )
                }
                ("DELETE", rest)
                    if rest == format!("/{TRACK}") || rest == format!("/{WISH_LIST}") =>
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
// omni track create
// ---------------------------------------------------------------------------

#[test]
fn track_create_sends_the_name_and_description_and_prints_the_id_and_name() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "track",
            "create",
            "Search",
            "--description",
            "Find products by name.",
            "--part",
            "Wision",
            "--project",
            PROJECT,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);

    let posts = requests_to(&server, "POST", &tracks_path(PROJECT));
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    let body = body_of(&posts[0]);
    assert_eq!(body["name"], "Search", "{body}");
    assert_eq!(body["description"], "Find products by name.", "{body}");
    assert_eq!(
        body["part_id"], WISION,
        "the part's name is sent as its ID: {body}"
    );
    assert_eq!(
        posts[0].header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(posts[0].header("x-omni-run"), Some("run-7"));
    assert_eq!(posts[0].header("x-omni-role"), Some("planner"));

    assert_eq!(
        non_empty_lines(&out.stdout),
        [format!("{NEW_TRACK}  Search")],
        "create prints `<id>  <name>`"
    );

    let json = run(
        &home,
        &[
            "track",
            "create",
            "Search",
            "--part",
            "Wision",
            "--project",
            PROJECT,
            "--json",
        ],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "track": track_json(NEW_TRACK, PROJECT, "Search", "", WISION) }),
        "--json prints the whole response"
    );
}

#[test]
fn track_create_without_a_name_is_one_line_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["track", "create", "--project", PROJECT]);
    assert_one_line_error(&out, &["name"]);
    assert_nothing_posted(&server);
}

#[test]
fn track_create_without_a_part_is_part_required_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["track", "create", "Search", "--project", PROJECT]);
    assert_one_line_error(&out, &["part_required"]);
    assert!(
        server.requests().is_empty(),
        "nothing is sent: {:?}",
        server.requests()
    );
}

#[test]
fn track_create_on_a_part_name_the_project_does_not_have_is_unknown_part_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "track",
            "create",
            "Search",
            "--part",
            "Wison",
            "--project",
            PROJECT,
        ],
    );
    assert_one_line_error(&out, &["unknown_part"]);
    assert_nothing_posted(&server);
}

#[test]
fn track_create_on_a_part_id_sends_the_id_and_shows_the_apis_invalid_part() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "track",
            "create",
            "Search",
            "--part",
            GONE_PART,
            "--project",
            PROJECT,
        ],
    );
    assert_one_line_error(&out, &["invalid_part"]);
    let posts = requests_to(&server, "POST", &tracks_path(PROJECT));
    assert_eq!(posts.len(), 1, "{:?}", server.requests());
    assert_eq!(body_of(&posts[0])["part_id"], GONE_PART);
}

// ---------------------------------------------------------------------------
// omni track list
// ---------------------------------------------------------------------------

#[test]
fn track_list_prints_one_line_per_track_with_its_state() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["track", "list", "--project", PROJECT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        requests_to(&server, "GET", &tracks_path(PROJECT)).len(),
        1,
        "{:?}",
        server.requests()
    );
    assert_eq!(
        non_empty_lines(&out.stdout),
        [
            format!("{TRACK}  Checkout  Wision  planning"),
            format!("{WISH_LIST}  Wish list  iot-service  building"),
        ],
        "one `<id>  <name>  <part name>  <state>` line per track, in the API's order"
    );

    let json = run(&home, &["track", "list", "--project", PROJECT, "--json"]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(
        json.json(),
        json!({ "tracks": [checkout(PROJECT), wish_list(PROJECT)] }),
        "--json prints the whole response"
    );
}

#[test]
fn track_list_with_a_part_lists_only_that_parts_tracks() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "track",
            "list",
            "--part",
            "iot-service",
            "--project",
            PROJECT,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        non_empty_lines(&out.stdout),
        [format!("{WISH_LIST}  Wish list  iot-service  building")]
    );
}

#[test]
fn track_list_with_a_part_name_the_project_does_not_have_is_unknown_part() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["track", "list", "--part", "Wison", "--project", PROJECT],
    );
    assert_one_line_error(&out, &["unknown_part"]);
}

// ---------------------------------------------------------------------------
// omni track move (US-305)
// ---------------------------------------------------------------------------

#[test]
fn track_move_sends_the_parts_id_and_prints_the_track_on_its_new_part() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "track",
            "move",
            "Checkout",
            "--part",
            "iot-service",
            "--project",
            PROJECT,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let moves = requests_to(
        &server,
        "POST",
        &format!("{}/{TRACK}/move", tracks_path(PROJECT)),
    );
    assert_eq!(moves.len(), 1, "{:?}", server.requests());
    assert_eq!(body_of(&moves[0]), json!({ "part_id": IOT_SERVICE }));
    assert_eq!(
        non_empty_lines(&out.stdout),
        [format!("{TRACK}  Checkout  iot-service  planning")]
    );
}

// ---------------------------------------------------------------------------
// omni track state (US-311)
// ---------------------------------------------------------------------------

#[test]
fn track_state_sends_the_new_state_and_prints_the_track_in_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "track",
            "state",
            "Checkout",
            "building",
            "--project",
            PROJECT,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let changes = requests_to(
        &server,
        "POST",
        &format!("{}/{TRACK}/state", tracks_path(PROJECT)),
    );
    assert_eq!(changes.len(), 1, "{:?}", server.requests());
    assert_eq!(body_of(&changes[0]), json!({ "state": "building" }));
    assert_eq!(
        non_empty_lines(&out.stdout),
        [format!("{TRACK}  Checkout  Wision  building")]
    );
}

#[test]
fn track_state_of_a_move_that_is_not_allowed_names_invalid_transition_and_the_current_state() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["track", "state", "Checkout", "done", "--project", PROJECT],
    );
    assert_one_line_error(&out, &["invalid_transition", "planning"]);
}

#[test]
fn track_move_to_a_part_name_the_project_does_not_have_is_unknown_part_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "track",
            "move",
            "Checkout",
            "--part",
            "Wison",
            "--project",
            PROJECT,
        ],
    );
    assert_one_line_error(&out, &["unknown_part"]);
    assert_nothing_posted(&server);
}

// ---------------------------------------------------------------------------
// omni track show
// ---------------------------------------------------------------------------

#[test]
fn track_show_prints_the_track_named_by_its_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["track", "show", TRACK, "--project", PROJECT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        requests_to(&server, "GET", &format!("{}/{TRACK}", tracks_path(PROJECT))).len(),
        1,
        "an ID is shown with GET .../tracks/{{id}}: {:?}",
        server.requests()
    );
    for needle in [TRACK, "Checkout", "Pay for the basket in one step."] {
        assert!(
            out.stdout.contains(needle),
            "the text shows {needle:?}: {:?}",
            out.stdout
        );
    }
    assert_no_status(&out.stdout);

    let json = run(
        &home,
        &["track", "show", TRACK, "--project", PROJECT, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), json!({ "track": checkout(PROJECT) }));
}

#[test]
fn track_show_finds_a_track_by_its_name() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["track", "show", "Wish list", "--project", PROJECT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    for needle in [WISH_LIST, "Wish list", "Keep products for later."] {
        assert!(
            out.stdout.contains(needle),
            "the text shows {needle:?}: {:?}",
            out.stdout
        );
    }
    assert!(
        out.stdout.contains(&format!("project: {PROJECT}\n")),
        "the text names the track's project: {:?}",
        out.stdout
    );
    assert_no_status(&out.stdout);
    assert!(
        !out.stdout.contains(TRACK),
        "only the named track is shown: {:?}",
        out.stdout
    );

    let json = run(
        &home,
        &["track", "show", "Wish list", "--project", PROJECT, "--json"],
    );
    assert_eq!(json.code, 0, "{}", json.stderr);
    assert_eq!(json.json(), json!({ "track": wish_list(PROJECT) }));
    assert_nothing_posted(&server);
}

#[test]
fn track_show_of_a_name_the_project_does_not_have_is_one_line() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &["track", "show", "Loyalty points", "--project", PROJECT],
    );
    assert_one_line_error(&out, &["Loyalty points"]);
    assert_nothing_posted(&server);
}

// ---------------------------------------------------------------------------
// The project
// ---------------------------------------------------------------------------

#[test]
fn the_project_flag_takes_a_name_or_an_id() {
    let server = mock();
    let home = logged_in(&server.url);
    let in_project = |args: &[&str]| {
        omni(&home.path)
            .args(args)
            .args(&["--as", "run-7/planner"])
            .run()
    };

    for args in [
        &["track", "create", "Search", "--part", "Wision"][..],
        &["track", "list"][..],
        &["track", "show", TRACK][..],
    ] {
        let out = in_project(&[args, &["--project", "brand-site"]].concat());
        assert_eq!(out.code, 0, "{args:?}: {}", out.stderr);
    }
    assert_eq!(
        requests_to(&server, "POST", &tracks_path(PROJECT)).len(),
        1,
        "the name `brand-site` is looked up and its ID is used: {:?}",
        server.requests()
    );
    assert_eq!(
        requests_to(&server, "GET", &tracks_path(PROJECT)).len(),
        1,
        "{:?}",
        server.requests()
    );
    assert_eq!(
        requests_to(&server, "GET", &format!("{}/{TRACK}", tracks_path(PROJECT))).len(),
        1,
        "{:?}",
        server.requests()
    );

    // --project takes a name as well as an ID.
    for project in ["other", OTHER_PROJECT] {
        let out = in_project(&[
            "track",
            "create",
            "Search",
            "--part",
            "Wision",
            "--project",
            project,
        ]);
        assert_eq!(out.code, 0, "--project {project}: {}", out.stderr);
        let out = in_project(&["track", "list", "--project", project]);
        assert_eq!(out.code, 0, "--project {project}: {}", out.stderr);
    }
    assert_eq!(
        requests_to(&server, "POST", &tracks_path(OTHER_PROJECT)).len(),
        2,
        "{:?}",
        server.requests()
    );
    assert_eq!(
        requests_to(&server, "GET", &tracks_path(OTHER_PROJECT)).len(),
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
            "track",
            "create",
            "Search",
            "--part",
            "Wision",
            "--project",
            "no-such-project",
        ],
    );
    assert_one_line_error(&unknown, &["no-such-project"]);

    let missing = run(&home, &["track", "list"]);
    assert_one_line_error(&missing, &["project"]);
    assert_nothing_posted(&server);
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.path.contains("/tracks")),
        "no track route is called without a project: {:?}",
        server.requests()
    );
}

/// What `omni track show` prints has no status: no line names one, and no
/// field is left as an unknown `?`.
fn assert_no_status(stdout: &str) {
    for line in non_empty_lines(stdout) {
        let lower = line.to_lowercase();
        assert!(
            !lower.contains("status") && !lower.contains("active") && !lower.contains("  ?  "),
            "a track has no status: {line:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// omni track done / reopen are gone (US-110)
// ---------------------------------------------------------------------------

#[test]
fn track_done_and_reopen_are_gone() {
    let server = mock();
    let home = logged_in(&server.url);
    for command in ["done", "reopen"] {
        for args in [
            &["track", command, "Checkout", "--project", PROJECT][..],
            &["track", command, TRACK, "--project", PROJECT][..],
            &["track", command][..],
        ] {
            let out = run(&home, args);
            assert_one_line_error(&out, &["usage", "rename", "delete"]);
            assert!(
                !out.stderr.contains("reviewer's call"),
                "no track can be marked done, by anyone: {:?}",
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
// omni track rename / delete (US-110)
// ---------------------------------------------------------------------------

fn track_path(project: &str, track: &str) -> String {
    format!("{}/{track}", tracks_path(project))
}

#[test]
fn track_rename_sends_the_new_name_and_prints_the_id_and_name() {
    let server = mock();
    let home = logged_in(&server.url);
    for given in ["Checkout", "checkout", TRACK] {
        let out = run(
            &home,
            &[
                "track",
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
            [format!("{TRACK}  Express checkout")],
            "rename prints `<id>  <new name>`"
        );
    }
    let patches = requests_to(&server, "PATCH", &track_path(PROJECT, TRACK));
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
            "track",
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
        json!({ "track": track_json(WISH_LIST, PROJECT, "Saved items", "", IOT_SERVICE) }),
        "--json prints the whole response"
    );
}

#[test]
fn track_rename_without_a_track_or_a_new_name_is_one_line_and_sends_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    for args in [
        &["track", "rename", "--project", PROJECT][..],
        &["track", "rename", "Checkout", "--project", PROJECT][..],
        &["track", "rename", "Checkout", "  ", "--project", PROJECT][..],
    ] {
        let out = run(&home, args);
        assert_one_line_error(&out, &["name"]);
    }
    assert_nothing_posted(&server);
}

#[test]
fn track_delete_sends_a_delete_and_prints_the_id() {
    let server = mock();
    let home = logged_in(&server.url);
    for (given, id) in [("Checkout", TRACK), (WISH_LIST, WISH_LIST)] {
        let out = run(&home, &["track", "delete", given, "--project", PROJECT]);
        assert_eq!(out.code, 0, "delete {given}: {}", out.stderr);
        assert!(
            out.stdout.contains(id),
            "delete prints the track's ID: {:?}",
            out.stdout
        );
        let deletes = requests_to(&server, "DELETE", &track_path(PROJECT, id));
        assert_eq!(deletes.len(), 1, "{:?}", server.requests());
        assert_eq!(deletes[0].header("x-omni-role"), Some("planner"));
    }
}

// ---------------------------------------------------------------------------
// omni feature and OMNI_FEATURE are renamed, with no aliases (US-302)
// ---------------------------------------------------------------------------

fn run_with_env(home: &Scratch, args: &[&str], name: &str, value: &str) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", "run-7/planner"])
        .env(name, value)
        .run()
}

#[test]
fn feature_create_without_a_part_is_part_required_and_creates_no_track() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(&home, &["feature", "create", "A", "--project", PROJECT]);
    assert_one_line_error(&out, &["part_required"]);
    assert!(
        server.requests().is_empty(),
        "nothing is sent: {:?}",
        server.requests()
    );
}

#[test]
fn the_feature_flag_picks_no_track_for_the_story_list() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run(
        &home,
        &[
            "story",
            "list",
            "--feature",
            "Checkout",
            "--project",
            PROJECT,
        ],
    );
    assert_ne!(out.code, 0, "no track is picked: {}", out.stdout);
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.path.ends_with("/stories")),
        "no track's stories are asked for: {:?}",
        server.requests()
    );
}

#[test]
fn help_lists_the_track_commands_and_names_omni_track_not_omni_feature() {
    let home = Scratch::new("home");
    let help = omni(&home.path).args(&["--help"]).run();
    assert_eq!(help.code, 0, "{}", help.stderr);
    let lines = non_empty_lines(&help.stdout);
    for command in [
        "omni track create <name>",
        "omni track list",
        "omni track show <name|id>",
        "omni track rename <name|id> <new name>",
        "omni track delete <name|id>",
    ] {
        assert!(
            lines.iter().any(|line| line.starts_with(command)),
            "omni --help lists {command:?}"
        );
    }
    assert!(
        !help.stdout.contains("OMNI_FEATURE"),
        "omni --help names OMNI_TRACK, not OMNI_FEATURE: {}",
        help.stdout
    );
}

#[test]
fn omni_feature_picks_no_track_for_the_story_list() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run_with_env(
        &home,
        &["story", "list", "--project", PROJECT],
        "OMNI_FEATURE",
        "Checkout",
    );
    assert_ne!(out.code, 0, "no track is picked: {}", out.stdout);
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.path.ends_with("/stories")),
        "no track's stories are asked for: {:?}",
        server.requests()
    );
}

#[test]
fn a_set_omni_feature_prints_a_one_line_notice_on_standard_error_and_leaves_standard_output_alone()
{
    let server = mock();
    let home = logged_in(&server.url);
    let args = ["track", "list", "--project", PROJECT, "--json"];
    let without = run(&home, &args);
    assert_eq!(without.code, 0, "{}", without.stderr);

    let with = run_with_env(&home, &args, "OMNI_FEATURE", "Checkout");
    assert_eq!(with.code, 0, "{}", with.stderr);
    assert_eq!(with.stdout, without.stdout, "standard output is unchanged");
    let lines = with.stderr_lines();
    assert_eq!(
        lines.len(),
        1,
        "one line on standard error: {:?}",
        with.stderr
    );
    assert!(
        lines[0].contains("OMNI_FEATURE") && lines[0].contains("OMNI_TRACK"),
        "the notice says OMNI_FEATURE was renamed to OMNI_TRACK: {:?}",
        lines[0]
    );
}

#[test]
fn an_empty_omni_feature_prints_no_notice() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = run_with_env(
        &home,
        &["track", "list", "--project", PROJECT],
        "OMNI_FEATURE",
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stderr, "");
}

#[test]
fn help_and_version_print_no_omni_feature_notice() {
    let home = Scratch::new("home");
    let help = run_with_env(&home, &["--help"], "OMNI_FEATURE", "Checkout");
    assert_eq!(help.code, 0, "{}", help.stderr);
    assert_eq!(help.stderr, "", "--help prints no notice");

    let version = run_with_env(&home, &["--version"], "OMNI_FEATURE", "Checkout");
    assert_eq!(version.code, 0, "{}", version.stderr);
    assert_eq!(version.stderr, "", "--version prints no notice");
}
