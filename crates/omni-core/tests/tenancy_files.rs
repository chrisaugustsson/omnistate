//! US-011: the part of the tenancy schema that can be checked from the
//! repository alone: the API's database login in the Compose file.
//! tests/tenancy.rs covers the schema itself against a running Postgres.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

#[test]
fn the_api_service_connects_to_postgres_as_the_non_owner_api_role() {
    let output = Command::new("docker")
        .args(["compose", "--profile", "*", "config", "--format", "json"])
        .current_dir(repo_root())
        .output()
        .expect("docker compose config");
    assert!(
        output.status.success(),
        "docker compose config failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let compose: Value = serde_json::from_slice(&output.stdout).expect("compose JSON");
    let url = compose["services"]["api"]["environment"]["OMNI_DATABASE_URL"]
        .as_str()
        .expect("the api service's OMNI_DATABASE_URL");
    let credentials = url
        .strip_prefix("postgres://")
        .and_then(|rest| rest.split('@').next())
        .unwrap_or_else(|| panic!("OMNI_DATABASE_URL is not a postgres:// URL: {url}"));
    let user = credentials.split(':').next().unwrap();
    assert_eq!(
        user, "omni_api",
        "the API must connect as omni_api, not as the table owner: {url}"
    );
}
