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
        if request.method == "GET"
            && (request.path == format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/features")
                || request.path
                    == format!("/api/v1/w/{WORKSPACE}/projects/{OTHER_PROJECT}/features"))
        {
            return Reply::json(200, &json!({ "features": [] }));
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
    assert_eq!(
        links_text(&home),
        format!(
            "[{WORKSPACE}]\n\"{path}\" = \"{PROJECT}\"\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&by_id)
        )
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
fn link_again_in_a_folder_whose_path_holds_a_dot_replaces_its_line() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("brand.site");
    assert_eq!(link_in(&home, &folder, &["link", "brand-site"]).code, 0);
    assert_eq!(link_in(&home, &folder, &["link", "other"]).code, 0);

    assert_eq!(
        links_text(&home),
        format!(
            "[{WORKSPACE}]\n\"{}\" = \"{OTHER_PROJECT}\"\n",
            canonical(&folder)
        )
    );
}

#[test]
fn link_to_an_unknown_project_fails_and_writes_nothing() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    for given in ["nope", &UNKNOWN_PROJECT.to_uppercase()] {
        let out = link_in(&home, &folder, &["link", given]);
        assert_eq!(out.code, 1);
        assert_eq!(out.stdout, "");
        assert_eq!(
            out.stderr_lines(),
            [format!(
                "omni: no project named {given:?}; `omni project list` shows the projects"
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

const OTHER_WORKSPACE: &str = "01900000-0000-7000-8000-0000000000cc";

fn feature_list_in(home: &Scratch, folder: &std::path::Path, extra: &[&str]) -> Out {
    omni(&home.path)
        .args(&["feature", "list", "--as", "run-7/author"])
        .args(extra)
        .cwd(folder)
        .run()
}

fn feature_requests(server: &MockServer) -> Vec<String> {
    server
        .requests()
        .iter()
        .map(|r| format!("{} {}", r.method, r.path))
        .collect()
}

#[test]
fn a_linked_folder_and_the_folders_under_it_find_their_project() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    std::fs::create_dir_all(folder.join("a/b")).unwrap();
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&folder)
        ),
    )
    .unwrap();

    for dir in [folder.path.clone(), folder.join("a/b")] {
        let out = feature_list_in(&home, &dir, &[]);
        assert_eq!(out.code, 0, "{}", out.stderr);
    }
    let want = format!("GET /api/v1/w/{WORKSPACE}/projects/{PROJECT}/features");
    assert_eq!(feature_requests(&server), vec![want.clone(), want]);
}

#[test]
fn an_unlinked_folder_is_an_error_whatever_its_omni_toml_says() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    std::fs::write(folder.join(".omni.toml"), "project = \"brand-site\"\n").unwrap();

    let out = feature_list_in(&home, &folder.path, &[]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr.trim_end(),
        "omni: this folder is not linked to a project: run omni link <project name|id> here, or pass --project"
    );
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

#[test]
fn a_link_of_another_workspace_is_reported_as_such() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{OTHER_WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&folder)
        ),
    )
    .unwrap();

    let out = feature_list_in(&home, &folder.join("sub"), &[]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr.trim_end(),
        format!(
            "omni: {} is linked to a project of another workspace ({OTHER_WORKSPACE}); run omni link here, or omni login",
            canonical(&folder)
        )
    );
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

#[test]
fn a_link_of_the_logged_in_workspace_beats_a_nearer_link_of_another() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{OTHER_WORKSPACE}]\n\"{}\" = \"{OTHER_PROJECT}\"\n\n[{WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            format_args!("{}/sub", canonical(&folder)),
            canonical(&folder)
        ),
    )
    .unwrap();

    let out = feature_list_in(&home, &folder.join("sub"), &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        feature_requests(&server),
        vec![format!(
            "GET /api/v1/w/{WORKSPACE}/projects/{PROJECT}/features"
        )]
    );
}

#[test]
fn project_flag_wins_over_the_folders_link() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&folder)
        ),
    )
    .unwrap();

    let out = feature_list_in(&home, &folder.path, &["--project", "other"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        feature_requests(&server),
        vec![
            format!("GET /api/v1/w/{WORKSPACE}/projects"),
            format!("GET /api/v1/w/{WORKSPACE}/projects/{OTHER_PROJECT}/features")
        ]
    );
}
