//! US-042: the list of a doc's versions, which the doc viewer's version picker
//! shows, through the API. Run with `--features stack-tests` in the Compose
//! test container, which reaches Postgres at `postgres:5432` and RustFS at
//! `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. The web
//! app's side is web/src/doc-viewer.test.tsx.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; change sets are those of
//! US-016 and US-025, docs and versions those of US-022 and US-024):
//!
//! - `GET /docs/{doc_id}/versions` (both: the human's viewer and agents read
//!   it) answers 200 `{"doc_id", "versions"}`: every saved version of the doc,
//!   in every change set (merged, in review, open or not), the removal of the
//!   doc included, oldest first, and no version of another doc.
//! - Each item carries the `id`, `doc_id`, `changeset_id`, `draft_id`,
//!   `removed`, `created_at` and `agent` of the version, each as `GET
//!   /versions/{version_id}` answers it. The web app needs nothing else of an
//!   item, and an item has no `nodes`.
//! - A doc with no saved version has an empty list. An unknown doc, or a doc of
//!   another project named under this one, is 404 `{"error": "not_found"}`.
//! - The route is in the route registry as `both`, and the isolation suite
//!   (tests/isolation.rs) attacks it like every other route.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us042_doc_versions";
const TEST_BUCKET: &str = "omni-us042-doc-versions";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const VERSIONS_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/versions";

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

async fn ensure_bucket() {
    let region = Region::Custom {
        region: "us-east-1".to_string(),
        endpoint: s3_endpoint(),
    };
    let credentials = Credentials::new(
        Some(&s3_access_key()),
        Some(&s3_secret_key()),
        None,
        None,
        None,
    )
    .expect("storage credentials");
    let bucket = Bucket::new(TEST_BUCKET, region.clone(), credentials.clone())
        .expect("bucket settings")
        .with_path_style();
    if !bucket.exists().await.unwrap_or(false) {
        Bucket::create_with_path_style(
            TEST_BUCKET,
            region,
            credentials,
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
    format!("us042-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

/// Who sends a request: an agent (the API key with a run and role) or the
/// human (a session cookie).
#[derive(Clone, Copy)]
enum Auth<'a> {
    Agent {
        key: &'a str,
        run: &'a str,
        role: &'a str,
    },
    Session(&'a str),
}

impl Auth<'_> {
    fn headers(&self) -> Vec<(&'static str, String)> {
        match self {
            Auth::Agent { key, run, role } => vec![
                ("Authorization", format!("Bearer {key}")),
                ("X-Omni-Run", run.to_string()),
                ("X-Omni-Role", role.to_string()),
                ("X-Omni-Model", "test-model".to_string()),
                ("X-Omni-Harness", "test-harness".to_string()),
            ],
            Auth::Session(token) => vec![("Cookie", cookie(token))],
        }
    }
}

/// The agent that writes the docs: `run-1/author`.
fn author(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-1",
        role: "author",
    }
}

/// Another agent, which edits the doc in a later change set: `run-2/editor`.
fn editor(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-2",
        role: "editor",
    }
}

fn human(f: &Fixture) -> Auth<'_> {
    Auth::Session(&f.alice_token)
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let owned = auth.headers();
    let headers: Vec<(&str, &str)> = owned.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let body = body.map(Value::to_string);
    client.request(method, path, &headers, body.as_deref())
}

fn must(response: Response, status: u16, what: &str) -> Value {
    assert_eq!(response.status, status, "{what}: {}", response.text());
    response.json()
}

fn assert_error(response: &Response, status: u16, code: &str, what: &str) {
    assert_eq!(response.status, status, "{what}: {}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": code }),
        "{what}: the error body carries the code and nothing else"
    );
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn id_of(value: &Value) -> String {
    value["id"]
        .as_str()
        .unwrap_or_else(|| panic!("there is an `id`: {value}"))
        .to_string()
}

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
}

fn changeset_path(f: &Fixture, project: &str, changeset: &str) -> String {
    format!("{}/changesets/{changeset}", project_base(f, project))
}

// ---------------------------------------------------------------------------
// Projects, change sets, docs and versions
// ---------------------------------------------------------------------------

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &author(f),
                Some(&json!({ "name": unique("viewer-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

fn new_changeset(f: &Fixture, client: &Client, auth: &Auth, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/changesets", project_base(f, project)),
                auth,
                Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
            ),
            201,
            "create a change set",
        )["changeset"],
    )
}

fn create_doc(f: &Fixture, client: &Client, project: &str, changeset: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", changeset_path(f, project, changeset)),
                &author(f),
                Some(&json!({ "kind": "html", "pillar": "spec", "title": unique("Checkout") })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

/// `auth` opens the doc's mainline version in the change set.
fn open_doc(f: &Fixture, client: &Client, auth: &Auth, project: &str, changeset: &str, doc: &str) {
    let response = call(
        client,
        "POST",
        &format!("{}/docs/{doc}/open", changeset_path(f, project, changeset)),
        auth,
        None,
    );
    assert!(
        response.status == 200 || response.status == 201,
        "open the doc in the change set: {} {}",
        response.status,
        response.text()
    );
}

/// `auth` inserts one paragraph into the change set's draft of `doc`.
fn insert_paragraph(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    doc: &str,
) {
    must(
        call(
            client,
            "POST",
            &format!("{}/docs/{doc}/ops", changeset_path(f, project, changeset)),
            auth,
            Some(&json!({ "ops": [{
                "op": "insert",
                "parent": null,
                "position": { "after": null },
                "content": format!("<p>{}</p>", unique("Orders ship in one day"))
            }] })),
        ),
        200,
        "edit the draft",
    );
}

/// `auth` saves the change set's draft of `doc` (`versions`) or removes the
/// doc (`remove`); returns the version's ID.
fn save(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    doc: &str,
    action: &str,
) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!(
                    "{}/docs/{doc}/{action}",
                    changeset_path(f, project, changeset)
                ),
                auth,
                Some(&json!({})),
            ),
            201,
            &format!("{action} the doc"),
        )["version"],
    )
}

/// The change set goes to review, and the human approves it, which merges it.
fn merge(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let submitted = must(
        call(
            client,
            "POST",
            &format!("{}/submit", changeset_path(f, project, changeset)),
            &author(f),
            None,
        ),
        200,
        "submit the change set",
    );
    assert_eq!(submitted["changeset"]["status"], "in_review", "{submitted}");
    let approved = must(
        call(
            client,
            "POST",
            &format!("{}/approve", changeset_path(f, project, changeset)),
            &human(f),
            Some(&json!({})),
        ),
        200,
        "approve the change set",
    );
    assert_eq!(approved["changeset"]["status"], "merged", "{approved}");
}

fn versions_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    doc: &str,
) -> Response {
    call(
        client,
        "GET",
        &format!("{}/docs/{doc}/versions", project_base(f, project)),
        auth,
        None,
    )
}

/// The versions of `doc` as `auth` lists them.
fn versions(f: &Fixture, client: &Client, auth: &Auth, project: &str, doc: &str) -> Vec<Value> {
    let answer = must(
        versions_response(f, client, auth, project, doc),
        200,
        "list the doc's versions",
    );
    assert_eq!(answer["doc_id"], doc, "{answer}");
    answer["versions"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer has a list of versions: {answer}"))
        .clone()
}

/// The version as `GET /versions/{version_id}` answers it.
fn version(f: &Fixture, client: &Client, project: &str, version: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/versions/{version}", project_base(f, project)),
            &human(f),
            None,
        ),
        200,
        "read the version",
    )["version"]
        .clone()
}

/// A doc with four versions in three change sets: two saved in a change set
/// that merged, one saved in an open change set and the removal of the doc in
/// another open one; (doc, versions oldest first).
fn doc_with_history(f: &Fixture, client: &Client, project: &str) -> (String, Vec<String>) {
    let first = new_changeset(f, client, &author(f), project);
    let doc = create_doc(f, client, project, &first);
    insert_paragraph(f, client, &author(f), project, &first, &doc);
    let one = save(f, client, &author(f), project, &first, &doc, "versions");
    insert_paragraph(f, client, &author(f), project, &first, &doc);
    let two = save(f, client, &author(f), project, &first, &doc, "versions");
    merge(f, client, project, &first);

    let second = new_changeset(f, client, &editor(f), project);
    open_doc(f, client, &editor(f), project, &second, &doc);
    insert_paragraph(f, client, &editor(f), project, &second, &doc);
    let three = save(f, client, &editor(f), project, &second, &doc, "versions");

    let third = new_changeset(f, client, &editor(f), project);
    open_doc(f, client, &editor(f), project, &third, &doc);
    let removal = save(f, client, &editor(f), project, &third, &doc, "remove");
    (doc, vec![one, two, three, removal])
}

// ---------------------------------------------------------------------------
// The route
// ---------------------------------------------------------------------------

#[test]
fn the_versions_route_is_in_the_registry_for_agents_and_the_human() {
    let entry = registry()
        .into_iter()
        .find(|r| r.method == "GET" && r.path == VERSIONS_ROUTE)
        .unwrap_or_else(|| panic!("the registry has no route GET {VERSIONS_ROUTE}"));
    assert_eq!(entry.access, Access::Both, "GET {VERSIONS_ROUTE}");
}

// ---------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------

#[test]
fn a_doc_lists_every_saved_version_of_every_change_set_oldest_first() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, saved) = doc_with_history(f, client, &project);

    // Another doc of the project has versions of its own.
    let other_changeset = new_changeset(f, client, &author(f), &project);
    let other = create_doc(f, client, &project, &other_changeset);
    insert_paragraph(f, client, &author(f), &project, &other_changeset, &other);
    let foreign = save(
        f,
        client,
        &author(f),
        &project,
        &other_changeset,
        &other,
        "versions",
    );

    let listed = versions(f, client, &human(f), &project, &doc);
    let ids: Vec<String> = listed.iter().map(id_of).collect();
    assert_eq!(
        ids, saved,
        "every version of the doc, the removal included, oldest first: {listed:?}"
    );
    assert!(
        !ids.contains(&foreign),
        "no version of another doc is listed: {listed:?}"
    );
    for item in &listed {
        let read = version(f, client, &project, &id_of(item));
        for field in [
            "id",
            "doc_id",
            "changeset_id",
            "draft_id",
            "removed",
            "created_at",
            "agent",
        ] {
            assert!(item.get(field).is_some(), "an item has `{field}`: {item}");
            assert_eq!(
                item[field], read[field],
                "`{field}` of the item is the version's: {item} against {read}"
            );
        }
        assert!(item.get("nodes").is_none(), "an item has no nodes: {item}");
    }
    assert_eq!(
        listed[3]["removed"], true,
        "the removal is marked: {listed:?}"
    );
    assert_eq!(listed[0]["removed"], false, "{listed:?}");
    assert_eq!(listed[2]["agent"]["role"], "editor", "{listed:?}");
}

#[test]
fn an_agent_lists_the_versions_too_and_a_doc_without_one_has_none() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, saved) = doc_with_history(f, client, &project);

    let ids: Vec<String> = versions(f, client, &author(f), &project, &doc)
        .iter()
        .map(id_of)
        .collect();
    assert_eq!(ids, saved, "an API key reads the same list");

    let changeset = new_changeset(f, client, &author(f), &project);
    let fresh = create_doc(f, client, &project, &changeset);
    insert_paragraph(f, client, &author(f), &project, &changeset, &fresh);
    assert!(
        versions(f, client, &human(f), &project, &fresh).is_empty(),
        "a doc whose draft was never saved has no versions"
    );
}

#[test]
fn an_unknown_doc_and_a_doc_of_another_project_are_not_found() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let elsewhere = new_project(f, client);
    let (foreign, _) = doc_with_history(f, client, &elsewhere);

    assert_error(
        &versions_response(f, client, &human(f), &project, UNKNOWN_ID),
        404,
        "not_found",
        "the versions of a doc that does not exist",
    );
    assert_error(
        &versions_response(f, client, &human(f), &project, &foreign),
        404,
        "not_found",
        "the versions of another project's doc, named under this project",
    );
    assert_error(
        &versions_response(f, client, &human(f), &project, "not-a-uuid"),
        404,
        "not_found",
        "the versions of a doc ID that is no UUID",
    );
}
