//! US-001: the Cargo workspace, its first two crates and the Makefile entry points.

mod common;

use serde_json::Value;
use std::path::{Path, PathBuf};

fn metadata() -> Value {
    let output = common::cargo(&["metadata", "--no-deps", "--format-version", "1"]);
    assert!(
        output.status.success(),
        "`cargo metadata` failed at the repository root:\n{}",
        common::describe(&output)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata prints JSON")
}

fn workspace_members(metadata: &Value) -> Vec<&Value> {
    let ids = metadata["workspace_members"]
        .as_array()
        .expect("cargo metadata lists workspace_members");
    metadata["packages"]
        .as_array()
        .expect("cargo metadata lists packages")
        .iter()
        .filter(|package| ids.contains(&package["id"]))
        .collect()
}

fn canonical(path: &str) -> PathBuf {
    Path::new(path)
        .canonicalize()
        .unwrap_or_else(|error| panic!("{path} does not exist: {error}"))
}

#[test]
fn workspace_at_the_repo_root_contains_omni_core_and_omni_render() {
    let root = common::repo_root();
    let metadata = metadata();
    let workspace_root = metadata["workspace_root"]
        .as_str()
        .expect("cargo metadata has a workspace_root");
    assert_eq!(
        canonical(workspace_root),
        root,
        "the Cargo workspace must be rooted at the repository root"
    );

    let members = workspace_members(&metadata);
    let names: Vec<&str> = members
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect();
    for crate_name in ["omni-core", "omni-render"] {
        let package = members
            .iter()
            .find(|package| package["name"] == crate_name)
            .unwrap_or_else(|| {
                panic!("the workspace members {names:?} do not include `{crate_name}`")
            });
        let manifest = package["manifest_path"]
            .as_str()
            .expect("every package has a manifest_path");
        assert_eq!(
            canonical(manifest),
            root.join("crates").join(crate_name).join("Cargo.toml"),
            "`{crate_name}` must live in crates/{crate_name}"
        );
    }
}

#[test]
fn omni_render_depends_on_skia_safe() {
    let metadata = metadata();
    let omni_render = workspace_members(&metadata)
        .into_iter()
        .find(|package| package["name"] == "omni-render")
        .expect("omni-render is a workspace member");
    let dependencies = omni_render["dependencies"]
        .as_array()
        .expect("every package lists its dependencies");
    let declared: Vec<String> = dependencies
        .iter()
        .map(|dependency| format!("{} ({})", dependency["name"], dependency["kind"]))
        .collect();
    assert!(
        dependencies
            .iter()
            .any(|dependency| dependency["name"] == "skia-safe" && dependency["kind"].is_null()),
        "omni-render must draw with Skia through a normal dependency on the skia-safe crate; it declares {declared:?}"
    );
}

#[test]
fn makefile_lint_and_test_targets_run_the_specified_cargo_commands() {
    let lint = common::make_dry_run("lint");
    assert_runs("lint", &lint, "cargo fmt --check");
    assert_runs(
        "lint",
        &lint,
        "cargo clippy --workspace --all-targets -- -D warnings",
    );

    let test = common::make_dry_run("test");
    assert_runs("test", &test, "cargo test --workspace");
}

fn assert_runs(target: &str, commands: &str, expected: &str) {
    let runs_it = commands
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .any(|line| line.contains(expected));
    assert!(
        runs_it,
        "`make {target}` must run `{expected}`; it runs:\n{commands}"
    );
}
