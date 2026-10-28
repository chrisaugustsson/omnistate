//! US-010: the repository layout, the Docker Compose file and the make targets
//! that every later story builds on. These tests read the repository from the
//! host; tests/stack.rs covers the running stack.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn run(program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("could not run {program}: {e}"));
    assert!(
        output.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn make_dry_run(target: &str) -> String {
    run("make", &["-n", target])
}

/// The Compose file as `docker compose config` resolves it, with every profile
/// switched on so that the test container is included.
fn compose() -> Value {
    assert!(
        repo_root().join("docker-compose.yml").is_file(),
        "docker-compose.yml is missing"
    );
    let json = run(
        "docker",
        &["compose", "--profile", "*", "config", "--format", "json"],
    );
    serde_json::from_str(&json).expect("docker compose config prints JSON")
}

fn services(compose: &Value) -> &serde_json::Map<String, Value> {
    compose["services"].as_object().expect("services")
}

#[test]
fn only_the_proxy_publishes_a_port() {
    let compose = compose();
    for (name, service) in services(&compose) {
        let published = service["ports"].as_array().map_or(0, Vec::len);
        if name == "proxy" {
            assert_eq!(published, 1, "the proxy must publish exactly one port");
        } else {
            assert_eq!(published, 0, "service {name} must not publish a port");
        }
        assert_ne!(
            service["network_mode"].as_str(),
            Some("host"),
            "service {name} must stay inside the Compose network"
        );
    }
}

#[test]
fn make_lint_runs_pnpm_typecheck_and_lint() {
    let lines = make_dry_run("lint");
    let runs = |script: &str| {
        lines
            .lines()
            .any(|line| line.contains("pnpm") && line.split_whitespace().any(|w| w == script))
    };
    assert!(
        runs("typecheck"),
        "make lint does not run pnpm typecheck:\n{lines}"
    );
    assert!(runs("lint"), "make lint does not run pnpm lint:\n{lines}");
    assert!(
        lines.contains("cargo clippy"),
        "make lint stopped running clippy:\n{lines}"
    );
}

#[test]
fn make_test_runs_pnpm_tests_and_the_integration_tests_in_the_test_container() {
    let lines = make_dry_run("test");
    assert!(
        lines.contains("pnpm -r test"),
        "make test does not run pnpm -r test:\n{lines}"
    );
    assert!(
        lines.lines().any(|line| line.contains("docker compose")
            && line.split_whitespace().any(|w| w == "run")
            && line.split_whitespace().any(|w| w == "test")),
        "make test does not run the integration tests in the test container:\n{lines}"
    );
    assert!(
        lines.contains("cargo test --workspace"),
        "make test stopped running the Rust tests:\n{lines}"
    );
}
