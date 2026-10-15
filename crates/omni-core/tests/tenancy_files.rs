//! US-011: the parts of the tenancy schema that can be checked from the
//! repository alone: the migration files, the checklist in migrations/README.md
//! and the API's database login in the Compose file. tests/tenancy.rs covers
//! the schema itself against a running Postgres.

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
fn migrations_are_sqlx_migration_files() {
    let dir = repo_root().join("migrations");
    let migrations: Vec<String> = std::fs::read_dir(&dir)
        .expect("migrations/")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            let mut parts = name.splitn(2, '_');
            let version = parts.next().unwrap_or("");
            !version.is_empty()
                && version.chars().all(|c| c.is_ascii_digit())
                && parts.next().is_some_and(|rest| rest.ends_with(".sql"))
        })
        .collect();
    assert!(
        !migrations.is_empty(),
        "migrations/ needs sqlx migration files named <version>_<description>.sql"
    );
    let sql: String = migrations
        .iter()
        .map(|m| std::fs::read_to_string(dir.join(m)).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    assert!(
        sql.contains("row level security"),
        "the migrations never enable row-level security"
    );
}

#[test]
fn the_readme_states_the_rules_for_every_future_table_as_a_checklist() {
    let path = repo_root().join("migrations/README.md");
    let readme = std::fs::read_to_string(&path).expect("migrations/README.md");
    let items: Vec<String> = readme
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("- [ ]") || l.starts_with("* [ ]"))
        .map(|l| l.to_lowercase())
        .collect();
    assert!(
        items.len() >= 6,
        "migrations/README.md needs a checklist of `- [ ]` items, found {}",
        items.len()
    );
    let has = |needles: &[&str]| {
        items
            .iter()
            .any(|item| needles.iter().all(|n| item.contains(&n.to_lowercase())))
    };
    let rules: [(&str, &[&str]); 9] = [
        ("workspace_id NOT NULL", &["workspace_id", "not null"]),
        (
            "unique key on (workspace_id, id)",
            &["unique", "(workspace_id, id)"],
        ),
        (
            "composite foreign keys with workspace_id",
            &["foreign key", "workspace_id"],
        ),
        ("UUIDv7 record IDs", &["uuidv7"]),
        ("RLS enabled and forced", &["row-level security", "forced"]),
        (
            "the policy expression",
            &["NULLIF(current_setting('app.workspace_id', true), '')::uuid"],
        ),
        (
            "no UPDATE privilege on append-only tables",
            &["no update privilege", "append-only"],
        ),
        (
            "no DELETE privilege on tenant tables",
            &["no delete privilege"],
        ),
        ("SECURITY DEFINER lookups only", &["security definer"]),
    ];
    for (name, needles) in rules {
        assert!(
            has(needles),
            "migrations/README.md has no checklist item for: {name} (looked for {needles:?})"
        );
    }
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
