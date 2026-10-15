//! US-013: the comparison of an API key's hash with the stored one is made by
//! the `subtle` crate, whose `ConstantTimeEq` is the audited constant-time
//! comparison, and not by a loop of the project's own. That is a property of
//! the source, so these tests read it: the crate is a dependency of `omni-api`
//! and `src/api_keys.rs` compares the hashes with `ct_eq`. What the comparison
//! answers, mismatched lengths included, is pinned by the unit tests at the end
//! of `src/api_keys.rs`. They need no database, so they run everywhere.

use std::fs;
use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn subtle_is_a_dependency_of_the_api() {
    let manifest = read("Cargo.toml");
    let dependencies: Vec<&str> = manifest
        .lines()
        .skip_while(|line| line.trim() != "[dependencies]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .collect();
    assert!(
        dependencies
            .iter()
            .any(|line| line.trim_start().starts_with("subtle")),
        "Cargo.toml lists `subtle` under [dependencies]: {dependencies:?}"
    );
}

#[test]
fn the_key_hash_is_compared_with_subtle_constant_time_eq() {
    let source = read("src/api_keys.rs");
    let code: String = source
        .lines()
        .take_while(|line| !line.starts_with("#[cfg(test)]"))
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("subtle::ConstantTimeEq") || code.contains("subtle::{"),
        "src/api_keys.rs imports subtle's ConstantTimeEq"
    );
    assert!(
        code.contains(".ct_eq("),
        "src/api_keys.rs compares the hashes with `.ct_eq(`"
    );
    assert!(
        !code.contains(".fold(0u8"),
        "src/api_keys.rs no longer has its own xor-and-or loop"
    );
}
