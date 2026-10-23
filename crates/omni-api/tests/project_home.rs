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
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; change sets are those of
//! US-016 and US-025, docs and versions those of US-022 and US-024, threads
//! and posts those of US-033 to US-035, decisions those of US-037 and comments
//! those of US-038):
//!
//! - `GET /home` (human) answers the reviewer's page of the project in one
//!   call: 200 `{"project_id", "docs", "changesets", "threads", "waiting"}`.
//!   An API key is 403 `{"error": "forbidden"}` (the page is the human's, and
//!   an agent must not read past a sealed phase through it); an unknown
//!   project is 404 `{"error": "not_found"}`.
//! - `docs` lists every doc of the project, oldest first, each the Doc that
//!   `GET /docs/{doc_id}` answers (so with its `module`, `kind`, `title` and
//!   `mainline_version_id`, which is `null` while its first version has not
//!   merged). The web app groups them by module.
//! - `changesets` lists every change set of the project, oldest first, each
//!   the Changeset that `GET /changesets/{changeset_id}` answers the human,
//!   with its `status`.
//! - `threads` lists every thread of the project, oldest first, each with at
//!   least the `id`, `changeset_id` and `question` of the Thread that `GET
//!   /threads/{thread_id}` answers.
//! - `waiting` is what waits for the human, each list oldest first:
//!   - `decisions`: the decisions whose status is `proposed`, each the
//!     Decision that `GET /decisions/{decision_id}` answers the human. One
//!     that the human approved or rejected, or that a later proposal
//!     superseded, is not listed.
//!   - `changesets`: the change sets whose status is `in_review`, each as in
//!     `changesets`.
//!   - `comments`: the comments whose status is `addressed`, each the Comment
//!     that `GET /comments/{comment_id}` answers. A resolved or reopened one
//!     is not listed.
//!   - `questions`: the posts of type `question` that an agent wrote, in any
//!     phase (the human sees sealed phases too), each the Post that `GET
//!     /posts/{post_id}` answers the human, until a post of the human links to
//!     it as a reply: a post with `{"kind": "post", "post_id"}` naming it in
//!     its `links`, made with the posts route or the directives route. An
//!     agent's post that links to it does not take it off the list, and the
//!     human's own questions are never listed.
//!
//!   Nothing of another project is listed anywhere.
//! - Every change that moves an item onto or off these lists already emits
//!   its event on the project's live channel (`decision.*`, `changeset.*`,
//!   `comment.status_changed`, `post.created`); the web app reloads the home
//!   on the project's events.
//!
//! US-063 ties the home to the project's features (Q51; features are those of
//! US-059 to US-062):
//!
//! - `features` lists every feature of the project, done ones too, in
//!   creation order, each a FeatureSummary `{"feature", "docs",
//!   "open_changesets", "waiting"}`: `feature` is the Feature that `GET
//!   /features/{feature_id}` answers, `docs` the number of the feature's own
//!   docs (not the global docs its change sets draft), `open_changesets` the
//!   number of its change sets that are `open`, `in_review`, `approved` or
//!   `needs_rebase` (not `merged`, `rejected` or `abandoned`), and `waiting`
//!   the number of the items of `waiting` that carry its ID.
//! - Every item of `waiting` carries `feature_id`: a decision, change set or
//!   question that of its change set, a comment that of its doc (so a comment
//!   on a global doc is project level even when a feature's change set saved
//!   the version). `null` is project level.
//! - `docs`, `changesets` and `threads` stay complete, project level and every
//!   feature, each item with its `feature_id`.

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

const TEST_DB: &str = "omni_us040_project_home";
const TEST_BUCKET: &str = "omni-us040-project-home";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
const HOME_ROUTE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}/home";

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

/// The agent that writes docs, starts threads and proposes decisions:
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

fn changeset_path(f: &Fixture, project: &str, changeset: &str) -> String {
    format!("{}/changesets/{changeset}", project_base(f, project))
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

/// The list at `pointer` (such as `/docs` or `/waiting/decisions`) of a home.
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
// Projects, change sets and docs
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

fn new_changeset(f: &Fixture, client: &Client, project: &str) -> String {
    new_changeset_in(f, client, project, None)
}

/// Opens a change set in `feature`, or at project level without one.
fn new_changeset_in(f: &Fixture, client: &Client, project: &str, feature: Option<&str>) -> String {
    let mut body = json!({ "title": unique("change"), "run_tag": "run-1" });
    if let Some(feature) = feature {
        body["feature_id"] = json!(feature);
    }
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/changesets", project_base(f, project)),
                &author(f),
                Some(&body),
            ),
            201,
            "create a change set",
        )["changeset"],
    )
}

fn create_doc(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    kind: &str,
    module: &str,
) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/docs", changeset_path(f, project, changeset)),
                &author(f),
                Some(&json!({ "kind": kind, "module": module, "title": unique("Doc") })),
            ),
            201,
            "create a doc",
        )["doc"],
    )
}

/// Inserts one paragraph into the change set's draft of the HTML doc `doc`;
/// returns the new node's ID.
fn insert_paragraph(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
) -> String {
    let edited = must(
        call(
            client,
            "POST",
            &format!("{}/docs/{doc}/ops", changeset_path(f, project, changeset)),
            &author(f),
            Some(&json!({ "ops": [{
                "op": "insert",
                "parent": null,
                "position": { "after": null },
                "content": format!("<p>{}</p>", unique("The hero is split"))
            }] })),
        ),
        200,
        "edit the draft",
    );
    edited["results"][0]["node"]
        .as_str()
        .unwrap_or_else(|| panic!("an insert names its node: {edited}"))
        .to_string()
}

/// Saves the change set's draft of `doc`; returns the version's ID.
fn save(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!(
                    "{}/docs/{doc}/versions",
                    changeset_path(f, project, changeset)
                ),
                &author(f),
                Some(&json!({})),
            ),
            201,
            "save a version",
        )["version"],
    )
}

/// A change set of `project` with one saved version of a new PRD doc, still
/// open: (change set, doc, paragraph node, version).
fn changeset_with_version(
    f: &Fixture,
    client: &Client,
    project: &str,
) -> (String, String, String, String) {
    changeset_with_version_in(f, client, project, None)
}

/// As `changeset_with_version`, with the change set in `feature` or at
/// project level without one.
fn changeset_with_version_in(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: Option<&str>,
) -> (String, String, String, String) {
    let changeset = new_changeset_in(f, client, project, feature);
    let doc = create_doc(f, client, project, &changeset, "html", "prd");
    let node = insert_paragraph(f, client, project, &changeset, &doc);
    let version = save(f, client, project, &changeset, &doc);
    (changeset, doc, node, version)
}

/// `auth` moves the change set on with `action` (`submit`, `approve`,
/// `reject`, `reopen`); returns the status it has after that.
fn act_on_changeset(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    action: &str,
    body: Option<&Value>,
) -> String {
    let answer = must(
        call(
            client,
            "POST",
            &format!("{}/{action}", changeset_path(f, project, changeset)),
            auth,
            body,
        ),
        200,
        &format!("{action} the change set"),
    );
    answer["changeset"]["status"]
        .as_str()
        .unwrap_or_else(|| panic!("the change set has a status: {answer}"))
        .to_string()
}

fn submit(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let status = act_on_changeset(f, client, &author(f), project, changeset, "submit", None);
    assert_eq!(status, "in_review");
}

/// The human approves the change set, which merges it.
fn approve_changeset(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let status = act_on_changeset(
        f,
        client,
        &human(f),
        project,
        changeset,
        "approve",
        Some(&json!({})),
    );
    assert_eq!(status, "merged");
}

fn reject_changeset(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let status = act_on_changeset(
        f,
        client,
        &human(f),
        project,
        changeset,
        "reject",
        Some(&json!({ "reason": "The hero needs a dark variant." })),
    );
    assert_eq!(status, "rejected");
}

fn reopen_changeset(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let status = act_on_changeset(f, client, &author(f), project, changeset, "reopen", None);
    assert_eq!(status, "open");
}

// ---------------------------------------------------------------------------
// Threads, posts and decisions
// ---------------------------------------------------------------------------

/// The author starts a thread in the change set; returns its ID.
fn new_thread(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    sealed: bool,
) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/threads", changeset_path(f, project, changeset)),
                &author(f),
                Some(&json!({
                    "question": unique("Which hero layout?"),
                    "criteria": ["Readable at 320 px", "Fits above the fold"],
                    "sealed": sealed
                })),
            ),
            201,
            "start a thread",
        )["thread"],
    )
}

/// `auth` posts a post of `kind` that links to `links` in the thread; returns
/// the post's ID.
fn post(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    kind: &str,
    links: &[&str],
) -> String {
    let links: Vec<Value> = links
        .iter()
        .map(|id| json!({ "kind": "post", "post_id": id }))
        .collect();
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/posts", thread_path(f, project, thread)),
                auth,
                Some(&json!({
                    "type": kind,
                    "summary": unique(&format!("A {kind}")),
                    "body": unique("What the post says"),
                    "links": links
                })),
            ),
            201,
            &format!("post a {kind}"),
        )["post"],
    )
}

/// The human gives a directive in the thread that links to `links`; returns
/// the post's ID.
fn directive(f: &Fixture, client: &Client, project: &str, thread: &str, links: &[&str]) -> String {
    let links: Vec<Value> = links
        .iter()
        .map(|id| json!({ "kind": "post", "post_id": id }))
        .collect();
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/directives", thread_path(f, project, thread)),
                &human(f),
                Some(&json!({
                    "summary": unique("Use the split hero"),
                    "body": "Go with the split hero; the photo fails contrast.",
                    "links": links
                })),
            ),
            201,
            "give a directive",
        )["post"],
    )
}

/// The author proposes a decision for the thread; returns its ID.
fn propose(f: &Fixture, client: &Client, project: &str, thread: &str) -> String {
    let body = json!({
        "direction": unique("Use the split hero: copy on the left, product shot on the right"),
        "chosen": "Split hero",
        "criteria": ["Readable at 320 px"],
        "options": [
            { "name": "Split hero", "scores": { "Readable at 320 px": 8 }, "lost_because": null },
            {
                "name": "Full-bleed photo",
                "scores": { "Readable at 320 px": 5 },
                "lost_because": "Text over the photo fails contrast at 320 px."
            }
        ],
        "assumptions": [],
        "open_questions": [],
        "dissent": [],
        "posts": []
    });
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/decisions", thread_path(f, project, thread)),
                &author(f),
                Some(&body),
            ),
            201,
            "propose a decision",
        )["decision"],
    )
}

/// The human approves or rejects the decision.
fn decide(f: &Fixture, client: &Client, project: &str, decision: &str, action: &str) {
    let body = if action == "reject" {
        json!({ "reason": "The photo is off-brand." })
    } else {
        json!({})
    };
    must(
        call(
            client,
            "POST",
            &format!("{}/decisions/{decision}/{action}", project_base(f, project)),
            &human(f),
            Some(&body),
        ),
        200,
        &format!("the human acts on the decision: {action}"),
    );
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

/// The human comments on `node` of `version`; returns the comment's ID.
fn comment(f: &Fixture, client: &Client, project: &str, version: &str, node: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/versions/{version}/comments", project_base(f, project)),
                &human(f),
                Some(&json!({ "node": node, "text": unique("Make the copy shorter") })),
            ),
            201,
            "comment on a node",
        )["comment"],
    )
}

/// `auth` acts on the comment with `action` and `body`.
fn act_on_comment(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    comment: &str,
    action: &str,
    body: &Value,
) {
    let status = if action == "replies" { 201 } else { 200 };
    must(
        call(
            client,
            "POST",
            &format!("{}/comments/{comment}/{action}", project_base(f, project)),
            auth,
            Some(body),
        ),
        status,
        &format!("act on the comment: {action}"),
    );
}

// ---------------------------------------------------------------------------
// The route
// ---------------------------------------------------------------------------

#[test]
fn the_home_route_is_a_human_route_in_the_registry() {
    let entry = registry()
        .into_iter()
        .find(|r| r.method == "GET" && r.path == HOME_ROUTE)
        .unwrap_or_else(|| panic!("the registry has no route GET {HOME_ROUTE}"));
    assert_eq!(entry.access, Access::Human, "GET {HOME_ROUTE}");
}

#[test]
fn an_api_key_is_refused_the_home_and_an_unknown_project_is_not_found() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (changeset, _, _, _) = changeset_with_version(f, client, &project);
    let thread = new_thread(f, client, &project, &changeset, true);
    post(f, client, &critic(f), &project, &thread, "question", &[]);

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
// Docs, change sets and threads
// ---------------------------------------------------------------------------

#[test]
fn the_home_lists_every_doc_of_the_project_with_its_module() {
    let (f, client) = ready();
    let project = new_project(f, client);

    // A PRD doc that merged, and docs of the other modules whose first version
    // has not merged yet.
    let (merged, prd, _, _) = changeset_with_version(f, client, &project);
    submit(f, client, &project, &merged);
    approve_changeset(f, client, &project, &merged);
    let open = new_changeset(f, client, &project);
    let architecture = create_doc(f, client, &project, &open, "html", "architecture");
    let db = create_doc(f, client, &project, &open, "html", "db");
    let ui = create_doc(f, client, &project, &open, "design", "ui");

    // Another project's doc is not this project's.
    let elsewhere = new_project(f, client);
    let (_, foreign, _, _) = changeset_with_version(f, client, &elsewhere);

    let home = home(f, client, &project);
    assert_eq!(
        ids_at(&home, "/docs"),
        vec![prd.clone(), architecture.clone(), db.clone(), ui.clone()],
        "every doc of the project, oldest first, and nothing else: {home}"
    );
    assert!(!home.to_string().contains(&foreign), "{home}");
    for (doc, module) in [
        (&prd, "prd"),
        (&architecture, "architecture"),
        (&db, "db"),
        (&ui, "ui"),
    ] {
        let listed = item_at(&home, "/docs", doc);
        assert_eq!(listed["module"], module, "{listed}");
        assert_eq!(
            listed,
            read(f, client, &human(f), &project, &format!("/docs/{doc}"))["doc"],
            "the home lists the doc as its read answers it"
        );
    }
    assert!(
        item_at(&home, "/docs", &prd)["mainline_version_id"].is_string(),
        "the merged doc has its mainline version: {home}"
    );
    assert!(
        item_at(&home, "/docs", &ui)["mainline_version_id"].is_null(),
        "a doc whose first version has not merged has none: {home}"
    );
}

#[test]
fn the_home_lists_every_change_set_with_its_status_oldest_first() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let open = new_changeset(f, client, &project);
    let (in_review, _, _, _) = changeset_with_version(f, client, &project);
    submit(f, client, &project, &in_review);
    let (rejected, _, _, _) = changeset_with_version(f, client, &project);
    submit(f, client, &project, &rejected);
    reject_changeset(f, client, &project, &rejected);
    let (merged, _, _, _) = changeset_with_version(f, client, &project);
    submit(f, client, &project, &merged);
    approve_changeset(f, client, &project, &merged);
    let elsewhere = new_project(f, client);
    let foreign = new_changeset(f, client, &elsewhere);

    let home = home(f, client, &project);
    assert_eq!(
        ids_at(&home, "/changesets"),
        vec![
            open.clone(),
            in_review.clone(),
            rejected.clone(),
            merged.clone()
        ],
        "every change set of the project, oldest first: {home}"
    );
    assert!(!home.to_string().contains(&foreign), "{home}");
    for (changeset, status) in [
        (&open, "open"),
        (&in_review, "in_review"),
        (&rejected, "rejected"),
        (&merged, "merged"),
    ] {
        let listed = item_at(&home, "/changesets", changeset);
        assert_eq!(listed["status"], status, "{listed}");
        assert_eq!(
            listed,
            read(
                f,
                client,
                &human(f),
                &project,
                &format!("/changesets/{changeset}")
            )["changeset"],
            "the home lists the change set as its read answers the human"
        );
    }
}

#[test]
fn the_home_lists_every_thread_of_the_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (merged, _, _, _) = changeset_with_version(f, client, &project);
    let first = new_thread(f, client, &project, &merged, false);
    submit(f, client, &project, &merged);
    approve_changeset(f, client, &project, &merged);
    let open = new_changeset(f, client, &project);
    let second = new_thread(f, client, &project, &open, true);
    let third = new_thread(f, client, &project, &open, false);
    let elsewhere = new_project(f, client);
    let foreign_changeset = new_changeset(f, client, &elsewhere);
    let foreign = new_thread(f, client, &elsewhere, &foreign_changeset, false);

    let home = home(f, client, &project);
    assert_eq!(
        ids_at(&home, "/threads"),
        vec![first.clone(), second.clone(), third.clone()],
        "every thread of the project, those of a merged change set too, oldest first: {home}"
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
        for field in ["id", "changeset_id", "question"] {
            assert_eq!(listed[field], read[field], "{field} of {listed}");
        }
    }
}

// ---------------------------------------------------------------------------
// Waiting for the human
// ---------------------------------------------------------------------------

#[test]
fn decisions_wait_for_the_human_while_they_are_proposed() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let approving = new_thread(f, client, &project, &changeset, false);
    let rejecting = new_thread(f, client, &project, &changeset, false);
    let superseding = new_thread(f, client, &project, &changeset, false);
    let to_approve = propose(f, client, &project, &approving);
    let to_reject = propose(f, client, &project, &rejecting);
    let superseded = propose(f, client, &project, &superseding);
    let elsewhere = new_project(f, client);
    let foreign_changeset = new_changeset(f, client, &elsewhere);
    let foreign_thread = new_thread(f, client, &elsewhere, &foreign_changeset, false);
    let foreign = propose(f, client, &elsewhere, &foreign_thread);

    let first = home(f, client, &project);
    assert_eq!(
        ids_at(&first, "/waiting/decisions"),
        vec![to_approve.clone(), to_reject.clone(), superseded.clone()],
        "the proposed decisions of the project wait, oldest first: {first}"
    );
    assert!(!first.to_string().contains(&foreign), "{first}");
    assert_eq!(
        item_at(&first, "/waiting/decisions", &to_approve),
        read(
            f,
            client,
            &human(f),
            &project,
            &format!("/decisions/{to_approve}")
        )["decision"],
        "the home lists the decision as its read answers the human"
    );

    decide(f, client, &project, &to_approve, "approve");
    decide(f, client, &project, &to_reject, "reject");
    let successor = propose(f, client, &project, &superseding);

    let after = home(f, client, &project);
    assert_eq!(
        ids_at(&after, "/waiting/decisions"),
        vec![successor.clone()],
        "approved, rejected and superseded decisions no longer wait; the new proposal does: {after}"
    );
}

#[test]
fn change_sets_wait_for_the_human_while_they_are_in_review() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (open, _, _, _) = changeset_with_version(f, client, &project);
    let (approving, _, _, _) = changeset_with_version(f, client, &project);
    let (rejecting, _, _, _) = changeset_with_version(f, client, &project);
    for changeset in [&approving, &rejecting] {
        submit(f, client, &project, changeset);
    }

    let first = home(f, client, &project);
    assert_eq!(
        ids_at(&first, "/waiting/changesets"),
        vec![approving.clone(), rejecting.clone()],
        "the change sets in review wait, oldest first, and the open one does not: {first}"
    );
    assert_eq!(
        item_at(&first, "/waiting/changesets", &approving),
        read(
            f,
            client,
            &human(f),
            &project,
            &format!("/changesets/{approving}")
        )["changeset"],
        "the home lists the change set as its read answers the human"
    );

    approve_changeset(f, client, &project, &approving);
    reject_changeset(f, client, &project, &rejecting);
    let decided = home(f, client, &project);
    assert_eq!(
        ids_at(&decided, "/waiting/changesets"),
        Vec::<String>::new(),
        "merged and rejected change sets no longer wait: {decided}"
    );

    reopen_changeset(f, client, &project, &rejecting);
    submit(f, client, &project, &rejecting);
    submit(f, client, &project, &open);
    let again = home(f, client, &project);
    assert_eq!(
        ids_at(&again, "/waiting/changesets"),
        vec![open.clone(), rejecting.clone()],
        "a change set submitted again waits again: {again}"
    );
}

#[test]
fn comments_wait_for_the_human_while_they_are_marked_addressed() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (changeset, doc, node, v1) = changeset_with_version(f, client, &project);
    let open = comment(f, client, &project, &v1, &node);
    let resolving = comment(f, client, &project, &v1, &node);
    let reopening = comment(f, client, &project, &v1, &node);
    let replied = comment(f, client, &project, &v1, &node);
    insert_paragraph(f, client, &project, &changeset, &doc);
    let v2 = save(f, client, &project, &changeset, &doc);
    for addressed in [&resolving, &reopening, &replied] {
        act_on_comment(
            f,
            client,
            &author(f),
            &project,
            addressed,
            "addressed",
            &json!({ "version_id": v2 }),
        );
    }

    let first = home(f, client, &project);
    assert_eq!(
        ids_at(&first, "/waiting/comments"),
        vec![resolving.clone(), reopening.clone(), replied.clone()],
        "the comments marked addressed wait, oldest first, and the open one does not: {first}"
    );
    assert!(
        !ids_at(&first, "/waiting/comments").contains(&open),
        "{first}"
    );
    assert_eq!(
        item_at(&first, "/waiting/comments", &resolving),
        read(
            f,
            client,
            &human(f),
            &project,
            &format!("/comments/{resolving}")
        )["comment"],
        "the home lists the comment as its read answers it"
    );

    act_on_comment(
        f,
        client,
        &human(f),
        &project,
        &resolving,
        "resolve",
        &json!({}),
    );
    act_on_comment(
        f,
        client,
        &human(f),
        &project,
        &reopening,
        "reopen",
        &json!({}),
    );
    act_on_comment(
        f,
        client,
        &critic(f),
        &project,
        &replied,
        "replies",
        &json!({ "text": "Shorter now." }),
    );
    let after = home(f, client, &project);
    assert_eq!(
        ids_at(&after, "/waiting/comments"),
        vec![replied.clone()],
        "resolved and reopened comments no longer wait; a reply leaves an addressed one waiting: {after}"
    );
}

#[test]
fn agents_questions_wait_until_a_human_post_links_to_them_as_a_reply() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let open_thread = new_thread(f, client, &project, &changeset, false);
    let sealed_thread = new_thread(f, client, &project, &changeset, true);

    let question = post(
        f,
        client,
        &critic(f),
        &project,
        &open_thread,
        "question",
        &[],
    );
    // In a sealed phase that no other agent has posted in: the human sees it.
    let sealed = post(
        f,
        client,
        &critic(f),
        &project,
        &sealed_thread,
        "question",
        &[],
    );
    let position = post(
        f,
        client,
        &critic(f),
        &project,
        &open_thread,
        "position",
        &[],
    );
    let own = post(
        f,
        client,
        &human(f),
        &project,
        &open_thread,
        "question",
        &[],
    );
    let elsewhere = new_project(f, client);
    let foreign_changeset = new_changeset(f, client, &elsewhere);
    let foreign_thread = new_thread(f, client, &elsewhere, &foreign_changeset, false);
    let foreign = post(
        f,
        client,
        &critic(f),
        &elsewhere,
        &foreign_thread,
        "question",
        &[],
    );

    let first = home(f, client, &project);
    assert_eq!(
        ids_at(&first, "/waiting/questions"),
        vec![question.clone(), sealed.clone()],
        "the agents' questions wait, oldest first; other posts and the human's own question do not: {first}"
    );
    for not_waiting in [&position, &own, &foreign] {
        assert!(
            !ids_at(&first, "/waiting/questions").contains(not_waiting),
            "{first}"
        );
    }
    assert_eq!(
        item_at(&first, "/waiting/questions", &sealed),
        read(f, client, &human(f), &project, &format!("/posts/{sealed}"))["post"],
        "the home lists the question as the post's read answers the human"
    );

    // An agent's answer, and a human post that links to another post, are no
    // reply of the human to the question.
    post(
        f,
        client,
        &author(f),
        &project,
        &open_thread,
        "note",
        &[&question],
    );
    post(
        f,
        client,
        &human(f),
        &project,
        &open_thread,
        "note",
        &[&position],
    );
    let unanswered = home(f, client, &project);
    assert_eq!(
        ids_at(&unanswered, "/waiting/questions"),
        vec![question.clone(), sealed.clone()],
        "no human post links to either question yet: {unanswered}"
    );

    // The human replies with a post that links to the question.
    post(
        f,
        client,
        &human(f),
        &project,
        &open_thread,
        "note",
        &[&question],
    );
    let replied = home(f, client, &project);
    assert_eq!(
        ids_at(&replied, "/waiting/questions"),
        vec![sealed.clone()],
        "a human post that links to the question takes it off the list: {replied}"
    );

    // A directive that links to the question is a reply too.
    directive(f, client, &project, &sealed_thread, &[&sealed]);
    let answered = home(f, client, &project);
    assert_eq!(
        ids_at(&answered, "/waiting/questions"),
        Vec::<String>::new(),
        "a directive that links to the question takes it off the list: {answered}"
    );
}

// ---------------------------------------------------------------------------
// Features (US-063)
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

/// The human marks the feature done.
fn finish_feature(f: &Fixture, client: &Client, project: &str, feature: &str) {
    must(
        call(
            client,
            "POST",
            &format!("{}/features/{feature}/done", project_base(f, project)),
            &human(f),
            Some(&json!({})),
        ),
        200,
        "the human marks the feature done",
    );
}

/// The change set opens a draft of the existing doc `doc`.
fn open_draft(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) {
    must(
        call(
            client,
            "POST",
            &format!("{}/docs/{doc}/open", changeset_path(f, project, changeset)),
            &author(f),
            None,
        ),
        201,
        "open a draft of an existing doc",
    );
}

/// The author abandons the change set.
fn abandon_changeset(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let status = act_on_changeset(f, client, &author(f), project, changeset, "abandon", None);
    assert_eq!(status, "abandoned");
}

/// Puts the change set in status `approved`. The API passes through it only
/// inside the transaction of an approval, so the test sets it directly.
fn force_status(changeset: &str, status: &str) {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query("update changesets set status = $2 where id = $1::uuid")
            .bind(changeset)
            .bind(status)
            .execute(&mut conn)
            .await
            .expect("set the change set's status");
    });
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

/// (docs, open_changesets, waiting) of a FeatureSummary.
fn counts(summary: &Value) -> (Value, Value, Value) {
    (
        summary["docs"].clone(),
        summary["open_changesets"].clone(),
        summary["waiting"].clone(),
    )
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
    let finished = new_feature(f, client, &project);

    // A global doc that merged, and project-level change sets that count for
    // no feature.
    let (global_changeset, global, _, _) = changeset_with_version(f, client, &project);
    submit(f, client, &project, &global_changeset);
    approve_changeset(f, client, &project, &global_changeset);
    let (project_in_review, _, _, _) = changeset_with_version(f, client, &project);
    submit(f, client, &project, &project_in_review);
    new_changeset(f, client, &project);

    // The first feature: a doc that merged ...
    let (merged, base, _, _) = changeset_with_version_in(f, client, &project, Some(&first));
    submit(f, client, &project, &merged);
    approve_changeset(f, client, &project, &merged);
    // ... an open change set that drafts its doc and the global doc and
    // creates a second doc ...
    let open = new_changeset_in(f, client, &project, Some(&first));
    open_draft(f, client, &project, &open, &base);
    open_draft(f, client, &project, &open, &global);
    create_doc(f, client, &project, &open, "html", "architecture");
    let thread = new_thread(f, client, &project, &open, false);
    propose(f, client, &project, &thread);
    // ... one in review with a third doc ...
    let (in_review, _, _, _) = changeset_with_version_in(f, client, &project, Some(&first));
    submit(f, client, &project, &in_review);
    // ... two on the merged doc, of which the second needs a rebase once the
    // first merges ...
    let ahead = new_changeset_in(f, client, &project, Some(&first));
    let behind = new_changeset_in(f, client, &project, Some(&first));
    for changeset in [&ahead, &behind] {
        open_draft(f, client, &project, changeset, &base);
        insert_paragraph(f, client, &project, changeset, &base);
        save(f, client, &project, changeset, &base);
        submit(f, client, &project, changeset);
    }
    approve_changeset(f, client, &project, &ahead);
    let status = act_on_changeset(
        f,
        client,
        &human(f),
        &project,
        &behind,
        "approve",
        Some(&json!({})),
    );
    assert_eq!(status, "needs_rebase");
    // ... one approved, one rejected and one abandoned.
    let approved = new_changeset_in(f, client, &project, Some(&first));
    open_draft(f, client, &project, &approved, &base);
    force_status(&approved, "approved");
    let rejected = new_changeset_in(f, client, &project, Some(&first));
    open_draft(f, client, &project, &rejected, &base);
    insert_paragraph(f, client, &project, &rejected, &base);
    save(f, client, &project, &rejected, &base);
    submit(f, client, &project, &rejected);
    reject_changeset(f, client, &project, &rejected);
    let abandoned = new_changeset_in(f, client, &project, Some(&first));
    abandon_changeset(f, client, &project, &abandoned);

    // The second feature: one open change set with one doc.
    let second_open = new_changeset_in(f, client, &project, Some(&second));
    create_doc(f, client, &project, &second_open, "html", "prd");

    // The third has nothing and is done.
    finish_feature(f, client, &project, &finished);

    // Another project's feature is not this project's.
    let elsewhere = new_project(f, client);
    let foreign = new_feature(f, client, &elsewhere);

    let home = home(f, client, &project);
    assert_eq!(
        feature_ids(&home),
        vec![first.clone(), second.clone(), finished.clone()],
        "every feature of the project, the done one too, in creation order: {home}"
    );
    assert!(!home.to_string().contains(&foreign), "{home}");
    for feature in [&first, &second, &finished] {
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
    assert_eq!(summary_of(&home, &finished)["feature"]["status"], "done");
    assert_eq!(
        counts(&summary_of(&home, &first)),
        (json!(3), json!(4), json!(2)),
        "the first feature: 3 docs of its own (not the global one it drafts); open, in review, \
         needs rebase and approved change sets (not merged, rejected or abandoned); the proposed \
         decision and the change set in review wait: {home}"
    );
    assert_eq!(
        counts(&summary_of(&home, &second)),
        (json!(1), json!(1), json!(0)),
        "the second feature: {home}"
    );
    assert_eq!(
        counts(&summary_of(&home, &finished)),
        (json!(0), json!(0), json!(0)),
        "the done feature: {home}"
    );
}

#[test]
fn a_project_without_features_has_an_empty_feature_list_in_its_home() {
    let (f, client) = ready();
    let project = new_project(f, client);
    changeset_with_version(f, client, &project);
    let home = home(f, client, &project);
    assert_eq!(list_at(&home, "/features"), Vec::<Value>::new(), "{home}");
}

/// One of each waiting kind in a change set of `feature` (or at project
/// level): (decision, change set, comment, question).
fn one_of_each_waiting(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: Option<&str>,
) -> (String, String, String, String) {
    let (changeset, doc, node, v1) = changeset_with_version_in(f, client, project, feature);
    let thread = new_thread(f, client, project, &changeset, false);
    let question = post(f, client, &critic(f), project, &thread, "question", &[]);
    let decision = propose(f, client, project, &thread);
    let comment = comment(f, client, project, &v1, &node);
    insert_paragraph(f, client, project, &changeset, &doc);
    let v2 = save(f, client, project, &changeset, &doc);
    act_on_comment(
        f,
        client,
        &author(f),
        project,
        &comment,
        "addressed",
        &json!({ "version_id": v2 }),
    );
    submit(f, client, project, &changeset);
    (decision, changeset, comment, question)
}

#[test]
fn every_waiting_item_carries_its_feature_and_counts_for_it() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let idle = new_feature(f, client, &project);
    let (p_decision, p_changeset, p_comment, p_question) =
        one_of_each_waiting(f, client, &project, None);
    let (f_decision, f_changeset, f_comment, f_question) =
        one_of_each_waiting(f, client, &project, Some(&feature));

    let home = home(f, client, &project);
    for (pointer, project_level, in_feature) in [
        ("/waiting/decisions", &p_decision, &f_decision),
        ("/waiting/changesets", &p_changeset, &f_changeset),
        ("/waiting/comments", &p_comment, &f_comment),
        ("/waiting/questions", &p_question, &f_question),
    ] {
        assert_eq!(
            ids_at(&home, pointer),
            vec![project_level.clone(), in_feature.clone()],
            "{pointer} lists both, oldest first: {home}"
        );
        assert_feature_of(&home, pointer, project_level, &Value::Null);
        assert_feature_of(&home, pointer, in_feature, &json!(feature));
    }
    assert_eq!(
        summary_of(&home, &feature)["waiting"],
        json!(4),
        "the decision, change set, comment and question of the feature wait for it: {home}"
    );
    assert_eq!(
        summary_of(&home, &idle)["waiting"],
        json!(0),
        "nothing waits for a feature that has nothing: {home}"
    );
}

#[test]
fn a_comment_takes_its_feature_from_its_doc() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let feature = new_feature(f, client, &project);
    let (global_changeset, global, _, _) = changeset_with_version(f, client, &project);
    submit(f, client, &project, &global_changeset);
    approve_changeset(f, client, &project, &global_changeset);

    // A feature's change set saves a version of the global doc, which the
    // human comments on.
    let changeset = new_changeset_in(f, client, &project, Some(&feature));
    open_draft(f, client, &project, &changeset, &global);
    let node = insert_paragraph(f, client, &project, &changeset, &global);
    let v1 = save(f, client, &project, &changeset, &global);
    let on_global = comment(f, client, &project, &v1, &node);
    insert_paragraph(f, client, &project, &changeset, &global);
    let v2 = save(f, client, &project, &changeset, &global);
    act_on_comment(
        f,
        client,
        &author(f),
        &project,
        &on_global,
        "addressed",
        &json!({ "version_id": v2 }),
    );

    let home = home(f, client, &project);
    assert_eq!(
        ids_at(&home, "/waiting/comments"),
        vec![on_global.clone()],
        "{home}"
    );
    assert_eq!(
        item_at(&home, "/waiting/comments", &on_global)["changeset_id"],
        changeset,
        "the comment is on a version of the feature's change set: {home}"
    );
    assert_feature_of(&home, "/waiting/comments", &on_global, &Value::Null);
    assert_eq!(
        summary_of(&home, &feature)["waiting"],
        json!(0),
        "a comment on a global doc waits at project level, not for the feature: {home}"
    );
}

#[test]
fn the_home_lists_the_docs_change_sets_and_threads_of_the_project_and_every_feature() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let first = new_feature(f, client, &project);
    let second = new_feature(f, client, &project);
    let (global_changeset, global, _, _) = changeset_with_version(f, client, &project);
    let global_thread = new_thread(f, client, &project, &global_changeset, false);
    let (first_changeset, first_doc, _, _) =
        changeset_with_version_in(f, client, &project, Some(&first));
    let first_thread = new_thread(f, client, &project, &first_changeset, false);
    let (second_changeset, second_doc, _, _) =
        changeset_with_version_in(f, client, &project, Some(&second));
    let second_thread = new_thread(f, client, &project, &second_changeset, true);
    // A merged change set of a feature stays listed, with its doc and thread.
    submit(f, client, &project, &first_changeset);
    approve_changeset(f, client, &project, &first_changeset);
    finish_feature(f, client, &project, &second);

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
            "/changesets",
            [
                (&global_changeset, &none),
                (&first_changeset, &first_id),
                (&second_changeset, &second_id),
            ],
            "changesets",
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
            "{pointer}: project level and every feature, done ones too, oldest first: {home}"
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
