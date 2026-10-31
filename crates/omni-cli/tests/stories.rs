//! US-136: `omni story add`, `list` and `read`, tested as the built binary
//! against a mock HTTP server. The API's own rules (numbering, the pick rule,
//! the version) are in crates/omni-api/tests/stories.rs; these tests pin what
//! the CLI adds on top: reading a story in the prd format into the create
//! body, and the short text of `list` and `read`.
//!
//! The contract (all paths under `/api/v1/w/{workspace}/projects/{project}`
//! `/features/{feature}/stories`, the API answering `{"story": Story}` or
//! `{"stories": [Story, ...]}`):
//!
//! - `omni story add <file>` posts `title` (a `US-<n>:` heading prefix
//!   stripped), `text` (the Description paragraph without its label, then the
//!   Topic, Interfaces and Files lines verbatim), `topic`, `files_planned`
//!   (the backticked paths of the Files line's `change:` segment, up to `;`
//!   or ` · `, without anything in parentheses), `criteria` (each `- [ ]`
//!   line without its marker; `human_check` for a line ending in `Verify in
//!   browser using dev-browser skill` or holding `(human check)`),
//!   `blocked_by` (keys resolved by listing the feature), `priority` and
//!   `links`. It prints only the new key. An unknown blocker key is
//!   `not_found: no story US-<n> in this feature`, exit 1. `--link`'s doc is
//!   a doc ID only, as for `omni doc read`.
//! - `omni story list [--status <s>] [--pickable]` prints `US-<n>  P<priority>
//!   <status>  <title>` per story in the API's order; `--json` the API's list.
//! - `omni story read <key>` prints the text, `[x]`/`[ ]` criteria with
//!   `(pending)`, `blocked by: <keys>`, `holder: <run> <stage> <at>` and one
//!   `[<stage>] <text>` line per unresolved finding, leaving out a line whose
//!   value is empty; `--json` prints the story.
//!
//! US-137 adds the writes (`edit`, `cancel`, `claim`, `release`, `move`,
//! `block`, `unblock`, `tick`, `finding`, `resolve`, `decision`, `files`):
//! each lists the feature to read the story, sends its `version` as
//! `expected_version` to `<stories>/<id>/<route>`, and with `--json` prints
//! the story the API answers. A 409 of any kind is `conflict: <the API's
//! error> ; read the story again` on stderr and exit code 3, which `omni
//! --help` documents. `tick` sends `pending` only with `--pending`; `finding`
//! always sends `files`, `[]` without `--file`. There is no `omni story
//! check`: the human clears checks on the board.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const FEATURE: &str = "01900000-0000-7000-8000-0000000000f1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
const STORY_131: &str = "01900000-0000-7000-8000-000000000131";
const STORY_132: &str = "01900000-0000-7000-8000-000000000132";
const STORY_133: &str = "01900000-0000-7000-8000-000000000133";

fn stories_path() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/features/{FEATURE}/stories")
}

fn story_json(id: &str, key: &str, priority: i64, status: &str, title: &str) -> Value {
    json!({
        "id": id,
        "project_id": PROJECT,
        "feature_id": FEATURE,
        "key": key,
        "status": status,
        "priority": priority,
        "title": title,
        "text": "",
        "topic": "",
        "criteria": [],
        "blocked_by": [],
        "files_planned": [],
        "files_changed": null,
        "links": [],
        "blocked": false,
        "holder": null,
        "findings": [],
        "decisions": [],
        "version": 1
    })
}

/// US-132, held by run-7 in review, blocked by US-131, with a ticked
/// criterion, a ticked one still pending a human check, an open one, and a
/// resolved and an unresolved finding.
fn held_story() -> Value {
    let mut story = story_json(STORY_132, "US-132", 2, "review", "Pick rule");
    story["text"] = json!("As an agent, I want the pickable stories.");
    story["criteria"] = json!([
        { "text": "Lists the pickable stories", "ticked": true, "human_check": false, "pending": false },
        { "text": "The board shows it. Verify in browser using dev-browser skill", "ticked": true, "human_check": true, "pending": true },
        { "text": "Typecheck/lint passes", "ticked": false, "human_check": false, "pending": false }
    ]);
    story["blocked_by"] = json!([STORY_131]);
    story["holder"] = json!({ "run": "run-7", "stage": "review", "at": "2026-10-31T10:00:00Z" });
    story["findings"] = json!([
        { "stage": "review", "text": "Old finding", "files": [], "resolved": true },
        { "stage": "review", "text": "Order ignores priority", "files": ["src/stories.rs"], "resolved": false }
    ]);
    story
}

/// US-133, a fix story nothing holds or blocks.
fn fix_story() -> Value {
    let mut story = story_json(STORY_133, "US-133", 0, "backlog", "Fix the pick order");
    story["text"] = json!("Order by priority first.");
    story
}

/// The feature's stories, deliberately not in key order.
fn listed() -> Value {
    json!({ "stories": [
        fix_story(),
        story_json(STORY_131, "US-131", 3, "done", "Stories table"),
        held_story()
    ] })
}

/// An API that lists `listed()` under any query and answers a create with
/// US-134.
fn mock() -> MockServer {
    MockServer::start(|request| {
        let path = request.path.split('?').next().unwrap_or_default();
        if path != stories_path() {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        match request.method.as_str() {
            "GET" => Reply::json(200, &listed()),
            "POST" => Reply::json(
                201,
                &json!({ "story": story_json(
                    "01900000-0000-7000-8000-000000000134",
                    "US-134",
                    2,
                    "backlog",
                    "Board"
                ) }),
            ),
            _ => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

fn logged_in(server: &MockServer) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, &server.url, WORKSPACE, API_KEY);
    home
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&[
            "--project",
            PROJECT,
            "--feature",
            FEATURE,
            "--as",
            "run-7/implementer",
        ])
        .run()
}

fn posts(server: &MockServer) -> Vec<Value> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .map(|r| serde_json::from_slice(&r.body).expect("the request body is JSON"))
        .collect()
}

fn get_paths(server: &MockServer) -> Vec<String> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method == "GET")
        .map(|r| r.path)
        .collect()
}

fn criteria_of(body: &Value) -> Vec<(String, bool)> {
    body["criteria"]
        .as_array()
        .expect("criteria is a list")
        .iter()
        .map(|c| {
            (
                c["text"].as_str().expect("criterion text").to_string(),
                c["human_check"].as_bool().expect("criterion human_check"),
            )
        })
        .collect()
}

const DESCRIPTION: &str =
    "As an agent, I want to save a story from a file, so that skills never call the API by hand.";
const TOPIC_LINE: &str = "**Topic:** feature · cli, stories";
const INTERFACES_LINE: &str =
    "**Interfaces:** command group `story` in `crates/omni-cli/src/stories.rs`";
const FILES_LINE: &str = "**Files:** change: `crates/omni-cli/src/stories.rs` (new), `crates/omni-cli/src/main.rs` (`HELP`), `migrations/<next>_stories.sql`; test: `crates/omni-cli/tests/stories.rs` (new, mock server) · read first: `crates/omni-cli/src/threads.rs`";

fn story_file(home: &Scratch, blocked_by: &str) -> String {
    let text = format!(
        "### US-132: Pick rule\n\
         **Description:** {DESCRIPTION}\n\
         {TOPIC_LINE}\n\
         **Blocked by:** {blocked_by}\n\
         {INTERFACES_LINE}\n\
         {FILES_LINE}\n\
         \n\
         **Acceptance Criteria:**\n\
         - [ ] `omni story add` posts the story\n\
         - [ ] The board shows the new card. Verify in browser using dev-browser skill\n\
         - [ ] Typecheck/lint passes (`make lint`)\n"
    );
    let path = home.join("story.md");
    std::fs::write(&path, text).unwrap();
    path.display().to_string()
}

#[test]
fn add_posts_the_story_read_from_the_prd_format_and_prints_only_its_key() {
    let server = mock();
    let home = logged_in(&server);
    let file = story_file(&home, "US-131");
    let link = format!("{DOC}:n1:design");

    let out = run(
        &home,
        &["story", "add", &file, "--priority", "2", "--link", &link],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, "US-134\n");
    let sent = posts(&server);
    assert_eq!(sent.len(), 1, "{sent:?}");
    let body = &sent[0];
    assert_eq!(body["title"], "Pick rule");
    let text = body["text"].as_str().unwrap();
    assert!(text.starts_with(DESCRIPTION), "{text:?}");
    assert!(text.contains(TOPIC_LINE), "{text:?}");
    assert!(text.contains(INTERFACES_LINE), "{text:?}");
    assert!(text.contains(FILES_LINE), "{text:?}");
    assert_eq!(body["topic"], "feature · cli, stories");
    assert_eq!(
        body["files_planned"],
        json!([
            "crates/omni-cli/src/stories.rs",
            "crates/omni-cli/src/main.rs",
            "migrations/<next>_stories.sql"
        ])
    );
    assert_eq!(
        criteria_of(body),
        vec![
            ("`omni story add` posts the story".to_string(), false),
            (
                "The board shows the new card. Verify in browser using dev-browser skill"
                    .to_string(),
                true
            ),
            ("Typecheck/lint passes (`make lint`)".to_string(), false),
        ]
    );
    assert_eq!(body["blocked_by"], json!([STORY_131]));
    assert_eq!(body["priority"], 2);
    assert_eq!(
        body["links"],
        json!([{ "doc_id": DOC, "node_id": "n1", "label": "design" }])
    );
}

#[test]
fn a_heading_without_a_key_a_human_check_marker_and_a_files_line_cut_at_a_middle_dot() {
    let server = mock();
    let home = logged_in(&server);
    let path = home.join("story.md");
    std::fs::write(
        &path,
        "### Board page\n\
         **Description:** As a human, I want a board.\n\
         **Topic:** feature · web\n\
         **Files:** change: `web/src/Board.tsx` · read first: `web/src/Threads.tsx`; test: `web/src/Board.test.tsx`\n\
         \n\
         **Acceptance Criteria:**\n\
         - [ ] Five columns show (human check) on a phone\n",
    )
    .unwrap();

    let out = run(&home, &["story", "add", &path.display().to_string()]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let sent = posts(&server);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["title"], "Board page");
    assert_eq!(sent[0]["files_planned"], json!(["web/src/Board.tsx"]));
    assert_eq!(
        criteria_of(&sent[0]),
        vec![(
            "Five columns show (human check) on a phone".to_string(),
            true
        )]
    );
}

#[test]
fn add_with_a_blocker_the_feature_does_not_have_exits_1_and_posts_nothing() {
    let server = mock();
    let home = logged_in(&server);
    let file = story_file(&home, "US-131, US-9");

    let out = run(&home, &["story", "add", &file]);

    assert_eq!(out.code, 1, "stdout: {}", out.stdout);
    assert_eq!(out.stdout, "");
    let lines = out.stderr_lines();
    assert_eq!(lines.len(), 1, "{:?}", out.stderr);
    assert!(
        lines[0].contains("not_found: no story US-9 in this feature"),
        "{:?}",
        lines[0]
    );
    assert!(posts(&server).is_empty());
}

#[test]
fn a_link_naming_its_doc_by_anything_but_an_id_is_refused_and_posts_nothing() {
    let server = mock();
    let home = logged_in(&server);
    let file = story_file(&home, "US-131");

    let out = run(
        &home,
        &["story", "add", &file, "--link", "Architecture:n1:design"],
    );

    assert_ne!(out.code, 0, "stdout: {}", out.stdout);
    assert_eq!(out.stdout, "");
    assert!(posts(&server).is_empty());
}

#[test]
fn list_prints_one_line_per_story_in_the_order_the_api_gives() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["story", "list", "--pickable"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "US-133  P0  backlog  Fix the pick order\n\
         US-131  P3  done  Stories table\n\
         US-132  P2  review  Pick rule\n"
    );
    assert_eq!(
        get_paths(&server),
        vec![format!("{}?pickable=true", stories_path())]
    );
}

#[test]
fn list_passes_the_status_filter_and_json_prints_the_api_list() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["story", "list", "--status", "review", "--json"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.json(), listed());
    assert_eq!(
        get_paths(&server),
        vec![format!("{}?status=review", stories_path())]
    );
}

#[test]
fn read_by_key_prints_text_criteria_blockers_holder_and_the_unresolved_findings() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["story", "read", "US-132"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        non_empty_lines(&out.stdout),
        vec![
            "As an agent, I want the pickable stories.",
            "[x] Lists the pickable stories",
            "[x] The board shows it. Verify in browser using dev-browser skill (pending)",
            "[ ] Typecheck/lint passes",
            "blocked by: US-131",
            "holder: run-7 review 2026-10-31T10:00:00Z",
            "[review] Order ignores priority",
        ]
    );
}

#[test]
fn read_leaves_out_the_blockers_and_holder_of_a_story_without_them() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["story", "read", "US-133"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        non_empty_lines(&out.stdout),
        vec!["Order by priority first."]
    );
}

#[test]
fn read_json_prints_the_story_itself() {
    let server = mock();
    let home = logged_in(&server);

    let out = run(&home, &["story", "read", "US-132", "--json"]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.json(), held_story());
}

/// The version every story has in `write_mock`'s list, so a sent
/// `expected_version` of 4 can only come from reading the story.
const READ_VERSION: i64 = 4;

fn written_story(id: &str, key: &str, status: &str) -> Value {
    let mut story = story_json(id, key, 3, status, "Stories table");
    story["version"] = json!(READ_VERSION);
    story
}

/// An API that lists US-131..US-133 at version 4 and answers every write
/// with `conflict`'s 409, or else 200 with US-131 at version 5 in review.
fn write_mock(conflict: Option<&'static str>) -> MockServer {
    MockServer::start(move |request| {
        let path = request.path.split('?').next().unwrap_or_default();
        if !path.starts_with(&stories_path()) {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        if request.method == "GET" {
            return Reply::json(
                200,
                &json!({ "stories": [
                    written_story(STORY_131, "US-131", "implementing"),
                    written_story(STORY_132, "US-132", "backlog"),
                    written_story(STORY_133, "US-133", "backlog")
                ] }),
            );
        }
        match conflict {
            Some(code) => Reply::json(409, &json!({ "error": code })),
            None => Reply::json(200, &json!({ "story": updated_story() })),
        }
    })
}

fn updated_story() -> Value {
    let mut story = written_story(STORY_131, "US-131", "review");
    story["version"] = json!(READ_VERSION + 1);
    story
}

/// Every request but the reads: method, path below the feature's stories,
/// and the JSON body.
fn writes(server: &MockServer) -> Vec<(String, String, Value)> {
    server
        .requests()
        .into_iter()
        .filter(|r| r.method != "GET")
        .map(|r| {
            let path = r
                .path
                .strip_prefix(&stories_path())
                .expect("a write under the feature's stories")
                .to_string();
            let body = serde_json::from_slice(&r.body).expect("the request body is JSON");
            (r.method, path, body)
        })
        .collect()
}

fn post(path: String, body: Value) -> (String, String, Value) {
    ("POST".to_string(), path, body)
}

#[test]
fn move_reads_the_story_sends_its_version_and_json_prints_the_answer() {
    let server = write_mock(None);
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "story", "move", "US-131", "--run", "r1", "--to", "review", "--json",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(get_paths(&server), vec![stories_path()]);
    assert_eq!(
        writes(&server),
        vec![post(
            format!("/{STORY_131}/move"),
            json!({ "expected_version": 4, "run": "r1", "to": "review" })
        )]
    );
    assert_eq!(out.json(), updated_story());
}

#[test]
fn a_409_of_any_kind_names_the_api_error_as_a_conflict_and_exits_3() {
    let stale = write_mock(Some("conflict"));
    let not_holder = write_mock(Some("not_holder"));

    let moved = run(
        &logged_in(&stale),
        &["story", "move", "US-131", "--run", "r1", "--to", "review"],
    );
    let ticked = run(
        &logged_in(&not_holder),
        &["story", "tick", "US-131", "--run", "r2", "--criterion", "0"],
    );

    assert_eq!(moved.code, 3, "stderr: {}", moved.stderr);
    assert_eq!(moved.stdout, "");
    assert_eq!(moved.stderr_lines().len(), 1, "stderr: {}", moved.stderr);
    assert!(
        moved
            .stderr
            .contains("conflict: conflict ; read the story again"),
        "stderr: {}",
        moved.stderr
    );
    assert_eq!(ticked.code, 3, "stderr: {}", ticked.stderr);
    assert!(
        ticked
            .stderr
            .contains("conflict: not_holder ; read the story again"),
        "stderr: {}",
        ticked.stderr
    );
    assert!(
        documented_exit_codes().contains(&3),
        "omni --help does not document exit code 3"
    );
}

#[test]
fn tick_sends_pending_only_with_pending_and_ticked_false_with_unticked() {
    let server = write_mock(None);
    let home = logged_in(&server);
    let tick = ["story", "tick", "US-131", "--run", "r1", "--criterion", "0"];

    let pending = run(&home, &[&tick[..], &["--pending"]].concat());
    let plain = run(&home, &tick);
    let unticked = run(&home, &[&tick[..], &["--unticked"]].concat());

    assert_eq!(pending.code, 0, "stderr: {}", pending.stderr);
    assert_eq!(plain.code, 0, "stderr: {}", plain.stderr);
    assert_eq!(unticked.code, 0, "stderr: {}", unticked.stderr);
    let path = format!("/{STORY_131}/tick");
    assert_eq!(
        writes(&server),
        vec![
            post(
                path.clone(),
                json!({ "expected_version": 4, "run": "r1", "criterion": 0, "ticked": true, "pending": true })
            ),
            post(
                path.clone(),
                json!({ "expected_version": 4, "run": "r1", "criterion": 0, "ticked": true })
            ),
            post(
                path,
                json!({ "expected_version": 4, "run": "r1", "criterion": 0, "ticked": false })
            ),
        ]
    );
}

#[test]
fn finding_sends_every_file_given_and_an_empty_list_without_one() {
    let server = write_mock(None);
    let home = logged_in(&server);
    let finding = [
        "story", "finding", "US-131", "--run", "r1", "--stage", "review", "--text", "x",
    ];

    let with_files = run(
        &home,
        &[&finding[..], &["--file", "a.rs", "--file", "b.rs"]].concat(),
    );
    let without = run(&home, &finding);

    assert_eq!(with_files.code, 0, "stderr: {}", with_files.stderr);
    assert_eq!(without.code, 0, "stderr: {}", without.stderr);
    let path = format!("/{STORY_131}/findings");
    assert_eq!(
        writes(&server),
        vec![
            post(
                path.clone(),
                json!({ "expected_version": 4, "run": "r1", "stage": "review", "text": "x", "files": ["a.rs", "b.rs"] })
            ),
            post(
                path,
                json!({ "expected_version": 4, "run": "r1", "stage": "review", "text": "x", "files": [] })
            ),
        ]
    );
}

#[test]
fn story_check_is_not_a_command_and_no_usage_offers_it() {
    let server = write_mock(None);
    let home = logged_in(&server);

    let check = run(
        &home,
        &[
            "story",
            "check",
            "US-131",
            "--run",
            "r1",
            "--criterion",
            "0",
        ],
    );
    let story = run(&home, &["story"]);
    let help = omni(&home.path).args(&["--help"]).run();

    assert_ne!(check.code, 0, "stdout: {}", check.stdout);
    assert_eq!(writes(&server), vec![]);
    assert!(
        !story.stderr.contains("check"),
        "omni story lists check: {}",
        story.stderr
    );
    assert!(
        !help.stdout.contains("story check"),
        "omni --help lists omni story check"
    );
}

#[test]
fn edit_resolves_blocker_keys_to_ids_and_sends_only_the_fields_given() {
    let server = write_mock(None);
    let home = logged_in(&server);

    let out = run(
        &home,
        &[
            "story",
            "edit",
            "US-133",
            "--priority",
            "1",
            "--blocked-by",
            "US-131,US-132",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        writes(&server),
        vec![(
            "PATCH".to_string(),
            format!("/{STORY_133}"),
            json!({ "expected_version": 4, "priority": 1, "blocked_by": [STORY_131, STORY_132] })
        )]
    );
}

#[test]
fn the_other_writes_post_to_their_own_routes_with_the_version_read() {
    let server = write_mock(None);
    let home = logged_in(&server);

    let outs = [
        run(&home, &["story", "cancel", "US-132"]),
        run(
            &home,
            &[
                "story",
                "claim",
                "US-131",
                "--run",
                "r1",
                "--stage",
                "implementing",
            ],
        ),
        run(&home, &["story", "release", "US-131", "--run", "r1"]),
        run(&home, &["story", "block", "US-131", "--run", "r1"]),
        run(&home, &["story", "unblock", "US-131", "--run", "r1"]),
        run(
            &home,
            &[
                "story",
                "resolve",
                "US-131",
                "--run",
                "r1",
                "--finding",
                "1",
            ],
        ),
        run(
            &home,
            &[
                "story",
                "decision",
                "US-131",
                "--run",
                "r1",
                "--question",
                "Which port?",
                "--choice",
                "8080",
                "--by",
                "human",
            ],
        ),
        run(
            &home,
            &[
                "story", "files", "US-131", "--run", "r1", "src/a.rs", "src/b.rs",
            ],
        ),
    ];

    assert_eq!(
        outs.iter().map(|o| o.code).collect::<Vec<_>>(),
        vec![0; 8],
        "stderr: {:?}",
        outs.iter().map(|o| &o.stderr).collect::<Vec<_>>()
    );
    assert_eq!(
        writes(&server),
        vec![
            post(
                format!("/{STORY_132}/cancel"),
                json!({ "expected_version": 4 })
            ),
            post(
                format!("/{STORY_131}/claim"),
                json!({ "expected_version": 4, "run": "r1", "stage": "implementing" })
            ),
            post(
                format!("/{STORY_131}/release"),
                json!({ "expected_version": 4, "run": "r1" })
            ),
            post(
                format!("/{STORY_131}/block"),
                json!({ "expected_version": 4, "run": "r1" })
            ),
            post(
                format!("/{STORY_131}/unblock"),
                json!({ "expected_version": 4, "run": "r1" })
            ),
            post(
                format!("/{STORY_131}/findings/1/resolve"),
                json!({ "expected_version": 4, "run": "r1" })
            ),
            post(
                format!("/{STORY_131}/decisions"),
                json!({ "expected_version": 4, "run": "r1", "question": "Which port?", "choice": "8080", "by": "human" })
            ),
            post(
                format!("/{STORY_131}/files"),
                json!({ "expected_version": 4, "run": "r1", "files_changed": ["src/a.rs", "src/b.rs"] })
            ),
        ]
    );
}
