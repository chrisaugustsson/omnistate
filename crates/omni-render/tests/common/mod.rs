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

/// The WebAssembly renderer that `make render-wasm` built: render-sandbox/build/.
/// The tests never build it themselves, since a cargo started inside `cargo test`
/// deadlocks with any other cargo on the machine, such as a story worktree's;
/// the fast check runs `make render-wasm` before `cargo test`.
pub fn render_wasm() -> PathBuf {
    let build = repo_root().join("render-sandbox/build");
    for file in ["omni_render.js", "omni_render.wasm"] {
        assert!(
            build.join(file).is_file(),
            "render-sandbox/build/{file} is missing: run `make render-wasm` before these tests \
             (they never build it, since a cargo inside `cargo test` deadlocks with other cargos)"
        );
    }
    build
}

/// The `stress-file` binary that cargo built next to this test binary;
/// `cargo test --workspace` builds omni-core's binaries before it runs any test.
pub fn stress_file_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("the test binary has a path");
    let bin = exe
        .parent()
        .and_then(Path::parent)
        .expect("the test binary is in target/<profile>/deps")
        .join("stress-file");
    assert!(
        bin.is_file(),
        "{} is missing: run `cargo build -p omni-core --bin stress-file` before these tests \
         (they never run cargo themselves, since a cargo inside `cargo test` deadlocks with other cargos)",
        bin.display()
    );
    bin
}

/// `make stress-file <variables>` with the already-built binary, so that no cargo runs inside the tests.
pub fn make_stress_file(variables: &[&str]) -> Output {
    let bin = stress_file_bin();
    let stress_file = format!("STRESS_FILE={}", bin.display());
    let mut args = vec!["stress-file", stress_file.as_str()];
    args.extend_from_slice(variables);
    make(&args)
}
