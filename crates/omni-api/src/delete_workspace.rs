//! `omni-api delete-workspace <workspace_id>`: the admin command that erases a
//! workspace for good, every row in every tenant table and its whole folder in
//! RustFS. It is the only way anything is deleted: the API role `omni_api` has
//! no DELETE privilege and no route deletes. Like `omni-api setup`, it runs as
//! the database owner (`OMNI_ADMIN_DATABASE_URL`), never as the API role.

use crate::{credentials, region};
use s3::Bucket;
use sqlx::{Connection, PgConnection};
use std::collections::{HashMap, HashSet};

/// How many objects are deleted at once.
const DELETE_BATCH: usize = 32;
/// How often the folder is listed again to catch objects a listing missed.
const FOLDER_PASSES: usize = 5;

/// The settings the command needs: the owner's login and the storage. The API
/// role's login and password are not among them.
pub struct AdminConfig {
    pub admin_database_url: String,
    pub s3_endpoint: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_bucket: String,
}

impl AdminConfig {
    pub fn from_env() -> Result<Self, String> {
        let required = |name: &str| std::env::var(name).map_err(|_| format!("{name} is not set"));
        Ok(Self {
            admin_database_url: required("OMNI_ADMIN_DATABASE_URL")?,
            s3_endpoint: required("OMNI_S3_ENDPOINT")?,
            s3_access_key: required("OMNI_S3_ACCESS_KEY")?,
            s3_secret_key: required("OMNI_S3_SECRET_KEY")?,
            s3_bucket: required("OMNI_S3_BUCKET")?,
        })
    }
}

/// Parses `<workspace_id>` and returns it in lower case.
pub fn parse_args(args: &[String]) -> Result<String, String> {
    let usage = "usage: omni-api delete-workspace <workspace_id>";
    let [id] = args else {
        return Err(usage.to_string());
    };
    let id = id.trim().to_ascii_lowercase();
    let well_formed = id.len() == 36
        && id.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        });
    if !well_formed {
        return Err(format!("{id:?} is not a workspace ID (a UUID)\n{usage}"));
    }
    Ok(id)
}

/// What a deletion removed.
pub struct Deleted {
    pub rows: u64,
    pub files: usize,
}

/// Deletes the workspace's rows in one transaction, then its files. An
/// unknown workspace is an error and changes nothing, unless an earlier run
/// died after the rows and left files behind: then the files are finished off.
pub async fn run(config: &AdminConfig, workspace_id: &str) -> Result<Deleted, String> {
    let bucket = Bucket::new(
        &config.s3_bucket,
        region(&config.s3_endpoint),
        credentials(&config.s3_access_key, &config.s3_secret_key)?,
    )
    .map_err(|e| format!("invalid storage settings: {e}"))?
    .with_path_style();

    let mut conn = PgConnection::connect(&config.admin_database_url)
        .await
        .map_err(|e| format!("cannot connect to Postgres: {e}"))?;
    let rows = delete_rows(&mut conn, workspace_id)
        .await
        .map_err(|e| format!("deleting the rows failed, nothing was deleted: {e}"))?;
    let folder = format!("ws/{workspace_id}/");
    let files = delete_folder(&bucket, &folder).await?;
    match rows {
        None if files == 0 => Err(format!("there is no workspace {workspace_id}")),
        rows => Ok(Deleted {
            rows: rows.unwrap_or(0),
            files,
        }),
    }
}

/// Deletes the workspace's rows from every table with a `workspace_id` column
/// and then the `workspaces` row. Returns None when there is no such workspace.
async fn delete_rows(conn: &mut PgConnection, workspace_id: &str) -> Result<Option<u64>, String> {
    let mut tx = conn.begin().await.map_err(|e| e.to_string())?;
    // The tenant tables force row-level security even on their owner, so the
    // transaction is scoped to the workspace, as the API does for every request.
    sqlx::query("SELECT set_config('app.workspace_id', $1, true)")
        .bind(workspace_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM workspaces WHERE id = $1::uuid)")
            .bind(workspace_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    if !exists {
        return Ok(None);
    }

    let tables = tenant_tables(&mut tx).await?;
    let mut rows = 0;
    for table in delete_order(&tables, &foreign_keys(&mut tx).await?)? {
        let result = sqlx::query(&format!(
            "DELETE FROM \"{}\" WHERE workspace_id = $1::uuid",
            table.replace('"', "\"\"")
        ))
        .bind(workspace_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("{table}: {e}"))?;
        rows += result.rows_affected();
    }
    let result = sqlx::query("DELETE FROM workspaces WHERE id = $1::uuid")
        .bind(workspace_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("workspaces: {e}"))?;
    rows += result.rows_affected();
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(Some(rows))
}

/// Every table with a `workspace_id` column, found through `information_schema`
/// so that a table added by a later migration is covered without a change here.
async fn tenant_tables(conn: &mut PgConnection) -> Result<Vec<String>, String> {
    sqlx::query_scalar(
        "SELECT DISTINCT c.table_name::text \
         FROM information_schema.columns c \
         JOIN information_schema.tables t \
           ON t.table_schema = c.table_schema AND t.table_name = c.table_name \
         WHERE c.table_schema = 'public' AND c.column_name = 'workspace_id' \
           AND t.table_type = 'BASE TABLE' \
         ORDER BY 1",
    )
    .fetch_all(conn)
    .await
    .map_err(|e| e.to_string())
}

/// The foreign keys between tables of the public schema as (referencing table,
/// referenced table).
async fn foreign_keys(conn: &mut PgConnection) -> Result<Vec<(String, String)>, String> {
    sqlx::query_as(
        "SELECT child.relname::text, parent.relname::text \
         FROM pg_constraint c \
         JOIN pg_class child ON child.oid = c.conrelid \
         JOIN pg_class parent ON parent.oid = c.confrelid \
         JOIN pg_namespace n ON n.oid = child.relnamespace \
         WHERE c.contype = 'f' AND n.nspname = 'public'",
    )
    .fetch_all(conn)
    .await
    .map_err(|e| e.to_string())
}

/// Orders the tables so that a table comes after every table that references
/// it. A table referencing itself is fine (one statement deletes all its rows).
fn delete_order(tables: &[String], keys: &[(String, String)]) -> Result<Vec<String>, String> {
    let mut referenced_by: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (child, parent) in keys {
        if child != parent {
            referenced_by.entry(parent).or_default().insert(child);
        }
    }
    let mut remaining: Vec<&String> = tables.iter().collect();
    let mut order = Vec::new();
    while !remaining.is_empty() {
        let (ready, blocked): (Vec<&String>, Vec<&String>) = remaining.iter().partition(|table| {
            referenced_by
                .get(table.as_str())
                .is_none_or(|children| !children.iter().any(|c| remaining.iter().any(|r| r == c)))
        });
        if ready.is_empty() {
            let names: Vec<&str> = blocked.iter().map(|t| t.as_str()).collect();
            return Err(format!(
                "the foreign keys of {} reference each other in a cycle",
                names.join(", ")
            ));
        }
        order.extend(ready.into_iter().cloned());
        remaining = blocked;
    }
    Ok(order)
}

/// Deletes every object under `folder`, however many listing pages it spans,
/// and returns how many it deleted.
async fn delete_folder(bucket: &Bucket, folder: &str) -> Result<usize, String> {
    let mut deleted = 0;
    for _ in 0..FOLDER_PASSES {
        let pages = bucket
            .list(folder.to_string(), None)
            .await
            .map_err(|e| format!("cannot list {folder}: {e}"))?;
        let keys: Vec<String> = pages
            .into_iter()
            .flat_map(|page| page.contents)
            .map(|object| object.key)
            .collect();
        if keys.is_empty() {
            return Ok(deleted);
        }
        for batch in keys.chunks(DELETE_BATCH) {
            let mut deletes = tokio::task::JoinSet::new();
            for key in batch {
                let bucket = bucket.clone();
                let key = key.clone();
                deletes.spawn(async move {
                    let result = bucket.delete_object(&key).await;
                    (key, result)
                });
            }
            while let Some(done) = deletes.join_next().await {
                let (key, result) = done.map_err(|e| e.to_string())?;
                let response = result.map_err(|e| format!("cannot delete {key}: {e}"))?;
                if !matches!(response.status_code(), 200 | 204 | 404) {
                    return Err(format!(
                        "cannot delete {key}: storage answered {}",
                        response.status_code()
                    ));
                }
            }
        }
        deleted += keys.len();
    }
    Err(format!(
        "{folder} is still not empty after {FOLDER_PASSES} passes"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_workspace_id_is_a_lower_cased_uuid() {
        let id = "01900000-0000-7000-8000-00000000ABCD".to_string();
        assert_eq!(
            parse_args(&[id]).unwrap(),
            "01900000-0000-7000-8000-00000000abcd"
        );
        for bad in ["", "not-a-uuid", "01900000000070008000000000000000"] {
            assert!(parse_args(&[bad.to_string()]).is_err(), "{bad:?}");
        }
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn referencing_tables_are_deleted_before_the_tables_they_reference() {
        let tables = names(&["versions", "projects", "documents", "self_ref"]);
        let keys = vec![
            ("documents".to_string(), "projects".to_string()),
            ("versions".to_string(), "documents".to_string()),
            ("self_ref".to_string(), "self_ref".to_string()),
        ];
        let order = delete_order(&tables, &keys).unwrap();
        let at = |t: &str| order.iter().position(|x| x == t).unwrap();
        assert_eq!(order.len(), 4);
        assert!(at("versions") < at("documents"));
        assert!(at("documents") < at("projects"));
    }

    #[test]
    fn a_cycle_of_foreign_keys_is_an_error() {
        let tables = names(&["a", "b"]);
        let keys = vec![
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "a".to_string()),
        ];
        assert!(delete_order(&tables, &keys).is_err());
    }
}
