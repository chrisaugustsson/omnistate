//! `omni-api setup`: the one-time command that creates the first user, their
//! password login and their workspace. It runs as the database owner, like the
//! migrations, because no login exists yet to run it as.

use crate::password::{PASSWORD_PROVIDER, hash_password};
use crate::{Config, migrate};
use sqlx::{Connection, PgConnection};

/// Arguments of `omni-api setup`, minus the password, which comes from stdin.
pub struct SetupArgs {
    pub email: String,
    pub workspace: String,
}

const MIN_PASSWORD_LEN: usize = 8;

/// Parses `--email <email> --workspace <name> --password-stdin`.
pub fn parse_args(args: &[String]) -> Result<SetupArgs, String> {
    let usage = "usage: omni-api setup --email <email> --workspace <name> --password-stdin";
    let mut email = None;
    let mut workspace = None;
    let mut password_stdin = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--email" => email = args.next().cloned(),
            "--workspace" => workspace = args.next().cloned(),
            "--password-stdin" => password_stdin = true,
            other => return Err(format!("unknown argument {other}\n{usage}")),
        }
    }
    let (Some(email), Some(workspace), true) = (email, workspace, password_stdin) else {
        return Err(usage.to_string());
    };
    let email = email.trim().to_string();
    let workspace = workspace.trim().to_string();
    if !email.contains('@') {
        return Err("the email address is not valid".to_string());
    }
    if workspace.is_empty() {
        return Err("the workspace name is empty".to_string());
    }
    Ok(SetupArgs { email, workspace })
}

/// Applies the migrations, then creates the first user with a password
/// identity, a workspace and the membership that links them. Fails with
/// "already" when any user exists, and then creates nothing.
pub async fn run(config: &Config, args: &SetupArgs, password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!(
            "the password needs at least {MIN_PASSWORD_LEN} characters"
        ));
    }
    let hash = hash_password(password)?;
    migrate(config, 10).await?;

    let mut conn = PgConnection::connect(&config.admin_database_url)
        .await
        .map_err(|e| format!("cannot connect to Postgres: {e}"))?;
    create_first_user(&mut conn, args, &hash)
        .await
        .map_err(|e| match e {
            Failure::Already => "setup was already done: a user exists".to_string(),
            Failure::Database(e) => format!("setup failed: {e}"),
        })
}

enum Failure {
    Already,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for Failure {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e)
    }
}

async fn create_first_user(
    conn: &mut PgConnection,
    args: &SetupArgs,
    password_hash: &str,
) -> Result<(), Failure> {
    let mut tx = conn.begin().await?;
    // Two setup runs at once must not both see an empty users table.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('omni.setup'))")
        .execute(&mut *tx)
        .await?;
    let any_user: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users)")
        .fetch_one(&mut *tx)
        .await?;
    if any_user {
        return Err(Failure::Already);
    }

    let user_id: String =
        sqlx::query_scalar("INSERT INTO users (email) VALUES ($1) RETURNING id::text")
            .bind(&args.email)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(
        "INSERT INTO login_identities (user_id, provider, subject, secret_hash) \
         VALUES ($1::uuid, $2, lower($3), $4)",
    )
    .bind(&user_id)
    .bind(PASSWORD_PROVIDER)
    .bind(&args.email)
    .bind(password_hash)
    .execute(&mut *tx)
    .await?;

    // The tenant tables are protected by row-level security even for their
    // owner, so the workspace gets its id first and the transaction is scoped
    // to it, as the API does for every request.
    let workspace_id: String = sqlx::query_scalar("SELECT uuid_v7()::text")
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('app.workspace_id', $1, true)")
        .bind(&workspace_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO workspaces (id, name) VALUES ($1::uuid, $2)")
        .bind(&workspace_id)
        .bind(&args.workspace)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO memberships (workspace_id, user_id) VALUES ($1::uuid, $2::uuid)")
        .bind(&workspace_id)
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
