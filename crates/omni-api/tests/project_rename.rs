//! US-11: `PATCH /api/v1/w/{workspace_id}/projects/{project_id}` renames a
//! project for sessions and API keys. Run with `--features stack-tests` in the
//! Compose test container, which reaches Postgres at `postgres:5432` and RustFS
//! at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed workspace Alpha with one member
//! (alice), mint an API key for Alpha through the API and serve the API's
//! router on a local port.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us11_project_rename";
const TEST_BUCKET: &str = "omni-us11-project-rename";

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
    // The password that the stack's API gives omni_api (docker-compose.yml). The
    // role is cluster-wide, so the scratch database must use the same one.
    std::env::var("OMNI_API_DB_PASSWORD").unwrap_or_else(|_| "omni-api-dev".to_string())
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

fn config() -> Config {
    let admin = admin_options();
    let url = admin_url();
    let (server, _database) = url.rsplit_once('/').expect("OMNI_ADMIN_DATABASE_URL");
    Config {
        bind: "127.0.0.1:0".to_string(),
        database_url: format!(
            "postgres://omni_api:{}@{}:{}/{TEST_DB}",
            api_password(),
            admin.get_host(),
            admin.get_port()
        ),
        admin_database_url: format!("{server}/{TEST_DB}"),
        api_db_password: api_password(),
        s3_endpoint: s3_endpoint(),
        s3_access_key: s3_access_key(),
        s3_secret_key: s3_secret_key(),
        s3_bucket: TEST_BUCKET.to_string(),
    }
}

fn s3_region() -> Region {
    Region::Custom {
        region: "us-east-1".to_string(),
        endpoint: s3_endpoint(),
    }
}

fn s3_credentials() -> Credentials {
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
    Bucket::new(TEST_BUCKET, s3_region(), s3_credentials())
        .expect("bucket settings")
        .with_path_style()
}

async fn ensure_bucket() {
    let bucket = bucket();
    if !bucket.exists().await.unwrap_or(false) {
        Bucket::create_with_path_style(
            TEST_BUCKET,
            s3_region(),
            s3_credentials(),
            BucketConfiguration::default(),
        )
        .await
        .unwrap_or_else(|e| panic!("cannot create the scratch bucket: {e}"));
    }
}

async fn admin_connection() -> PgConnection {
    PgConnection::connect_with(&admin_options().database(TEST_DB))
        .await
        .expect("connect to the scratch database as the admin role")
}

struct Fixture {
    workspace: String,
    alice_token: String,
    key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us11-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
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
            omni_api::migrate(&config(), 30)
                .await
                .expect("apply the migrations to the scratch database");
            ensure_bucket().await;
        });
        let workspace = block(async {
            let mut conn = admin_connection().await;
            let mut text = async |sql: &str, binds: &[&str]| -> String {
                let mut query = sqlx::query_scalar::<_, String>(sql);
                for bind in binds {
                    query = query.bind(*bind);
                }
                query.fetch_one(&mut conn).await.expect("seed")
            };
            let workspace = text(
                "insert into workspaces (name) values ('Alpha') returning id::text",
                &[],
            )
            .await;
            let user = text(
                "insert into users (email) values ('alice@example.test') returning id::text",
                &[],
            )
            .await;
            text(
                "insert into sessions (user_id, token_hash, expires_at) \
                 values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
                 returning id::text",
                &[&user, &session_token("alice")],
            )
            .await;
            text(
                "insert into memberships (workspace_id, user_id) \
                 values ($1::uuid, $2::uuid) returning id::text",
                &[&workspace, &user],
            )
            .await;
            workspace
        });
        let server = TestServer::start(&config());
        let response = call(
            &server.client,
            "POST",
            &format!("/api/v1/w/{workspace}/api-keys"),
            &Auth::Session(&session_token("alice")),
            Some(&json!({})),
        );
        assert_eq!(response.status, 201, "mint a key: {}", response.text());
        let key = response.json()["secret"]
            .as_str()
            .expect("the new key has a secret")
            .to_string();
        Fixture {
            workspace,
            alice_token: session_token("alice"),
            key,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

/// Who sends a request: an agent (an API key) or the human (a session cookie).
#[derive(Clone, Copy)]
enum Auth<'a> {
    Agent(&'a str),
    Session(&'a str),
}

fn agent(f: &Fixture) -> Auth<'_> {
    Auth::Agent(&f.key)
}

fn human(f: &Fixture) -> Auth<'_> {
    Auth::Session(&f.alice_token)
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let headers: Vec<(&str, String)> = match auth {
        Auth::Agent(key) => vec![
            ("Authorization", format!("Bearer {key}")),
            ("X-Omni-Run", "run-11".to_string()),
            ("X-Omni-Role", "writer".to_string()),
            ("X-Omni-Model", "test-model".to_string()),
            ("X-Omni-Harness", "test-harness".to_string()),
        ],
        Auth::Session(token) => vec![("Cookie", cookie(token))],
    };
    let headers: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let body = body.map(Value::to_string);
    client.request(method, path, &headers, body.as_deref())
}

fn must(response: Response, status: u16, what: &str) -> Value {
    assert_eq!(response.status, status, "{what}: {}", response.text());
    response.json()
}

fn assert_error(response: &Response, status: u16, code: &str) {
    assert_eq!(response.status, status, "body: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": code }),
        "the error body carries the code and nothing else"
    );
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn new_project(f: &Fixture, client: &Client, name: &str) -> Value {
    must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.workspace),
            &agent(f),
            Some(&json!({ "name": name })),
        ),
        201,
        "create a project",
    )["project"]
        .clone()
}

fn rename(f: &Fixture, client: &Client, auth: &Auth, project: &str, body: &Value) -> Response {
    call(
        client,
        "PATCH",
        &format!("/api/v1/w/{}/projects/{project}", f.workspace),
        auth,
        Some(body),
    )
}

fn listed(f: &Fixture, client: &Client) -> Vec<Value> {
    must(
        call(
            client,
            "GET",
            &format!("/api/v1/w/{}/projects", f.workspace),
            &agent(f),
            None,
        ),
        200,
        "list the projects",
    )["projects"]
        .as_array()
        .expect("a list of projects")
        .clone()
}

fn name_in_list(f: &Fixture, client: &Client, id: &str) -> Option<String> {
    listed(f, client)
        .iter()
        .find(|p| p["id"] == id)
        .map(|p| p["name"].as_str().expect("a name").to_string())
}

fn get_ok(f: &Fixture, client: &Client, project: &str, rest: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("/api/v1/w/{}/projects/{project}{rest}", f.workspace),
            &agent(f),
            None,
        ),
        200,
        rest,
    )
}

#[test]
fn renaming_a_project_trims_the_name_and_keeps_its_id_and_contents() {
    let (f, client) = ready();
    let old = unique("Shop");
    let project = new_project(f, client, &old);
    let id = project["id"].as_str().unwrap().to_string();
    let feature = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects/{id}/features", f.workspace),
            &agent(f),
            Some(&json!({ "name": "Checkout", "description": "Pay" })),
        ),
        201,
        "create a feature",
    )["feature"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let new_name = unique("Shop v2");

    for auth in [agent(f), human(f)] {
        let body = json!({ "name": format!("  {new_name} ") });
        let renamed = must(rename(f, client, &auth, &id, &body), 200, "rename");
        assert_eq!(
            renamed,
            json!({ "project": {
                "id": id,
                "name": new_name,
                "created_at": project["created_at"],
            } })
        );
        assert_eq!(name_in_list(f, client, &id), Some(new_name.clone()));
        let shown = get_ok(f, client, &id, &format!("/features/{feature}"));
        assert_eq!(shown["feature"]["name"], "Checkout");
        // Renaming to the name it already has is a 200 that changes nothing.
        let again = must(rename(f, client, &auth, &id, &body), 200, "same name");
        assert_eq!(again, renamed);
    }
}

#[test]
fn a_refused_project_rename_changes_nothing() {
    let (f, client) = ready();
    let taken = unique("Shop");
    new_project(f, client, &taken);
    let name = unique("Blog");
    let id = new_project(f, client, &name)["id"]
        .as_str()
        .unwrap()
        .to_string();

    for body in [json!({ "name": "  " }), json!({}), json!({ "name": 7 })] {
        assert_error(
            &rename(f, client, &agent(f), &id, &body),
            400,
            "invalid_request",
        );
        assert_eq!(name_in_list(f, client, &id), Some(name.clone()));
    }
    assert_error(
        &rename(f, client, &agent(f), &id, &json!({ "name": taken })),
        409,
        "name_taken",
    );
    assert_eq!(name_in_list(f, client, &id), Some(name.clone()));

    // The comparison is case-sensitive: the lowercased name is free.
    let lower = taken.to_lowercase();
    let renamed = must(
        rename(f, client, &agent(f), &id, &json!({ "name": lower })),
        200,
        "rename to the lowercased name",
    );
    assert_eq!(renamed["project"]["name"], lower.as_str());
}

#[test]
fn an_unknown_project_gets_404_before_the_blank_name_is_checked() {
    let (f, client) = ready();
    let blank = json!({ "name": "  " });

    assert_error(
        &rename(f, client, &agent(f), "not-a-uuid", &blank),
        404,
        "not_found",
    );
    assert_error(
        &rename(
            f,
            client,
            &agent(f),
            "01900000-0000-7000-8000-000000000000",
            &blank,
        ),
        404,
        "not_found",
    );
}

/// The `(seq, data)` of the `project.renamed` events of a project, oldest first,
/// with the project's `last_event_seq`.
fn rename_events(project: &str) -> (Vec<(i64, Value)>, i64) {
    block(async {
        let mut conn = admin_connection().await;
        let rows = sqlx::query_as::<_, (i64, String)>(
            "select seq, data::jsonb::text from live_events \
             where project_id = $1::uuid and event = 'project.renamed' order by seq",
        )
        .bind(project)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the live events: {e}"));
        let last: i64 =
            sqlx::query_scalar("select last_event_seq from projects where id = $1::uuid")
                .bind(project)
                .fetch_one(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("read last_event_seq: {e}"));
        let events = rows
            .into_iter()
            .map(|(seq, data)| {
                (
                    seq,
                    serde_json::from_str(&data).expect("event data is JSON"),
                )
            })
            .collect();
        (events, last)
    })
}

#[test]
fn a_project_rename_records_project_renamed_only_when_the_name_changes() {
    let (f, client) = ready();
    let name = unique("Shop");
    let id = new_project(f, client, &name)["id"]
        .as_str()
        .unwrap()
        .to_string();
    let new_name = unique("Shop v2");

    // Refused renames and a rename to the current name record nothing.
    rename(f, client, &agent(f), &id, &json!({ "name": " " }));
    must(
        rename(f, client, &agent(f), &id, &json!({ "name": name })),
        200,
        "same name",
    );
    assert!(rename_events(&id).0.is_empty());

    must(
        rename(f, client, &human(f), &id, &json!({ "name": new_name })),
        200,
        "rename",
    );
    let (events, last) = rename_events(&id);
    assert_eq!(
        events,
        vec![(last, json!({ "project_id": id, "name": new_name }))]
    );
}
