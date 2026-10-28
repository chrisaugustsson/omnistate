//! US-011: tenant isolation enforced by Postgres itself. Run with
//! `--features stack-tests` in the Compose test container, which reaches
//! Postgres at `postgres:5432`.
//!
//! The tests apply `migrations/` to a scratch database with sqlx's migrator,
//! seed it as the admin role (a superuser, which skips row-level security),
//! and then act as the API role (`omni_api`) with `SET ROLE`. The schema these
//! tests pin down:
//!
//! - Global tables: `users` (`email`), `login_identities`, `sessions`. Tenant
//!   tables: `workspaces` (`name`, scoped by `id`), `memberships`
//!   (`workspace_id`, `user_id`) and `api_keys` (`workspace_id`, `prefix`,
//!   `key_hash`, `created_by_membership_id`, which is nullable).
//! - Every table has a `uuid` column `id` with a default that generates a
//!   UUIDv7, and the database refuses an explicit id that is not a UUIDv7.
//!   Every other column the tests do not name is nullable or has a default.
//! - `memberships.user_id` references `users (id)`.
//! - `resolve_api_key(prefix)` returns the matching `api_keys` rows with at
//!   least the columns `id` and `workspace_id`, `user_memberships(user_id)`
//!   the user's `memberships` rows with at least `id` and `workspace_id`.
//! - The API role is `omni_api`: it can log in, may SELECT, INSERT and UPDATE
//!   the tenant tables, and may not DELETE from or TRUNCATE them. The
//!   append-only tables (US-024, FR-18) are the exception to UPDATE: the
//!   operation log `draft_ops`, the `versions` and the `node_index` exist, and
//!   the API role may only SELECT and INSERT them.

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use sqlx::{Connection, Row};
use std::future::Future;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

const TEST_DB: &str = "omni_us011_tenancy";
const API_ROLE: &str = "omni_api";
const GLOBAL_TABLES: [&str; 3] = ["users", "login_identities", "sessions"];
const TENANT_TABLES: [&str; 3] = ["workspaces", "memberships", "api_keys"];
/// The tenant tables whose rows are never changed: the API role may not UPDATE
/// them (US-024), the chunk index of US-030, the forum's posts (US-033), the
/// links of change sets to the decisions they carry out (US-037), the
/// replies to comments (US-038) and the log of the writes to docs (US-102).
const APPEND_ONLY_TABLES: [&str; 8] = [
    "draft_ops",
    "versions",
    "node_index",
    "chunks",
    "posts",
    "changeset_decisions",
    "comment_replies",
    "doc_changes",
];
/// A UUIDv4, which the database must refuse as a record ID.
const V4_ID: &str = "f47ac10b-58cc-4372-a567-0e02b2c3d479";

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

/// The admin role's connection options: the Postgres superuser of the Compose
/// stack, overridable with OMNI_ADMIN_DATABASE_URL.
fn admin_options() -> PgConnectOptions {
    let url = std::env::var("OMNI_ADMIN_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://omni:omni-dev@postgres:5432/omni".to_string());
    PgConnectOptions::from_str(&url).expect("OMNI_ADMIN_DATABASE_URL")
}

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
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

/// The scratch database with `migrations/` applied, created once per test
/// run. Call it outside of `block`: it runs its own runtime.
fn test_db() -> &'static PgConnectOptions {
    static DB: OnceLock<PgConnectOptions> = OnceLock::new();
    DB.get_or_init(|| {
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

            let options = admin_options().database(TEST_DB);
            let mut conn = PgConnection::connect_with(&options)
                .await
                .expect("connect to the scratch database");
            let migrator = Migrator::new(migrations_dir())
                .await
                .expect("read migrations/");
            migrator
                .run(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("applying migrations/ failed: {e}"));
            options
        })
    })
}

async fn admin_connection(db: &PgConnectOptions) -> PgConnection {
    PgConnection::connect_with(db)
        .await
        .expect("connect to the scratch database as the admin role")
}

async fn assert_api_role_exists(conn: &mut PgConnection) {
    let roles: i64 = sqlx::query_scalar("select count(*) from pg_roles where rolname = $1")
        .bind(API_ROLE)
        .fetch_one(&mut *conn)
        .await
        .expect("query pg_roles");
    assert_eq!(
        roles, 1,
        "the migrations must create the API role {API_ROLE}"
    );
}

/// A connection that acts as the API role, as the API's own connections do.
async fn api_connection(db: &PgConnectOptions) -> PgConnection {
    let mut conn = admin_connection(db).await;
    assert_api_role_exists(&mut conn).await;
    sqlx::query(&format!("SET ROLE {API_ROLE}"))
        .execute(&mut conn)
        .await
        .expect("SET ROLE to the API role");
    conn
}

fn sqlstate(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .and_then(|e| e.code().map(|c| c.into_owned()))
        .unwrap_or_default()
}

fn scope_column(table: &str) -> &'static str {
    if table == "workspaces" {
        "id"
    } else {
        "workspace_id"
    }
}

async fn scalar_i64(conn: &mut PgConnection, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(&mut *conn)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

async fn strings(conn: &mut PgConnection, sql: &str) -> Vec<String> {
    sqlx::query_scalar(sql)
        .fetch_all(&mut *conn)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// Sets `app.workspace_id` for the current transaction, as the API does.
async fn set_workspace(conn: &mut PgConnection, workspace_id: &str) {
    sqlx::query("select set_config('app.workspace_id', $1, true)")
        .bind(workspace_id)
        .execute(&mut *conn)
        .await
        .expect("set app.workspace_id");
}

async fn count_rows(conn: &mut PgConnection, table: &str) -> i64 {
    scalar_i64(conn, &format!("select count(*) from {table}")).await
}

/// The base tables of the public schema.
async fn public_tables(conn: &mut PgConnection) -> Vec<String> {
    strings(
        conn,
        "select table_name::text from information_schema.tables \
         where table_schema = 'public' and table_type = 'BASE TABLE' \
         and table_name <> '_sqlx_migrations' order by table_name",
    )
    .await
}

async fn assert_tables_exist(conn: &mut PgConnection) {
    let tables = public_tables(conn).await;
    for table in GLOBAL_TABLES.iter().chain(TENANT_TABLES.iter()) {
        assert!(
            tables.iter().any(|t| t == table),
            "the migrations must create table {table}; the public schema has {tables:?}"
        );
    }
}

/// Every tenant table: the three the story names plus any other table that has
/// a `workspace_id` column.
async fn tenant_tables(conn: &mut PgConnection) -> Vec<String> {
    assert_tables_exist(conn).await;
    let mut tables = strings(
        conn,
        "select distinct table_name::text from information_schema.columns \
         where table_schema = 'public' and column_name = 'workspace_id' \
         and table_name <> '_sqlx_migrations'",
    )
    .await;
    tables.push("workspaces".to_string());
    tables.sort();
    tables.dedup();
    tables
}

async fn new_id(conn: &mut PgConnection, sql: &str) -> String {
    sqlx::query_scalar(sql)
        .fetch_one(&mut *conn)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

async fn new_user(conn: &mut PgConnection) -> String {
    new_id(
        conn,
        &format!(
            "insert into users (email) values ('u{}@example.test') returning id::text",
            unique_suffix()
        ),
    )
    .await
}

async fn new_workspace(conn: &mut PgConnection) -> String {
    new_id(
        conn,
        &format!(
            "insert into workspaces (name) values ('w{}') returning id::text",
            unique_suffix()
        ),
    )
    .await
}

async fn new_membership(conn: &mut PgConnection, workspace: &str, user: &str) -> String {
    new_id(
        conn,
        &format!(
            "insert into memberships (workspace_id, user_id) \
             values ('{workspace}', '{user}') returning id::text"
        ),
    )
    .await
}

async fn new_api_key(conn: &mut PgConnection, workspace: &str, prefix: &str) -> String {
    new_id(
        conn,
        &format!(
            "insert into api_keys (workspace_id, prefix, key_hash) \
             values ('{workspace}', '{prefix}', 'abc{}') returning id::text",
            unique_suffix()
        ),
    )
    .await
}

struct Seed {
    ws_a: String,
    ws_b: String,
    user_in_both: String,
    user_in_none: String,
    prefix_a: String,
    prefix_b: String,
}

/// Two workspaces, each with a membership and an API key, seeded as the admin
/// role. Call it outside of `block`.
fn seed() -> &'static Seed {
    static SEED: OnceLock<Seed> = OnceLock::new();
    let db = test_db();
    SEED.get_or_init(|| {
        block(async {
            let mut conn = admin_connection(db).await;
            assert_tables_exist(&mut conn).await;
            assert_api_role_exists(&mut conn).await;
            let ws_a = new_workspace(&mut conn).await;
            let ws_b = new_workspace(&mut conn).await;
            let user_in_both = new_user(&mut conn).await;
            let user_in_none = new_user(&mut conn).await;
            let prefix_a = format!("omnia{}", unique_suffix());
            let prefix_b = format!("omnib{}", unique_suffix());
            new_membership(&mut conn, &ws_a, &user_in_both).await;
            new_membership(&mut conn, &ws_b, &user_in_both).await;
            new_api_key(&mut conn, &ws_a, &prefix_a).await;
            new_api_key(&mut conn, &ws_b, &prefix_b).await;
            Seed {
                ws_a,
                ws_b,
                user_in_both,
                user_in_none,
                prefix_a,
                prefix_b,
            }
        })
    })
}

// ---------------------------------------------------------------------------
// Tables and record IDs
// ---------------------------------------------------------------------------

#[test]
fn tenant_tables_have_a_not_null_workspace_id_and_a_unique_workspace_id_id_key() {
    let db = test_db();
    block(async {
        let mut conn = admin_connection(db).await;
        for table in tenant_tables(&mut conn).await {
            if table != "workspaces" {
                let row = sqlx::query(
                    "select data_type::text, is_nullable::text from information_schema.columns \
                     where table_schema = 'public' and table_name = $1 and column_name = 'workspace_id'",
                )
                .bind(&table)
                .fetch_one(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("{table}.workspace_id: {e}"));
                assert_eq!(row.get::<String, _>(0), "uuid", "{table}.workspace_id type");
                assert_eq!(
                    row.get::<String, _>(1),
                    "NO",
                    "{table}.workspace_id NOT NULL"
                );
            }
            let key = if table == "workspaces" {
                "(id)"
            } else {
                "(workspace_id, id)"
            };
            let indexes = strings(
                &mut conn,
                &format!(
                    "select indexdef from pg_indexes \
                     where schemaname = 'public' and tablename = '{table}'"
                ),
            )
            .await;
            assert!(
                indexes
                    .iter()
                    .any(|d| d.starts_with("CREATE UNIQUE INDEX") && d.ends_with(key)),
                "{table} needs a unique key on {key}; its indexes are {indexes:?}"
            );
        }
    });
}

#[test]
fn every_table_has_a_uuid_id_that_defaults_to_a_current_uuid_v7() {
    let db = test_db();
    block(async {
        let mut conn = admin_connection(db).await;
        assert_tables_exist(&mut conn).await;
        for table in public_tables(&mut conn).await {
            let row = sqlx::query(
                "select data_type::text, is_nullable::text, column_default::text \
                 from information_schema.columns \
                 where table_schema = 'public' and table_name = $1 and column_name = 'id'",
            )
            .bind(&table)
            .fetch_optional(&mut conn)
            .await
            .expect("query columns")
            .unwrap_or_else(|| panic!("{table} has no id column"));
            assert_eq!(row.get::<String, _>(0), "uuid", "{table}.id type");
            assert_eq!(row.get::<String, _>(1), "NO", "{table}.id NOT NULL");
            let default = row
                .get::<Option<String>, _>(2)
                .unwrap_or_else(|| panic!("{table}.id has no default"));

            let generated: String = sqlx::query_scalar(&format!("select ({default})::text"))
                .fetch_one(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("{table}.id default {default}: {e}"));
            let hex: String = generated.chars().filter(|c| *c != '-').collect();
            assert_eq!(hex.len(), 32, "{table}.id default {generated}");
            assert_eq!(
                &hex[12..13],
                "7",
                "{table}.id default is not a UUIDv7: {generated}"
            );
            assert!(
                "89ab".contains(&hex[16..17]),
                "{table}.id default has the wrong variant: {generated}"
            );
            // The first 48 bits of a UUIDv7 are the Unix time in milliseconds.
            let embedded = i64::from_str_radix(&hex[..12], 16).unwrap();
            let now = scalar_i64(
                &mut conn,
                "select (extract(epoch from clock_timestamp()) * 1000)::bigint",
            )
            .await;
            assert!(
                (now - embedded).abs() < 60_000,
                "{table}.id default {generated} does not carry the current time"
            );
        }
    });
}

#[test]
fn the_database_refuses_a_record_id_that_is_not_a_uuid_v7() {
    let db = test_db();
    block(async {
        let mut conn = admin_connection(db).await;
        assert_tables_exist(&mut conn).await;
        let user = new_user(&mut conn).await;
        let workspace = new_workspace(&mut conn).await;
        let membership = new_membership(&mut conn, &workspace, &user).await;
        let rejected = [
            format!("insert into users (id, email) values ('{V4_ID}', 'v4@example.test')"),
            format!("insert into workspaces (id, name) values ('{V4_ID}', 'v4')"),
            format!(
                "insert into memberships (id, workspace_id, user_id) \
                 values ('{V4_ID}', '{workspace}', '{user}')"
            ),
            format!(
                "insert into api_keys (id, workspace_id, prefix, key_hash) \
                 values ('{V4_ID}', '{workspace}', 'v4prefix', 'abc123')"
            ),
        ];
        for sql in rejected {
            let error = sqlx::query(&sql)
                .execute(&mut conn)
                .await
                .expect_err(&format!("accepted a UUIDv4 record ID: {sql}"));
            assert_eq!(sqlstate(&error), "23514", "{sql}: {error}");
        }

        // A random UUID is not a UUIDv7 either, but a UUIDv7 is accepted when
        // it is given explicitly.
        let random = sqlx::query(&format!(
            "insert into memberships (id, workspace_id, user_id) \
             values (gen_random_uuid(), '{workspace}', '{user}')"
        ))
        .execute(&mut conn)
        .await;
        assert_eq!(
            sqlstate(&random.expect_err("random UUID accepted")),
            "23514"
        );
        let v7 = new_id(
            &mut conn,
            "select (substr(h, 1, 8) || '-' || substr(h, 9, 4) || '-7' || substr(u, 16, 3) || substr(u, 19)) \
             from (select lpad(to_hex((extract(epoch from clock_timestamp()) * 1000)::bigint), 12, '0') as h, \
                          gen_random_uuid()::text as u) t",
        )
        .await;
        sqlx::query(&format!(
            "insert into workspaces (id, name) values ('{v7}', 'explicit v7')"
        ))
        .execute(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("an explicit UUIDv7 {v7} was refused: {e}"));
        assert!(!membership.is_empty());
    });
}

// ---------------------------------------------------------------------------
// Foreign keys
// ---------------------------------------------------------------------------

struct ForeignKey {
    table: String,
    referenced: String,
    columns: Vec<String>,
    referenced_columns: Vec<String>,
}

async fn foreign_keys(conn: &mut PgConnection) -> Vec<ForeignKey> {
    let rows = sqlx::query(
        "select t.relname::text, r.relname::text, \
           (select array_agg(a.attname::text order by u.ord) \
              from unnest(c.conkey) with ordinality u(attnum, ord) \
              join pg_attribute a on a.attrelid = c.conrelid and a.attnum = u.attnum), \
           (select array_agg(a.attname::text order by u.ord) \
              from unnest(c.confkey) with ordinality u(attnum, ord) \
              join pg_attribute a on a.attrelid = c.confrelid and a.attnum = u.attnum) \
         from pg_constraint c \
         join pg_class t on t.oid = c.conrelid \
         join pg_class r on r.oid = c.confrelid \
         where c.contype = 'f' and c.connamespace = 'public'::regnamespace",
    )
    .fetch_all(&mut *conn)
    .await
    .expect("query foreign keys");
    rows.iter()
        .map(|row| ForeignKey {
            table: row.get(0),
            referenced: row.get(1),
            columns: row.get(2),
            referenced_columns: row.get(3),
        })
        .collect()
}

#[test]
fn foreign_keys_between_tenant_tables_include_workspace_id() {
    let db = test_db();
    block(async {
        let mut conn = admin_connection(db).await;
        let tenant = tenant_tables(&mut conn).await;
        let keys = foreign_keys(&mut conn).await;
        for key in keys
            .iter()
            .filter(|k| tenant.contains(&k.table) && tenant.contains(&k.referenced))
        {
            let description = format!(
                "{} {:?} -> {} {:?}",
                key.table, key.columns, key.referenced, key.referenced_columns
            );
            if key.referenced == "workspaces" {
                assert_eq!(key.columns, ["workspace_id"], "{description}");
                assert_eq!(key.referenced_columns, ["id"], "{description}");
            } else {
                let pairs: Vec<_> = key.columns.iter().zip(&key.referenced_columns).collect();
                assert!(
                    pairs
                        .iter()
                        .any(|(a, b)| *a == "workspace_id" && *b == "workspace_id"),
                    "foreign key between tenant tables without workspace_id: {description}"
                );
            }
        }
        let has = |table: &str, referenced: &str| {
            keys.iter()
                .any(|k| k.table == table && k.referenced == referenced)
        };
        assert!(
            has("memberships", "workspaces"),
            "memberships -> workspaces"
        );
        assert!(has("api_keys", "workspaces"), "api_keys -> workspaces");
        assert!(has("memberships", "users"), "memberships -> users");
        assert!(has("api_keys", "memberships"), "api_keys -> memberships");
    });
}

#[test]
fn the_database_rejects_a_row_that_references_another_workspaces_row() {
    let db = test_db();
    block(async {
        let mut conn = admin_connection(db).await;
        assert_tables_exist(&mut conn).await;
        let user = new_user(&mut conn).await;
        let a = new_workspace(&mut conn).await;
        let b = new_workspace(&mut conn).await;
        let membership_in_a = new_membership(&mut conn, &a, &user).await;
        let prefix = format!("fk{}", unique_suffix());

        // The same membership is fine for a key of its own workspace.
        sqlx::query(&format!(
            "insert into api_keys (workspace_id, prefix, key_hash, created_by_membership_id) \
             values ('{a}', '{prefix}a', 'abc1', '{membership_in_a}')"
        ))
        .execute(&mut conn)
        .await
        .expect("a key that references a membership of its own workspace");

        // A key of workspace B must not reference workspace A's membership.
        let error = sqlx::query(&format!(
            "insert into api_keys (workspace_id, prefix, key_hash, created_by_membership_id) \
             values ('{b}', '{prefix}b', 'abc2', '{membership_in_a}')"
        ))
        .execute(&mut conn)
        .await
        .expect_err("the database accepted a reference to another workspace's row");
        assert_eq!(sqlstate(&error), "23503", "{error}");

        // Nor can a row be moved to another workspace than its parent's.
        let moved = sqlx::query(&format!(
            "update api_keys set workspace_id = '{b}' where prefix = '{prefix}a'"
        ))
        .execute(&mut conn)
        .await
        .expect_err("moved a row away from the workspace of the row it references");
        assert_eq!(sqlstate(&moved), "23503", "{moved}");
    });
}

// ---------------------------------------------------------------------------
// Row-level security policy
// ---------------------------------------------------------------------------

#[test]
fn row_level_security_is_enabled_and_forced_on_every_tenant_table() {
    let db = test_db();
    block(async {
        let mut conn = admin_connection(db).await;
        for table in tenant_tables(&mut conn).await {
            let row = sqlx::query(
                "select relrowsecurity, relforcerowsecurity from pg_class \
                 where oid = ('public.' || $1)::regclass",
            )
            .bind(&table)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("{table}: {e}"));
            assert!(
                row.get::<bool, _>(0),
                "{table}: row-level security is not enabled"
            );
            assert!(
                row.get::<bool, _>(1),
                "{table}: row-level security is not forced"
            );
        }
    });
}

// ---------------------------------------------------------------------------
// The API role
// ---------------------------------------------------------------------------

#[test]
fn the_api_role_can_log_in_but_neither_owns_the_tables_nor_bypasses_row_level_security() {
    let db = test_db();
    block(async {
        let mut conn = admin_connection(db).await;
        assert_tables_exist(&mut conn).await;
        assert_api_role_exists(&mut conn).await;
        let role = sqlx::query(
            "select rolcanlogin, rolsuper, rolbypassrls, rolcreaterole, rolcreatedb, rolreplication \
             from pg_roles where rolname = $1",
        )
        .bind(API_ROLE)
        .fetch_one(&mut conn)
        .await
        .expect("query pg_roles");
        assert!(role.get::<bool, _>(0), "{API_ROLE} must be able to log in");
        for (i, attribute) in [
            "superuser",
            "BYPASSRLS",
            "CREATEROLE",
            "CREATEDB",
            "REPLICATION",
        ]
        .iter()
        .enumerate()
        {
            assert!(
                !role.get::<bool, _>(i + 1),
                "{API_ROLE} must not have {attribute}"
            );
        }

        let owners = sqlx::query(
            "select tablename::text, tableowner::text, pg_has_role($1, tableowner, 'USAGE') \
             from pg_tables where schemaname = 'public'",
        )
        .bind(API_ROLE)
        .fetch_all(&mut conn)
        .await
        .expect("query pg_tables");
        assert!(!owners.is_empty());
        for owner in owners {
            let table: String = owner.get(0);
            let owned_by: String = owner.get(1);
            assert_ne!(owned_by, API_ROLE, "{API_ROLE} owns {table}");
            assert!(
                !owner.get::<bool, _>(2),
                "{API_ROLE} has the privileges of {table}'s owner {owned_by}"
            );
        }
    });
}

#[test]
fn the_api_role_can_read_and_write_but_not_delete_or_truncate_tenant_tables() {
    let db = test_db();
    block(async {
        let mut conn = admin_connection(db).await;
        assert_api_role_exists(&mut conn).await;
        let usage: bool = sqlx::query_scalar("select has_schema_privilege($1, 'public', 'USAGE')")
            .bind(API_ROLE)
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert!(usage, "{API_ROLE} needs USAGE on the public schema");
        for table in tenant_tables(&mut conn).await {
            let append_only = APPEND_ONLY_TABLES.contains(&table.as_str());
            for (privilege, expected) in [
                ("SELECT", true),
                ("INSERT", true),
                ("UPDATE", !append_only),
                ("DELETE", false),
                ("TRUNCATE", false),
            ] {
                let granted: bool = sqlx::query_scalar("select has_table_privilege($1, $2, $3)")
                    .bind(API_ROLE)
                    .bind(format!("public.{table}"))
                    .bind(privilege)
                    .fetch_one(&mut conn)
                    .await
                    .unwrap();
                assert_eq!(
                    granted, expected,
                    "{API_ROLE} {privilege} on {table}: expected {expected}"
                );
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Isolation
// ---------------------------------------------------------------------------

#[test]
fn with_the_workspace_set_to_a_queries_return_no_rows_from_workspace_b() {
    let db = test_db();
    let seed = seed();
    block(async {
        let mut conn = api_connection(db).await;
        let tenant = {
            let mut admin = admin_connection(db).await;
            tenant_tables(&mut admin).await
        };
        for (own, other) in [(&seed.ws_a, &seed.ws_b), (&seed.ws_b, &seed.ws_a)] {
            let mut tx = conn.begin().await.unwrap();
            set_workspace(&mut tx, own).await;
            for table in &tenant {
                let column = scope_column(table);
                let visible = strings(
                    &mut tx,
                    &format!("select distinct {column}::text from {table}"),
                )
                .await;
                if TENANT_TABLES.contains(&table.as_str()) {
                    assert_eq!(&visible, &[own.clone()], "{table} rows visible in {own}");
                } else {
                    assert!(
                        visible.iter().all(|v| v == own),
                        "{table} shows other workspaces: {visible:?}"
                    );
                }
                let others = scalar_i64(
                    &mut tx,
                    &format!("select count(*) from {table} where {column} = '{other}'"),
                )
                .await;
                assert_eq!(others, 0, "{table}: rows of {other} visible in {own}");
            }
        }
    });
}

#[test]
fn rows_of_another_workspace_cannot_be_written_or_changed() {
    let db = test_db();
    let seed = seed();
    block(async {
        let mut conn = api_connection(db).await;

        // A key of the current workspace can be updated: the control case.
        let mut tx = conn.begin().await.unwrap();
        set_workspace(&mut tx, &seed.ws_a).await;
        let own = sqlx::query("update api_keys set prefix = prefix")
            .execute(&mut *tx)
            .await
            .expect("update own keys");
        assert_eq!(own.rows_affected(), 1, "the one key of workspace A");
        let other = sqlx::query(&format!(
            "update api_keys set prefix = prefix where workspace_id = '{}'",
            seed.ws_b
        ))
        .execute(&mut *tx)
        .await
        .expect("update other workspace's keys");
        assert_eq!(other.rows_affected(), 0, "workspace B's key is not visible");
        drop(tx);

        // A row cannot be inserted into, or moved to, another workspace.
        let mut tx = conn.begin().await.unwrap();
        set_workspace(&mut tx, &seed.ws_a).await;
        let error = sqlx::query(&format!(
            "insert into memberships (workspace_id, user_id) values ('{}', '{}')",
            seed.ws_b, seed.user_in_none
        ))
        .execute(&mut *tx)
        .await
        .expect_err("inserted a row into another workspace");
        assert_eq!(sqlstate(&error), "42501", "{error}");
        drop(tx);

        let mut tx = conn.begin().await.unwrap();
        set_workspace(&mut tx, &seed.ws_a).await;
        let error = sqlx::query(&format!(
            "update api_keys set workspace_id = '{}'",
            seed.ws_b
        ))
        .execute(&mut *tx)
        .await
        .expect_err("moved a row into another workspace");
        assert_eq!(sqlstate(&error), "42501", "{error}");
    });
}

#[test]
fn with_no_workspace_set_queries_return_no_rows() {
    let db = test_db();
    let seed = seed();
    block(async {
        let mut conn = api_connection(db).await;
        let tenant = {
            let mut admin = admin_connection(db).await;
            let tables = tenant_tables(&mut admin).await;
            // The seed must be there for "no rows" to mean something.
            for table in TENANT_TABLES {
                assert!(
                    count_rows(&mut admin, table).await >= 2,
                    "{table} is not seeded"
                );
            }
            tables
        };
        assert!(!seed.ws_a.is_empty());

        // Outside of any transaction, and in one that sets nothing.
        for table in &tenant {
            assert_eq!(count_rows(&mut conn, table).await, 0, "{table}: no setting");
        }
        let mut tx = conn.begin().await.unwrap();
        for table in &tenant {
            assert_eq!(
                count_rows(&mut tx, table).await,
                0,
                "{table}: in a transaction"
            );
        }
        // An empty setting is the same as none: no rows and no cast error.
        set_workspace(&mut tx, "").await;
        for table in &tenant {
            assert_eq!(
                count_rows(&mut tx, table).await,
                0,
                "{table}: empty setting"
            );
        }
        drop(tx);

        // Without a workspace nothing can be written either.
        let mut tx = conn.begin().await.unwrap();
        let error = sqlx::query(&format!(
            "insert into memberships (workspace_id, user_id) values ('{}', '{}')",
            seed.ws_a, seed.user_in_none
        ))
        .execute(&mut *tx)
        .await
        .expect_err("inserted a row without a workspace");
        assert_eq!(sqlstate(&error), "42501", "{error}");
    });
}

// ---------------------------------------------------------------------------
// Lookups before the workspace is known
// ---------------------------------------------------------------------------

#[test]
fn only_resolve_api_key_and_user_memberships_are_security_definer_functions() {
    let db = test_db();
    block(async {
        let mut conn = admin_connection(db).await;
        assert_tables_exist(&mut conn).await;
        assert_api_role_exists(&mut conn).await;
        let functions = sqlx::query(
            "select p.proname::text, \
               has_function_privilege($1, p.oid, 'EXECUTE'), \
               exists (select 1 from aclexplode(coalesce(p.proacl, acldefault('f', p.proowner))) a \
                       where a.grantee = 0 and a.privilege_type = 'EXECUTE'), \
               exists (select 1 from unnest(coalesce(p.proconfig, '{}')) c where c like 'search_path=%'), \
               pg_has_role($1, p.proowner, 'USAGE') \
             from pg_proc p \
             where p.pronamespace = 'public'::regnamespace and p.prosecdef \
             order by p.proname",
        )
        .bind(API_ROLE)
        .fetch_all(&mut conn)
        .await
        .expect("query pg_proc");
        let names: Vec<String> = functions.iter().map(|f| f.get(0)).collect();
        assert_eq!(
            names,
            ["resolve_api_key", "user_memberships"],
            "the SECURITY DEFINER functions of the public schema"
        );
        for function in functions {
            let name: String = function.get(0);
            assert!(
                function.get::<bool, _>(1),
                "{API_ROLE} cannot EXECUTE {name}"
            );
            assert!(
                !function.get::<bool, _>(2),
                "{name} can be executed by PUBLIC"
            );
            assert!(
                function.get::<bool, _>(3),
                "{name} must pin its search_path"
            );
            assert!(
                !function.get::<bool, _>(4),
                "{name} must not be owned by the API role"
            );
        }
    });
}

#[test]
fn resolve_api_key_finds_a_key_by_prefix_before_the_workspace_is_known() {
    let db = test_db();
    let seed = seed();
    block(async {
        let mut conn = api_connection(db).await;
        // The key's own table is hidden without a workspace...
        assert_eq!(count_rows(&mut conn, "api_keys").await, 0);

        // ...but the function resolves the key, and only that key.
        for (prefix, workspace) in [(&seed.prefix_a, &seed.ws_a), (&seed.prefix_b, &seed.ws_b)] {
            let rows = sqlx::query(&format!(
                "select workspace_id::text, id::text from resolve_api_key('{prefix}')"
            ))
            .fetch_all(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("resolve_api_key('{prefix}'): {e}"));
            assert_eq!(rows.len(), 1, "resolve_api_key('{prefix}')");
            assert_eq!(&rows[0].get::<String, _>(0), workspace);
        }
        let none = sqlx::query("select id::text from resolve_api_key('no-such-prefix')")
            .fetch_all(&mut conn)
            .await
            .expect("resolve an unknown prefix");
        assert!(none.is_empty(), "an unknown prefix resolves to nothing");

        // Resolving a key does not widen what the connection can read.
        assert_eq!(count_rows(&mut conn, "api_keys").await, 0);
    });
}

#[test]
fn user_memberships_lists_the_memberships_of_a_user_across_workspaces() {
    let db = test_db();
    let seed = seed();
    block(async {
        let mut conn = api_connection(db).await;
        assert_eq!(count_rows(&mut conn, "memberships").await, 0);

        let mut workspaces = strings(
            &mut conn,
            &format!(
                "select workspace_id::text from user_memberships('{}')",
                seed.user_in_both
            ),
        )
        .await;
        workspaces.sort();
        let mut expected = vec![seed.ws_a.clone(), seed.ws_b.clone()];
        expected.sort();
        assert_eq!(workspaces, expected);

        let none = strings(
            &mut conn,
            &format!(
                "select workspace_id::text from user_memberships('{}')",
                seed.user_in_none
            ),
        )
        .await;
        assert!(none.is_empty(), "a user without memberships: {none:?}");
        assert_eq!(count_rows(&mut conn, "memberships").await, 0);
    });
}
