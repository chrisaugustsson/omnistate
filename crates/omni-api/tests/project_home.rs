//! US-040: the project home, through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. Several
//! agents share the key and tell themselves apart by their identity headers.
//! The web app's side is web/src/project-home.test.tsx.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; docs are those of
//! tests/live_docs.rs, threads those of tests/live_threads.rs and comments
//! those of tests/comments.rs):
//!
//! - `GET /home` (human) answers the project's page in one call: 200
//!   `{"project_id", "features", "app", "docs", "threads", "waiting"}`. An API
//!   key is 403 `{"error": "forbidden"}` (the page is the human's, and an agent
//!   must not read past a sealed thread through it); an unknown project is 404
//!   `{"error": "not_found"}`.
//! - `docs` lists every doc of the project, oldest first, each the Doc that
//!   `GET /docs/{doc_id}` answers (so with its `pillar`, `kind` and `title`).
//!   The web app groups them by pillar.
//! - `threads` lists every thread of the project, oldest first, each with at
//!   least the `id`, `feature_id` and `question` of the Thread that `GET
//!   /threads/{thread_id}` answers.
//! - `waiting.comments` is what waits for the human, oldest first: the comments
//!   whose latest reply is an agent's, each the Comment that `GET
//!   /comments/{comment_id}` answers. A comment without replies, one whose
//!   latest reply is the human's and one that the human deleted are not listed.
//!
//!   Nothing of another project is listed anywhere.
//! - `features` lists every feature of the project in creation order, each a
//!   FeatureSummary `{"feature", "docs", "waiting"}`: `feature` is the Feature
//!   that `GET /features/{feature_id}` answers, `docs` the number of the
//!   feature's own docs, and `waiting` the number of the items of `waiting`
//!   that carry its ID.
//! - Every item of `waiting` carries `feature_id`, that of its doc. `null` is
//!   the app level.
//! - `docs` and `threads` stay complete, app level and every feature, each item
//!   with its `feature_id`.
//!
//! US-112 adds what the scope pages show:
//!
//! - `app.last_activity` and each feature's `last_activity`: the time of the
//!   scope's latest write, a row of `doc_changes` (create, edit, rename or
//!   delete) of one of its docs, or a thread's creation, post or conclusion;
//!   comments and replies do not count. `null` while there is none.
//! - each doc's `updated_at` and `last_writer_role`, the time and the agent's
//!   role of its latest write, and `waiting`, how many of its comments wait
//!   for the human.
//! - each thread's `post_count`, and `last_post_at`, the time of its newest
//!   post (`null` without posts).

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

const TEST_DB: &str = "omni_us040_project_home";
const TEST_BUCKET: &str = "omni-us040-project-home";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";

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
    format!("us040-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

/// The agent that writes docs and starts threads:
/// `run-1/author`.
fn author(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-1",
        role: "author",
    }
}

/// Another agent, which posts in the threads: `run-1/critic`.
fn critic(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-1",
        role: "critic",
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

fn thread_path(f: &Fixture, project: &str, thread: &str) -> String {
    format!("{}/threads/{thread}", project_base(f, project))
}

/// What `auth` reads at `path` under the project, which must answer 200.
fn read(f: &Fixture, client: &Client, auth: &Auth, project: &str, path: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &format!("{}{path}", project_base(f, project)),
            auth,
            None,
        ),
        200,
        &format!("read {path}"),
    )
}

// ---------------------------------------------------------------------------
// The home
// ---------------------------------------------------------------------------

fn home_response(f: &Fixture, client: &Client, auth: &Auth, project: &str) -> Response {
    call(
        client,
        "GET",
        &format!("{}/home", project_base(f, project)),
        auth,
        None,
    )
}

/// The project's home as the human reads it.
fn home(f: &Fixture, client: &Client, project: &str) -> Value {
    let answer = must(
        home_response(f, client, &human(f), project),
        200,
        "the human reads the project home",
    );
    assert_eq!(answer["project_id"], project, "{answer}");
    answer
}

/// The list at `pointer` (such as `/docs` or `/waiting/comments`) of a home.
fn list_at(home: &Value, pointer: &str) -> Vec<Value> {
    home.pointer(pointer)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("the home has a list at {pointer}: {home}"))
        .clone()
}

/// The IDs of the list at `pointer` of a home, in order.
fn ids_at(home: &Value, pointer: &str) -> Vec<String> {
    list_at(home, pointer).iter().map(id_of).collect()
}

/// The item with `id` of the list at `pointer` of a home.
fn item_at(home: &Value, pointer: &str, id: &str) -> Value {
    list_at(home, pointer)
        .into_iter()
        .find(|item| item["id"] == id)
        .unwrap_or_else(|| panic!("{pointer} of the home lists {id}: {home}"))
}

// ---------------------------------------------------------------------------
// Projects and docs
// ---------------------------------------------------------------------------

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &author(f),
                Some(&json!({ "name": unique("home-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

/// Creates a doc in the project; returns its ID.
fn create_live(f: &Fixture, client: &Client, project: &str, kind: &str, pillar: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", project_base(f, project)),
                &author(f),
                Some(&json!({ "kind": kind, "pillar": pillar, "title": unique("Doc") })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

/// `auth` replies `text` to the comment.
fn reply_to_comment(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    comment: &str,
    text: &str,
) {
    must(
        call(
            client,
            "POST",
            &format!("{}/comments/{comment}/replies", project_base(f, project)),
            auth,
            Some(&json!({ "text": text })),
        ),
        201,
        "reply to the comment",
    );
}

/// The human deletes the comment (US-106).
fn delete_comment(f: &Fixture, client: &Client, project: &str, comment: &str) {
    let response = call(
        client,
        "DELETE",
        &format!("{}/comments/{comment}", project_base(f, project)),
        &human(f),
        None,
    );
    assert_eq!(
        response.status,
        204,
        "delete the comment: {}",
        response.text()
    );
}

// ---------------------------------------------------------------------------
// The route
// ---------------------------------------------------------------------------

#[test]
fn an_api_key_is_refused_the_home_and_an_unknown_project_is_not_found() {
    let (f, client) = ready();
    let project = new_project(f, client);

    assert_error(
        &home_response(f, client, &author(f), &project),
        403,
        "forbidden",
        "an API key reads the project home",
    );
    assert_error(
        &home_response(f, client, &human(f), UNKNOWN_ID),
        404,
        "not_found",
        "the human reads the home of a project that does not exist",
    );
}

// ---------------------------------------------------------------------------
// Docs and threads
// ---------------------------------------------------------------------------

#[test]
fn the_home_lists_every_doc_of_the_project_with_its_pillar() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let spec = create_live(f, client, &project, "html", "spec");
    let architecture = create_live(f, client, &project, "html", "architecture");
    let db = create_live(f, client, &project, "html", "db");
    let ui = create_live(f, client, &project, "design", "ui");

    // Another project's doc is not this project's.
    let elsewhere = new_project(f, client);
    let foreign = create_live(f, client, &elsewhere, "html", "spec");

    let home = home(f, client, &project);
    assert_eq!(
        ids_at(&home, "/docs"),
        vec![spec.clone(), architecture.clone(), db.clone(), ui.clone()],
        "every doc of the project, oldest first, and nothing else: {home}"
    );
    assert!(!home.to_string().contains(&foreign), "{home}");
    for (doc, pillar) in [
        (&spec, "spec"),
        (&architecture, "architecture"),
        (&db, "db"),
        (&ui, "ui"),
    ] {
        let listed = item_at(&home, "/docs", doc);
        assert_eq!(listed["pillar"], pillar, "{listed}");
        // US-112 adds what the scope pages show of the doc to the read's body.
        let mut body = listed.clone();
        for field in ["updated_at", "last_writer_role", "waiting"] {
            body.as_object_mut()
                .expect("a doc is an object")
                .remove(field);
        }
        assert_eq!(
            body,
            read(f, client, &human(f), &project, &format!("/docs/{doc}"))["doc"],
            "the home lists the doc as its read answers it"
        );
    }
}

#[test]
fn the_home_lists_every_thread_of_the_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let first = id_of(&scope_thread(f, client, &project, None));
    let second = id_of(&scope_thread(f, client, &project, Some(&feature)));
    let third = id_of(&scope_thread(f, client, &project, None));
    let elsewhere = new_project(f, client);
    let foreign = id_of(&scope_thread(f, client, &elsewhere, None));

    let home = home(f, client, &project);
    assert_eq!(
        ids_at(&home, "/threads"),
        vec![first.clone(), second.clone(), third.clone()],
        "every thread of the project, oldest first: {home}"
    );
    assert!(!home.to_string().contains(&foreign), "{home}");
    for thread in [&first, &second, &third] {
        let listed = item_at(&home, "/threads", thread);
        let read = read(
            f,
            client,
            &human(f),
            &project,
            &format!("/threads/{thread}"),
        )["thread"]
            .clone();
        for field in ["id", "feature_id", "question"] {
            assert_eq!(listed[field], read[field], "{field} of {listed}");
        }
    }
}

// ---------------------------------------------------------------------------
// Waiting for the human
// ---------------------------------------------------------------------------

#[test]
fn comments_wait_for_the_human_while_an_agent_replied_last() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = live_doc_in(f, client, &project, None);
    let node = write_paragraph(f, client, &author(f), &project, &doc);
    let unanswered = comment_on_head(f, client, &project, &doc, &node);
    let answered = comment_on_head(f, client, &project, &doc, &node);
    let answered_back = comment_on_head(f, client, &project, &doc, &node);
    let deleting = comment_on_head(f, client, &project, &doc, &node);
    for replied in [&answered, &answered_back, &deleting] {
        reply_to_comment(f, client, &author(f), &project, replied, "Shorter now.");
    }

    let first = home(f, client, &project);
    assert_eq!(
        ids_at(&first, "/waiting/comments"),
        vec![answered.clone(), answered_back.clone(), deleting.clone()],
        "the comments an agent replied to last wait, oldest first, and the unanswered one does not: {first}"
    );
    assert!(
        !ids_at(&first, "/waiting/comments").contains(&unanswered),
        "{first}"
    );
    assert_eq!(
        item_at(&first, "/waiting/comments", &answered),
        read(
            f,
            client,
            &human(f),
            &project,
            &format!("/comments/{answered}")
        )["comment"],
        "the home lists the comment as its read answers it"
    );

    reply_to_comment(
        f,
        client,
        &human(f),
        &project,
        &answered_back,
        "Still long.",
    );
    delete_comment(f, client, &project, &deleting);
    reply_to_comment(f, client, &critic(f), &project, &unanswered, "Shortened.");
    let after = home(f, client, &project);
    assert_eq!(
        ids_at(&after, "/waiting/comments"),
        vec![unanswered.clone(), answered.clone()],
        "a comment the human answered or deleted no longer waits; one an agent answered now does: {after}"
    );
}

// ---------------------------------------------------------------------------
// Features
// ---------------------------------------------------------------------------

/// Creates a feature as the author; returns its ID.
fn new_feature(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/features", project_base(f, project)),
                &author(f),
                Some(&json!({ "name": unique("feature"), "description": "A slice of work" })),
            ),
            201,
            "create a feature",
        )["feature"],
    )
}

/// The FeatureSummary of `feature` in the home's `features`.
fn summary_of(home: &Value, feature: &str) -> Value {
    list_at(home, "/features")
        .into_iter()
        .find(|summary| summary["feature"]["id"] == feature)
        .unwrap_or_else(|| panic!("`features` of the home lists {feature}: {home}"))
}

/// The IDs of the features that the home's `features` lists, in order.
fn feature_ids(home: &Value) -> Vec<String> {
    list_at(home, "/features")
        .iter()
        .map(|summary| id_of(&summary["feature"]))
        .collect()
}

/// (docs, waiting) of a FeatureSummary.
fn counts(summary: &Value) -> (Value, Value) {
    (summary["docs"].clone(), summary["waiting"].clone())
}

/// Asserts that the item with `id` of the list at `pointer` carries
/// `feature_id` (JSON `null` is project level).
fn assert_feature_of(home: &Value, pointer: &str, id: &str, feature: &Value) {
    let item = item_at(home, pointer, id);
    assert!(
        item.get("feature_id").is_some(),
        "{pointer}: {id} carries `feature_id`: {item}"
    );
    assert_eq!(
        &item["feature_id"], feature,
        "{pointer}: feature_id of {id}"
    );
}

#[test]
fn the_home_lists_the_features_in_creation_order_with_their_counts() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let first = new_feature(f, client, &project);
    let second = new_feature(f, client, &project);
    let quiet = new_feature(f, client, &project);

    // A doc at the app level, whose waiting comment counts for no feature.
    let global = live_doc_in(f, client, &project, None);
    waiting_comment(f, client, &project, &global);

    // The first feature: three docs, of which two have a comment that waits.
    let with_comment = live_doc_in(f, client, &project, Some(&first));
    let with_comments = live_doc_in(f, client, &project, Some(&first));
    live_doc_in(f, client, &project, Some(&first));
    waiting_comment(f, client, &project, &with_comment);
    waiting_comment(f, client, &project, &with_comments);

    // The second feature: one doc.
    live_doc_in(f, client, &project, Some(&second));

    // The third has nothing.

    // Another project's feature is not this project's.
    let elsewhere = new_project(f, client);
    let foreign = new_feature(f, client, &elsewhere);

    let home = home(f, client, &project);
    assert_eq!(
        feature_ids(&home),
        vec![first.clone(), second.clone(), quiet.clone()],
        "every feature of the project, the empty one too, in creation order: {home}"
    );
    assert!(!home.to_string().contains(&foreign), "{home}");
    for feature in [&first, &second, &quiet] {
        assert_eq!(
            summary_of(&home, feature)["feature"],
            read(
                f,
                client,
                &human(f),
                &project,
                &format!("/features/{feature}")
            )["feature"],
            "the summary holds the feature as its read answers it"
        );
    }
    for feature in [&first, &second, &quiet] {
        assert!(
            summary_of(&home, feature)["feature"]
                .get("status")
                .is_none(),
            "a feature has no status (US-110): {home}"
        );
    }
    assert_eq!(
        counts(&summary_of(&home, &first)),
        (json!(3), json!(2)),
        "the first feature: 3 docs of its own, and the 2 comments that wait on them: {home}"
    );
    assert_eq!(
        counts(&summary_of(&home, &second)),
        (json!(1), json!(0)),
        "the second feature: {home}"
    );
    assert_eq!(
        counts(&summary_of(&home, &quiet)),
        (json!(0), json!(0)),
        "the feature without docs: {home}"
    );
}

#[test]
fn every_waiting_item_carries_its_feature_and_counts_for_it() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let idle = new_feature(f, client, &project);
    let app_doc = live_doc_in(f, client, &project, None);
    let feature_doc = live_doc_in(f, client, &project, Some(&feature));
    let at_app_level = waiting_comment(f, client, &project, &app_doc);
    let in_feature = waiting_comment(f, client, &project, &feature_doc);

    let home = home(f, client, &project);
    let pointer = "/waiting/comments";
    assert_eq!(
        ids_at(&home, pointer),
        vec![at_app_level.clone(), in_feature.clone()],
        "{pointer} lists both, oldest first: {home}"
    );
    assert_feature_of(&home, pointer, &at_app_level, &Value::Null);
    assert_feature_of(&home, pointer, &in_feature, &json!(feature));
    assert_eq!(
        summary_of(&home, &feature)["waiting"],
        json!(1),
        "the comment of the feature waits for it: {home}"
    );
    assert_eq!(
        summary_of(&home, &idle)["waiting"],
        json!(0),
        "nothing waits for a feature that has nothing: {home}"
    );
}

#[test]
fn the_home_lists_the_docs_and_threads_of_the_project_and_every_feature() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let first = new_feature(f, client, &project);
    let second = new_feature(f, client, &project);
    let global = live_doc_in(f, client, &project, None);
    let global_thread = id_of(&scope_thread(f, client, &project, None));
    let first_doc = live_doc_in(f, client, &project, Some(&first));
    let first_thread = id_of(&scope_thread(f, client, &project, Some(&first)));
    let second_doc = live_doc_in(f, client, &project, Some(&second));
    let second_thread = id_of(&scope_thread(f, client, &project, Some(&second)));

    let home = home(f, client, &project);
    let none = Value::Null;
    let first_id = json!(first);
    let second_id = json!(second);
    for (pointer, items, path) in [
        (
            "/docs",
            [
                (&global, &none),
                (&first_doc, &first_id),
                (&second_doc, &second_id),
            ],
            "docs",
        ),
        (
            "/threads",
            [
                (&global_thread, &none),
                (&first_thread, &first_id),
                (&second_thread, &second_id),
            ],
            "threads",
        ),
    ] {
        assert_eq!(
            ids_at(&home, pointer),
            items
                .iter()
                .map(|(id, _)| (*id).clone())
                .collect::<Vec<_>>(),
            "{pointer}: the app level and every feature, oldest first: {home}"
        );
        for (id, feature) in items {
            assert_feature_of(&home, pointer, id, feature);
            let read = read(f, client, &human(f), &project, &format!("/{path}/{id}"));
            let singular = &path[..path.len() - 1];
            assert_eq!(
                item_at(&home, pointer, id)["feature_id"],
                read[singular]["feature_id"],
                "{pointer}: {id} has the feature its read answers"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// US-112: what the scope pages show
// ---------------------------------------------------------------------------

/// Creates an HTML doc at the app level or in `feature`; returns its ID.
fn live_doc_in(f: &Fixture, client: &Client, project: &str, feature: Option<&str>) -> String {
    let path = match feature {
        None => format!("{}/docs", project_base(f, project)),
        Some(feature) => format!("{}/features/{feature}/docs", project_base(f, project)),
    };
    id_of(
        &must(
            call(
                client,
                "POST",
                &path,
                &author(f),
                Some(&json!({ "kind": "html", "pillar": "spec", "title": unique("Doc") })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

/// `auth` writes a paragraph to the head of the doc; returns its node ID.
fn write_paragraph(f: &Fixture, client: &Client, auth: &Auth, project: &str, doc: &str) -> String {
    let answer = must(
        call(
            client,
            "POST",
            &format!("{}/docs/{doc}/ops", project_base(f, project)),
            auth,
            Some(&json!({ "ops": [{
                "op": "insert",
                "parent": null,
                "position": { "after": null },
                "content": format!("<p>{}</p>", unique("Orders ship in a day"))
            }] })),
        ),
        200,
        "write to the doc",
    );
    answer["results"][0]["node"]
        .as_str()
        .unwrap_or_else(|| panic!("the insert names its node: {answer}"))
        .to_string()
}

/// The time of the doc's latest write, as its activity list gives it.
fn latest_write(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    let activity = read(
        f,
        client,
        &human(f),
        project,
        &format!("/docs/{doc}/activity"),
    );
    activity["entries"][0]["created_at"]
        .as_str()
        .unwrap_or_else(|| panic!("the doc has a write: {activity}"))
        .to_string()
}

/// The author starts a thread at the app level or in `feature`; returns the thread.
fn scope_thread(f: &Fixture, client: &Client, project: &str, feature: Option<&str>) -> Value {
    let path = match feature {
        None => format!("{}/threads", project_base(f, project)),
        Some(feature) => format!("{}/features/{feature}/threads", project_base(f, project)),
    };
    must(
        call(
            client,
            "POST",
            &path,
            &author(f),
            Some(&json!({ "question": unique("Which way?"), "sealed": false })),
        ),
        201,
        "start a thread",
    )["thread"]
        .clone()
}

/// The critic posts in the thread; returns the post.
fn post_in(f: &Fixture, client: &Client, project: &str, thread: &str) -> Value {
    must(
        call(
            client,
            "POST",
            &format!("{}/posts", thread_path(f, project, thread)),
            &critic(f),
            Some(&json!({
                "summary": unique("A finding"),
                "body": "What it says",
                "links": []
            })),
        ),
        201,
        "post in the thread",
    )["post"]
        .clone()
}

fn conclude(f: &Fixture, client: &Client, project: &str, thread: &str) {
    must(
        call(
            client,
            "PUT",
            &format!("{}/conclusion", thread_path(f, project, thread)),
            &author(f),
            Some(&json!({ "conclusion": "Go with the split hero." })),
        ),
        200,
        "conclude the thread",
    );
}

/// A comment on a new paragraph of the doc that waits for the human: the human
/// made it and an agent replied last. Returns the comment's ID.
fn waiting_comment(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    let node = write_paragraph(f, client, &author(f), project, doc);
    let comment = comment_on_head(f, client, project, doc, &node);
    reply_to_comment(f, client, &author(f), project, &comment, "Shorter now.");
    comment
}

/// The human comments on `node` of the doc's head; returns the comment's ID.
fn comment_on_head(f: &Fixture, client: &Client, project: &str, doc: &str, node: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs/{doc}/comments", project_base(f, project)),
                &human(f),
                Some(&json!({ "node": node, "text": unique("Say more") })),
            ),
            201,
            "comment on the doc",
        )["comment"],
    )
}

fn app_activity(home: &Value) -> Value {
    home["app"]["last_activity"].clone()
}

fn feature_activity(home: &Value, feature: &str) -> Value {
    list_at(home, "/features")
        .into_iter()
        .find(|summary| summary["feature"]["id"] == feature)
        .unwrap_or_else(|| panic!("the home lists the feature {feature}: {home}"))["last_activity"]
        .clone()
}

fn at(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("a time: {value}"))
        .to_string()
}

#[test]
fn each_scope_has_the_time_of_its_latest_doc_or_thread_write_as_its_last_activity() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let home_now = || home(f, client, &project);
    assert_eq!(
        app_activity(&home_now()),
        Value::Null,
        "nothing written yet"
    );
    assert_eq!(feature_activity(&home_now(), &feature), Value::Null);

    let doc = live_doc_in(f, client, &project, None);
    let created = latest_write(f, client, &project, &doc);
    assert_eq!(at(&app_activity(&home_now())), created, "a doc's creation");
    assert_eq!(
        feature_activity(&home_now(), &feature),
        Value::Null,
        "the app's doc is not the feature's"
    );

    let thread = scope_thread(f, client, &project, None);
    assert_eq!(
        at(&app_activity(&home_now())),
        at(&thread["created_at"]),
        "a thread's start"
    );
    let posted = at(&post_in(f, client, &project, &id_of(&thread))["created_at"]);
    assert_eq!(at(&app_activity(&home_now())), posted, "a post");
    conclude(f, client, &project, &id_of(&thread));
    let concluded = at(&app_activity(&home_now()));
    assert!(
        concluded > posted,
        "a conclusion: {concluded} after {posted}"
    );

    write_paragraph(f, client, &critic(f), &project, &doc);
    let edited = latest_write(f, client, &project, &doc);
    assert!(edited > concluded, "{edited}");
    assert_eq!(at(&app_activity(&home_now())), edited, "an edit");

    let feature_doc = live_doc_in(f, client, &project, Some(&feature));
    assert_eq!(
        at(&feature_activity(&home_now(), &feature)),
        latest_write(f, client, &project, &feature_doc),
        "the feature's own doc"
    );
    let feature_thread = scope_thread(f, client, &project, Some(&feature));
    assert_eq!(
        at(&feature_activity(&home_now(), &feature)),
        at(&feature_thread["created_at"]),
        "the feature's own thread"
    );
    assert_eq!(
        at(&app_activity(&home_now())),
        edited,
        "the feature's doc and thread are not the app's"
    );

    let response = call(
        client,
        "DELETE",
        &format!("{}/docs/{doc}", project_base(f, &project)),
        &author(f),
        None,
    );
    assert!(response.status < 300, "delete the doc: {}", response.text());
    let deleted = at(&app_activity(&home_now()));
    assert!(
        deleted > edited,
        "a deleted doc's delete counts: {deleted} after {edited}"
    );
}

#[test]
fn comments_and_replies_are_no_activity() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = live_doc_in(f, client, &project, None);
    let node = write_paragraph(f, client, &author(f), &project, &doc);
    let before = app_activity(&home(f, client, &project));
    assert_eq!(at(&before), latest_write(f, client, &project, &doc));

    let comment = comment_on_head(f, client, &project, &doc, &node);
    reply_to_comment(f, client, &critic(f), &project, &comment, "Done.");
    reply_to_comment(f, client, &human(f), &project, &comment, "Thanks.");
    assert_eq!(app_activity(&home(f, client, &project)), before);
}

#[test]
fn each_doc_has_when_and_by_which_role_it_was_last_written_and_how_many_of_its_comments_wait() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let doc = live_doc_in(f, client, &project, None);
    let other = live_doc_in(f, client, &project, None);
    let listed = item_at(&home(f, client, &project), "/docs", &doc);
    assert_eq!(
        listed["updated_at"],
        latest_write(f, client, &project, &doc),
        "{listed}"
    );
    assert_eq!(listed["last_writer_role"], "author", "{listed}");
    assert_eq!(listed["waiting"], 0, "{listed}");

    let node = write_paragraph(f, client, &critic(f), &project, &doc);
    let elsewhere = write_paragraph(f, client, &author(f), &project, &other);
    // Two comments on the doc that an agent replied to last, one that no one
    // replied to, and one waiting on the other doc.
    let first = comment_on_head(f, client, &project, &doc, &node);
    reply_to_comment(f, client, &author(f), &project, &first, "Done.");
    let second = comment_on_head(f, client, &project, &doc, &node);
    reply_to_comment(f, client, &author(f), &project, &second, "Done.");
    comment_on_head(f, client, &project, &doc, &node);
    let third = comment_on_head(f, client, &project, &other, &elsewhere);
    reply_to_comment(f, client, &author(f), &project, &third, "Done.");

    let home = home(f, client, &project);
    let listed = item_at(&home, "/docs", &doc);
    assert_eq!(
        listed["updated_at"],
        latest_write(f, client, &project, &doc),
        "{listed}"
    );
    assert_eq!(
        listed["last_writer_role"], "critic",
        "the role of the latest write: {listed}"
    );
    assert_eq!(listed["waiting"], 2, "{listed}");
    assert_eq!(item_at(&home, "/docs", &other)["waiting"], 1, "{home}");
}

#[test]
fn each_thread_has_its_post_count_and_the_time_of_its_newest_post() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let quiet = id_of(&scope_thread(f, client, &project, None));
    let busy = id_of(&scope_thread(f, client, &project, None));
    post_in(f, client, &project, &busy);
    let newest = post_in(f, client, &project, &busy);

    let home = home(f, client, &project);
    let listed = item_at(&home, "/threads", &quiet);
    assert_eq!(listed["post_count"], 0, "{listed}");
    assert_eq!(listed["last_post_at"], Value::Null, "{listed}");
    let listed = item_at(&home, "/threads", &busy);
    assert_eq!(listed["post_count"], 2, "{listed}");
    assert_eq!(listed["last_post_at"], newest["created_at"], "{listed}");
}
