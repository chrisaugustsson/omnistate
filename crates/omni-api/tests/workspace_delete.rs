//! US-014: workspace deletion. Run with `--features stack-tests` in the Compose
//! test container, which reaches Postgres at `postgres:5432` and RustFS at
//! `rustfs:9000`.
//!
//! The tests migrate a scratch database, add three tenant tables of their own
//! that mimic the tables of later stories (a chain of composite foreign keys,
//! row-level security forced, no DELETE privilege for the API role), then seed
//! two workspaces with rows in every tenant table and with files in a scratch
//! RustFS bucket.
//!
//! The contract these tests pin down:
//!
//! - `omni-api delete-workspace <workspace_id>` deletes one workspace. Like
//!   `omni-api setup` it reads its settings from the environment
//!   (`OMNI_ADMIN_DATABASE_URL`, `OMNI_S3_*`) and runs as the database admin
//!   login, never as the API role `omni_api`. It exits 0 once the workspace's
//!   rows in every table that has a `workspace_id` column (the `workspaces` row
//!   itself, scoped by `id`, included) and every object under `ws/{id}/` in the
//!   bucket are gone. It finds the tables itself, so a table added by a later
//!   migration is covered without changing the command.
//! - Nothing else changes: other workspaces keep all their rows and files, and
//!   so do users, who are global. An unknown workspace ID is a non-zero exit
//!   that touches nothing.

use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use sqlx::{Connection, Row};
use std::future::Future;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const TEST_DB: &str = "omni_us014_delete";
const TEST_BUCKET: &str = "omni-us014-scratch";
const API_ROLE: &str = "omni_api";
const DELETE_TIMEOUT: Duration = Duration::from_secs(90);
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
/// More objects than one S3 listing page (1,000 keys) holds.
const MANY_OBJECTS: usize = 1_010;
/// The tables the tests add to the schema, which a later story's tables stand in for.
const PROBE_TABLES: [&str; 3] = ["us014_projects", "us014_documents", "us014_versions"];

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

fn block<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(future)
}

fn admin_url() -> String {
    std::env::var("OMNI_ADMIN_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://omni:omni-dev@postgres:5432/omni".to_string())
}

fn admin_options() -> PgConnectOptions {
    PgConnectOptions::from_str(&admin_url()).expect("OMNI_ADMIN_DATABASE_URL")
}

fn api_password() -> String {
    // The password that the stack's API gives omni_api (docker-compose.yml).
    std::env::var("OMNI_API_DB_PASSWORD").unwrap_or_else(|_| "omni-api-dev".to_string())
}

fn scratch_admin_url() -> String {
    let url = admin_url();
    let (server, _database) = url.rsplit_once('/').expect("OMNI_ADMIN_DATABASE_URL");
    format!("{server}/{TEST_DB}")
}

fn scratch_api_url() -> String {
    let admin = admin_options();
    format!(
        "postgres://{API_ROLE}:{}@{}:{}/{TEST_DB}",
        api_password(),
        admin.get_host(),
        admin.get_port()
    )
}

fn s3_endpoint() -> String {
    std::env::var("OMNI_S3_ENDPOINT").unwrap_or_else(|_| "http://rustfs:9000".to_string())
}

fn s3_access_key() -> String {
    std::env::var("OMNI_S3_ACCESS_KEY").unwrap_or_else(|_| "omni-dev".to_string())
}

fn s3_secret_key() -> String {
    std::env::var("OMNI_S3_SECRET_KEY").unwrap_or_else(|_| "omni-dev-secret".to_string())
}

fn region() -> Region {
    Region::Custom {
        region: "us-east-1".to_string(),
        endpoint: s3_endpoint(),
    }
}

fn credentials() -> Credentials {
    Credentials::new(
        Some(&s3_access_key()),
        Some(&s3_secret_key()),
        None,
        None,
        None,
    )
    .expect("storage credentials")
}

fn bucket() -> Box<Bucket> {
    Bucket::new(TEST_BUCKET, region(), credentials())
        .expect("bucket settings")
        .with_path_style()
}

fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "{}{nanos}{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

/// The scratch database (migrated, plus the probe tables and trigger) and the
/// scratch bucket, created once per test run.
fn scratch() {
    static READY: OnceLock<()> = OnceLock::new();
    READY.get_or_init(|| {
        block(async {
            let mut admin = PgConnection::connect_with(&admin_options())
                .await
                .unwrap_or_else(|e| panic!("cannot connect to Postgres as the admin role: {e}"));
            sqlx::query(&format!("DROP DATABASE IF EXISTS {TEST_DB} WITH (FORCE)"))
                .execute(&mut admin)
                .await
                .expect("drop the scratch database");
            sqlx::query(&format!("CREATE DATABASE {TEST_DB}"))
                .execute(&mut admin)
                .await
                .expect("create the scratch database");

            let mut conn = scratch_connection().await;
            Migrator::new(migrations_dir())
                .await
                .expect("read migrations/")
                .run(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("applying migrations/ failed: {e}"));
            sqlx::raw_sql(&probe_schema())
                .execute(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("creating the probe tables failed: {e}"));

            let bucket = bucket();
            if !bucket.exists().await.unwrap_or(false) {
                Bucket::create_with_path_style(
                    TEST_BUCKET,
                    region(),
                    credentials(),
                    BucketConfiguration::default(),
                )
                .await
                .unwrap_or_else(|e| panic!("cannot create the scratch bucket: {e}"));
            }
        });
    });
}

/// Three tenant tables that follow migrations/README.md, the way the tables of
/// later stories will.
fn probe_schema() -> String {
    let mut sql = String::from(
        "CREATE TABLE us014_projects (
            id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT us014_projects_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
            workspace_id uuid NOT NULL REFERENCES workspaces (id),
            name text NOT NULL,
            UNIQUE (workspace_id, id)
        );
        CREATE TABLE us014_documents (
            id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT us014_documents_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
            workspace_id uuid NOT NULL REFERENCES workspaces (id),
            project_id uuid NOT NULL,
            created_by_membership_id uuid NOT NULL,
            title text NOT NULL,
            UNIQUE (workspace_id, id),
            FOREIGN KEY (workspace_id, project_id) REFERENCES us014_projects (workspace_id, id),
            FOREIGN KEY (workspace_id, created_by_membership_id) REFERENCES memberships (workspace_id, id)
        );
        CREATE TABLE us014_versions (
            id uuid PRIMARY KEY DEFAULT uuid_v7() CONSTRAINT us014_versions_id_is_uuid_v7 CHECK (is_uuid_v7(id)),
            workspace_id uuid NOT NULL REFERENCES workspaces (id),
            document_id uuid NOT NULL,
            body text NOT NULL,
            UNIQUE (workspace_id, id),
            FOREIGN KEY (workspace_id, document_id) REFERENCES us014_documents (workspace_id, id)
        );
        GRANT SELECT, INSERT, UPDATE ON us014_projects, us014_documents TO omni_api;
        GRANT SELECT, INSERT ON us014_versions TO omni_api;
        ",
    );
    for table in PROBE_TABLES {
        sql.push_str(&format!(
            "ALTER TABLE {table} ENABLE ROW LEVEL SECURITY;
            ALTER TABLE {table} FORCE ROW LEVEL SECURITY;
            CREATE POLICY workspace_isolation ON {table}
                USING (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid)
                WITH CHECK (workspace_id = NULLIF(current_setting('app.workspace_id', true), '')::uuid);
            "
        ));
    }
    sql
}

async fn scratch_connection() -> PgConnection {
    PgConnection::connect(&scratch_admin_url())
        .await
        .expect("connect to the scratch database as the admin role")
}

// ---------------------------------------------------------------------------
// Seeding
// ---------------------------------------------------------------------------

async fn new_user(conn: &mut PgConnection) -> String {
    sqlx::query_scalar("INSERT INTO users (email) VALUES ($1) RETURNING id::text")
        .bind(format!("user-{}@example.test", unique_suffix()))
        .fetch_one(conn)
        .await
        .expect("insert a user")
}

async fn new_workspace(conn: &mut PgConnection) -> String {
    sqlx::query_scalar("INSERT INTO workspaces (name) VALUES ($1) RETURNING id::text")
        .bind(format!("Workspace {}", unique_suffix()))
        .fetch_one(conn)
        .await
        .expect("insert a workspace")
}

async fn new_membership(conn: &mut PgConnection, workspace: &str, user: &str) -> String {
    sqlx::query_scalar(
        "INSERT INTO memberships (workspace_id, user_id) VALUES ($1::uuid, $2::uuid) RETURNING id::text",
    )
    .bind(workspace)
    .bind(user)
    .fetch_one(conn)
    .await
    .expect("insert a membership")
}

/// A key, two probe projects with two documents each and two versions of each
/// document, plus one row in every other tenant table: rows in every tenant
/// table, referencing each other and the workspace's memberships.
///
/// Every story that adds a tenant table must seed it here too, or the
/// deletion tests fail on purpose: they check that the seeded workspace has
/// a row in every table that has a `workspace_id` column.
async fn fill_workspace(conn: &mut PgConnection, workspace: &str, membership: &str) {
    sqlx::query(
        "INSERT INTO api_keys (workspace_id, prefix, key_hash, created_by_membership_id) \
         VALUES ($1::uuid, $2, 'hash', $3::uuid)",
    )
    .bind(workspace)
    .bind(format!("us014{}", unique_suffix()))
    .bind(membership)
    .execute(&mut *conn)
    .await
    .expect("insert an API key");
    for project in 0..2 {
        let project_id: String = sqlx::query_scalar(
            "INSERT INTO us014_projects (workspace_id, name) VALUES ($1::uuid, $2) RETURNING id::text",
        )
        .bind(workspace)
        .bind(format!("project {project}"))
        .fetch_one(&mut *conn)
        .await
        .expect("insert a project");
        for document in 0..2 {
            let document_id: String = sqlx::query_scalar(
                "INSERT INTO us014_documents (workspace_id, project_id, created_by_membership_id, title) \
                 VALUES ($1::uuid, $2::uuid, $3::uuid, $4) RETURNING id::text",
            )
            .bind(workspace)
            .bind(&project_id)
            .bind(membership)
            .bind(format!("document {document}"))
            .fetch_one(&mut *conn)
            .await
            .expect("insert a document");
            for version in 0..2 {
                sqlx::query(
                    "INSERT INTO us014_versions (workspace_id, document_id, body) \
                     VALUES ($1::uuid, $2::uuid, $3)",
                )
                .bind(workspace)
                .bind(&document_id)
                .bind(format!("version {version}"))
                .execute(&mut *conn)
                .await
                .expect("insert a version");
            }
        }
    }

    // The tables of the API, one row each. A table that a later story adds is
    // seeded here too, so the deletion tests fail until it is erased with the
    // rest.
    let project_id: String = sqlx::query_scalar(
        "INSERT INTO projects (workspace_id, name) VALUES ($1::uuid, 'deletion test') RETURNING id::text",
    )
    .bind(workspace)
    .fetch_one(&mut *conn)
    .await
    .expect("insert a project");
    sqlx::query(
        "INSERT INTO live_events (workspace_id, project_id, seq, event, data) \
         VALUES ($1::uuid, $2::uuid, 1, 'doc.created', '{}'::jsonb)",
    )
    .bind(workspace)
    .bind(&project_id)
    .execute(&mut *conn)
    .await
    .expect("insert a live event");
    sqlx::query(
        "INSERT INTO attachments (workspace_id, filename, content_type, size_bytes) \
         VALUES ($1::uuid, 'deletion-test.txt', 'text/plain', 12)",
    )
    .bind(workspace)
    .execute(&mut *conn)
    .await
    .expect("insert an attachment");

    // A doc, one row of the log of its writes, and what its head is stored in.
    let doc_id: String = sqlx::query_scalar(
        "INSERT INTO docs (workspace_id, project_id, kind, pillar, title) \
         VALUES ($1::uuid, $2::uuid, 'html', 'spec', 'deletion test') RETURNING id::text",
    )
    .bind(workspace)
    .bind(&project_id)
    .fetch_one(&mut *conn)
    .await
    .expect("insert a doc");
    sqlx::query(
        "INSERT INTO doc_changes (workspace_id, doc_id, seq, agent, operations, head_after) \
         VALUES ($1::uuid, $2::uuid, 1, $3::jsonb, '[]'::jsonb, \
         'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262')",
    )
    .bind(workspace)
    .bind(&doc_id)
    .bind(r#"{"run":"run-1","role":"writer","model":null,"harness":null}"#)
    .execute(&mut *conn)
    .await
    .expect("insert a doc change");
    sqlx::query(
        "INSERT INTO node_index (workspace_id, chunk, node, type, fingerprint) \
         VALUES ($1::uuid, 'deletion-test-chunk', 'k3f9x2qa', 'paragraph', 'fingerprint')",
    )
    .bind(workspace)
    .execute(&mut *conn)
    .await
    .expect("insert a node of the node index");
    sqlx::query(
        "INSERT INTO chunks (workspace_id, hash) \
         VALUES ($1::uuid, 'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262')",
    )
    .bind(workspace)
    .execute(&mut *conn)
    .await
    .expect("insert a chunk of the chunk index");

    // A comment on a node of the doc and a reply to it.
    let comment_id: String = sqlx::query_scalar(
        "INSERT INTO comments (workspace_id, doc_id, node, author, body) \
         VALUES ($1::uuid, $2::uuid, 'k3f9x2qa', $3::jsonb, 'deletion test') \
         RETURNING id::text",
    )
    .bind(workspace)
    .bind(&doc_id)
    .bind(r#"{"kind":"agent","run":"run-1","role":"critic","model":null,"harness":null}"#)
    .fetch_one(&mut *conn)
    .await
    .expect("insert a comment");
    sqlx::query(
        "INSERT INTO comment_replies (workspace_id, comment_id, author, body) \
         VALUES ($1::uuid, $2::uuid, $3::jsonb, 'deletion test')",
    )
    .bind(workspace)
    .bind(&comment_id)
    .bind(r#"{"kind":"agent","run":"run-1","role":"writer","model":null,"harness":null}"#)
    .execute(&mut *conn)
    .await
    .expect("insert a reply to a comment");

    // A thread, its phase and a post in it.
    let thread_id: String = sqlx::query_scalar(
        "INSERT INTO threads (workspace_id, project_id, question, criteria) \
         VALUES ($1::uuid, $2::uuid, 'deletion test', '[]'::jsonb) RETURNING id::text",
    )
    .bind(workspace)
    .bind(&project_id)
    .fetch_one(&mut *conn)
    .await
    .expect("insert a thread");
    sqlx::query(
        "INSERT INTO thread_phases (workspace_id, thread_id, number, sealed) \
         VALUES ($1::uuid, $2::uuid, 1, true)",
    )
    .bind(workspace)
    .bind(&thread_id)
    .execute(&mut *conn)
    .await
    .expect("insert a thread phase");
    sqlx::query(
        "INSERT INTO posts (workspace_id, thread_id, phase, summary, body, author) \
         VALUES ($1::uuid, $2::uuid, 1, 'deletion test', 'deletion test', $3::jsonb)",
    )
    .bind(workspace)
    .bind(&thread_id)
    .bind(r#"{"run":"run-1","role":"writer","model":null,"harness":null}"#)
    .execute(&mut *conn)
    .await
    .expect("insert a post");

    // A feature of the project, and a story in it.
    let feature_id: String = sqlx::query_scalar(
        "INSERT INTO features (workspace_id, project_id, name) \
         VALUES ($1::uuid, $2::uuid, 'deletion test') RETURNING id::text",
    )
    .bind(workspace)
    .bind(&project_id)
    .fetch_one(&mut *conn)
    .await
    .expect("insert a feature");
    sqlx::query(
        "INSERT INTO stories (workspace_id, project_id, feature_id, number, title, text, topic, criteria) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, 1, 'deletion test', '', '', '[]'::jsonb)",
    )
    .bind(workspace)
    .bind(&project_id)
    .bind(&feature_id)
    .execute(&mut *conn)
    .await
    .expect("insert a story");
}

async fn put_objects(keys: Vec<String>) {
    let bucket = bucket();
    for batch in keys.chunks(25) {
        let mut puts = tokio::task::JoinSet::new();
        for key in batch {
            let bucket = bucket.clone();
            let key = key.clone();
            puts.spawn(async move {
                let result = bucket.put_object(&key, &b"object bytes"[..]).await;
                (key, result)
            });
        }
        while let Some(done) = puts.join_next().await {
            let (key, result) = done.expect("upload task");
            let response = result.unwrap_or_else(|e| panic!("cannot upload {key}: {e}"));
            assert_eq!(response.status_code(), 200, "uploading {key}");
        }
    }
}

/// The folder of a workspace in RustFS: chunks, attachments and a nested file.
fn folder_keys(workspace: &str) -> Vec<String> {
    let suffix = unique_suffix();
    vec![
        format!("ws/{workspace}/chunks/{suffix}a"),
        format!("ws/{workspace}/chunks/{suffix}b"),
        format!("ws/{workspace}/attachments/{suffix}c"),
        format!("ws/{workspace}/attachments/{suffix}d"),
        format!("ws/{workspace}/exports/nested/deeper/{suffix}e.json"),
    ]
}

async fn keys_under(prefix: &str) -> Vec<String> {
    let pages = bucket()
        .list(prefix.to_string(), None)
        .await
        .unwrap_or_else(|e| panic!("cannot list {prefix}: {e}"));
    let mut keys: Vec<String> = pages
        .into_iter()
        .flat_map(|page| page.contents)
        .map(|object| object.key)
        .collect();
    keys.sort();
    keys
}

fn folder_prefix(workspace: &str) -> String {
    format!("ws/{workspace}/")
}

/// Two workspaces that share one user, each with rows in every tenant table
/// and files in the bucket: the one to delete and one that must survive.
struct World {
    doomed: String,
    bystander: String,
    shared_user: String,
}

fn world() -> World {
    scratch();
    block(async {
        let mut conn = scratch_connection().await;
        let shared_user = new_user(&mut conn).await;
        let mut ids = Vec::new();
        for _ in 0..2 {
            let workspace = new_workspace(&mut conn).await;
            let own_user = new_user(&mut conn).await;
            let membership = new_membership(&mut conn, &workspace, &own_user).await;
            new_membership(&mut conn, &workspace, &shared_user).await;
            fill_workspace(&mut conn, &workspace, &membership).await;
            put_objects(folder_keys(&workspace)).await;
            ids.push(workspace);
        }
        World {
            bystander: ids.pop().unwrap(),
            doomed: ids.pop().unwrap(),
            shared_user,
        }
    })
}

// ---------------------------------------------------------------------------
// Looking at the database through information_schema
// ---------------------------------------------------------------------------

/// Every table with a `workspace_id` column, found through
/// `information_schema`, and `workspaces`, which is scoped by `id`.
async fn tenant_tables(conn: &mut PgConnection) -> Vec<String> {
    let rows = sqlx::query(
        "SELECT DISTINCT c.table_name::text AS name \
         FROM information_schema.columns c \
         JOIN information_schema.tables t \
           ON t.table_schema = c.table_schema AND t.table_name = c.table_name \
         WHERE c.table_schema = 'public' AND c.column_name = 'workspace_id' \
           AND t.table_type = 'BASE TABLE' \
         ORDER BY 1",
    )
    .fetch_all(conn)
    .await
    .expect("read information_schema");
    let mut tables: Vec<String> = rows.iter().map(|r| r.get("name")).collect();
    tables.push("workspaces".to_string());
    tables.sort();
    tables
}

async fn rows_for(conn: &mut PgConnection, table: &str, workspace: &str) -> i64 {
    let column = if table == "workspaces" {
        "id"
    } else {
        "workspace_id"
    };
    sqlx::query_scalar(&format!(
        "SELECT count(*) FROM \"{table}\" WHERE {column} = $1::uuid"
    ))
    .bind(workspace)
    .fetch_one(conn)
    .await
    .unwrap_or_else(|e| panic!("count the rows of {table}: {e}"))
}

/// The number of rows a workspace has in each tenant table.
async fn snapshot(conn: &mut PgConnection, workspace: &str) -> Vec<(String, i64)> {
    let mut counts = Vec::new();
    for table in tenant_tables(conn).await {
        let count = rows_for(conn, &table, workspace).await;
        counts.push((table, count));
    }
    counts
}

// ---------------------------------------------------------------------------
// Running the admin command
// ---------------------------------------------------------------------------

struct DeleteRun {
    success: bool,
    timed_out: bool,
    stdout: String,
    stderr: String,
}

impl DeleteRun {
    fn output(&self) -> String {
        format!(
            "timed out: {}\nstdout: {}\nstderr: {}",
            self.timed_out, self.stdout, self.stderr
        )
    }
}

/// Runs `omni-api delete-workspace <workspace>` against the scratch database
/// and bucket, killing it after DELETE_TIMEOUT.
fn run_delete(workspace: &str) -> DeleteRun {
    scratch();
    let mut child = Command::new(env!("CARGO_BIN_EXE_omni-api"))
        .args(["delete-workspace", workspace])
        .env("OMNI_DATABASE_URL", scratch_api_url())
        .env("OMNI_ADMIN_DATABASE_URL", scratch_admin_url())
        .env("OMNI_API_DB_PASSWORD", api_password())
        .env("OMNI_S3_ENDPOINT", s3_endpoint())
        .env("OMNI_S3_ACCESS_KEY", s3_access_key())
        .env("OMNI_S3_SECRET_KEY", s3_secret_key())
        .env("OMNI_S3_BUCKET", TEST_BUCKET)
        .env("OMNI_API_BIND", "127.0.0.1:0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start omni-api");

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for omni-api") {
            break status;
        }
        if started.elapsed() > DELETE_TIMEOUT {
            timed_out = true;
            let _ = child.kill();
            break child.wait().expect("reap omni-api");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    let _ = child.stdout.take().unwrap().read_to_string(&mut stdout);
    let _ = child.stderr.take().unwrap().read_to_string(&mut stderr);
    DeleteRun {
        success: status.success() && !timed_out,
        timed_out,
        stdout,
        stderr,
    }
}

// ---------------------------------------------------------------------------
// The admin command
// ---------------------------------------------------------------------------

#[test]
fn the_admin_command_deletes_every_row_of_the_workspace_and_its_whole_rustfs_folder() {
    let world = world();
    block(async {
        let mut conn = scratch_connection().await;
        let tables = tenant_tables(&mut conn).await;
        for expected in ["workspaces", "memberships", "api_keys"]
            .into_iter()
            .chain(PROBE_TABLES)
        {
            assert!(
                tables.iter().any(|t| t == expected),
                "information_schema should list {expected} among the tenant tables, found {tables:?}"
            );
        }
        // The check below proves nothing unless the workspace has something to lose.
        for (table, count) in snapshot(&mut conn, &world.doomed).await {
            assert!(count > 0, "the seeded workspace has no row in {table}");
        }
        assert!(!keys_under(&folder_prefix(&world.doomed)).await.is_empty());
    });

    let run = run_delete(&world.doomed);
    assert!(
        run.success,
        "omni-api delete-workspace failed\n{}",
        run.output()
    );

    block(async {
        let mut conn = scratch_connection().await;
        for table in tenant_tables(&mut conn).await {
            assert_eq!(
                rows_for(&mut conn, &table, &world.doomed).await,
                0,
                "{table} still has rows of the deleted workspace"
            );
        }
        assert_eq!(
            keys_under(&folder_prefix(&world.doomed)).await,
            Vec::<String>::new(),
            "the RustFS folder of the deleted workspace is not empty"
        );
    });
}

#[test]
fn a_folder_larger_than_one_listing_page_is_erased_completely() {
    scratch();
    let workspace = block(async {
        let mut conn = scratch_connection().await;
        let workspace = new_workspace(&mut conn).await;
        let user = new_user(&mut conn).await;
        let membership = new_membership(&mut conn, &workspace, &user).await;
        fill_workspace(&mut conn, &workspace, &membership).await;
        let suffix = unique_suffix();
        let keys = (0..MANY_OBJECTS)
            .map(|n| format!("ws/{workspace}/chunks/{suffix}-{n:05}"))
            .collect();
        put_objects(keys).await;
        assert_eq!(
            keys_under(&folder_prefix(&workspace)).await.len(),
            MANY_OBJECTS
        );
        workspace
    });

    let run = run_delete(&workspace);
    assert!(
        run.success,
        "omni-api delete-workspace failed\n{}",
        run.output()
    );

    block(async {
        assert_eq!(
            keys_under(&folder_prefix(&workspace)).await,
            Vec::<String>::new(),
            "objects are left in the folder of the deleted workspace"
        );
    });
}

#[test]
fn deleting_a_workspace_leaves_other_workspaces_and_users_untouched() {
    let world = world();
    let (rows_before, files_before) = block(async {
        let mut conn = scratch_connection().await;
        (
            snapshot(&mut conn, &world.bystander).await,
            keys_under(&folder_prefix(&world.bystander)).await,
        )
    });
    assert!(rows_before.iter().all(|(_, count)| *count > 0));
    assert!(!files_before.is_empty());

    let run = run_delete(&world.doomed);
    assert!(
        run.success,
        "omni-api delete-workspace failed\n{}",
        run.output()
    );

    block(async {
        let mut conn = scratch_connection().await;
        assert_eq!(
            snapshot(&mut conn, &world.bystander).await,
            rows_before,
            "the other workspace lost or gained rows"
        );
        assert_eq!(
            keys_under(&folder_prefix(&world.bystander)).await,
            files_before,
            "the other workspace's files changed"
        );
        let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = $1::uuid")
            .bind(&world.shared_user)
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(
            users, 1,
            "a user who also belongs to another workspace was deleted"
        );
    });
}

#[test]
fn an_unknown_workspace_id_is_an_error_that_deletes_nothing() {
    let world = world();
    let (rows_before, files_before) = block(async {
        let mut conn = scratch_connection().await;
        (
            snapshot(&mut conn, &world.bystander).await,
            keys_under(&folder_prefix(&world.bystander)).await,
        )
    });

    let run = run_delete(UNKNOWN_ID);
    assert!(
        !run.success && !run.timed_out,
        "deleting an unknown workspace should fail\n{}",
        run.output()
    );
    assert!(
        !run.stderr.trim().is_empty(),
        "a failed deletion should say why on standard error"
    );

    block(async {
        let mut conn = scratch_connection().await;
        assert_eq!(snapshot(&mut conn, &world.bystander).await, rows_before);
        assert_eq!(
            keys_under(&folder_prefix(&world.bystander)).await,
            files_before
        );
    });
}
