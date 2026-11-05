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
        if request.method == "GET"
            && request
                .path
                .starts_with(&format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}/docs"))
        {
            return Reply::json(200, &json!({ "docs": [] }));
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

#[test]
fn the_nearest_link_of_the_logged_in_workspace_wins() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{WORKSPACE}]\n\"{}\" = \"{OTHER_PROJECT}\"\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&folder),
            format_args!("{}/sub", canonical(&folder))
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
fn a_path_linked_in_both_workspaces_finds_the_logged_in_project() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{OTHER_WORKSPACE}]\n\"{0}\" = \"{OTHER_PROJECT}\"\n\n[{WORKSPACE}]\n\"{0}\" = \"{PROJECT}\"\n",
            canonical(&folder)
        ),
    )
    .unwrap();

    let out = feature_list_in(&home, &folder.path, &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        feature_requests(&server),
        vec![format!(
            "GET /api/v1/w/{WORKSPACE}/projects/{PROJECT}/features"
        )]
    );
}

#[test]
fn the_nearest_link_of_another_workspace_is_the_one_named() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    let third_workspace = "01900000-0000-7000-8000-0000000000dd";
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{third_workspace}]\n\"{}\" = \"{PROJECT}\"\n\n[{OTHER_WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&folder),
            format_args!("{}/sub", canonical(&folder))
        ),
    )
    .unwrap();

    let out = feature_list_in(&home, &folder.join("sub"), &[]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr.trim_end(),
        format!(
            "omni: {}/sub is linked to a project of another workspace ({OTHER_WORKSPACE}); run omni link here, or omni login",
            canonical(&folder)
        )
    );
}

#[test]
fn a_folder_whose_name_extends_a_linked_folders_name_is_not_linked() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");
    std::fs::create_dir_all(folder.join("app")).unwrap();
    std::fs::create_dir_all(folder.join("app-old")).unwrap();
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{WORKSPACE}]\n\"{}/app\" = \"{PROJECT}\"\n",
            canonical(&folder)
        ),
    )
    .unwrap();

    let out = feature_list_in(&home, &folder.join("app-old"), &[]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr.trim_end(),
        "omni: this folder is not linked to a project: run omni link <project name|id> here, or pass --project"
    );
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

#[test]
fn config_show_names_the_project_the_folder_is_linked_to() {
    let home = logged_in("http://127.0.0.1:9");
    let folder = Scratch::new("folder");
    let elsewhere = Scratch::new("elsewhere");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&folder)
        ),
    )
    .unwrap();

    let show_in = |dir: &std::path::Path| {
        let out = omni(&home.path)
            .args(&["config", "show", "--json"])
            .cwd(dir)
            .run();
        assert_eq!(out.code, 0, "{}", out.stderr);
        out.json()["project"].clone()
    };
    assert_eq!(show_in(&folder.join("sub")), json!(PROJECT));
    assert_eq!(show_in(&elsewhere.path), Value::Null);

    let text = |dir: &std::path::Path| {
        let out = omni(&home.path).args(&["config", "show"]).cwd(dir).run();
        assert_eq!(out.code, 0, "{}", out.stderr);
        out.stdout
    };
    let links_line = format!("links_file: {}", links_path(&home.path).display());
    let linked = text(&folder.path);
    assert!(linked.contains(&format!("project: {PROJECT}")), "{linked}");
    assert!(linked.contains(&links_line), "{linked}");
    assert!(text(&elsewhere.path).contains("project: (none)"));

    std::fs::write(links_path(&home.path), "not [valid").unwrap();
    assert!(text(&folder.path).contains("project: (none)"));
}

#[test]
fn config_show_in_a_folder_linked_only_in_another_workspace_reports_no_project() {
    let home = logged_in("http://127.0.0.1:9");
    let folder = Scratch::new("folder");
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{OTHER_WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&folder)
        ),
    )
    .unwrap();

    let out = omni(&home.path)
        .args(&["config", "show", "--json"])
        .cwd(&folder.path)
        .run();
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json().get("project"), Some(&Value::Null));
}

#[test]
fn config_show_logged_out_in_a_linked_folder_reports_no_project() {
    let home = Scratch::new("home");
    let folder = Scratch::new("folder");
    std::fs::create_dir_all(links_path(&home.path).parent().unwrap()).unwrap();
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical(&folder)
        ),
    )
    .unwrap();

    let out = omni(&home.path)
        .args(&["config", "show", "--json"])
        .cwd(&folder.path)
        .run();
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.json().get("project"), Some(&Value::Null));
    assert_eq!(out.json()["logged_in"], json!(false));
}

const FEATURE: &str = "01900000-0000-7000-8000-0000000000f1";

fn linked_home(server: &MockServer, folder: &Scratch) -> Scratch {
    let home = logged_in(&server.url);
    std::fs::write(
        links_path(&home.path),
        format!("[{WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n", canonical(folder)),
    )
    .unwrap();
    home
}

fn doc_list_in(home: &Scratch, folder: &Scratch, env: &[(&str, &str)], extra: &[&str]) -> Out {
    let mut run = omni(&home.path)
        .args(&["doc", "list", "--as", "run-7/author"])
        .args(extra)
        .cwd(&folder.path);
    for (name, value) in env {
        run = run.env(name, value);
    }
    run.run()
}

#[test]
fn omni_project_names_the_project_in_an_unlinked_folder_and_the_flag_beats_it() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");

    let by_env = omni(&home.path)
        .args(&["feature", "list", "--as", "run-7/author"])
        .env("OMNI_PROJECT", "brand-site")
        .cwd(&folder.path)
        .run();
    assert_eq!(by_env.code, 0, "{}", by_env.stderr);
    let by_flag = omni(&home.path)
        .args(&[
            "feature",
            "list",
            "--as",
            "run-7/author",
            "--project",
            "other",
        ])
        .env("OMNI_PROJECT", "brand-site")
        .cwd(&folder.path)
        .run();
    assert_eq!(by_flag.code, 0, "{}", by_flag.stderr);
    let list = format!("GET /api/v1/w/{WORKSPACE}/projects");
    assert_eq!(
        feature_requests(&server),
        vec![
            list.clone(),
            format!("{list}/{PROJECT}/features"),
            list.clone(),
            format!("{list}/{OTHER_PROJECT}/features"),
        ]
    );

    let empty = omni(&home.path)
        .args(&["feature", "list", "--as", "run-7/author"])
        .env("OMNI_PROJECT", "")
        .cwd(&folder.path)
        .run();
    assert_eq!(empty.code, 1);
    assert!(empty.stderr.contains("not linked"), "{}", empty.stderr);
}

#[test]
fn omni_feature_is_the_feature_flag_and_the_omni_toml_feature_is_not_read() {
    let server = mock();
    let folder = Scratch::new("folder");
    let home = linked_home(&server, &folder);
    std::fs::write(folder.join(".omni.toml"), "feature = \"y\"\n").unwrap();

    let by_env = doc_list_in(&home, &folder, &[("OMNI_FEATURE", FEATURE)], &[]);
    assert_eq!(by_env.code, 0, "{}", by_env.stderr);
    let by_flag = doc_list_in(&home, &folder, &[], &["--feature", FEATURE]);
    assert_eq!(by_flag.code, 0, "{}", by_flag.stderr);
    let neither = doc_list_in(&home, &folder, &[], &[]);
    assert_eq!(neither.code, 0, "{}", neither.stderr);

    let docs = format!("GET /api/v1/w/{WORKSPACE}/projects/{PROJECT}/docs");
    assert_eq!(
        feature_requests(&server),
        vec![
            format!("{docs}?feature={FEATURE}"),
            format!("{docs}?feature={FEATURE}"),
            docs,
        ]
    );
}

#[test]
fn global_beats_omni_feature() {
    let server = mock();
    let folder = Scratch::new("folder");
    let home = linked_home(&server, &folder);

    let out = doc_list_in(&home, &folder, &[("OMNI_FEATURE", FEATURE)], &["--global"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        feature_requests(&server),
        vec![format!(
            "GET /api/v1/w/{WORKSPACE}/projects/{PROJECT}/docs?feature=global"
        )]
    );
}

#[test]
fn a_whitespace_only_omni_feature_counts_as_unset() {
    let server = mock();
    let folder = Scratch::new("folder");
    let home = linked_home(&server, &folder);

    let out = doc_list_in(&home, &folder, &[("OMNI_FEATURE", "  ")], &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        feature_requests(&server),
        vec![format!("GET /api/v1/w/{WORKSPACE}/projects/{PROJECT}/docs")]
    );
}

#[test]
fn an_explicit_feature_with_global_is_refused() {
    let server = mock();
    let folder = Scratch::new("folder");
    let home = linked_home(&server, &folder);

    let out = doc_list_in(&home, &folder, &[], &["--feature", FEATURE, "--global"]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr.trim_end(),
        "omni: give --feature or --global, not both"
    );
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

#[test]
fn leftover_keys_in_omni_toml_are_warned_about_once_and_checks_stays_quiet() {
    let server = mock();
    let folder = Scratch::new("folder");
    let home = linked_home(&server, &folder);
    let warning = |keys: &str| {
        format!(
            ".omni.toml sets {keys}, which omni no longer reads; link this folder with omni link <project> and pass --feature or OMNI_FEATURE"
        )
    };

    std::fs::write(
        folder.join(".omni.toml"),
        "feature = \"y\"\nproject = \"x\"\n[checks]\nfast = \"a\"\nstack_up = \"b\"\nstack = \"c\"\n",
    )
    .unwrap();
    let both = doc_list_in(&home, &folder, &[("OMNI_FEATURE", FEATURE)], &[]);
    assert_eq!(both.code, 0, "{}", both.stderr);
    assert_eq!(both.stderr.trim_end(), warning("project, feature"));

    let checks = omni(&home.path).args(&["checks"]).cwd(&folder.path).run();
    assert_eq!(checks.code, 0, "{}", checks.stderr);
    assert_eq!(checks.stderr, "");

    std::fs::write(folder.join(".omni.toml"), "feature = \"y\"\n").unwrap();
    let only = doc_list_in(&home, &folder, &[], &[]);
    assert_eq!(only.stderr.trim_end(), warning("feature"));

    std::fs::write(folder.join(".omni.toml"), "[checks]\nfast = \"a\"\n").unwrap();
    let none = doc_list_in(&home, &folder, &[], &[]);
    assert_eq!(none.stderr, "");
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repository with one commit and a worktree of it outside the repository.
fn repo_with_worktree() -> (Scratch, Scratch) {
    let repo = Scratch::new("repo");
    git(&repo.path, &["init", "-q"]);
    git(
        &repo.path,
        &["commit", "-q", "--allow-empty", "-m", "first"],
    );
    let worktrees = Scratch::new("worktrees");
    git(
        &repo.path,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt",
            worktrees.join("w").to_str().unwrap(),
        ],
    );
    (repo, worktrees)
}

fn link_file(home: &Scratch, folder: &std::path::Path, project: &str) {
    let path = links_path(&home.path);
    let mut text = std::fs::read_to_string(&path).unwrap_or_else(|_| format!("[{WORKSPACE}]\n"));
    text.push_str(&format!(
        "\"{}\" = \"{project}\"\n",
        std::fs::canonicalize(folder).unwrap().display()
    ));
    std::fs::write(path, text).unwrap();
}

#[test]
fn a_git_worktree_of_a_linked_repository_finds_the_repositorys_project() {
    let server = mock();
    let home = logged_in(&server.url);
    let (repo, worktrees) = repo_with_worktree();
    let worktree = worktrees.join("w");
    std::fs::create_dir_all(worktree.join("sub")).unwrap();
    link_file(&home, &repo.path, PROJECT);

    for dir in [worktree.clone(), worktree.join("sub")] {
        let out = feature_list_in(&home, &dir, &[]);
        assert_eq!(out.code, 0, "{}", out.stderr);
    }
    let want = format!("GET /api/v1/w/{WORKSPACE}/projects/{PROJECT}/features");
    assert_eq!(feature_requests(&server), vec![want.clone(), want]);
}

#[test]
fn a_link_of_the_worktree_itself_beats_the_repositorys() {
    let server = mock();
    let home = logged_in(&server.url);
    let (repo, worktrees) = repo_with_worktree();
    let worktree = worktrees.join("w");
    link_file(&home, &repo.path, PROJECT);
    link_file(&home, &worktree, OTHER_PROJECT);

    let out = feature_list_in(&home, &worktree, &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        feature_requests(&server),
        vec![format!(
            "GET /api/v1/w/{WORKSPACE}/projects/{OTHER_PROJECT}/features"
        )]
    );
}

#[test]
fn without_git_an_unlinked_folder_gets_the_not_linked_error_alone() {
    let server = mock();
    let home = logged_in(&server.url);
    let folder = Scratch::new("folder");

    let out = omni(&home.path)
        .args(&["feature", "list", "--as", "run-7/author"])
        .env("PATH", "")
        .cwd(&folder.path)
        .run();
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr.trim_end(),
        "omni: this folder is not linked to a project: run omni link <project name|id> here, or pass --project"
    );
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

fn canonical_path(folder: &std::path::Path) -> String {
    std::fs::canonicalize(folder).unwrap().display().to_string()
}

#[test]
fn the_repositorys_link_in_the_logged_in_workspace_beats_the_worktrees_link_of_another() {
    let server = mock();
    let home = logged_in(&server.url);
    let (repo, worktrees) = repo_with_worktree();
    let worktree = worktrees.join("w");
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{OTHER_WORKSPACE}]\n\"{}\" = \"{OTHER_PROJECT}\"\n\n[{WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical_path(&worktree),
            canonical_path(&repo.path)
        ),
    )
    .unwrap();

    let out = feature_list_in(&home, &worktree, &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        feature_requests(&server),
        vec![format!(
            "GET /api/v1/w/{WORKSPACE}/projects/{PROJECT}/features"
        )]
    );
}

#[test]
fn with_only_links_of_other_workspaces_the_worktrees_own_is_named_over_the_repositorys() {
    let server = mock();
    let home = logged_in(&server.url);
    let (repo, worktrees) = repo_with_worktree();
    let worktree = worktrees.join("w");
    let third_workspace = "01900000-0000-7000-8000-0000000000dd";
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{third_workspace}]\n\"{}\" = \"{PROJECT}\"\n\n[{OTHER_WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical_path(&repo.path),
            canonical_path(&worktree)
        ),
    )
    .unwrap();

    let out = feature_list_in(&home, &worktree, &[]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr.trim_end(),
        format!(
            "omni: {} is linked to a project of another workspace ({OTHER_WORKSPACE}); run omni link here, or omni login",
            canonical_path(&worktree)
        )
    );
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

#[test]
fn an_unlinked_worktree_of_a_repository_linked_in_another_workspace_names_the_repository() {
    let server = mock();
    let home = logged_in(&server.url);
    let (repo, worktrees) = repo_with_worktree();
    let worktree = worktrees.join("w");
    std::fs::write(
        links_path(&home.path),
        format!(
            "[{OTHER_WORKSPACE}]\n\"{}\" = \"{PROJECT}\"\n",
            canonical_path(&repo.path)
        ),
    )
    .unwrap();

    let out = feature_list_in(&home, &worktree, &[]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr.trim_end(),
        format!(
            "omni: {} is linked to a project of another workspace ({OTHER_WORKSPACE}); run omni link here, or omni login",
            canonical_path(&repo.path)
        )
    );
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}
