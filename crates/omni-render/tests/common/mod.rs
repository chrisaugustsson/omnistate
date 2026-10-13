//! Helpers shared by the omni-render integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The repository root, where the Cargo workspace and the Makefile live.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root exists")
}

/// The cargo running these tests, started at the repository root.
pub fn cargo(args: &[&str]) -> Output {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("cargo can be started")
}

/// `make` at the repository root, detached from any `make` that started this test run.
pub fn make(args: &[&str]) -> Output {
    Command::new("make")
        .args(args)
        .current_dir(repo_root())
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .env_remove("MAKELEVEL")
        .output()
        .expect("make can be started")
}

/// The commands `make <target>` runs, printed without running them.
pub fn make_dry_run(target: &str) -> String {
    let output = make(&["--dry-run", "--always-make", target]);
    assert!(
        output.status.success(),
        "`make {target}` is not a working Makefile target:\n{}",
        describe(&output)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Exit status plus the tail of stdout and stderr, for failure messages.
pub fn describe(output: &Output) -> String {
    format!(
        "{}\n--- stdout (tail) ---\n{}\n--- stderr (tail) ---\n{}",
        output.status,
        tail(&output.stdout),
        tail(&output.stderr)
    )
}

fn tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(40)..].join("\n")
}
