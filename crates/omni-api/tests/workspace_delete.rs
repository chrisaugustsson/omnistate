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

/// A key, two projects with two documents each and two versions of each
/// document, plus one row in every real tenant table that later stories add:
/// rows in every tenant table, referencing each other and the workspace's
/// memberships.
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

    // US-016: projects, change sets and their status history.
    let project_id: String = sqlx::query_scalar(
        "INSERT INTO projects (workspace_id, name) VALUES ($1::uuid, 'deletion test') RETURNING id::text",
    )
    .bind(workspace)
    .fetch_one(&mut *conn)
    .await
    .expect("insert a project");
    let changeset_id: String = sqlx::query_scalar(
        "INSERT INTO changesets (workspace_id, project_id, title, status) \
         VALUES ($1::uuid, $2::uuid, 'deletion test', 'open') RETURNING id::text",
    )
    .bind(workspace)
    .bind(&project_id)
    .fetch_one(&mut *conn)
    .await
    .expect("insert a change set");
    sqlx::query(
        "INSERT INTO changeset_status_history (workspace_id, changeset_id, from_status, to_status, actor) \
         VALUES ($1::uuid, $2::uuid, NULL, 'open', 'agent')",
    )
    .bind(workspace)
    .bind(&changeset_id)
    .execute(&mut *conn)
    .await
    .expect("insert a change set status");

    // US-017: the live events of a project. The table is seeded as soon as it
    // exists (tests/live_events.rs pins its name and columns), so a migration
    // that adds it fails the deletion tests until it is erased with the rest.
    let live_events_exist: bool =
        sqlx::query_scalar("SELECT to_regclass('public.live_events') IS NOT NULL")
            .fetch_one(&mut *conn)
            .await
            .expect("look for the live events table");
    if live_events_exist {
        sqlx::query(
            "INSERT INTO live_events (workspace_id, project_id, seq, event, data) \
             VALUES ($1::uuid, $2::uuid, 1, 'changeset.open', '{}'::jsonb)",
        )
        .bind(workspace)
        .bind(&project_id)
        .execute(&mut *conn)
        .await
        .expect("insert a live event");
    }

    // US-019: the metadata of an attachment. Seeded as soon as the table
    // exists (tests/attachments.rs pins its name and columns), for the same
    // reason as the live events above.
    let attachments_exist: bool =
        sqlx::query_scalar("SELECT to_regclass('public.attachments') IS NOT NULL")
            .fetch_one(&mut *conn)
            .await
            .expect("look for the attachments table");
    if attachments_exist {
        sqlx::query(
            "INSERT INTO attachments (workspace_id, filename, content_type, size_bytes) \
             VALUES ($1::uuid, 'deletion-test.txt', 'text/plain', 12)",
        )
        .bind(workspace)
        .execute(&mut *conn)
        .await
        .expect("insert an attachment");
    }

    // US-022: a doc, its draft in the change set and one row of the draft's
    // log. Seeded as soon as the tables exist (tests/docs.rs pins their names
    // and the columns without a default), for the same reason as above.
    let docs_exist: bool = sqlx::query_scalar(
        "SELECT to_regclass('public.docs') IS NOT NULL \
         AND to_regclass('public.drafts') IS NOT NULL \
         AND to_regclass('public.draft_ops') IS NOT NULL",
    )
    .fetch_one(&mut *conn)
    .await
    .expect("look for the doc tables");
    if docs_exist {
        let doc_id: String = sqlx::query_scalar(
            "INSERT INTO docs (workspace_id, project_id, kind, pillar, title) \
             VALUES ($1::uuid, $2::uuid, 'html', 'spec', 'deletion test') RETURNING id::text",
        )
        .bind(workspace)
        .bind(&project_id)
        .fetch_one(&mut *conn)
        .await
        .expect("insert a doc");
        let draft_id: String = sqlx::query_scalar(
            "INSERT INTO drafts (workspace_id, doc_id, changeset_id) \
             VALUES ($1::uuid, $2::uuid, $3::uuid) RETURNING id::text",
        )
        .bind(workspace)
        .bind(&doc_id)
        .bind(&changeset_id)
        .fetch_one(&mut *conn)
        .await
        .expect("insert a draft");
        sqlx::query(
            "INSERT INTO draft_ops (workspace_id, draft_id, seq, op, agent) \
             VALUES ($1::uuid, $2::uuid, 1, $3::jsonb, $4::jsonb)",
        )
        .bind(workspace)
        .bind(&draft_id)
        .bind(r#"{"op":"delete","node":"k3f9x2qa","rev":1}"#)
        .bind(r#"{"run":"run-1","role":"writer","model":null,"harness":null}"#)
        .execute(&mut *conn)
        .await
        .expect("insert an operation of the draft's log");

        // US-102: one row of the log of the writes to docs, the append-only
        // tenant table `doc_changes` (tests/live_docs.rs pins its name and the
        // columns without a default). Seeded as soon as it exists, for the same
        // reason as above.
        let changes_exist: bool =
            sqlx::query_scalar("SELECT to_regclass('public.doc_changes') IS NOT NULL")
                .fetch_one(&mut *conn)
                .await
                .expect("look for the doc changes table");
        if changes_exist {
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
        }

        // US-023: a claim on a node of the draft. Claims may live in memory;
        // if they live in the tenant table `claims` (tests/claims.rs pins its
        // name and the columns without a default), it is seeded as soon as it
        // exists, for the same reason as above.
        let claims_exist: bool =
            sqlx::query_scalar("SELECT to_regclass('public.claims') IS NOT NULL")
                .fetch_one(&mut *conn)
                .await
                .expect("look for the claims table");
        if claims_exist {
            sqlx::query(
                "INSERT INTO claims (workspace_id, draft_id, node, agent) \
                 VALUES ($1::uuid, $2::uuid, 'k3f9x2qa', $3::jsonb)",
            )
            .bind(workspace)
            .bind(&draft_id)
            .bind(r#"{"run":"run-1","role":"writer","model":null,"harness":null}"#)
            .execute(&mut *conn)
            .await
            .expect("insert a claim");
        }

        // US-024: a saved version of the doc and one row of the node index.
        // Seeded as soon as the tables exist (tests/versions.rs pins their names;
        // the columns without a default are `workspace_id`, `doc_id` and
        // `changeset_id` for `versions`, and `workspace_id`, `chunk`, `node`,
        // `type` and `fingerprint` for `node_index`, where `chunk` names the
        // stored content that the nodes belong to), for the same reason as above.
        let versions_exist: bool = sqlx::query_scalar(
            "SELECT to_regclass('public.versions') IS NOT NULL \
             AND to_regclass('public.node_index') IS NOT NULL",
        )
        .fetch_one(&mut *conn)
        .await
        .expect("look for the version tables");
        if versions_exist {
            let version_id: String = sqlx::query_scalar(
                "INSERT INTO versions (workspace_id, doc_id, changeset_id) \
                 VALUES ($1::uuid, $2::uuid, $3::uuid) RETURNING id::text",
            )
            .bind(workspace)
            .bind(&doc_id)
            .bind(&changeset_id)
            .fetch_one(&mut *conn)
            .await
            .expect("insert a version");
            sqlx::query(
                "INSERT INTO node_index (workspace_id, chunk, node, type, fingerprint) \
                 VALUES ($1::uuid, 'deletion-test-chunk', 'k3f9x2qa', 'paragraph', 'fingerprint')",
            )
            .bind(workspace)
            .execute(&mut *conn)
            .await
            .expect("insert a node of the node index");

            // US-030: one row of the chunk index, the tenant table `chunks`
            // (tests/boards.rs pins its name and the columns without a
            // default, `workspace_id` and `hash`). Seeded as soon as it
            // exists, for the same reason as above.
            let chunks_exist: bool =
                sqlx::query_scalar("SELECT to_regclass('public.chunks') IS NOT NULL")
                    .fetch_one(&mut *conn)
                    .await
                    .expect("look for the chunk index");
            if chunks_exist {
                sqlx::query(
                    "INSERT INTO chunks (workspace_id, hash) \
                     VALUES ($1::uuid, 'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262')",
                )
                .bind(workspace)
                .execute(&mut *conn)
                .await
                .expect("insert a chunk of the chunk index");
            }

            // US-025: the approval of the change set, which records the
            // version. It may live in the existing tables; if it lives in the
            // tenant table `approvals` (tests/approve_merge.rs pins its name
            // and the columns without a default), it is seeded as soon as it
            // exists, for the same reason as above.
            let approvals_exist: bool =
                sqlx::query_scalar("SELECT to_regclass('public.approvals') IS NOT NULL")
                    .fetch_one(&mut *conn)
                    .await
                    .expect("look for the approvals table");
            if approvals_exist {
                sqlx::query(
                    "INSERT INTO approvals (workspace_id, changeset_id, doc_id, version_id) \
                     VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid)",
                )
                .bind(workspace)
                .bind(&changeset_id)
                .bind(&doc_id)
                .bind(&version_id)
                .execute(&mut *conn)
                .await
                .expect("insert an approval");
            }

            // US-026: a conflict that a rebase recorded on a node of the doc,
            // with the version of each side. Conflicts may live in the
            // existing tables; if they live in the tenant table `conflicts`
            // (tests/rebase.rs pins its name and the columns without a
            // default), it is seeded as soon as it exists, for the same reason
            // as above.
            let conflicts_exist: bool =
                sqlx::query_scalar("SELECT to_regclass('public.conflicts') IS NOT NULL")
                    .fetch_one(&mut *conn)
                    .await
                    .expect("look for the conflicts table");
            if conflicts_exist {
                sqlx::query(
                    "INSERT INTO conflicts (workspace_id, changeset_id, doc_id, node, \
                     mainline_version_id, changeset_version_id) \
                     VALUES ($1::uuid, $2::uuid, $3::uuid, 'k3f9x2qa', $4::uuid, $4::uuid)",
                )
                .bind(workspace)
                .bind(&changeset_id)
                .bind(&doc_id)
                .bind(&version_id)
                .execute(&mut *conn)
                .await
                .expect("insert a conflict");
            }

            // US-038: a comment on a node of the version (of the doc, after
            // US-106) and a reply to it.
            // Both are seeded as soon as their tables exist
            // (tests/comments.rs pins their names and the columns without a
            // default), for the same reason as above.
            let comments_exist: bool =
                sqlx::query_scalar("SELECT to_regclass('public.comments') IS NOT NULL")
                    .fetch_one(&mut *conn)
                    .await
                    .expect("look for the comments table");
            if comments_exist {
                // US-106: a comment belongs to a doc and a node of its head
                // once the table has `doc_id`; before that, to a version.
                let on_docs: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
                     WHERE table_schema = 'public' AND table_name = 'comments' \
                       AND column_name = 'doc_id')",
                )
                .fetch_one(&mut *conn)
                .await
                .expect("look for the comments' doc column");
                let (sql, record) = if on_docs {
                    (
                        "INSERT INTO comments (workspace_id, doc_id, node, author, body) \
                         VALUES ($1::uuid, $2::uuid, 'k3f9x2qa', $3::jsonb, 'deletion test') \
                         RETURNING id::text",
                        &doc_id,
                    )
                } else {
                    (
                        "INSERT INTO comments (workspace_id, version_id, node, author, body) \
                         VALUES ($1::uuid, $2::uuid, 'k3f9x2qa', $3::jsonb, 'deletion test') \
                         RETURNING id::text",
                        &version_id,
                    )
                };
                let comment_id: String = sqlx::query_scalar(sql)
                    .bind(workspace)
                    .bind(record)
                    .bind(
                        r#"{"kind":"agent","run":"run-1","role":"critic","model":null,"harness":null}"#,
                    )
                    .fetch_one(&mut *conn)
                    .await
                    .expect("insert a comment");
                let replies_exist: bool =
                    sqlx::query_scalar("SELECT to_regclass('public.comment_replies') IS NOT NULL")
                        .fetch_one(&mut *conn)
                        .await
                        .expect("look for the comment replies table");
                if replies_exist {
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
                }
            }
        }
    }

    // US-033: a thread of the change set, a phase of it and a post in it.
    // Seeded as soon as the tables exist (tests/threads.rs pins their names
    // and the columns without a default; phases may live in `threads`, so
    // `thread_phases` is seeded only if it exists), for the same reason as
    // above.
    let threads_exist: bool = sqlx::query_scalar(
        "SELECT to_regclass('public.threads') IS NOT NULL \
         AND to_regclass('public.posts') IS NOT NULL",
    )
    .fetch_one(&mut *conn)
    .await
    .expect("look for the thread tables");
    if threads_exist {
        let thread_id: String = sqlx::query_scalar(
            "INSERT INTO threads (workspace_id, project_id, changeset_id, question, criteria) \
             VALUES ($1::uuid, $2::uuid, $3::uuid, 'deletion test', '[\"deletion test\"]'::jsonb) \
             RETURNING id::text",
        )
        .bind(workspace)
        .bind(&project_id)
        .bind(&changeset_id)
        .fetch_one(&mut *conn)
        .await
        .expect("insert a thread");
        let phases_exist: bool =
            sqlx::query_scalar("SELECT to_regclass('public.thread_phases') IS NOT NULL")
                .fetch_one(&mut *conn)
                .await
                .expect("look for the thread phases table");
        if phases_exist {
            sqlx::query(
                "INSERT INTO thread_phases (workspace_id, thread_id, number, sealed) \
                 VALUES ($1::uuid, $2::uuid, 2, true)",
            )
            .bind(workspace)
            .bind(&thread_id)
            .execute(&mut *conn)
            .await
            .expect("insert a thread phase");
        }
        sqlx::query(
            "INSERT INTO posts (workspace_id, thread_id, phase, type, summary, body, author) \
             VALUES ($1::uuid, $2::uuid, 1, 'note', 'deletion test', 'deletion test', $3::jsonb)",
        )
        .bind(workspace)
        .bind(&thread_id)
        .bind(r#"{"run":"run-1","role":"writer","model":null,"harness":null}"#)
        .execute(&mut *conn)
        .await
        .expect("insert a post");

        // US-034: a directive of the human and the record that an agent
        // identity was given it. The receipts are seeded as soon as their
        // table exists (tests/posts.rs pins its name and the columns without a
        // default), for the same reason as above.
        let receipts_exist: bool =
            sqlx::query_scalar("SELECT to_regclass('public.directive_receipts') IS NOT NULL")
                .fetch_one(&mut *conn)
                .await
                .expect("look for the directive receipts table");
        if receipts_exist {
            let post_id: String = sqlx::query_scalar(
                "INSERT INTO posts (workspace_id, thread_id, phase, type, summary, body, author) \
                 VALUES ($1::uuid, $2::uuid, 1, 'directive', 'deletion test', 'deletion test', $3::jsonb) \
                 RETURNING id::text",
            )
            .bind(workspace)
            .bind(&thread_id)
            .bind(r#"{"kind":"human","user_id":null}"#)
            .fetch_one(&mut *conn)
            .await
            .expect("insert a directive");
            sqlx::query(
                "INSERT INTO directive_receipts (workspace_id, post_id, agent) \
                 VALUES ($1::uuid, $2::uuid, $3::jsonb)",
            )
            .bind(workspace)
            .bind(&post_id)
            .bind(r#"{"run":"run-1","role":"writer"}"#)
            .execute(&mut *conn)
            .await
            .expect("insert a directive receipt");
        }

        // US-037: a decision for the thread and the change set's link to it.
        // Both are seeded as soon as their tables exist (tests/decisions.rs
        // pins their names and the columns without a default), for the same
        // reason as above.
        let decisions_exist: bool =
            sqlx::query_scalar("SELECT to_regclass('public.decisions') IS NOT NULL")
                .fetch_one(&mut *conn)
                .await
                .expect("look for the decisions table");
        if decisions_exist {
            let decision_id: String = sqlx::query_scalar(
                "INSERT INTO decisions (workspace_id, thread_id, content, author) \
                 VALUES ($1::uuid, $2::uuid, $3::jsonb, $4::jsonb) RETURNING id::text",
            )
            .bind(workspace)
            .bind(&thread_id)
            .bind(
                r#"{"direction":"deletion test","chosen":"deletion test","criteria":["deletion test"],"options":[{"name":"deletion test","scores":{"deletion test":1},"lost_because":null}],"assumptions":[],"open_questions":[],"dissent":[],"posts":[]}"#,
            )
            .bind(r#"{"kind":"agent","run":"run-1","role":"writer","model":null,"harness":null}"#)
            .fetch_one(&mut *conn)
            .await
            .expect("insert a decision");
            let links_exist: bool =
                sqlx::query_scalar("SELECT to_regclass('public.changeset_decisions') IS NOT NULL")
                    .fetch_one(&mut *conn)
                    .await
                    .expect("look for the change set decisions table");
            if links_exist {
                sqlx::query(
                    "INSERT INTO changeset_decisions (workspace_id, changeset_id, decision_id) \
                     VALUES ($1::uuid, $2::uuid, $3::uuid)",
                )
                .bind(workspace)
                .bind(&changeset_id)
                .bind(&decision_id)
                .execute(&mut *conn)
                .await
                .expect("insert a change set's link to a decision");
            }
        }
    }

    // US-059: a feature of the project. Seeded as soon as the table exists
    // (tests/features.rs pins its name and the columns without a default), for
    // the same reason as above.
    let features_exist: bool =
        sqlx::query_scalar("SELECT to_regclass('public.features') IS NOT NULL")
            .fetch_one(&mut *conn)
            .await
            .expect("look for the features table");
    if features_exist {
        sqlx::query(
            "INSERT INTO features (workspace_id, project_id, name) \
             VALUES ($1::uuid, $2::uuid, 'deletion test')",
        )
        .bind(workspace)
        .bind(&project_id)
        .execute(&mut *conn)
        .await
        .expect("insert a feature");
    }
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
