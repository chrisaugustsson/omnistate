//! US-061: docs in a feature (Q51). Run with `--features stack-tests` in the
//! Compose test container, which reaches Postgres at `postgres:5432` and RustFS
//! at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; features and change sets
//! in a feature are those of US-059 and US-060, docs and drafts those of
//! US-022):
//!
//! - `docs.feature_id` is nullable (null is a global doc of the project) with
//!   the foreign key `(workspace_id, project_id, feature_id)` to `features
//!   (workspace_id, project_id, id)`.
//! - A doc created through `POST /changesets/{changeset_id}/docs` takes its
//!   change set's `feature_id` and never changes scope. A Doc (`DocBody`)
//!   carries `feature_id`, and so does the `data` of `doc.created`.
//! - The design system is always global: creating a `design-system` doc in a
//!   feature's change set is 422 `{"error": "design_system_is_global"}` and
//!   creates nothing, whether or not the project has one already. The project
//!   keeps at most one design system. A design doc in a feature resolves its
//!   instances against the project's design system.
//! - A feature's change set may draft its own feature's docs and the
//!   project's global docs. Drafting a doc of another feature, or a feature's
//!   doc from a project-level change set, is 422 `{"error":
//!   "doc_in_other_feature"}` and creates no draft. That holds for every way
//!   a change set gets a draft of an existing doc: `POST
//!   /changesets/{changeset_id}/docs/{doc_id}/open` and the design import
//!   `PUT /changesets/{changeset_id}/docs/{doc_id}/design`.
//! - `GET /docs` (both) answers `{"docs": [Doc, ...]}`, every doc of the
//!   project oldest first; `?feature=<id>` lists that feature's docs only and
//!   `?feature=global` the global ones only. Another value is 400, an unknown
//!   project 404 `not_found`.
//!
//! US-062 adds finishing a feature (FR-52, FR-55):
//!
//! - `POST /features/{feature_id}/done` and `POST /features/{feature_id}/reopen`
//!   are human routes: an API key is 403 `{"error": "forbidden"}` and changes
//!   nothing. `done` sets an active feature's status to `done` and `reopen`
//!   sets a done one back to `active`; each answers 200 `{"feature":
//!   Feature}` and emits `feature.done` or `feature.reopened` with `data` that
//!   has at least `{"feature_id"}`. `done` on a done feature is 409 `{"error":
//!   "already_done"}`, `reopen` on an active one 409 `{"error": "not_done"}`,
//!   and an unknown feature is 404 `not_found`; a refused call emits nothing.
//! - Creating a change set in a done feature is 409 `{"error":
//!   "feature_done"}` and creates nothing. A change set that was open in it
//!   before keeps its whole lifecycle: edits, saves, submit, reject, reopen,
//!   approve and merge.
//! - Every read of a done feature's docs, versions, threads, posts, decisions
//!   and comments answers as it did before the feature was done.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::HashMap;
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us061_feature_docs";
const TEST_BUCKET: &str = "omni-us061-feature-docs";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const DOCS_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/docs";

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
        // Versions keep their content in RustFS, and a doc that another change
        // set drafts needs a merged version, so the API gets a real bucket.
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
    format!("us061-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
            ("X-Omni-Run", "run-61".to_string()),
            ("X-Omni-Role", "planner".to_string()),
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

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

/// Asserts that `value` has the key `feature_id` and that it is `expected`
/// (`Value::Null` for a global doc).
fn assert_feature_id(value: &Value, expected: &Value, what: &str) {
    assert!(
        value
            .as_object()
            .is_some_and(|o| o.contains_key("feature_id")),
        "{what} carries `feature_id`: {value}"
    );
    assert_eq!(&value["feature_id"], expected, "{what}: {value}");
}

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
}

fn changeset_path(f: &Fixture, project: &str, changeset: &str) -> String {
    format!("{}/changesets/{changeset}", project_base(f, project))
}

fn docs_path(f: &Fixture, project: &str) -> String {
    format!("{}/docs", project_base(f, project))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &agent(f),
                Some(&json!({ "name": unique("feature-docs-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

/// Creates a feature as the agent; returns its ID.
fn new_feature(f: &Fixture, client: &Client, project: &str, name: &str) -> String {
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

/// Opens a change set in `feature`, or at project level without one.
fn new_changeset(f: &Fixture, client: &Client, project: &str, feature: Option<&str>) -> String {
    let mut body = json!({ "title": unique("change"), "run_tag": "run-61" });
    if let Some(feature) = feature {
        body["feature_id"] = json!(feature);
    }
    let created = must(
        call(
            client,
            "POST",
            &format!("{}/changesets", project_base(f, project)),
            &agent(f),
            Some(&body),
        ),
        201,
        "create a change set",
    );
    assert_feature_id(&created["changeset"], &json!(feature), "the new change set");
    id_of(&created["changeset"])
}

fn create_doc_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    kind: &str,
    module: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/docs", changeset_path(f, project, changeset)),
        &agent(f),
        Some(&json!({ "kind": kind, "module": module, "title": unique(module) })),
    )
}

/// Starts a doc in the change set; returns the whole answer.
fn create_doc(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    kind: &str,
    module: &str,
) -> Value {
    must(
        create_doc_response(f, client, project, changeset, kind, module),
        201,
        "create a doc",
    )
}

fn add_paragraph(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    text: &str,
) {
    must(
        call(
            client,
            "POST",
            &format!("{}/docs/{doc}/ops", changeset_path(f, project, changeset)),
            &agent(f),
            Some(&json!({ "ops": [{
                "op": "insert",
                "parent": null,
                "position": { "after": null },
                "content": format!("<p>{text}</p>")
            }] })),
        ),
        200,
        "insert a paragraph",
    );
}

/// Saves a version of the change set's draft of `doc`; returns the version.
fn save(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "POST",
            &format!(
                "{}/docs/{doc}/versions",
                changeset_path(f, project, changeset)
            ),
            &agent(f),
            Some(&json!({})),
        ),
        201,
        "save a version",
    )["version"]
        .clone()
}

/// Submits the change set and has the human approve it, which merges it.
fn merge(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let path = changeset_path(f, project, changeset);
    must(
        call(client, "POST", &format!("{path}/submit"), &agent(f), None),
        200,
        "submit the change set",
    );
    let approved = must(
        call(
            client,
            "POST",
            &format!("{path}/approve"),
            &human(f),
            Some(&json!({})),
        ),
        200,
        "the human approves",
    );
    assert_eq!(approved["changeset"]["status"], "merged", "{approved}");
}

/// A PRD doc started in a change set of `feature` (project level without
/// one), with a paragraph, saved and merged; returns the doc's ID.
fn merged_doc(f: &Fixture, client: &Client, project: &str, feature: Option<&str>) -> String {
    let changeset = new_changeset(f, client, project, feature);
    let doc = id_of(&create_doc(f, client, project, &changeset, "html", "prd")["doc"]);
    add_paragraph(f, client, project, &changeset, &doc, "Pay in one step.");
    save(f, client, project, &changeset, &doc);
    merge(f, client, project, &changeset);
    doc
}

fn show_doc(f: &Fixture, client: &Client, auth: &Auth, project: &str, doc: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}/{doc}", docs_path(f, project)),
            auth,
            None,
        ),
        200,
        "show the doc",
    )["doc"]
        .clone()
}

fn open_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/docs/{doc}/open", changeset_path(f, project, changeset)),
        &agent(f),
        None,
    )
}

fn design_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    tags: &str,
) -> Response {
    call(
        client,
        "PUT",
        &format!(
            "{}/docs/{doc}/design",
            changeset_path(f, project, changeset)
        ),
        &agent(f),
        Some(&json!({ "tags": tags })),
    )
}

fn list_docs(client: &Client, auth: &Auth, path: &str) -> Vec<Value> {
    let listed = must(call(client, "GET", path, auth, None), 200, "list the docs");
    listed["docs"]
        .as_array()
        .unwrap_or_else(|| panic!("`docs` is a list: {listed}"))
        .clone()
}

fn ids(items: &[Value]) -> Vec<String> {
    items.iter().map(id_of).collect()
}

/// Counts rows as the admin role.
fn count(sql: &str, binds: &[&str]) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        let mut query = sqlx::query_scalar::<_, i64>(sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count: {e}"))
    })
}

/// How many drafts the change set has of the doc.
fn drafts_of(doc: &str, changeset: &str) -> i64 {
    count(
        "select count(*) from drafts where doc_id = $1::uuid and changeset_id = $2::uuid",
        &[doc, changeset],
    )
}

/// How many docs of `kind` the project has.
fn docs_of_kind(project: &str, kind: &str) -> i64 {
    count(
        "select count(*) from docs where project_id = $1::uuid and kind = $2",
        &[project, kind],
    )
}

/// The `data` of the live events of a project named `event`, oldest first.
fn events_of(project: &str, event: &str) -> Vec<Value> {
    block(async {
        let mut conn = admin_connection().await;
        let rows = sqlx::query_scalar::<_, String>(
            "select data::jsonb::text from live_events \
             where project_id = $1::uuid and event = $2 order by seq",
        )
        .bind(project)
        .bind(event)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the live events: {e}"));
        rows.iter().map(|text| parse(text)).collect()
    })
}

/// Runs `sql` as the admin role and says whether the database accepted it;
/// whatever it did is rolled back.
fn accepted(sql: &str, binds: &[&str]) -> bool {
    block(async {
        let mut conn = admin_connection().await;
        let mut tx = conn.begin().await.expect("begin");
        let mut query = sqlx::query(sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        let result = query.execute(&mut *tx).await;
        tx.rollback().await.expect("roll back");
        result.is_ok()
    })
}

// ---------------------------------------------------------------------------
// The scope of a doc
// ---------------------------------------------------------------------------

#[test]
fn a_doc_takes_the_feature_of_the_change_set_that_creates_it() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project, "Checkout");
    let in_feature = new_changeset(f, client, &project, Some(&feature));
    let global = new_changeset(f, client, &project, None);

    for (changeset, expected) in [(&in_feature, json!(feature)), (&global, Value::Null)] {
        for (kind, module) in [
            ("html", "prd"),
            ("html", "architecture"),
            ("html", "db"),
            ("design", "ui"),
        ] {
            let created = create_doc(f, client, &project, changeset, kind, module);
            let doc = &created["doc"];
            assert_feature_id(doc, &expected, &format!("a new {kind} {module} doc"));
            assert_eq!(
                created["draft"]["changeset_id"],
                changeset.as_str(),
                "{created}"
            );
            let id = id_of(doc);
            for auth in [agent(f), human(f)] {
                let shown = show_doc(f, client, &auth, &project, &id);
                assert_feature_id(&shown, &expected, "the shown doc");
            }
            let event = events_of(&project, "doc.created")
                .into_iter()
                .find(|data| data["doc_id"] == id.as_str())
                .unwrap_or_else(|| panic!("doc.created for {id}"));
            assert_feature_id(&event, &expected, "the data of doc.created");
        }
    }
}

#[test]
fn a_doc_keeps_its_scope_after_its_change_set_merges() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project, "Checkout");
    let in_feature = merged_doc(f, client, &project, Some(&feature));
    let global = merged_doc(f, client, &project, None);

    for (doc, expected) in [(&in_feature, json!(feature)), (&global, Value::Null)] {
        let shown = show_doc(f, client, &agent(f), &project, doc);
        assert!(
            shown["mainline_version_id"].is_string(),
            "the doc merged: {shown}"
        );
        assert_feature_id(&shown, &expected, "a merged doc");
    }

    // Another change set of the same feature edits the feature's doc, and
    // the doc stays in the feature.
    let next = new_changeset(f, client, &project, Some(&feature));
    let opened = open_response(f, client, &project, &next, &in_feature);
    assert_eq!(opened.status, 201, "{}", opened.text());
    add_paragraph(f, client, &project, &next, &in_feature, "Then confirm.");
    save(f, client, &project, &next, &in_feature);
    merge(f, client, &project, &next);
    let shown = show_doc(f, client, &agent(f), &project, &in_feature);
    assert_feature_id(&shown, &json!(feature), "the doc after a second merge");
}

#[test]
fn the_docs_table_ties_a_doc_to_a_feature_of_its_own_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let other = new_project(f, client);
    let own = new_feature(f, client, &project, "Checkout");
    let foreign = new_feature(f, client, &other, "Checkout");
    let workspace = f.workspace.as_str();
    let insert = "insert into docs (workspace_id, project_id, kind, module, title, feature_id) \
                  values ($1::uuid, $2::uuid, 'html', 'prd', 'Direct', $3::uuid)";
    assert!(
        accepted(insert, &[workspace, &project, &own]),
        "a doc may belong to a feature of its project"
    );
    assert!(
        !accepted(insert, &[workspace, &project, &foreign]),
        "a doc cannot belong to a feature of another project"
    );
    assert!(
        !accepted(insert, &[workspace, &project, UNKNOWN_ID]),
        "a doc cannot belong to a feature that does not exist"
    );
    assert!(
        accepted(
            "insert into docs (workspace_id, project_id, kind, module, title) \
             values ($1::uuid, $2::uuid, 'html', 'prd', 'Global')",
            &[workspace, &project]
        ),
        "a doc without a feature is global"
    );

    // The foreign key is (workspace_id, project_id, feature_id) to features.
    let keys: Vec<String> = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar(
            "select (select string_agg(a.attname::text, ',' order by a.attname::text) \
                     from pg_attribute a \
                     where a.attrelid = c.conrelid and a.attnum = any(c.conkey)) \
             from pg_constraint c \
             where c.contype = 'f' and c.conrelid = 'docs'::regclass \
               and c.confrelid = 'features'::regclass",
        )
        .fetch_all(&mut conn)
        .await
        .expect("look for the foreign key")
    });
    assert_eq!(
        keys,
        ["feature_id,project_id,workspace_id"],
        "docs has the foreign key (workspace_id, project_id, feature_id) to features"
    );
    let nullable: String = block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar(
            "select coalesce((select is_nullable from information_schema.columns \
             where table_name = 'docs' and column_name = 'feature_id'), 'missing')",
        )
        .fetch_one(&mut conn)
        .await
        .expect("look for the column")
    });
    assert_eq!(nullable, "YES", "docs.feature_id is nullable");
}

// ---------------------------------------------------------------------------
// The design system is global
// ---------------------------------------------------------------------------

#[test]
fn a_feature_change_set_cannot_create_the_design_system() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project, "Checkout");
    let in_feature = new_changeset(f, client, &project, Some(&feature));

    // The control: a design doc may start in the feature.
    create_doc(f, client, &project, &in_feature, "design", "ui");
    let created_before = events_of(&project, "doc.created").len();

    let response = create_doc_response(f, client, &project, &in_feature, "design-system", "ui");
    assert_error(&response, 422, "design_system_is_global");
    assert_eq!(
        docs_of_kind(&project, "design-system"),
        0,
        "a refused request creates no doc"
    );
    assert_eq!(
        events_of(&project, "doc.created").len(),
        created_before,
        "a refused request emits nothing"
    );

    // A project-level change set starts the one design system.
    let global = new_changeset(f, client, &project, None);
    let design_system = create_doc(f, client, &project, &global, "design-system", "ui");
    assert_feature_id(&design_system["doc"], &Value::Null, "the design system");

    // With a design system in the project, a feature still cannot start one,
    // and a second project-level one is the usual conflict.
    let response = create_doc_response(f, client, &project, &in_feature, "design-system", "ui");
    assert_error(&response, 422, "design_system_is_global");
    let again = new_changeset(f, client, &project, None);
    let response = create_doc_response(f, client, &project, &again, "design-system", "ui");
    assert_error(&response, 409, "design_system_exists");
    assert_eq!(
        docs_of_kind(&project, "design-system"),
        1,
        "the project keeps one design system"
    );
}

/// ` id="<id>"` for a node the draft already has, nothing for a new one.
fn id_attr(ids: &HashMap<String, String>, name: &str) -> String {
    ids.get(name)
        .map(|id| format!(r#" id="{id}""#))
        .unwrap_or_default()
}

/// The ID of every node in `nodes` and their subtrees, by name.
fn ids_by_name(nodes: &Value) -> HashMap<String, String> {
    let mut ids = HashMap::new();
    let mut stack: Vec<&Value> = nodes
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    while let Some(node) = stack.pop() {
        if let (Some(name), Some(id)) = (node["name"].as_str(), node["id"].as_str()) {
            ids.insert(name.to_string(), id.to_string());
        }
        if let Some(children) = node["children"].as_array() {
            stack.extend(children);
        }
    }
    ids
}

fn cart_tags(ids: &HashMap<String, String>) -> String {
    format!(
        r#"<Component{} name="Cart" width={{16}} height={{16}}>
  <Vector{} name="Cart glyph" x={{0}} y={{0}} width={{16}} height={{16}} path="M0 0 L16 16"/>
</Component>
"#,
        id_attr(ids, "Cart"),
        id_attr(ids, "Cart glyph"),
    )
}

fn pay_page(component: &str) -> String {
    format!(
        r#"<Page name="Checkout">
  <Board name="Pay" x={{0}} y={{0}} width={{375}} height={{812}} dir="col">
    <Instance name="Cart icon" component="{component}"/>
  </Board>
</Page>
"#
    )
}

#[test]
fn a_design_in_a_feature_resolves_its_instances_against_the_project_design_system() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project, "Checkout");

    // The project's design system, with one component, merged at project
    // level.
    let global = new_changeset(f, client, &project, None);
    let design_system =
        id_of(&create_doc(f, client, &project, &global, "design-system", "ui")["doc"]);
    must(
        call(
            client,
            "PUT",
            &format!("{}/tokens", changeset_path(f, &project, &global)),
            &agent(f),
            Some(&json!({ "tokens": {
                "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
                "color": {
                    "$type": "color",
                    "surface": {
                        "$value": "#ffffff",
                        "$extensions": {"omnistate": {"modes": {"light": "#ffffff", "dark": "#0f172a"}}}
                    }
                }
            } })),
        ),
        200,
        "import the tokens",
    );
    let imported = must(
        call(
            client,
            "PUT",
            &format!("{}/components", changeset_path(f, &project, &global)),
            &agent(f),
            Some(&json!({ "tags": cart_tags(&HashMap::new()) })),
        ),
        200,
        "import the components",
    );
    let cart = ids_by_name(&imported["components"])
        .get("Cart")
        .unwrap_or_else(|| panic!("the import holds Cart: {imported}"))
        .clone();
    let design_system_version = id_of(&save(f, client, &project, &global, &design_system));
    merge(f, client, &project, &global);

    // A design doc of the feature draws an instance of the project's
    // component.
    let in_feature = new_changeset(f, client, &project, Some(&feature));
    let design = create_doc(f, client, &project, &in_feature, "design", "ui");
    assert_feature_id(&design["doc"], &json!(feature), "the feature's design");
    let design = id_of(&design["doc"]);
    let unknown = design_response(
        f,
        client,
        &project,
        &in_feature,
        &design,
        &pay_page("zzzzzzzz"),
    );
    assert_eq!(
        unknown.status,
        400,
        "an instance of a component the project does not have is refused: {}",
        unknown.text()
    );
    assert_eq!(
        unknown.json()["error"],
        "invalid_design",
        "{}",
        unknown.text()
    );
    must(
        design_response(f, client, &project, &in_feature, &design, &pay_page(&cart)),
        200,
        "import a design with an instance of the project's component",
    );

    let draft = must(
        call(
            client,
            "GET",
            &format!("{}/docs/{design}", changeset_path(f, &project, &in_feature)),
            &agent(f),
            None,
        ),
        200,
        "read the draft",
    )["draft"]
        .clone();
    assert_eq!(
        draft["design_system"]["doc_id"],
        design_system.as_str(),
        "the design resolves against the project's design system: {draft}"
    );
    assert_eq!(draft["design_system"]["source"], "mainline", "{draft}");
    let components = draft["components"]
        .as_array()
        .unwrap_or_else(|| panic!("the draft lists its components: {draft}"));
    assert!(
        components.iter().any(|c| c["id"] == cart.as_str()),
        "the instance resolves to the project's Cart: {draft}"
    );

    let saved = save(f, client, &project, &in_feature, &design);
    assert_eq!(
        saved["design_system_version_id"],
        design_system_version.as_str(),
        "the feature's design version records the project's design system version: {saved}"
    );
}

// ---------------------------------------------------------------------------
// Which docs a change set may draft
// ---------------------------------------------------------------------------

/// A project with the features Checkout and Search, and a merged PRD doc of
/// each scope: global, Checkout's and Search's.
struct Scoped {
    project: String,
    checkout: String,
    global_doc: String,
    checkout_doc: String,
    search_doc: String,
}

fn scoped(f: &Fixture, client: &Client) -> Scoped {
    let project = new_project(f, client);
    let checkout = new_feature(f, client, &project, "Checkout");
    let search = new_feature(f, client, &project, "Search");
    let global_doc = merged_doc(f, client, &project, None);
    let checkout_doc = merged_doc(f, client, &project, Some(&checkout));
    let search_doc = merged_doc(f, client, &project, Some(&search));
    Scoped {
        project,
        checkout,
        global_doc,
        checkout_doc,
        search_doc,
    }
}

#[test]
fn a_feature_change_set_drafts_its_own_feature_docs_and_the_global_docs() {
    let (f, client) = ready();
    let s = scoped(f, client);
    let changeset = new_changeset(f, client, &s.project, Some(&s.checkout));
    for doc in [&s.checkout_doc, &s.global_doc] {
        let opened = open_response(f, client, &s.project, &changeset, doc);
        assert_eq!(opened.status, 201, "open {doc}: {}", opened.text());
        assert_eq!(drafts_of(doc, &changeset), 1, "the change set drafts {doc}");
        add_paragraph(
            f,
            client,
            &s.project,
            &changeset,
            doc,
            "Edited in the feature.",
        );
    }
}

#[test]
fn a_feature_change_set_cannot_draft_a_doc_of_another_feature() {
    let (f, client) = ready();
    let s = scoped(f, client);
    let changeset = new_changeset(f, client, &s.project, Some(&s.checkout));
    let opened_before = events_of(&s.project, "draft.opened").len();

    let response = open_response(f, client, &s.project, &changeset, &s.search_doc);
    assert_error(&response, 422, "doc_in_other_feature");
    assert_eq!(
        drafts_of(&s.search_doc, &changeset),
        0,
        "a refused request creates no draft"
    );
    assert_eq!(
        events_of(&s.project, "draft.opened").len(),
        opened_before,
        "a refused request emits nothing"
    );
}

#[test]
fn a_project_level_change_set_cannot_draft_a_feature_doc() {
    let (f, client) = ready();
    let s = scoped(f, client);
    let changeset = new_changeset(f, client, &s.project, None);

    // The control: a project-level change set drafts a global doc.
    let opened = open_response(f, client, &s.project, &changeset, &s.global_doc);
    assert_eq!(opened.status, 201, "{}", opened.text());
    let opened_before = events_of(&s.project, "draft.opened").len();

    for doc in [&s.checkout_doc, &s.search_doc] {
        let response = open_response(f, client, &s.project, &changeset, doc);
        assert_error(&response, 422, "doc_in_other_feature");
        assert_eq!(
            drafts_of(doc, &changeset),
            0,
            "a refused request creates no draft of {doc}"
        );
    }
    assert_eq!(
        events_of(&s.project, "draft.opened").len(),
        opened_before,
        "a refused request emits nothing"
    );
}

#[test]
fn a_design_import_cannot_draft_a_design_of_another_scope() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let checkout = new_feature(f, client, &project, "Checkout");
    let search = new_feature(f, client, &project, "Search");

    // Search has a merged design.
    let in_search = new_changeset(f, client, &project, Some(&search));
    let design = id_of(&create_doc(f, client, &project, &in_search, "design", "ui")["doc"]);
    let tags = r#"<Page name="Search">
  <Board name="Results" x={0} y={0} width={375} height={812} dir="col">
    <Text name="Heading" content="Results" fontFamily="Inter" fontSize={14}/>
  </Board>
</Page>
"#;
    must(
        design_response(f, client, &project, &in_search, &design, tags),
        200,
        "import Search's design",
    );
    save(f, client, &project, &in_search, &design);
    merge(f, client, &project, &in_search);

    // The control: another change set of Search imports into it.
    let next = new_changeset(f, client, &project, Some(&search));
    must(
        design_response(f, client, &project, &next, &design, tags),
        200,
        "import into Search's design from Search",
    );

    let in_checkout = new_changeset(f, client, &project, Some(&checkout));
    let global = new_changeset(f, client, &project, None);
    for changeset in [&in_checkout, &global] {
        let response = design_response(f, client, &project, changeset, &design, tags);
        assert_error(&response, 422, "doc_in_other_feature");
        assert_eq!(
            drafts_of(&design, changeset),
            0,
            "a refused import creates no draft"
        );
    }
}

// ---------------------------------------------------------------------------
// The list of docs
// ---------------------------------------------------------------------------

#[test]
fn the_docs_route_is_in_the_registry_for_both() {
    let entry = registry()
        .into_iter()
        .find(|r| r.method == "GET" && r.path == DOCS_ROUTE)
        .unwrap_or_else(|| panic!("the registry has no route GET {DOCS_ROUTE}"));
    assert_eq!(entry.access, Access::Both, "GET {DOCS_ROUTE}");
}

#[test]
fn the_doc_list_has_every_doc_of_the_project_and_filters_by_feature_or_global() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let checkout = new_feature(f, client, &project, "Checkout");
    let search = new_feature(f, client, &project, "Search");
    let global = new_changeset(f, client, &project, None);
    let in_checkout = new_changeset(f, client, &project, Some(&checkout));
    let in_search = new_changeset(f, client, &project, Some(&search));
    let first = id_of(&create_doc(f, client, &project, &global, "html", "prd")["doc"]);
    let second = id_of(&create_doc(f, client, &project, &in_checkout, "html", "prd")["doc"]);
    let third = id_of(&create_doc(f, client, &project, &in_search, "design", "ui")["doc"]);
    let fourth = id_of(&create_doc(f, client, &project, &global, "html", "architecture")["doc"]);

    let path = docs_path(f, &project);
    for auth in [agent(f), human(f)] {
        let all = list_docs(client, &auth, &path);
        assert_eq!(
            ids(&all),
            [first.clone(), second.clone(), third.clone(), fourth.clone()],
            "without a filter, every doc of the project, oldest first"
        );
        for (listed, expected) in
            all.iter()
                .zip([Value::Null, json!(checkout), json!(search), Value::Null])
        {
            assert_feature_id(listed, &expected, "a listed doc");
            let shown = show_doc(f, client, &auth, &project, &id_of(listed));
            for field in [
                "project_id",
                "kind",
                "module",
                "title",
                "mainline_version_id",
            ] {
                assert_eq!(listed[field], shown[field], "{field}: {listed}");
            }
        }

        let of_checkout = list_docs(client, &auth, &format!("{path}?feature={checkout}"));
        assert_eq!(
            ids(&of_checkout),
            [second.clone()],
            "?feature=<id> lists that feature's docs only"
        );
        let global_only = list_docs(client, &auth, &format!("{path}?feature=global"));
        assert_eq!(
            ids(&global_only),
            [first.clone(), fourth.clone()],
            "?feature=global lists the global docs only"
        );
    }

    let response = call(
        client,
        "GET",
        &format!("{path}?feature=not-a-feature"),
        &agent(f),
        None,
    );
    assert_eq!(
        response.status,
        400,
        "a filter that is neither `global` nor an ID: {}",
        response.text()
    );
    let response = call(client, "GET", &docs_path(f, UNKNOWN_ID), &agent(f), None);
    assert_error(&response, 404, "not_found");

    // A project without docs lists none.
    let empty = new_project(f, client);
    assert!(list_docs(client, &agent(f), &docs_path(f, &empty)).is_empty());
}

// ---------------------------------------------------------------------------
// Finishing a feature (US-062)
// ---------------------------------------------------------------------------

const FEATURE_DONE_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/done";
const FEATURE_REOPEN_ROUTE: &str =
    "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/reopen";

fn feature_path(f: &Fixture, project: &str, feature: &str) -> String {
    format!("{}/features/{feature}", project_base(f, project))
}

/// `auth` calls `POST /features/{feature}/{action}` (`done` or `reopen`).
fn mark(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    feature: &str,
    action: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/{action}", feature_path(f, project, feature)),
        auth,
        Some(&json!({})),
    )
}

/// The human marks the feature done (`done`) or reopens it (`reopen`);
/// returns the Feature of the answer.
fn human_marks(f: &Fixture, client: &Client, project: &str, feature: &str, action: &str) -> Value {
    must(
        mark(f, client, &human(f), project, feature, action),
        200,
        &format!("the human calls {action}"),
    )["feature"]
        .clone()
}

fn status_of_feature(f: &Fixture, client: &Client, project: &str, feature: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &feature_path(f, project, feature),
            &agent(f),
            None,
        ),
        200,
        "show the feature",
    )["feature"]["status"]
        .clone()
}

/// The status of each of the project's features, in the list's order.
fn listed_statuses(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
) -> Vec<(String, Value)> {
    let listed = must(
        call(
            client,
            "GET",
            &format!("{}/features", project_base(f, project)),
            auth,
            None,
        ),
        200,
        "list the features",
    );
    listed["features"]
        .as_array()
        .unwrap_or_else(|| panic!("`features` is a list: {listed}"))
        .iter()
        .map(|feature| (id_of(feature), feature["status"].clone()))
        .collect()
}

fn changesets_of_feature(feature: &str) -> i64 {
    count(
        "select count(*) from changesets where feature_id = $1::uuid",
        &[feature],
    )
}

fn create_changeset_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/changesets", project_base(f, project)),
        &agent(f),
        Some(&json!({ "title": unique("change"), "run_tag": "run-61", "feature_id": feature })),
    )
}

/// Inserts a paragraph in the change set's draft of `doc`; returns its node.
fn paragraph_node(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    text: &str,
) -> String {
    let edited = must(
        call(
            client,
            "POST",
            &format!("{}/docs/{doc}/ops", changeset_path(f, project, changeset)),
            &agent(f),
            Some(&json!({ "ops": [{
                "op": "insert",
                "parent": null,
                "position": { "after": null },
                "content": format!("<p>{text}</p>")
            }] })),
        ),
        200,
        "insert a paragraph",
    );
    edited["results"][0]["node"]
        .as_str()
        .unwrap_or_else(|| panic!("an insert names its node: {edited}"))
        .to_string()
}

fn changeset_action(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    action: &str,
    body: Option<&Value>,
) -> Value {
    must(
        call(
            client,
            "POST",
            &format!("{}/{action}", changeset_path(f, project, changeset)),
            auth,
            body,
        ),
        200,
        &format!("{action} the change set"),
    )["changeset"]
        .clone()
}

#[test]
fn the_done_and_reopen_routes_are_human_routes_in_the_registry() {
    for path in [FEATURE_DONE_ROUTE, FEATURE_REOPEN_ROUTE] {
        let entry = registry()
            .into_iter()
            .find(|r| r.method == "POST" && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route POST {path}"));
        assert_eq!(entry.access, Access::Human, "POST {path}");
    }
}

#[test]
fn the_human_marks_a_feature_done_and_reopens_it_with_their_events() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project, "Checkout");
    let other = new_feature(f, client, &project, "Search");

    let done = human_marks(f, client, &project, &feature, "done");
    assert_eq!(done["id"], feature.as_str(), "{done}");
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(status_of_feature(f, client, &project, &feature), "done");
    for auth in [agent(f), human(f)] {
        assert_eq!(
            listed_statuses(f, client, &auth, &project),
            [
                (feature.clone(), json!("done")),
                (other.clone(), json!("active"))
            ],
            "the list shows the done feature as done and leaves the other active"
        );
    }
    let events = events_of(&project, "feature.done");
    assert_eq!(events.len(), 1, "one feature.done: {events:?}");
    assert_eq!(events[0]["feature_id"], feature.as_str(), "{}", events[0]);
    assert!(
        events_of(&project, "feature.reopened").is_empty(),
        "nothing was reopened yet"
    );

    let reopened = human_marks(f, client, &project, &feature, "reopen");
    assert_eq!(reopened["id"], feature.as_str(), "{reopened}");
    assert_eq!(reopened["status"], "active", "{reopened}");
    assert_eq!(status_of_feature(f, client, &project, &feature), "active");
    let events = events_of(&project, "feature.reopened");
    assert_eq!(events.len(), 1, "one feature.reopened: {events:?}");
    assert_eq!(events[0]["feature_id"], feature.as_str(), "{}", events[0]);

    // It can be marked done again after it was reopened.
    let again = human_marks(f, client, &project, &feature, "done");
    assert_eq!(again["status"], "done", "{again}");
    assert_eq!(events_of(&project, "feature.done").len(), 2);
}

#[test]
fn done_on_a_done_feature_and_reopen_on_an_active_one_are_conflicts() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project, "Checkout");

    let response = mark(f, client, &human(f), &project, &feature, "reopen");
    assert_error(&response, 409, "not_done");
    assert_eq!(status_of_feature(f, client, &project, &feature), "active");
    assert!(events_of(&project, "feature.reopened").is_empty());

    human_marks(f, client, &project, &feature, "done");
    let response = mark(f, client, &human(f), &project, &feature, "done");
    assert_error(&response, 409, "already_done");
    assert_eq!(status_of_feature(f, client, &project, &feature), "done");
    assert_eq!(
        events_of(&project, "feature.done").len(),
        1,
        "a refused done emits nothing"
    );

    for action in ["done", "reopen"] {
        let response = mark(f, client, &human(f), &project, UNKNOWN_ID, action);
        assert_error(&response, 404, "not_found");
    }
}

#[test]
fn an_api_key_cannot_mark_a_feature_done_or_reopen_it() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project, "Checkout");

    let response = mark(f, client, &agent(f), &project, &feature, "done");
    assert_error(&response, 403, "forbidden");
    assert_eq!(status_of_feature(f, client, &project, &feature), "active");
    assert!(events_of(&project, "feature.done").is_empty());

    human_marks(f, client, &project, &feature, "done");
    let response = mark(f, client, &agent(f), &project, &feature, "reopen");
    assert_error(&response, 403, "forbidden");
    assert_eq!(status_of_feature(f, client, &project, &feature), "done");
    assert!(events_of(&project, "feature.reopened").is_empty());
}

#[test]
fn a_done_feature_takes_no_new_change_sets() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project, "Checkout");
    new_changeset(f, client, &project, Some(&feature));
    human_marks(f, client, &project, &feature, "done");

    let before = changesets_of_feature(&feature);
    let response = create_changeset_response(f, client, &project, &feature);
    assert_error(&response, 409, "feature_done");
    assert_eq!(
        changesets_of_feature(&feature),
        before,
        "a refused change set is not created"
    );

    // The project and its other features still take change sets.
    new_changeset(f, client, &project, None);
    let other = new_feature(f, client, &project, "Search");
    new_changeset(f, client, &project, Some(&other));

    // Reopened, the feature takes change sets again.
    human_marks(f, client, &project, &feature, "reopen");
    new_changeset(f, client, &project, Some(&feature));
    assert_eq!(changesets_of_feature(&feature), before + 1);
}

#[test]
fn a_change_set_open_in_a_feature_keeps_its_lifecycle_after_the_feature_is_done() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project, "Checkout");
    let changeset = new_changeset(f, client, &project, Some(&feature));
    let doc = id_of(&create_doc(f, client, &project, &changeset, "html", "prd")["doc"]);
    add_paragraph(f, client, &project, &changeset, &doc, "Pay in one step.");
    save(f, client, &project, &changeset, &doc);

    human_marks(f, client, &project, &feature, "done");

    // The open change set goes on: edits, saves, submit, reject, reopen,
    // submit again, approve and merge.
    add_paragraph(
        f,
        client,
        &project,
        &changeset,
        &doc,
        "Pay with a saved card.",
    );
    let version = save(f, client, &project, &changeset, &doc);
    let submitted = changeset_action(f, client, &agent(f), &project, &changeset, "submit", None);
    assert_eq!(submitted["status"], "in_review", "{submitted}");
    let rejected = changeset_action(
        f,
        client,
        &human(f),
        &project,
        &changeset,
        "reject",
        Some(&json!({ "reason": "Name the card brands." })),
    );
    assert_eq!(rejected["status"], "rejected", "{rejected}");
    let reopened = changeset_action(f, client, &agent(f), &project, &changeset, "reopen", None);
    assert_eq!(reopened["status"], "open", "{reopened}");
    add_paragraph(
        f,
        client,
        &project,
        &changeset,
        &doc,
        "Visa and Mastercard.",
    );
    let last = save(f, client, &project, &changeset, &doc);
    assert_ne!(last["id"], version["id"], "a new version was saved");
    merge(f, client, &project, &changeset);

    let shown = show_doc(f, client, &human(f), &project, &doc);
    assert_eq!(
        shown["mainline_version_id"], last["id"],
        "the merge made the last version the doc's mainline: {shown}"
    );
    assert_feature_id(&shown, &json!(feature), "the merged doc");
    assert_eq!(status_of_feature(f, client, &project, &feature), "done");
}

#[test]
fn every_read_of_a_done_features_records_answers_as_before() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project, "Checkout");
    let changeset = new_changeset(f, client, &project, Some(&feature));
    let doc = id_of(&create_doc(f, client, &project, &changeset, "html", "prd")["doc"]);
    let node = paragraph_node(f, client, &project, &changeset, &doc, "Pay in one step.");
    let version = id_of(&save(f, client, &project, &changeset, &doc));
    let base = project_base(f, &project);

    let criteria = ["Fewest steps", "Works on phones"];
    let thread = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/threads", changeset_path(f, &project, &changeset)),
                &agent(f),
                Some(&json!({
                    "question": unique("How many steps to pay?"),
                    "criteria": criteria,
                    "sealed": false
                })),
            ),
            201,
            "start a thread",
        )["thread"],
    );
    let post = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/threads/{thread}/posts"),
                &agent(f),
                Some(&json!({
                    "type": "position",
                    "summary": unique("One step"),
                    "body": "One page with the card form keeps it short."
                })),
            ),
            201,
            "post in the thread",
        )["post"],
    );
    let decision = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/threads/{thread}/decisions"),
                &agent(f),
                Some(&json!({
                    "direction": unique("Pay on one page"),
                    "chosen": "One page",
                    "criteria": criteria,
                    "options": [
                        {
                            "name": "One page",
                            "scores": { "Fewest steps": 9, "Works on phones": 8 },
                            "lost_because": null
                        },
                        {
                            "name": "Three steps",
                            "scores": { "Fewest steps": 4, "Works on phones": 7 },
                            "lost_because": "Two more pages to load."
                        },
                        {
                            "name": "Pop-up",
                            "scores": { "Fewest steps": 7, "Works on phones": 3 },
                            "lost_because": "Pop-ups are hard to use on phones."
                        }
                    ],
                    "assumptions": ["Most buyers pay by card."],
                    "open_questions": [],
                    "dissent": [],
                    "posts": [post]
                })),
            ),
            201,
            "propose a decision",
        )["decision"],
    );
    let comment = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/versions/{version}/comments"),
                &human(f),
                Some(&json!({ "node": node, "text": "Say which cards." })),
            ),
            201,
            "comment on the paragraph",
        )["comment"],
    );
    merge(f, client, &project, &changeset);

    let reads = [
        format!("{base}/docs/{doc}"),
        format!("{base}/docs/{doc}/html"),
        format!("{base}/docs?feature={feature}"),
        format!("{base}/docs/{doc}/versions"),
        format!("{base}/versions/{version}"),
        format!("{base}/changesets/{changeset}"),
        format!("{base}/changesets?feature={feature}"),
        format!("{base}/threads/{thread}"),
        format!("{base}/threads/{thread}/digest"),
        format!("{base}/posts/{post}"),
        format!("{base}/decisions/{decision}"),
        format!("{base}/comments?doc={doc}"),
        format!("{base}/comments/{comment}"),
    ];
    let read_all = || -> Vec<(String, u16, String)> {
        let mut answers = Vec::new();
        for auth in [agent(f), human(f)] {
            for path in &reads {
                let response = call(client, "GET", path, &auth, None);
                answers.push((path.clone(), response.status, response.text()));
            }
        }
        answers
    };
    let before = read_all();
    for (path, status, text) in &before {
        assert_eq!(
            *status, 200,
            "GET {path} before the feature is done: {text}"
        );
    }
    // Each kind of record is really in what is read.
    let joined: String = before.iter().map(|(_, _, text)| text.as_str()).collect();
    for id in [&doc, &version, &thread, &post, &decision, &comment] {
        assert!(joined.contains(id.as_str()), "the reads show {id}");
    }

    human_marks(f, client, &project, &feature, "done");

    let after = read_all();
    for ((path, status, text), (_, _, text_before)) in after.iter().zip(&before) {
        assert_eq!(*status, 200, "GET {path} after the feature is done: {text}");
        let (now, then) = (parse(text), parse(text_before));
        assert_eq!(
            now, then,
            "GET {path} answers as it did before the feature was done"
        );
    }
}
