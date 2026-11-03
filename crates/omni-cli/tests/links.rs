//! `omni link`: the folder's link in `links.toml`, tested as the built binary
//! against a mock HTTP server that lists two projects.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const OTHER_PROJECT: &str = "01900000-0000-7000-8000-0000000000b2";
const UNKNOWN_PROJECT: &str = "01900000-0000-7000-8000-0000000000b9";

fn mock() -> MockServer {
    MockServer::start(|request| {
        if request.method == "GET" && request.path == format!("/api/v1/w/{WORKSPACE}/projects") {
            return Reply::json(
                200,
                &json!({ "projects": [
                    { "id": PROJECT, "name": "brand-site" },
                    { "id": OTHER_PROJECT, "name": "other" }
                ] }),
            );
        }
        Reply::json(404, &json!({ "error": "not_found" }))
    })
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

fn link_in(home: &Scratch, folder: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", "run-7/author"])
        .cwd(&folder.path)
        .run()
}

fn canonical(folder: &Scratch) -> String {
    std::fs::canonicalize(&folder.path)
        .unwrap()
        .display()
        .to_string()
}

fn links_text(home: &Scratch) -> String {
    std::fs::read_to_string(links_path(&home.path)).expect("links.toml exists")
}

#[test]
fn link_writes_the_canonical_path_and_project_id_under_the_workspace() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    let path = canonical(&folder);

    let out = link_in(&home, &folder, &["link", "brand-site"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.stdout,
        format!("linked {path} to brand-site ({PROJECT})\n")
    );
    assert_eq!(
        links_text(&home),
        format!("[{WORKSPACE}]\n\"{path}\" = \"{PROJECT}\"\n")
    );

    let by_id = Scratch::new("by-id");
    let out = link_in(&home, &by_id, &["link", &PROJECT.to_uppercase()]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains(&format!("to brand-site ({PROJECT})")),
        "{}",
        out.stdout
    );
}

#[test]
fn link_again_replaces_the_line_and_keeps_other_folders() {
    let server = mock();
    let home = logged_in(&server.url);
    let first = Scratch::new("first");
    let second = Scratch::new("second");
    assert_eq!(link_in(&home, &first, &["link", "brand-site"]).code, 0);
    assert_eq!(link_in(&home, &second, &["link", "brand-site"]).code, 0);
    assert_eq!(link_in(&home, &first, &["link", "other"]).code, 0);

    assert_eq!(
        links_text(&home),
        format!(
            "[{WORKSPACE}]\n\"{}\" = \"{OTHER_PROJECT}\"\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&first),
            canonical(&second)
        )
    );
}

#[test]
fn link_keeps_the_links_of_another_workspace() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    let elsewhere = "01900000-0000-7000-8000-0000000000cc";
    let path = links_path(&home.path);
    std::fs::write(
        &path,
        format!("[{elsewhere}]\n\"/x/y\" = \"{OTHER_PROJECT}\"\n"),
    )
    .unwrap();

    assert_eq!(link_in(&home, &folder, &["link", "brand-site"]).code, 0);
    assert_eq!(
        links_text(&home),
        format!(
            "[{elsewhere}]\n\"/x/y\" = \"{OTHER_PROJECT}\"\n\n[{WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&folder)
        )
    );
}

#[test]
fn link_to_an_unknown_project_fails_and_writes_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    for given in ["nope", UNKNOWN_PROJECT] {
        let out = link_in(&home, &folder, &["link", given]);
        assert_eq!(out.code, 1);
        assert_eq!(out.stdout, "");
        assert_eq!(
            out.stderr_lines(),
            [format!(
                "no project named {given:?}; `omni project list` shows the projects"
            )]
        );
    }
    assert!(!links_path(&home.path).exists());
}

#[test]
fn link_without_a_project_is_a_usage_error() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    let out = link_in(&home, &folder, &["link"]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr_lines(),
        ["omni: a link needs the project: omni link <project name|id>"]
    );
}

#[test]
fn link_with_json_prints_the_path_workspace_and_project() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    let out = link_in(&home, &folder, &["link", "brand-site", "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let expected: Value = json!({
        "path": canonical(&folder),
        "workspace_id": WORKSPACE,
        "project": { "id": PROJECT, "name": "brand-site" }
    });
    assert_eq!(out.json(), expected);
}
