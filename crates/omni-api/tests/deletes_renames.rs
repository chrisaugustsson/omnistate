//! US-110: agents delete and rename docs and features. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API (the agent),
//! create a scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; the rest is the green
//! step's choice):
//!
//! - `PATCH /docs/{doc_id}` (agent) takes `{"title"}` and answers 200 `{"doc":
//!   Doc}`, the doc with its new title and the same ID. A missing or blank
//!   title is 400 `invalid_request`. The rename adds one `doc_changes` row
//!   whose `operations` is `[{"op": "rename", "title": <the new title>}]`,
//!   with the agent's identity and the head unchanged (`head_before` and
//!   `head_after` are both the doc's head). The doc's comments stay on it.
//! - `DELETE /docs/{doc_id}` (agent) answers 204. The delete is soft: the
//!   `docs` row stays with `deleted_at` set, and one `doc_changes` row whose
//!   `operations` is `[{"op": "delete"}]`, with the agent's identity and
//!   `head_before` the doc's head, records it. From then on the doc is in no
//!   list (`GET /docs` with or without filters, the project home) and every
//!   route about it answers 404 `not_found`: its reads, its writes, its
//!   comments, a rename and a second delete. Its comments go with it: they are
//!   in no comment list and their reads answer 404.
//! - `PATCH /features/{feature_id}` (agent) takes `{"name"}` and answers 200
//!   `{"feature": Feature}` with the same ID. A missing or blank name is 400
//!   `invalid_request`, and the name of another feature of the project,
//!   ignoring case, is 409 `name_taken`. The feature's docs and threads stay
//!   in it.
//! - `DELETE /features/{feature_id}` (agent) answers 204 and soft deletes the
//!   feature (`features.deleted_at`). The feature, its docs, its threads and
//!   its comments then disappear: they are in no list (features, docs,
//!   threads, comments, the project home and its waiting comments) and every
//!   read or write of them answers 404, as does starting a doc or a thread in
//!   the feature. The app level and other features keep everything. The name
//!   of a deleted feature can be used again, for a new feature or a rename.
//! - A session (the human) is 403 `forbidden` on all four routes, an unknown
//!   record is 404 `not_found`, and a refused call changes nothing.
//! - A feature has no status: no Feature carries `status`, and the routes
//!   `POST /features/{feature_id}/done` and `/reopen` are gone (404 or 405).

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

const TEST_DB: &str = "omni_us110_deletes_renames";
const TEST_BUCKET: &str = "omni-us110-deletes-renames";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const DOC_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}";
const FEATURE_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}";

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
    format!("us110-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
            ("X-Omni-Run", "run-110".to_string()),
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

fn id_of(value: &Value) -> String {
    value["id"]
        .as_str()
        .unwrap_or_else(|| panic!("there is an `id`: {value}"))
        .to_string()
}

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
}

fn doc_path(f: &Fixture, project: &str, doc: &str) -> String {
    format!("{}/docs/{doc}", project_base(f, project))
}

fn feature_path(f: &Fixture, project: &str, feature: &str) -> String {
    format!("{}/features/{feature}", project_base(f, project))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &agent(f),
                Some(&json!({ "name": unique("tidy-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

fn new_feature_named(f: &Fixture, client: &Client, project: &str, name: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/features", project_base(f, project)),
                &agent(f),
                Some(&json!({ "name": name })),
            ),
            201,
            "create a feature",
        )["feature"],
    )
}

fn new_feature(f: &Fixture, client: &Client, project: &str) -> String {
    new_feature_named(f, client, project, &unique("Checkout"))
}

/// Creates a live HTML doc at the app level or in a feature; returns its ID.
fn new_doc(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: Option<&str>,
    title: &str,
) -> String {
    let path = match feature {
        None => format!("{}/docs", project_base(f, project)),
        Some(feature) => format!("{}/docs", feature_path(f, project, feature)),
    };
    id_of(
        &must(
            call(
                client,
                "POST",
                &path,
                &agent(f),
                Some(&json!({ "kind": "html", "pillar": "spec", "title": title })),
            ),
            201,
            "create a doc without a change set",
        )["doc"],
    )
}

/// The batch that puts a paragraph first in a doc.
fn paragraph_batch(text: &str) -> Value {
    json!({ "ops": [{
        "op": "insert",
        "parent": null,
        "position": { "after": null },
        "content": format!("<p>{text}</p>")
    }] })
}

/// Writes a paragraph to the doc's head; returns its node ID.
fn write_paragraph(f: &Fixture, client: &Client, project: &str, doc: &str, text: &str) -> String {
    let answer = must(
        call(
            client,
            "POST",
            &format!("{}/ops", doc_path(f, project, doc)),
            &agent(f),
            Some(&paragraph_batch(text)),
        ),
        200,
        "write a paragraph",
    );
    answer["results"][0]["node"]
        .as_str()
        .unwrap_or_else(|| panic!("the insert names its node: {answer}"))
        .to_string()
}

/// The human comments on `node` of the doc and the agent replies, so the
/// comment waits for the human; returns the comment's ID.
fn waiting_comment(f: &Fixture, client: &Client, project: &str, doc: &str, node: &str) -> String {
    let comment = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/comments", doc_path(f, project, doc)),
                &human(f),
                Some(&json!({ "node": node, "text": unique("Say more") })),
            ),
            201,
            "comment on the paragraph",
        )["comment"],
    );
    must(
        call(
            client,
            "POST",
            &format!("{}/comments/{comment}/replies", project_base(f, project)),
            &agent(f),
            Some(&json!({ "text": "Done, see the paragraph." })),
        ),
        201,
        "reply to the comment",
    );
    comment
}

/// Starts a thread at the app level or in a feature; returns its ID.
fn new_thread(f: &Fixture, client: &Client, project: &str, feature: Option<&str>) -> String {
    let path = match feature {
        None => format!("{}/threads", project_base(f, project)),
        Some(feature) => format!("{}/threads", feature_path(f, project, feature)),
    };
    id_of(
        &must(
            call(
                client,
                "POST",
                &path,
                &agent(f),
                Some(&json!({ "question": unique("Which way?"), "sealed": false })),
            ),
            201,
            "start a thread",
        )["thread"],
    )
}

/// Opens a change set in the feature; returns its ID.
fn new_changeset(f: &Fixture, client: &Client, project: &str, feature: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/changesets", project_base(f, project)),
                &agent(f),
                Some(&json!({
                    "title": unique("change"),
                    "run_tag": "run-110",
                    "feature_id": feature
                })),
            ),
            201,
            "open a change set in the feature",
        )["changeset"],
    )
}

fn rename_doc(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    doc: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "PATCH",
        &doc_path(f, project, doc),
        auth,
        Some(body),
    )
}

fn delete_doc(f: &Fixture, client: &Client, auth: &Auth, project: &str, doc: &str) -> Response {
    call(client, "DELETE", &doc_path(f, project, doc), auth, None)
}

fn rename_feature(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    feature: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "PATCH",
        &feature_path(f, project, feature),
        auth,
        Some(body),
    )
}

fn delete_feature(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    feature: &str,
) -> Response {
    call(
        client,
        "DELETE",
        &feature_path(f, project, feature),
        auth,
        None,
    )
}

fn assert_deleted(response: &Response, what: &str) {
    assert_eq!(response.status, 204, "{what}: {}", response.text());
}

fn get(f: &Fixture, client: &Client, auth: &Auth, project: &str, rest: &str) -> Response {
    call(
        client,
        "GET",
        &format!("{}{rest}", project_base(f, project)),
        auth,
        None,
    )
}

/// The IDs of the list at `key` of what `GET <rest>` answers.
fn listed(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    rest: &str,
    key: &str,
) -> Vec<String> {
    let answer = must(
        get(f, client, auth, project, rest),
        200,
        &format!("GET {rest}"),
    );
    answer[key]
        .as_array()
        .unwrap_or_else(|| panic!("GET {rest} answers a list `{key}`: {answer}"))
        .iter()
        .map(id_of)
        .collect()
}

/// The project's home, which only the human reads.
fn home(f: &Fixture, client: &Client, project: &str) -> Value {
    must(
        get(f, client, &human(f), project, "/home"),
        200,
        "read the home",
    )
}

/// The IDs of the home's features.
fn home_features(home: &Value) -> Vec<String> {
    home["features"]
        .as_array()
        .unwrap_or_else(|| panic!("the home lists features: {home}"))
        .iter()
        .map(|summary| id_of(&summary["feature"]))
        .collect()
}

fn ids_at(home: &Value, pointer: &str) -> Vec<String> {
    home.pointer(pointer)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("the home has the list {pointer}: {home}"))
        .iter()
        .map(id_of)
        .collect()
}

fn scalar(sql: &str, binds: &[&str]) -> Option<String> {
    block(async {
        let mut conn = admin_connection().await;
        let mut query = sqlx::query_scalar::<_, Option<String>>(sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    })
}

/// The doc's head as the database holds it.
fn head_of(doc: &str) -> String {
    scalar("SELECT head::text FROM docs WHERE id = $1::uuid", &[doc])
        .unwrap_or_else(|| panic!("the doc {doc} has a head"))
}

/// The `data` of every `doc.changed` event of the project that names `doc`,
/// oldest first.
fn doc_changed_events(project: &str, doc: &str) -> Vec<Value> {
    block(async {
        let mut conn = admin_connection().await;
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT data::jsonb::text FROM live_events WHERE project_id = $1::uuid \
             AND event = 'doc.changed' AND data ->> 'doc_id' = $2 ORDER BY seq",
        )
        .bind(project)
        .bind(doc)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the live events: {e}"));
        rows.iter()
            .map(|text| serde_json::from_str(text).expect("the data is JSON"))
            .collect()
    })
}

/// The `deleted_at` of a row of `table`, as text; `None` while it is not
/// deleted (or while the table has no such column).
fn deleted_at(table: &str, id: &str) -> Option<String> {
    scalar(
        &format!("SELECT to_jsonb(t) ->> 'deleted_at' FROM {table} t WHERE t.id = $1::uuid"),
        &[id],
    )
}

/// One row of `doc_changes`.
#[derive(Debug)]
struct Change {
    seq: i64,
    agent: Value,
    operations: Value,
    head_before: Option<String>,
    head_after: Option<String>,
}

/// `seq`, `agent`, `operations`, `head_before` and `head_after`.
type ChangeRow = (i64, String, String, Option<String>, Option<String>);

fn changes_of(doc: &str) -> Vec<Change> {
    block(async {
        let mut conn = admin_connection().await;
        let rows: Vec<ChangeRow> = sqlx::query_as(
            "SELECT seq::bigint, agent::text, operations::text, head_before::text, \
                 head_after::text FROM doc_changes WHERE doc_id = $1::uuid ORDER BY seq",
        )
        .bind(doc)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the doc changes of {doc}: {e}"));
        rows.into_iter()
            .map(|(seq, agent, operations, head_before, head_after)| Change {
                seq,
                agent: serde_json::from_str(&agent).expect("the agent is JSON"),
                operations: serde_json::from_str(&operations).expect("the operations are JSON"),
                head_before,
                head_after,
            })
            .collect()
    })
}

/// The agent identity that `call` declares.
fn assert_writer(change: &Change) {
    assert_eq!(change.agent["run"], "run-110", "{change:?}");
    assert_eq!(change.agent["role"], "writer", "{change:?}");
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_rename_and_delete_routes_are_agent_routes_of_the_registry() {
    let routes = registry();
    for path in [DOC_ROUTE, FEATURE_ROUTE] {
        for method in ["PATCH", "DELETE"] {
            let entry = routes
                .iter()
                .find(|r| r.method == method && r.path == path)
                .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
            assert_eq!(entry.access, Access::Agent, "{method} {path}");
        }
    }
}

// ---------------------------------------------------------------------------
// Deleting a doc
// ---------------------------------------------------------------------------

#[test]
fn a_deleted_doc_disappears_from_every_list_and_read() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let kept = new_doc(f, client, &project, None, "Kept plan");
    let doomed = new_doc(f, client, &project, None, "Old plan");
    let in_feature = new_doc(f, client, &project, Some(&feature), "Old feature plan");
    let node = write_paragraph(f, client, &project, &doomed, "Pay in one step.");
    let comment = waiting_comment(f, client, &project, &doomed, &node);
    let feature_node = write_paragraph(f, client, &project, &in_feature, "Save for later.");
    let feature_comment = waiting_comment(f, client, &project, &in_feature, &feature_node);

    // Before the delete, every list and read has them.
    for auth in [agent(f), human(f)] {
        let docs = listed(f, client, &auth, &project, "/docs", "docs");
        assert!(
            docs.contains(&doomed) && docs.contains(&in_feature),
            "{docs:?}"
        );
        for doc in [&doomed, &in_feature] {
            must(
                get(f, client, &auth, &project, &format!("/docs/{doc}")),
                200,
                "read the doc",
            );
        }
        let comments = listed(f, client, &auth, &project, "/comments", "comments");
        assert!(comments.contains(&comment), "{comments:?}");
    }

    for doc in [&doomed, &in_feature] {
        assert_deleted(
            &delete_doc(f, client, &agent(f), &project, doc),
            "delete the doc",
        );
    }

    for auth in [agent(f), human(f)] {
        for rest in ["/docs", "/docs?pillar=spec", "/docs?feature=global"] {
            let docs = listed(f, client, &auth, &project, rest, "docs");
            assert!(
                !docs.contains(&doomed) && !docs.contains(&in_feature),
                "GET {rest} lists no deleted doc: {docs:?}"
            );
            assert!(
                docs.contains(&kept),
                "GET {rest} still lists the kept doc: {docs:?}"
            );
        }
        let feature_docs = listed(
            f,
            client,
            &auth,
            &project,
            &format!("/docs?feature={feature}"),
            "docs",
        );
        assert!(
            feature_docs.is_empty(),
            "the feature has no doc left: {feature_docs:?}"
        );
        for doc in [&doomed, &in_feature] {
            for rest in [format!("/docs/{doc}"), format!("/docs/{doc}/html")] {
                assert_error(&get(f, client, &auth, &project, &rest), 404, "not_found");
            }
            let left = listed(
                f,
                client,
                &auth,
                &project,
                &format!("/comments?doc={doc}"),
                "comments",
            );
            assert!(left.is_empty(), "a deleted doc lists no comments: {left:?}");
        }
        let comments = listed(f, client, &auth, &project, "/comments", "comments");
        assert!(
            !comments.contains(&comment) && !comments.contains(&feature_comment),
            "the deleted docs' comments are in no list: {comments:?}"
        );
        for c in [&comment, &feature_comment] {
            assert_error(
                &get(f, client, &auth, &project, &format!("/comments/{c}")),
                404,
                "not_found",
            );
        }
    }
    let home = home(f, client, &project);
    let docs = ids_at(&home, "/docs");
    assert!(
        docs.contains(&kept) && !docs.contains(&doomed) && !docs.contains(&in_feature),
        "the home lists the kept doc and no deleted one: {home}"
    );
    let waiting = ids_at(&home, "/waiting/comments");
    assert!(
        !waiting.contains(&comment) && !waiting.contains(&feature_comment),
        "no comment of a deleted doc waits: {home}"
    );
    let summary = home["features"]
        .as_array()
        .and_then(|all| all.iter().find(|s| s["feature"]["id"] == feature.as_str()))
        .unwrap_or_else(|| panic!("the home lists the feature: {home}"));
    assert_eq!(
        summary["docs"], 0,
        "the feature counts no deleted doc: {summary}"
    );
}

#[test]
fn a_deleted_doc_takes_no_more_writes() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = new_doc(f, client, &project, None, "Gone plan");
    let node = write_paragraph(f, client, &project, &doc, "Pay in one step.");
    let comment = waiting_comment(f, client, &project, &doc, &node);
    assert_deleted(
        &delete_doc(f, client, &agent(f), &project, &doc),
        "delete the doc",
    );
    let rows = changes_of(&doc).len();
    let head = head_of(&doc);

    assert_error(
        &call(
            client,
            "POST",
            &format!("{}/ops", doc_path(f, &project, &doc)),
            &agent(f),
            Some(&paragraph_batch("Too late.")),
        ),
        404,
        "not_found",
    );
    assert_error(
        &call(
            client,
            "POST",
            &format!("{}/comments", doc_path(f, &project, &doc)),
            &human(f),
            Some(&json!({ "node": node, "text": "Too late." })),
        ),
        404,
        "not_found",
    );
    assert_error(
        &call(
            client,
            "POST",
            &format!("{}/comments/{comment}/replies", project_base(f, &project)),
            &agent(f),
            Some(&json!({ "text": "Too late." })),
        ),
        404,
        "not_found",
    );
    assert_error(
        &rename_doc(
            f,
            client,
            &agent(f),
            &project,
            &doc,
            &json!({ "title": "Back again" }),
        ),
        404,
        "not_found",
    );
    assert_error(
        &delete_doc(f, client, &agent(f), &project, &doc),
        404,
        "not_found",
    );
    assert_eq!(changes_of(&doc).len(), rows, "a refused write logs nothing");
    assert_eq!(head_of(&doc), head, "a refused write changes no head");
}

#[test]
fn deleting_a_doc_is_soft_and_records_one_doc_changes_row() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = new_doc(f, client, &project, None, "Logged plan");
    write_paragraph(f, client, &project, &doc, "Pay in one step.");
    let head = head_of(&doc);
    let before = changes_of(&doc);
    assert_eq!(before.len(), 2, "the creation and the write: {before:?}");
    assert!(
        deleted_at("docs", &doc).is_none(),
        "a live doc is not deleted"
    );

    assert_deleted(
        &delete_doc(f, client, &agent(f), &project, &doc),
        "delete the doc",
    );

    assert!(
        deleted_at("docs", &doc).is_some(),
        "the doc's row stays, with deleted_at set"
    );
    let after = changes_of(&doc);
    assert_eq!(after.len(), 3, "one row more: {after:?}");
    let last = &after[2];
    assert_eq!(last.seq, 3, "{last:?}");
    assert_eq!(last.operations, json!([{ "op": "delete" }]), "{last:?}");
    assert_eq!(last.head_before.as_deref(), Some(head.as_str()), "{last:?}");
    assert_writer(last);
}

#[test]
fn creating_renaming_and_deleting_a_doc_each_send_doc_changed_with_the_doc_and_its_head() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = new_doc(f, client, &project, None, "Live plan");
    let sent = |what: &str, head: &str| {
        let events = doc_changed_events(&project, &doc);
        let last = events
            .last()
            .unwrap_or_else(|| panic!("{what}: no doc.changed was sent"));
        assert_eq!(last["doc_id"], json!(doc), "{what}: {last}");
        assert_eq!(last["head"], json!(head), "{what}: {last}");
        events.len()
    };
    assert_eq!(sent("the creation", &head_of(&doc)), 1);

    write_paragraph(f, client, &project, &doc, "Pay in one step.");
    let head = head_of(&doc);
    let written = doc_changed_events(&project, &doc).len();
    must(
        rename_doc(
            f,
            client,
            &agent(f),
            &project,
            &doc,
            &json!({ "title": "Live plan, renamed" }),
        ),
        200,
        "rename the doc",
    );
    assert_eq!(sent("the rename", &head), written + 1);

    assert_deleted(
        &delete_doc(f, client, &agent(f), &project, &doc),
        "delete the doc",
    );
    assert_eq!(sent("the delete", &head), written + 2);
}

#[test]
fn a_refused_doc_delete_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = new_doc(f, client, &project, None, "Safe plan");
    let rows = changes_of(&doc).len();

    assert_error(
        &delete_doc(f, client, &human(f), &project, &doc),
        403,
        "forbidden",
    );
    assert_error(
        &delete_doc(f, client, &agent(f), &project, UNKNOWN_ID),
        404,
        "not_found",
    );
    assert_error(
        &delete_doc(f, client, &agent(f), &project, "not-a-uuid"),
        404,
        "not_found",
    );
    let other = new_project(f, client);
    assert_error(
        &delete_doc(f, client, &agent(f), &other, &doc),
        404,
        "not_found",
    );

    assert!(deleted_at("docs", &doc).is_none(), "the doc is not deleted");
    assert_eq!(changes_of(&doc).len(), rows, "nothing is logged");
    must(
        get(f, client, &agent(f), &project, &format!("/docs/{doc}")),
        200,
        "the doc is there",
    );
}

// ---------------------------------------------------------------------------
// Deleting a feature
// ---------------------------------------------------------------------------

#[test]
fn a_deleted_feature_disappears_with_its_docs_threads_and_comments() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let other = new_feature(f, client, &project);

    // The feature's records ...
    let doc = new_doc(f, client, &project, Some(&feature), "Feature plan");
    let node = write_paragraph(f, client, &project, &doc, "Pay in one step.");
    let comment = waiting_comment(f, client, &project, &doc, &node);
    let thread = new_thread(f, client, &project, Some(&feature));
    // ... and those of the app level and another feature, which stay.
    let app_doc = new_doc(f, client, &project, None, "App plan");
    let app_node = write_paragraph(f, client, &project, &app_doc, "The shop.");
    let app_comment = waiting_comment(f, client, &project, &app_doc, &app_node);
    let app_thread = new_thread(f, client, &project, None);
    let other_doc = new_doc(f, client, &project, Some(&other), "Other plan");
    let other_thread = new_thread(f, client, &project, Some(&other));

    assert_deleted(
        &delete_feature(f, client, &agent(f), &project, &feature),
        "delete the feature",
    );
    assert!(
        deleted_at("features", &feature).is_some(),
        "the feature's row stays, with deleted_at set"
    );

    for auth in [agent(f), human(f)] {
        let features = listed(f, client, &auth, &project, "/features", "features");
        assert_eq!(
            features,
            vec![other.clone()],
            "only the other feature is listed"
        );
        let docs = listed(f, client, &auth, &project, "/docs", "docs");
        assert!(
            !docs.contains(&doc),
            "the feature's doc is in no list: {docs:?}"
        );
        assert!(
            docs.contains(&app_doc) && docs.contains(&other_doc),
            "{docs:?}"
        );
        let threads = listed(f, client, &auth, &project, "/threads", "threads");
        assert!(
            !threads.contains(&thread),
            "the feature's thread is in no list: {threads:?}"
        );
        assert!(
            threads.contains(&app_thread) && threads.contains(&other_thread),
            "{threads:?}"
        );
        let comments = listed(f, client, &auth, &project, "/comments", "comments");
        assert!(
            !comments.contains(&comment),
            "the feature's comment is in no list: {comments:?}"
        );
        assert!(comments.contains(&app_comment), "{comments:?}");

        for rest in [
            format!("/features/{feature}"),
            format!("/docs/{doc}"),
            format!("/docs/{doc}/html"),
            format!("/threads/{thread}"),
            format!("/comments/{comment}"),
        ] {
            let response = get(f, client, &auth, &project, &rest);
            assert_error(&response, 404, "not_found");
        }
        for (rest, key) in [
            (format!("/docs?feature={feature}"), "docs"),
            (format!("/comments?feature={feature}"), "comments"),
        ] {
            let left = listed(f, client, &auth, &project, &rest, key);
            assert!(left.is_empty(), "GET {rest} lists nothing: {left:?}");
        }
        for rest in [
            format!("/features/{other}"),
            format!("/docs/{app_doc}"),
            format!("/threads/{app_thread}"),
            format!("/comments/{app_comment}"),
        ] {
            must(
                get(f, client, &auth, &project, &rest),
                200,
                &format!("GET {rest} stays"),
            );
        }
    }

    let home = home(f, client, &project);
    assert_eq!(home_features(&home), vec![other.clone()], "{home}");
    assert!(!ids_at(&home, "/docs").contains(&doc), "{home}");
    assert!(!ids_at(&home, "/threads").contains(&thread), "{home}");
    let waiting = ids_at(&home, "/waiting/comments");
    assert!(
        !waiting.contains(&comment),
        "the feature's comment no longer waits: {home}"
    );
    assert!(
        waiting.contains(&app_comment),
        "the app level's comment still waits: {home}"
    );
}

#[test]
fn a_deleted_feature_takes_nothing_new() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let doc = new_doc(f, client, &project, Some(&feature), "Feature plan");
    let node = write_paragraph(f, client, &project, &doc, "Pay in one step.");
    let comment = waiting_comment(f, client, &project, &doc, &node);
    let thread = new_thread(f, client, &project, Some(&feature));
    let changeset = new_changeset(f, client, &project, &feature);
    assert_deleted(
        &delete_feature(f, client, &agent(f), &project, &feature),
        "delete the feature",
    );
    let base = project_base(f, &project);
    let feature_base = feature_path(f, &project, &feature);
    let changeset_base = format!("{base}/changesets/{changeset}");
    for (auth, path, body) in [
        (
            agent(f),
            format!("{changeset_base}/docs"),
            json!({ "kind": "html", "pillar": "spec", "title": "Late draft" }),
        ),
        (
            agent(f),
            format!("{changeset_base}/threads"),
            json!({ "question": "Too late in the change set?", "criteria": ["Speed"] }),
        ),
        (
            agent(f),
            format!("{feature_base}/docs"),
            json!({ "kind": "html", "pillar": "spec", "title": "Late plan" }),
        ),
        (
            agent(f),
            format!("{feature_base}/threads"),
            json!({ "question": "Too late?", "sealed": false }),
        ),
        (
            agent(f),
            format!("{base}/docs/{doc}/ops"),
            paragraph_batch("Too late."),
        ),
        (
            human(f),
            format!("{base}/docs/{doc}/comments"),
            json!({ "node": node, "text": "Too late." }),
        ),
        (
            agent(f),
            format!("{base}/comments/{comment}/replies"),
            json!({ "text": "Too late." }),
        ),
        (
            agent(f),
            format!("{base}/threads/{thread}/posts"),
            json!({ "summary": "Too late", "body": "Too late." }),
        ),
    ] {
        let response = call(client, "POST", &path, &auth, Some(&body));
        assert_error(&response, 404, "not_found");
    }
    for auth in [agent(f), human(f)] {
        let docs = listed(f, client, &auth, &project, "/docs", "docs");
        assert_eq!(
            docs,
            Vec::<String>::new(),
            "no doc of the deleted feature is listed"
        );
        let threads = listed(f, client, &auth, &project, "/threads", "threads");
        assert_eq!(
            threads,
            Vec::<String>::new(),
            "no thread of the deleted feature is listed"
        );
    }
    assert_error(
        &rename_feature(
            f,
            client,
            &agent(f),
            &project,
            &feature,
            &json!({ "name": "Back" }),
        ),
        404,
        "not_found",
    );
    assert_error(
        &delete_feature(f, client, &agent(f), &project, &feature),
        404,
        "not_found",
    );
}

#[test]
fn a_deleted_features_name_can_be_used_again() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let name = unique("Wish list");
    let first = new_feature_named(f, client, &project, &name);
    assert_deleted(
        &delete_feature(f, client, &agent(f), &project, &first),
        "delete the feature",
    );

    let again = new_feature_named(f, client, &project, &name.to_uppercase());
    assert_ne!(again, first, "the new feature is another one");
    assert_error(
        &get(
            f,
            client,
            &agent(f),
            &project,
            &format!("/features/{first}"),
        ),
        404,
        "not_found",
    );
    assert_deleted(
        &delete_feature(f, client, &agent(f), &project, &again),
        "delete the second feature",
    );

    // A rename may take the name of a deleted feature too.
    let other = new_feature(f, client, &project);
    let renamed = must(
        rename_feature(
            f,
            client,
            &agent(f),
            &project,
            &other,
            &json!({ "name": name }),
        ),
        200,
        "rename a feature to a deleted one's name",
    );
    assert_eq!(renamed["feature"]["name"], name.as_str(), "{renamed}");
}

#[test]
fn a_refused_feature_delete_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let doc = new_doc(f, client, &project, Some(&feature), "Safe plan");

    assert_error(
        &delete_feature(f, client, &human(f), &project, &feature),
        403,
        "forbidden",
    );
    assert_error(
        &delete_feature(f, client, &agent(f), &project, UNKNOWN_ID),
        404,
        "not_found",
    );
    let other = new_project(f, client);
    assert_error(
        &delete_feature(f, client, &agent(f), &other, &feature),
        404,
        "not_found",
    );

    assert!(
        deleted_at("features", &feature).is_none(),
        "the feature is not deleted"
    );
    for rest in [format!("/features/{feature}"), format!("/docs/{doc}")] {
        must(
            get(f, client, &agent(f), &project, &rest),
            200,
            &format!("GET {rest}"),
        );
    }
}

// ---------------------------------------------------------------------------
// Renaming
// ---------------------------------------------------------------------------

#[test]
fn renaming_a_doc_keeps_its_id_and_comments_and_is_logged() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = new_doc(f, client, &project, None, "Draft title");
    let node = write_paragraph(f, client, &project, &doc, "Pay in one step.");
    let comment = waiting_comment(f, client, &project, &doc, &node);
    let head = head_of(&doc);
    let rows = changes_of(&doc).len();

    let renamed = must(
        rename_doc(
            f,
            client,
            &agent(f),
            &project,
            &doc,
            &json!({ "title": "Checkout plan" }),
        ),
        200,
        "rename the doc",
    );
    assert_eq!(renamed["doc"]["id"], doc.as_str(), "{renamed}");
    assert_eq!(renamed["doc"]["title"], "Checkout plan", "{renamed}");

    for auth in [agent(f), human(f)] {
        let shown = must(
            get(f, client, &auth, &project, &format!("/docs/{doc}")),
            200,
            "read",
        );
        assert_eq!(shown["doc"]["title"], "Checkout plan", "{shown}");
        let listed = must(get(f, client, &auth, &project, "/docs"), 200, "list");
        let entry = listed["docs"]
            .as_array()
            .and_then(|docs| docs.iter().find(|d| d["id"] == doc.as_str()))
            .unwrap_or_else(|| panic!("the list has the doc: {listed}"));
        assert_eq!(entry["title"], "Checkout plan", "{entry}");
        let html = must(
            get(f, client, &auth, &project, &format!("/docs/{doc}/html")),
            200,
            "html",
        );
        assert!(
            html["html"]
                .as_str()
                .is_some_and(|h| h.contains("Pay in one step.")),
            "the content is unchanged: {html}"
        );
        let comments = listed_comments(f, client, &auth, &project, &doc);
        let kept = comments
            .iter()
            .find(|c| c["id"] == comment.as_str())
            .unwrap_or_else(|| panic!("the comment stays on the doc: {comments:?}"));
        assert_eq!(kept["doc_id"], doc.as_str(), "{kept}");
        assert_eq!(kept["node"], node.as_str(), "{kept}");
        assert_eq!(kept["orphaned"], false, "{kept}");
    }

    assert_eq!(head_of(&doc), head, "a rename leaves the head as it was");
    let changes = changes_of(&doc);
    assert_eq!(changes.len(), rows + 1, "one row more: {changes:?}");
    let last = changes.last().expect("a row");
    assert_eq!(
        last.operations,
        json!([{ "op": "rename", "title": "Checkout plan" }]),
        "{last:?}"
    );
    assert_eq!(last.head_before.as_deref(), Some(head.as_str()), "{last:?}");
    assert_eq!(last.head_after.as_deref(), Some(head.as_str()), "{last:?}");
    assert_writer(last);
}

fn listed_comments(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    doc: &str,
) -> Vec<Value> {
    must(
        get(f, client, auth, project, &format!("/comments?doc={doc}")),
        200,
        "list the doc's comments",
    )["comments"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn a_refused_doc_rename_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = new_doc(f, client, &project, None, "Steady title");
    let rows = changes_of(&doc).len();

    for body in [
        json!({}),
        json!({ "title": "" }),
        json!({ "title": "  \n" }),
        json!({ "title": 7 }),
    ] {
        let response = rename_doc(f, client, &agent(f), &project, &doc, &body);
        assert_error(&response, 400, "invalid_request");
    }
    assert_error(
        &rename_doc(
            f,
            client,
            &human(f),
            &project,
            &doc,
            &json!({ "title": "Mine" }),
        ),
        403,
        "forbidden",
    );
    assert_error(
        &rename_doc(
            f,
            client,
            &agent(f),
            &project,
            UNKNOWN_ID,
            &json!({ "title": "Nobody" }),
        ),
        404,
        "not_found",
    );

    let shown = must(
        get(f, client, &agent(f), &project, &format!("/docs/{doc}")),
        200,
        "read",
    );
    assert_eq!(shown["doc"]["title"], "Steady title", "{shown}");
    assert_eq!(changes_of(&doc).len(), rows, "nothing is logged");
}

#[test]
fn renaming_a_feature_keeps_its_id_docs_and_threads() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let doc = new_doc(f, client, &project, Some(&feature), "Feature plan");
    let thread = new_thread(f, client, &project, Some(&feature));
    let new_name = unique("Express checkout");

    let renamed = must(
        rename_feature(
            f,
            client,
            &agent(f),
            &project,
            &feature,
            &json!({ "name": new_name }),
        ),
        200,
        "rename the feature",
    );
    assert_eq!(renamed["feature"]["id"], feature.as_str(), "{renamed}");
    assert_eq!(renamed["feature"]["name"], new_name.as_str(), "{renamed}");

    for auth in [agent(f), human(f)] {
        let shown = must(
            get(f, client, &auth, &project, &format!("/features/{feature}")),
            200,
            "read the feature",
        );
        assert_eq!(shown["feature"]["name"], new_name.as_str(), "{shown}");
        let doc = must(
            get(f, client, &auth, &project, &format!("/docs/{doc}")),
            200,
            "doc",
        );
        assert_eq!(doc["doc"]["feature_id"], feature.as_str(), "{doc}");
        let thread = must(
            get(f, client, &auth, &project, &format!("/threads/{thread}")),
            200,
            "thread",
        );
        assert_eq!(thread["thread"]["feature_id"], feature.as_str(), "{thread}");
    }
}

#[test]
fn a_refused_feature_rename_changes_nothing() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let taken = unique("Gift cards");
    new_feature_named(f, client, &project, &taken);
    let name = unique("Loyalty");
    let feature = new_feature_named(f, client, &project, &name);

    for body in [
        json!({}),
        json!({ "name": "" }),
        json!({ "name": "   " }),
        json!({ "name": 7 }),
    ] {
        let response = rename_feature(f, client, &agent(f), &project, &feature, &body);
        assert_error(&response, 400, "invalid_request");
    }
    assert_error(
        &rename_feature(
            f,
            client,
            &agent(f),
            &project,
            &feature,
            &json!({ "name": taken.to_lowercase() }),
        ),
        409,
        "name_taken",
    );
    assert_error(
        &rename_feature(
            f,
            client,
            &human(f),
            &project,
            &feature,
            &json!({ "name": "Mine" }),
        ),
        403,
        "forbidden",
    );
    assert_error(
        &rename_feature(
            f,
            client,
            &agent(f),
            &project,
            UNKNOWN_ID,
            &json!({ "name": "Nobody" }),
        ),
        404,
        "not_found",
    );
    let shown = must(
        get(
            f,
            client,
            &agent(f),
            &project,
            &format!("/features/{feature}"),
        ),
        200,
        "read the feature",
    );
    assert_eq!(shown["feature"]["name"], name.as_str(), "{shown}");
}
