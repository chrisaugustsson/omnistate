//! US-001: crates/omni-render/README.md pins the toolchain for a clean build on macOS.

mod common;

use std::fs;

/// Exact `major.minor.patch` versions written in `line`.
fn exact_versions(line: &str) -> Vec<String> {
    line.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .map(|token| token.trim_matches('.'))
        .filter(|token| {
            let parts: Vec<&str> = token.split('.').collect();
            parts.len() == 3 && parts.iter().all(|part| !part.is_empty())
        })
        .map(str::to_string)
        .collect()
}

/// Exact versions on the README lines that mention any of `names`.
fn versions_near(readme: &str, names: &[&str]) -> Vec<String> {
    readme
        .lines()
        .filter(|line| {
            let line = line.to_lowercase();
            names.iter().any(|name| line.contains(name))
        })
        .flat_map(exact_versions)
        .collect()
}

/// The version Cargo.lock pins for `package`.
fn locked_version(lock: &str, package: &str) -> Option<String> {
    let name_line = format!("name = \"{package}\"");
    let mut lines = lock.lines().skip_while(|line| line.trim() != name_line);
    lines.next()?;
    let version = lines.next()?.trim().strip_prefix("version = \"")?;
    Some(version.trim_end_matches('"').to_string())
}

#[test]
fn readme_lists_the_exact_toolchain_for_building_on_macos() {
    let root = common::repo_root();
    let readme_path = root.join("crates/omni-render/README.md");
    let readme = fs::read_to_string(&readme_path)
        .unwrap_or_else(|error| panic!("cannot read {readme_path:?}: {error}"));
    let lower = readme.to_lowercase();

    assert!(
        lower.contains("macos"),
        "the README must explain the build on macOS"
    );
    assert!(
        readme.contains("make render-wasm"),
        "the README must show how to build with `make render-wasm`"
    );
    assert!(
        readme.contains("wasm32-unknown-emscripten"),
        "the README must name the wasm32-unknown-emscripten target"
    );

    let rust: Vec<String> = versions_near(&readme, &["rust"])
        .into_iter()
        .filter(|version| version.starts_with("1."))
        .collect();
    assert!(
        !rust.is_empty(),
        "the README must list the exact Rust version (1.x.y)"
    );

    let emscripten: Vec<String> = versions_near(&readme, &["emscripten", "emsdk", "emcc"])
        .into_iter()
        .filter(|version| !rust.contains(version))
        .collect();
    assert!(
        !emscripten.is_empty(),
        "the README must list the exact Emscripten (emsdk) version (x.y.z)"
    );

    assert!(
        lower.contains("skia") && lower.contains("binaries"),
        "the README must cover the Skia binaries"
    );
    let lock_path = root.join("Cargo.lock");
    let lock = fs::read_to_string(&lock_path)
        .unwrap_or_else(|error| panic!("cannot read {lock_path:?}: {error}"));
    for package in ["skia-safe", "skia-bindings"] {
        let version = locked_version(&lock, package)
            .unwrap_or_else(|| panic!("Cargo.lock does not pin {package}"));
        assert!(
            readme.contains(&version),
            "the README must list the {package} version Cargo.lock pins ({version}), which selects the prebuilt Skia binaries"
        );
    }
}
