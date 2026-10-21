//! US-038: comments on nodes, through the API. Run with `--features
//! stack-tests` in the Compose test container, which reaches Postgres at
//! `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. The
//! CLI side is in crates/omni-cli/tests/comments.rs and stack_comments.rs.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; docs, drafts and
//! operations are those of US-022, tests/docs.rs, versions and their node
//! index those of US-024, tests/versions.rs, approving and merging those of
//! US-025, tests/approve_merge.rs, and design docs with components and
//! instances those of US-029, tests/components.rs):
//!
//! - `POST /versions/{version_id}/comments` (both) pins a comment to one node
//!   of a saved version (comments never go on drafts): it takes `{"node",
//!   "text"}` and answers 201 `{"comment": Comment}`. `node` is a node
//!   address: the ID of a node that the version has, or, for a node inside a
//!   component instance of a design version, `<instanceId>/<componentNodeId>`
//!   (US-029), where the instance is a node of the version and the component
//!   node is a node of the instance's component in the design system version
//!   that the design version records. `text` is a string that is not blank.
//!   A node address that the version does not have (a node of a later
//!   version, of the component alone, or of no doc at all), and a missing or
//!   blank text, are 400 `{"error": "invalid_request"}`; an unknown version,
//!   or one of another project, is 404 `not_found`. A refused call records
//!   nothing and emits nothing.
//! - A Comment is `{"id" (UUIDv7), "project_id", "doc_id", "version_id",
//!   "node", "changeset_id", "author", "text", "status", "addressed_in",
//!   "replies", "node_status", "compared_to", "created_at"}`:
//!   - `version_id` is the version it was made on, and `node` the address as
//!     sent; both never change.
//!   - `changeset_id` is the change set it was made in during a review (Q19):
//!     the change set of the version, unless that change set has merged, in
//!     which case the comment is on the mainline and `changeset_id` is
//!     `null`.
//!   - `author` is `{"kind": "human", "user_id"}` for the human and `{"kind":
//!     "agent", "run", "role", "model", "harness"}` for an agent, as for
//!     posts (US-034).
//!   - `status` is `open`, `addressed` or `resolved`, and `addressed_in` the
//!     version that an agent named when it marked the comment addressed
//!     (`null` before that).
//!   - `replies` lists the replies, oldest first, each `{"id", "author",
//!     "text", "created_at"}`.
//!   - `compared_to` is the version the comment's node is compared with: for
//!     a comment with a change set, the newest version of the doc saved in
//!     that change set; for a comment on the mainline, the doc's mainline
//!     version now. `node_status` is `current` when that version has the
//!     node with the same fingerprint as the version it was made on,
//!     `changed` when it has the node with another fingerprint (the comment
//!     is marked as made on an older version) and `removed` when it no
//!     longer has the node (the comment stays, marked as such).
//! - `GET /comments` (both) lists the project's comments, oldest first: 200
//!   `{"project_id", "comments": [Comment, ...]}`. The query parameters
//!   narrow it and combine: `doc=<id>` to one doc's comments (on any of its
//!   versions), `changeset=<id>` to the comments made in that change set and
//!   `status=<open|addressed|resolved>` to one status. Another status is 400
//!   `invalid_request`.
//! - `GET /comments/{comment_id}` (both) answers 200 `{"project_id",
//!   "comment": Comment}`; an unknown comment is 404 `not_found`.
//! - `POST /comments/{comment_id}/replies` (both) takes `{"text"}` (not
//!   blank, else 400 `invalid_request`) and adds a reply: 201 `{"comment":
//!   Comment}`, with the new reply last in `replies`. A reply leaves the
//!   status as it is. An unknown comment is 404.
//! - Statuses (FR-40): `POST /comments/{comment_id}/addressed` (agent) takes
//!   `{"version_id"}`, the version that fixes the comment: a version of the
//!   comment's doc saved after the version the comment was made on. The
//!   comment becomes `addressed` with that `addressed_in`. A version of
//!   another doc, the comment's own version, an unknown version or none is
//!   400 `invalid_request`. `POST /comments/{comment_id}/resolve` (human)
//!   takes `{}` and makes an addressed comment `resolved`; `POST
//!   /comments/{comment_id}/reopen` (human) takes `{}` and makes a resolved
//!   or addressed comment `open` again. Each answers 200 `{"comment":
//!   Comment}`. Marking a resolved comment addressed, resolving a resolved
//!   one and reopening an open one are 409 `{"error":
//!   "invalid_transition"}`. An API key on resolve or reopen, and a session
//!   on addressed, is 403 `forbidden`; an unknown comment is 404. A refused
//!   call changes nothing and emits nothing.
//! - Saving a version (`POST /changesets/{changeset_id}/docs/{doc_id}/versions`,
//!   US-024) whose doc loses a node that an `open` comment of the doc is
//!   pinned to (a node that the doc's previous version in the change set
//!   had, or the draft's base version when the change set has saved none of
//!   the doc yet, and that the new version does not have; for an address
//!   inside an instance, the instance) is 409 `{"error":
//!   "commented_nodes_removed", "comments": [Comment, ...]}`, listing those
//!   comments, and saves nothing, unless the body is `{"remove_commented":
//!   true}`. With it the version is saved as before, and the comments stay,
//!   marked `removed`. A resolved comment never stops a save.
//! - Creating a comment emits `comment.created` with `data` at least
//!   `{"comment_id", "doc_id", "version_id", "node", "changeset_id",
//!   "status"}`, a reply `comment.replied` with at least `{"comment_id",
//!   "reply_id"}`, and every status change `comment.status_changed` with at
//!   least `{"comment_id", "doc_id", "from_status", "status"}`. A refused call
//!   emits nothing.
//! - Storage: comments live in the tenant table `comments` (the API role
//!   keeps its UPDATE privilege there, since the status changes), whose
//!   columns without a default are `workspace_id`, `version_id`, `node`,
//!   `author` (jsonb) and `body` (the text); the change set, the status and
//!   the version that addressed it are columns with a default or nullable.
//!   Replies live in the append-only tenant table `comment_replies`, whose
//!   columns without a default are `workspace_id`, `comment_id`, `author`
//!   (jsonb) and `body` (crates/omni-core/tests/tenancy.rs lists it in
//!   `APPEND_ONLY_TABLES`). The story adds no other tenant table;
//!   tests/workspace_delete.rs and the isolation suite seed both.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_api::routes::{Access, registry};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us038_comments";
const TEST_BUCKET: &str = "omni-us038-comments";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
/// A node ID in the right format that no node has.
const UNKNOWN_NODE: &str = "zzzzzzzz";
const ROUTE_BASE: &str = "/api/v1/w/{workspace_id}/projects/{project_id}";

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
    /// Alice's user ID, the human's in `author`.
    alice: String,
    alice_token: String,
    key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us038-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
        let (workspace, alice) = block(async {
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
            (workspace, user)
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
            alice,
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

/// The agent that writes the docs and answers comments: `run-1/author`.
fn author(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-1",
        role: "author",
    }
}

/// Another agent, which reviews: `run-1/critic`.
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

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
}

fn changeset_path(f: &Fixture, project: &str, changeset: &str) -> String {
    format!("{}/changesets/{changeset}", project_base(f, project))
}

fn comment_path(f: &Fixture, project: &str, comment: &str) -> String {
    format!("{}/comments/{comment}", project_base(f, project))
}

// ---------------------------------------------------------------------------
// Docs and versions
// ---------------------------------------------------------------------------

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &author(f),
                Some(&json!({ "name": unique("comments-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

fn new_changeset(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/changesets", project_base(f, project)),
                &author(f),
                Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
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

fn open_draft(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) {
    let response = call(
        client,
        "POST",
        &format!("{}/docs/{doc}/open", changeset_path(f, project, changeset)),
        &author(f),
        None,
    );
    assert!(
        response.status == 200 || response.status == 201,
        "open a draft of the doc: {}",
        response.text()
    );
}

/// Applies a batch of operations to the change set's draft of `doc`, which
/// must succeed; returns its results.
fn edit(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    ops: Value,
) -> Vec<Value> {
    must(
        call(
            client,
            "POST",
            &format!("{}/docs/{doc}/ops", changeset_path(f, project, changeset)),
            &author(f),
            Some(&json!({ "ops": ops })),
        ),
        200,
        "edit the draft",
    )["results"]
        .as_array()
        .expect("the results of the batch")
        .clone()
}

fn insert(text: &str) -> Value {
    json!({
        "op": "insert",
        "parent": null,
        "position": { "after": null },
        "content": format!("<p>{text}</p>")
    })
}

fn set_text(node: &str, rev: u64, text: &str) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": { "html": text } })
}

fn delete(node: &str, rev: u64) -> Value {
    json!({ "op": "delete", "node": node, "rev": rev })
}

fn save_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!(
            "{}/docs/{doc}/versions",
            changeset_path(f, project, changeset)
        ),
        &author(f),
        Some(body),
    )
}

/// Saves the change set's draft of `doc` with `body`; returns the version's ID.
fn save_with(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    body: &Value,
) -> String {
    id_of(
        &must(
            save_response(f, client, project, changeset, doc, body),
            201,
            "save a version",
        )["version"],
    )
}

fn save(f: &Fixture, client: &Client, project: &str, changeset: &str, doc: &str) -> String {
    save_with(f, client, project, changeset, doc, &json!({}))
}

/// The revision that the change set's draft of `doc` shows for `node` (its
/// `data-rev` in the draft's HTML).
fn rev_in_draft(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
    doc: &str,
    node: &str,
) -> u64 {
    let draft = must(
        call(
            client,
            "GET",
            &format!("{}/docs/{doc}", changeset_path(f, project, changeset)),
            &author(f),
            None,
        ),
        200,
        "read the draft",
    );
    let html = draft["draft"]["html"]
        .as_str()
        .unwrap_or_else(|| panic!("the draft has its html: {draft}"));
    let needle = format!("data-node-id=\"{node}\"");
    let at = html
        .find(&needle)
        .unwrap_or_else(|| panic!("the draft has {node}: {html}"));
    let tag_start = html[..at].rfind('<').expect("a start tag");
    let tag_end = at + html[at..].find('>').expect("the tag's end");
    let tag = &html[tag_start..tag_end];
    let start = tag.find("data-rev=\"").expect("a data-rev") + "data-rev=\"".len();
    let end = start + tag[start..].find('"').expect("the end of data-rev");
    tag[start..end].parse().expect("a revision number")
}

/// Submits the change set and has the human approve it, which merges it.
fn merge(f: &Fixture, client: &Client, project: &str, changeset: &str) {
    let path = changeset_path(f, project, changeset);
    must(
        call(client, "POST", &format!("{path}/submit"), &author(f), None),
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

fn mainline_of(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    must(
        call(
            client,
            "GET",
            &format!("{}/docs/{doc}", project_base(f, project)),
            &author(f),
            None,
        ),
        200,
        "read the doc",
    )["doc"]["mainline_version_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the doc {doc} has a mainline version"))
        .to_string()
}

/// An HTML doc of a new project, started in an open change set with three
/// paragraphs (`a`, `b` and `c`, each at revision 1) and saved as `v1`.
struct Paper {
    project: String,
    changeset: String,
    doc: String,
    v1: String,
    a: String,
    b: String,
    c: String,
}

fn paper_in(f: &Fixture, client: &Client, project: &str) -> Paper {
    let changeset = new_changeset(f, client, project);
    let doc = create_doc(f, client, project, &changeset, "html", "prd");
    let results = edit(
        f,
        client,
        project,
        &changeset,
        &doc,
        json!([
            insert("The hero is split."),
            insert("The copy sits on the left."),
            insert("The product shot sits on the right.")
        ]),
    );
    let node = |i: usize| {
        results[i]["node"]
            .as_str()
            .unwrap_or_else(|| panic!("an insert names its node: {results:?}"))
            .to_string()
    };
    let (a, b, c) = (node(0), node(1), node(2));
    let v1 = save(f, client, project, &changeset, &doc);
    Paper {
        project: project.to_string(),
        changeset,
        doc,
        v1,
        a,
        b,
        c,
    }
}

fn paper(f: &Fixture, client: &Client) -> Paper {
    let project = new_project(f, client);
    paper_in(f, client, &project)
}

impl Paper {
    fn edit(&self, f: &Fixture, client: &Client, ops: Value) -> Vec<Value> {
        edit(f, client, &self.project, &self.changeset, &self.doc, ops)
    }

    fn save(&self, f: &Fixture, client: &Client) -> String {
        save(f, client, &self.project, &self.changeset, &self.doc)
    }

    fn save_response(&self, f: &Fixture, client: &Client, body: &Value) -> Response {
        save_response(f, client, &self.project, &self.changeset, &self.doc, body)
    }
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

fn comment_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    version: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/versions/{version}/comments", project_base(f, project)),
        auth,
        Some(body),
    )
}

/// `auth` comments `text` on `node` of `version`; returns the Comment.
fn comment(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    version: &str,
    node: &str,
    text: &str,
) -> Value {
    must(
        comment_response(
            f,
            client,
            auth,
            project,
            version,
            &json!({ "node": node, "text": text }),
        ),
        201,
        "comment on a node",
    )["comment"]
        .clone()
}

fn comment_of(f: &Fixture, client: &Client, auth: &Auth, project: &str, comment: &str) -> Value {
    let answer = must(
        call(
            client,
            "GET",
            &comment_path(f, project, comment),
            auth,
            None,
        ),
        200,
        "read the comment",
    );
    assert_eq!(answer["project_id"], project, "{answer}");
    answer["comment"].clone()
}

/// The comments that `GET /comments?<query>` lists, as `auth` reads them.
fn list(f: &Fixture, client: &Client, auth: &Auth, project: &str, query: &str) -> Vec<Value> {
    let path = if query.is_empty() {
        format!("{}/comments", project_base(f, project))
    } else {
        format!("{}/comments?{query}", project_base(f, project))
    };
    let answer = must(
        call(client, "GET", &path, auth, None),
        200,
        "list the comments",
    );
    assert_eq!(answer["project_id"], project, "{answer}");
    answer["comments"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer lists `comments`: {answer}"))
        .clone()
}

fn ids(comments: &[Value]) -> Vec<String> {
    comments.iter().map(id_of).collect()
}

fn act_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    comment: &str,
    action: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/{action}", comment_path(f, project, comment)),
        auth,
        Some(body),
    )
}

fn reply(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    comment: &str,
    text: &str,
) -> Value {
    must(
        act_response(
            f,
            client,
            auth,
            project,
            comment,
            "replies",
            &json!({ "text": text }),
        ),
        201,
        "reply to the comment",
    )["comment"]
        .clone()
}

/// The author marks the comment addressed in `version`; returns the Comment.
fn addressed(f: &Fixture, client: &Client, project: &str, comment: &str, version: &str) -> Value {
    must(
        act_response(
            f,
            client,
            &author(f),
            project,
            comment,
            "addressed",
            &json!({ "version_id": version }),
        ),
        200,
        "mark the comment addressed",
    )["comment"]
        .clone()
}

/// The human resolves or reopens the comment; returns the Comment.
fn human_acts(f: &Fixture, client: &Client, project: &str, comment: &str, action: &str) -> Value {
    must(
        act_response(f, client, &human(f), project, comment, action, &json!({})),
        200,
        &format!("the human acts on the comment: {action}"),
    )["comment"]
        .clone()
}

fn status_of(f: &Fixture, client: &Client, project: &str, comment: &str) -> String {
    comment_of(f, client, &author(f), project, comment)["status"]
        .as_str()
        .unwrap_or_else(|| panic!("the comment {comment} has a status"))
        .to_string()
}

/// The live events of a project named `event`, oldest first, as their
/// `data`.
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

/// Every live event of a project whose name starts with `comment.`.
fn comment_events(project: &str) -> usize {
    block(async {
        let mut conn = admin_connection().await;
        let rows: i64 = sqlx::query_scalar(
            "select count(*) from live_events \
             where project_id = $1::uuid and event like 'comment.%'",
        )
        .bind(project)
        .fetch_one(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("count the live events: {e}"));
        rows as usize
    })
}

/// How many comments on `versions` the table `comments` holds, as the admin
/// sees them (0 while the table does not exist).
fn stored_comments(versions: &[&str]) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        let exists: bool = sqlx::query_scalar("select to_regclass('public.comments') is not null")
            .fetch_one(&mut conn)
            .await
            .expect("look for the comments table");
        if !exists {
            return 0;
        }
        let versions: Vec<String> = versions.iter().map(|v| v.to_string()).collect();
        sqlx::query_scalar("select count(*) from comments where version_id::text = any($1)")
            .bind(versions)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the comments: {e}"))
    })
}

/// How many versions of `doc` the change set has saved.
fn versions_of(doc: &str, changeset: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar(
            "select count(*) from versions where doc_id = $1::uuid and changeset_id = $2::uuid",
        )
        .bind(doc)
        .bind(changeset)
        .fetch_one(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("count the versions: {e}"))
    })
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_comment_routes_are_in_the_registry_with_their_access() {
    let expected = [
        ("POST", "/versions/{version_id}/comments", Access::Both),
        ("GET", "/comments", Access::Both),
        ("GET", "/comments/{comment_id}", Access::Both),
        ("POST", "/comments/{comment_id}/replies", Access::Both),
        ("POST", "/comments/{comment_id}/addressed", Access::Agent),
        ("POST", "/comments/{comment_id}/resolve", Access::Human),
        ("POST", "/comments/{comment_id}/reopen", Access::Human),
    ];
    let routes = registry();
    for (method, tail, access) in expected {
        let path = format!("{ROUTE_BASE}{tail}");
        let entry = routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no route {method} {path}"));
        assert_eq!(entry.access, access, "{method} {path}");
    }
}

// ---------------------------------------------------------------------------
// What a comment records
// ---------------------------------------------------------------------------

#[test]
fn a_comment_records_the_doc_the_node_the_version_the_change_set_the_author_and_the_text() {
    let (f, client) = ready();
    let p = paper(f, client);
    let text = unique("The copy is too long for 320 px");

    let made = comment(f, client, &human(f), &p.project, &p.v1, &p.b, &text);
    let id = id_of(&made);
    assert_eq!(
        id.chars().nth(14),
        Some('7'),
        "the comment's ID is a UUIDv7: {made}"
    );
    assert_eq!(made["project_id"], p.project.as_str(), "{made}");
    assert_eq!(made["doc_id"], p.doc.as_str(), "{made}");
    assert_eq!(made["version_id"], p.v1.as_str(), "{made}");
    assert_eq!(made["node"], p.b.as_str(), "{made}");
    assert_eq!(
        made["changeset_id"],
        p.changeset.as_str(),
        "a comment made during the review records the change set: {made}"
    );
    assert_eq!(
        made["author"],
        json!({ "kind": "human", "user_id": f.alice }),
        "{made}"
    );
    assert_eq!(made["text"], text.as_str(), "{made}");
    assert_eq!(made["status"], "open", "{made}");
    assert_eq!(made["addressed_in"], Value::Null, "{made}");
    assert_eq!(made["replies"], json!([]), "{made}");
    assert_eq!(made["node_status"], "current", "{made}");
    assert_eq!(made["compared_to"], p.v1.as_str(), "{made}");
    assert!(made["created_at"].is_string(), "{made}");

    for auth in [author(f), human(f)] {
        assert_eq!(
            comment_of(f, client, &auth, &p.project, &id),
            made,
            "the comment reads as it was made"
        );
    }
}

#[test]
fn an_agent_may_comment_too_and_is_recorded_as_the_agent() {
    let (f, client) = ready();
    let p = paper(f, client);
    let made = comment(
        f,
        client,
        &critic(f),
        &p.project,
        &p.v1,
        &p.a,
        "The heading should name the product",
    );
    let author = &made["author"];
    assert_eq!(author["kind"], "agent", "{made}");
    assert_eq!(author["run"], "run-1", "{made}");
    assert_eq!(author["role"], "critic", "{made}");
}

#[test]
fn a_comment_on_a_mainline_version_names_no_change_set() {
    let (f, client) = ready();
    let p = paper(f, client);
    merge(f, client, &p.project, &p.changeset);
    let mainline = mainline_of(f, client, &p.project, &p.doc);
    assert_eq!(mainline, p.v1, "the merged version is the mainline");

    let made = comment(
        f,
        client,
        &human(f),
        &p.project,
        &mainline,
        &p.a,
        "Say who the hero is for",
    );
    assert_eq!(made["version_id"], mainline.as_str(), "{made}");
    assert_eq!(
        made["changeset_id"],
        Value::Null,
        "a comment outside a review has no change set: {made}"
    );
    assert_eq!(made["compared_to"], mainline.as_str(), "{made}");
    assert!(
        list(
            f,
            client,
            &author(f),
            &p.project,
            &format!("changeset={}", p.changeset)
        )
        .is_empty(),
        "it is listed under no change set"
    );
}

#[test]
fn a_comment_needs_a_node_of_its_version_and_a_text() {
    let (f, client) = ready();
    let p = paper(f, client);
    // A paragraph that only the next version has.
    let later = p.edit(f, client, json!([insert("Added after the first version.")]))[0]["node"]
        .as_str()
        .expect("the new node")
        .to_string();
    p.save(f, client);
    let other = paper(f, client);
    let events = comment_events(&p.project);
    let stored = stored_comments(&[&p.v1, &other.v1]);

    let refused: Vec<(&str, Value)> = vec![
        (
            "a node that no doc has",
            json!({ "node": UNKNOWN_NODE, "text": "Fix this" }),
        ),
        (
            "a node of a later version",
            json!({ "node": later, "text": "Fix this" }),
        ),
        ("no node", json!({ "text": "Fix this" })),
        (
            "a node that is no string",
            json!({ "node": 7, "text": "Fix this" }),
        ),
        ("no text", json!({ "node": p.a })),
        ("a blank text", json!({ "node": p.a, "text": "  \n " })),
        (
            "a text that is no string",
            json!({ "node": p.a, "text": ["Fix"] }),
        ),
    ];
    for (what, body) in refused {
        let response = comment_response(f, client, &human(f), &p.project, &p.v1, &body);
        assert_error(&response, 400, "invalid_request", what);
    }
    let response = comment_response(
        f,
        client,
        &human(f),
        &p.project,
        UNKNOWN_ID,
        &json!({ "node": p.a, "text": "Fix this" }),
    );
    assert_error(&response, 404, "not_found", "an unknown version");
    let response = comment_response(
        f,
        client,
        &human(f),
        &p.project,
        &other.v1,
        &json!({ "node": other.a, "text": "Fix this" }),
    );
    assert_error(&response, 404, "not_found", "a version of another project");

    assert_eq!(
        stored_comments(&[&p.v1, &other.v1]),
        stored,
        "a refused comment stores nothing"
    );
    assert_eq!(
        comment_events(&p.project),
        events,
        "a refused comment emits nothing"
    );
    // The same body with a node of the version is taken, so the refusals
    // above were the rules.
    comment(f, client, &human(f), &p.project, &p.v1, &p.a, "Fix this");
}

// ---------------------------------------------------------------------------
// Nodes inside component instances
// ---------------------------------------------------------------------------

type Ids = HashMap<String, String>;

fn collect_ids(nodes: &Value, ids: &mut Ids) {
    for node in nodes.as_array().unwrap_or(&Vec::new()) {
        if let (Some(name), Some(id)) = (node["name"].as_str(), node["id"].as_str()) {
            ids.insert(name.to_string(), id.to_string());
        }
        collect_ids(&node["children"], ids);
    }
}

fn ids_in(nodes: &Value) -> Ids {
    let mut ids = Ids::new();
    collect_ids(nodes, &mut ids);
    ids
}

/// A design doc whose board holds an instance of a button component, saved
/// as a version: (project, version, instance ID, the ID of the button's
/// label in the component, the board's ID).
fn design_with_instance(f: &Fixture, client: &Client) -> (String, String, String, String, String) {
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let base = changeset_path(f, &project, &changeset);
    create_doc(f, client, &project, &changeset, "design-system", "ui");
    must(
        call(
            client,
            "PUT",
            &format!("{base}/tokens"),
            &author(f),
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
    let components = must(
        call(
            client,
            "PUT",
            &format!("{base}/components"),
            &author(f),
            Some(
                &json!({ "tags": r##"<Component name="Button" dir="row" pad={12} fill="#2563eb" radius={8}>
  <Text name="Label" content="Buy" fontFamily="Inter" fontSize={14} color="#ffffff"/>
</Component>
"## }),
            ),
        ),
        200,
        "import the components",
    );
    let parts = ids_in(&components["components"]);
    let button = parts.get("Button").expect("the button's ID").clone();
    let label = parts.get("Label").expect("the label's ID").clone();
    let design = create_doc(f, client, &project, &changeset, "design", "ui");
    let imported = must(
        call(
            client,
            "PUT",
            &format!("{base}/docs/{design}/design"),
            &author(f),
            Some(&json!({ "tags": format!(
                r#"<Page name="Home">
  <Board name="Checkout" x={{0}} y={{0}} width={{375}} height={{812}} dir="col">
    <Instance name="Pay button" component="{button}"/>
  </Board>
</Page>
"#
            ) })),
        ),
        200,
        "import the design",
    );
    let nodes = ids_in(&imported["pages"]);
    let instance = nodes.get("Pay button").expect("the instance's ID").clone();
    let board = nodes.get("Checkout").expect("the board's ID").clone();
    let version = save(f, client, &project, &changeset, &design);
    (project, version, instance, label, board)
}

#[test]
fn a_comment_may_pin_a_node_inside_a_component_instance() {
    let (f, client) = ready();
    let (project, version, instance, label, board) = design_with_instance(f, client);

    let address = format!("{instance}/{label}");
    let made = comment(
        f,
        client,
        &human(f),
        &project,
        &version,
        &address,
        "The label of this one button should say Pay",
    );
    assert_eq!(
        made["node"],
        address.as_str(),
        "the comment lands on the exact node inside the instance: {made}"
    );
    assert_eq!(
        comment_of(f, client, &author(f), &project, &id_of(&made))["node"],
        address.as_str()
    );
    // The board and the instance itself are nodes of the version too.
    for node in [&board, &instance] {
        comment(f, client, &human(f), &project, &version, node, "Look here");
    }

    for (what, node) in [
        (
            "a component node that the instance's component does not have",
            format!("{instance}/{UNKNOWN_NODE}"),
        ),
        (
            "a component node outside an instance (it is no node of the design)",
            label.clone(),
        ),
        (
            "a component node under a node that is no instance",
            format!("{board}/{label}"),
        ),
    ] {
        let response = comment_response(
            f,
            client,
            &human(f),
            &project,
            &version,
            &json!({ "node": node, "text": "Look here" }),
        );
        assert_error(&response, 400, "invalid_request", what);
    }
}

// ---------------------------------------------------------------------------
// Replies
// ---------------------------------------------------------------------------

#[test]
fn a_comment_has_replies_in_order_and_a_reply_keeps_its_status() {
    let (f, client) = ready();
    let p = paper(f, client);
    let made = comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.b,
        "Is the copy on the left on phones too?",
    );
    let id = id_of(&made);

    let first = reply(
        f,
        client,
        &author(f),
        &p.project,
        &id,
        "On phones it stacks above the shot.",
    );
    let second = reply(
        f,
        client,
        &human(f),
        &p.project,
        &id,
        "Then say so in the PRD.",
    );
    assert_eq!(
        second["status"], "open",
        "a reply keeps the status: {second}"
    );
    let replies = second["replies"]
        .as_array()
        .unwrap_or_else(|| panic!("the comment lists replies: {second}"));
    assert_eq!(replies.len(), 2, "{second}");
    assert_eq!(
        first["replies"].as_array().map(Vec::len),
        Some(1),
        "{first}"
    );
    assert_eq!(replies[0]["text"], "On phones it stacks above the shot.");
    assert_eq!(replies[0]["author"]["kind"], "agent", "{second}");
    assert_eq!(replies[0]["author"]["role"], "author", "{second}");
    assert_eq!(replies[1]["text"], "Then say so in the PRD.");
    assert_eq!(
        replies[1]["author"],
        json!({ "kind": "human", "user_id": f.alice }),
        "{second}"
    );
    for entry in replies {
        let reply_id = id_of(entry);
        assert_eq!(reply_id.chars().nth(14), Some('7'), "{entry}");
        assert!(entry["created_at"].is_string(), "{entry}");
    }
    assert_eq!(
        comment_of(f, client, &critic(f), &p.project, &id)["replies"],
        second["replies"],
        "the replies read as they were made"
    );

    let events = comment_events(&p.project);
    for (what, body) in [
        ("no text", json!({})),
        ("a blank text", json!({ "text": " " })),
    ] {
        let response = act_response(f, client, &author(f), &p.project, &id, "replies", &body);
        assert_error(&response, 400, "invalid_request", what);
    }
    let response = act_response(
        f,
        client,
        &author(f),
        &p.project,
        UNKNOWN_ID,
        "replies",
        &json!({ "text": "Hello" }),
    );
    assert_error(&response, 404, "not_found", "a reply to an unknown comment");
    assert_eq!(
        comment_of(f, client, &author(f), &p.project, &id)["replies"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "a refused reply adds nothing"
    );
    assert_eq!(comment_events(&p.project), events, "and emits nothing");
}

// ---------------------------------------------------------------------------
// Statuses
// ---------------------------------------------------------------------------

#[test]
fn a_comment_goes_open_then_addressed_naming_the_fixing_version_then_resolved() {
    let (f, client) = ready();
    let p = paper(f, client);
    let id = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.b,
        "Shorter, please",
    ));
    p.edit(f, client, json!([set_text(&p.b, 1, "Copy left.")]));
    let v2 = p.save(f, client);

    let marked = addressed(f, client, &p.project, &id, &v2);
    assert_eq!(marked["status"], "addressed", "{marked}");
    assert_eq!(marked["addressed_in"], v2.as_str(), "{marked}");
    assert_eq!(
        marked["version_id"],
        p.v1.as_str(),
        "the comment stays on its version"
    );

    let resolved = human_acts(f, client, &p.project, &id, "resolve");
    assert_eq!(resolved["status"], "resolved", "{resolved}");
    assert_eq!(status_of(f, client, &p.project, &id), "resolved");

    let reopened = human_acts(f, client, &p.project, &id, "reopen");
    assert_eq!(reopened["status"], "open", "{reopened}");
    assert_eq!(status_of(f, client, &p.project, &id), "open");
}

#[test]
fn the_human_may_reopen_an_addressed_comment() {
    let (f, client) = ready();
    let p = paper(f, client);
    let id = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.c,
        "Use a real product shot",
    ));
    p.edit(f, client, json!([set_text(&p.c, 1, "A real shot.")]));
    let v2 = p.save(f, client);
    addressed(f, client, &p.project, &id, &v2);

    let reopened = human_acts(f, client, &p.project, &id, "reopen");
    assert_eq!(reopened["status"], "open", "{reopened}");
}

#[test]
fn only_the_human_resolves_and_reopens_and_only_an_agent_marks_addressed() {
    let (f, client) = ready();
    let p = paper(f, client);
    let id = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.a,
        "Name the product",
    ));
    p.edit(f, client, json!([set_text(&p.a, 1, "The Roadster hero.")]));
    let v2 = p.save(f, client);
    let events = comment_events(&p.project);

    let response = act_response(
        f,
        client,
        &human(f),
        &p.project,
        &id,
        "addressed",
        &json!({ "version_id": v2 }),
    );
    assert_error(&response, 403, "forbidden", "the human marks addressed");
    assert_eq!(status_of(f, client, &p.project, &id), "open");

    addressed(f, client, &p.project, &id, &v2);
    let events = events + 1;
    assert_eq!(comment_events(&p.project), events);
    for action in ["resolve", "reopen"] {
        let response = act_response(f, client, &author(f), &p.project, &id, action, &json!({}));
        assert_error(&response, 403, "forbidden", &format!("an agent: {action}"));
        assert_eq!(
            status_of(f, client, &p.project, &id),
            "addressed",
            "a refused {action} changes nothing"
        );
    }
    assert_eq!(
        comment_events(&p.project),
        events,
        "the refused calls emit nothing"
    );
}

#[test]
fn addressed_names_a_later_version_of_the_comments_doc() {
    let (f, client) = ready();
    let p = paper(f, client);
    let id = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.a,
        "Name the product",
    ));
    // Another doc of the same project, with a version of its own.
    let other = paper_in(f, client, &p.project);
    let events = comment_events(&p.project);

    for (what, body) in [
        (
            "a version of another doc",
            json!({ "version_id": other.v1 }),
        ),
        (
            "the version the comment was made on",
            json!({ "version_id": p.v1 }),
        ),
        ("an unknown version", json!({ "version_id": UNKNOWN_ID })),
        ("no version", json!({})),
        ("a version that is no UUID", json!({ "version_id": "v2" })),
    ] {
        let response = act_response(f, client, &author(f), &p.project, &id, "addressed", &body);
        assert_error(&response, 400, "invalid_request", what);
    }
    let response = act_response(
        f,
        client,
        &author(f),
        &p.project,
        UNKNOWN_ID,
        "addressed",
        &json!({ "version_id": p.v1 }),
    );
    assert_error(&response, 404, "not_found", "an unknown comment");
    let read = comment_of(f, client, &author(f), &p.project, &id);
    assert_eq!(read["status"], "open", "{read}");
    assert_eq!(read["addressed_in"], Value::Null, "{read}");
    assert_eq!(comment_events(&p.project), events, "nothing was emitted");
}

#[test]
fn a_status_change_that_does_not_fit_the_status_is_409_and_changes_nothing() {
    let (f, client) = ready();
    let p = paper(f, client);
    let id = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.a,
        "Name the product",
    ));
    let response = act_response(f, client, &human(f), &p.project, &id, "reopen", &json!({}));
    assert_error(
        &response,
        409,
        "invalid_transition",
        "reopen an open comment",
    );

    p.edit(f, client, json!([set_text(&p.a, 1, "The Roadster hero.")]));
    let v2 = p.save(f, client);
    addressed(f, client, &p.project, &id, &v2);
    human_acts(f, client, &p.project, &id, "resolve");
    let events = comment_events(&p.project);

    let response = act_response(f, client, &human(f), &p.project, &id, "resolve", &json!({}));
    assert_error(
        &response,
        409,
        "invalid_transition",
        "resolve a resolved one",
    );
    let response = act_response(
        f,
        client,
        &author(f),
        &p.project,
        &id,
        "addressed",
        &json!({ "version_id": v2 }),
    );
    assert_error(
        &response,
        409,
        "invalid_transition",
        "mark a resolved one addressed",
    );
    for action in ["resolve", "reopen"] {
        let response = act_response(
            f,
            client,
            &human(f),
            &p.project,
            UNKNOWN_ID,
            action,
            &json!({}),
        );
        assert_error(
            &response,
            404,
            "not_found",
            &format!("{action} an unknown one"),
        );
    }
    assert_eq!(status_of(f, client, &p.project, &id), "resolved");
    assert_eq!(comment_events(&p.project), events, "nothing was emitted");
}

// ---------------------------------------------------------------------------
// Changed and removed nodes
// ---------------------------------------------------------------------------

#[test]
fn a_comment_on_a_node_that_a_later_version_changed_or_removed_is_marked_so() {
    let (f, client) = ready();
    let p = paper(f, client);
    let on = |node: &str| -> String {
        id_of(&comment(
            f,
            client,
            &human(f),
            &p.project,
            &p.v1,
            node,
            "Look at this one",
        ))
    };
    let (kept, changed, removed) = (on(&p.a), on(&p.b), on(&p.c));
    // A comment that is resolved first, so it does not stop the removal.
    let resolved_on_removed = on(&p.c);
    p.edit(f, client, json!([set_text(&p.c, 1, "The shot.")]));
    let fixed = p.save(f, client);
    addressed(f, client, &p.project, &resolved_on_removed, &fixed);
    human_acts(f, client, &p.project, &resolved_on_removed, "resolve");

    p.edit(
        f,
        client,
        json!([set_text(&p.b, 1, "Copy on the left."), delete(&p.c, 2)]),
    );
    let latest = save_with(
        f,
        client,
        &p.project,
        &p.changeset,
        &p.doc,
        &json!({ "remove_commented": true }),
    );

    for auth in [author(f), human(f)] {
        let read = |id: &str| comment_of(f, client, &auth, &p.project, id);
        let current = read(&kept);
        assert_eq!(current["node_status"], "current", "{current}");
        assert_eq!(current["compared_to"], latest.as_str(), "{current}");

        let older = read(&changed);
        assert_eq!(
            older["node_status"], "changed",
            "the node changed in a later version, so the comment was made on an older one: {older}"
        );
        assert_eq!(older["version_id"], p.v1.as_str(), "{older}");
        assert_eq!(older["compared_to"], latest.as_str(), "{older}");

        let gone = read(&removed);
        assert_eq!(
            gone["node_status"], "removed",
            "the node is gone, the comment stays and says so: {gone}"
        );
        assert_eq!(gone["node"], p.c.as_str(), "{gone}");
        assert_eq!(gone["status"], "open", "{gone}");
    }
    let listed = list(f, client, &author(f), &p.project, &format!("doc={}", p.doc));
    assert!(
        listed
            .iter()
            .any(|c| c["id"] == removed.as_str() && c["node_status"] == "removed"),
        "a comment on a removed node is still listed: {listed:?}"
    );
}

#[test]
fn a_comment_on_the_mainline_follows_the_mainline_and_not_drafts_or_open_change_sets() {
    let (f, client) = ready();
    let p = paper(f, client);
    merge(f, client, &p.project, &p.changeset);
    let id = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.b,
        "Too long",
    ));

    // A new change set changes the node and saves a version, which is not
    // the mainline yet.
    let next = new_changeset(f, client, &p.project);
    open_draft(f, client, &p.project, &next, &p.doc);
    let rev = rev_in_draft(f, client, &p.project, &next, &p.doc, &p.b);
    edit(
        f,
        client,
        &p.project,
        &next,
        &p.doc,
        json!([set_text(&p.b, rev, "Short copy.")]),
    );
    save(f, client, &p.project, &next, &p.doc);
    let read = comment_of(f, client, &author(f), &p.project, &id);
    assert_eq!(read["node_status"], "current", "{read}");
    assert_eq!(read["compared_to"], p.v1.as_str(), "{read}");

    merge(f, client, &p.project, &next);
    let mainline = mainline_of(f, client, &p.project, &p.doc);
    let read = comment_of(f, client, &author(f), &p.project, &id);
    assert_eq!(
        read["node_status"], "changed",
        "the new mainline changed the node: {read}"
    );
    assert_eq!(read["compared_to"], mainline.as_str(), "{read}");
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

#[test]
fn comments_are_listed_by_project_doc_change_set_and_status() {
    let (f, client) = ready();
    let p = paper(f, client);
    let q = paper_in(f, client, &p.project);
    let first = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.a,
        "One",
    ));
    let second = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &q.v1,
        &q.a,
        "Two",
    ));
    let third = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.b,
        "Three",
    ));
    p.edit(f, client, json!([set_text(&p.a, 1, "Fixed.")]));
    let v2 = p.save(f, client);
    addressed(f, client, &p.project, &first, &v2);

    for auth in [author(f), human(f)] {
        assert_eq!(
            ids(&list(f, client, &auth, &p.project, "")),
            [first.clone(), second.clone(), third.clone()],
            "every comment of the project, oldest first"
        );
        assert_eq!(
            ids(&list(
                f,
                client,
                &auth,
                &p.project,
                &format!("doc={}", p.doc)
            )),
            [first.clone(), third.clone()],
            "the comments of one doc"
        );
        assert_eq!(
            ids(&list(
                f,
                client,
                &auth,
                &p.project,
                &format!("changeset={}", q.changeset)
            )),
            [second.clone()],
            "the comments of one change set"
        );
        assert_eq!(
            ids(&list(f, client, &auth, &p.project, "status=open")),
            [second.clone(), third.clone()],
            "the open comments"
        );
        assert_eq!(
            ids(&list(f, client, &auth, &p.project, "status=addressed")),
            [first.clone()],
            "the addressed comments"
        );
        assert_eq!(
            ids(&list(
                f,
                client,
                &auth,
                &p.project,
                &format!("changeset={}&status=open", p.changeset)
            )),
            [third.clone()],
            "the filters combine"
        );
        assert!(
            list(f, client, &auth, &p.project, "status=resolved").is_empty(),
            "no comment is resolved"
        );
    }
    let other_project = paper(f, client);
    assert!(
        list(f, client, &author(f), &other_project.project, "").is_empty(),
        "another project lists none of them"
    );
    let response = call(
        client,
        "GET",
        &format!("{}/comments?status=done", project_base(f, &p.project)),
        &author(f),
        None,
    );
    assert_error(&response, 400, "invalid_request", "a status that is none");
}

// ---------------------------------------------------------------------------
// Saving a version that removes a commented node
// ---------------------------------------------------------------------------

#[test]
fn saving_a_version_that_removes_a_node_with_open_comments_needs_remove_commented() {
    let (f, client) = ready();
    let p = paper(f, client);
    let on_c = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.c,
        "Drop the shot?",
    ));
    let also_on_c = id_of(&comment(
        f,
        client,
        &critic(f),
        &p.project,
        &p.v1,
        &p.c,
        "The shot is too wide",
    ));
    let on_a = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.a,
        "Keep",
    ));
    p.edit(f, client, json!([delete(&p.c, 1)]));
    let saved_before = versions_of(&p.doc, &p.changeset);
    let saves_before = events_of(&p.project, "version.saved").len();

    for body in [json!({}), json!({ "remove_commented": false })] {
        let response = p.save_response(f, client, &body);
        assert_eq!(
            response.status,
            409,
            "saving with {body} removes commented nodes: {}",
            response.text()
        );
        let answer = response.json();
        assert_eq!(answer["error"], "commented_nodes_removed", "{answer}");
        let listed: BTreeSet<String> = answer["comments"]
            .as_array()
            .unwrap_or_else(|| panic!("the answer lists the comments: {answer}"))
            .iter()
            .map(id_of)
            .collect();
        assert_eq!(
            listed,
            [on_c.clone(), also_on_c.clone()].into_iter().collect(),
            "the answer lists exactly the open comments on removed nodes: {answer}"
        );
        assert!(
            answer["comments"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["node"] == p.c.as_str() && c["text"].is_string()),
            "{answer}"
        );
    }
    assert_eq!(
        versions_of(&p.doc, &p.changeset),
        saved_before,
        "the refused save stored no version"
    );
    assert_eq!(
        events_of(&p.project, "version.saved").len(),
        saves_before,
        "and emitted nothing"
    );

    let saved = save_with(
        f,
        client,
        &p.project,
        &p.changeset,
        &p.doc,
        &json!({ "remove_commented": true }),
    );
    assert_eq!(versions_of(&p.doc, &p.changeset), saved_before + 1);
    for id in [&on_c, &also_on_c] {
        let read = comment_of(f, client, &author(f), &p.project, id);
        assert_eq!(read["node_status"], "removed", "{read}");
        assert_eq!(read["compared_to"], saved.as_str(), "{read}");
        assert_eq!(read["status"], "open", "the comment stays: {read}");
    }
    assert_eq!(
        comment_of(f, client, &author(f), &p.project, &on_a)["node_status"],
        "current"
    );
}

#[test]
fn a_resolved_comment_does_not_stop_a_save_and_a_comment_on_a_kept_node_is_not_listed() {
    let (f, client) = ready();
    let p = paper(f, client);
    let on_c = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.c,
        "Shot?",
    ));
    comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.a,
        "Keep the hero",
    );
    p.edit(f, client, json!([set_text(&p.c, 1, "Wider shot.")]));
    let v2 = p.save(f, client);
    addressed(f, client, &p.project, &on_c, &v2);
    human_acts(f, client, &p.project, &on_c, "resolve");

    p.edit(f, client, json!([delete(&p.c, 2)]));
    let response = p.save_response(f, client, &json!({}));
    assert_eq!(
        response.status,
        201,
        "only a resolved comment is on the removed node: {}",
        response.text()
    );
}

#[test]
fn a_new_change_set_that_removes_a_node_with_an_open_mainline_comment_needs_remove_commented() {
    let (f, client) = ready();
    let p = paper(f, client);
    merge(f, client, &p.project, &p.changeset);
    let id = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.b,
        "Rewrite this",
    ));
    let next = new_changeset(f, client, &p.project);
    open_draft(f, client, &p.project, &next, &p.doc);
    let rev = rev_in_draft(f, client, &p.project, &next, &p.doc, &p.b);
    edit(
        f,
        client,
        &p.project,
        &next,
        &p.doc,
        json!([delete(&p.b, rev)]),
    );

    let response = save_response(f, client, &p.project, &next, &p.doc, &json!({}));
    assert_eq!(
        response.status,
        409,
        "the draft's base, the mainline, has the commented node: {}",
        response.text()
    );
    let answer = response.json();
    assert_eq!(answer["error"], "commented_nodes_removed", "{answer}");
    assert_eq!(answer["comments"][0]["id"], id.as_str(), "{answer}");
    assert_eq!(versions_of(&p.doc, &next), 0, "nothing was saved");
    save_with(
        f,
        client,
        &p.project,
        &next,
        &p.doc,
        &json!({ "remove_commented": true }),
    );
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[test]
fn commenting_replying_and_changing_a_status_emit_their_events() {
    let (f, client) = ready();
    let p = paper(f, client);
    let id = id_of(&comment(
        f,
        client,
        &human(f),
        &p.project,
        &p.v1,
        &p.b,
        "Shorter",
    ));
    let created = events_of(&p.project, "comment.created");
    assert_eq!(created.len(), 1, "{created:?}");
    let data = &created[0];
    assert_eq!(data["comment_id"], id.as_str(), "{data}");
    assert_eq!(data["doc_id"], p.doc.as_str(), "{data}");
    assert_eq!(data["version_id"], p.v1.as_str(), "{data}");
    assert_eq!(data["node"], p.b.as_str(), "{data}");
    assert_eq!(data["changeset_id"], p.changeset.as_str(), "{data}");
    assert_eq!(data["status"], "open", "{data}");

    let answered = reply(f, client, &author(f), &p.project, &id, "Will do.");
    let reply_id = id_of(&answered["replies"][0]);
    let replied = events_of(&p.project, "comment.replied");
    assert_eq!(replied.len(), 1, "{replied:?}");
    assert_eq!(replied[0]["comment_id"], id.as_str(), "{replied:?}");
    assert_eq!(replied[0]["reply_id"], reply_id.as_str(), "{replied:?}");

    p.edit(f, client, json!([set_text(&p.b, 1, "Short.")]));
    let v2 = p.save(f, client);
    addressed(f, client, &p.project, &id, &v2);
    human_acts(f, client, &p.project, &id, "resolve");
    human_acts(f, client, &p.project, &id, "reopen");
    let changes: Vec<(String, String)> = events_of(&p.project, "comment.status_changed")
        .iter()
        .map(|data| {
            assert_eq!(data["comment_id"], id.as_str(), "{data}");
            assert_eq!(data["doc_id"], p.doc.as_str(), "{data}");
            (
                data["from_status"].as_str().unwrap_or("?").to_string(),
                data["status"].as_str().unwrap_or("?").to_string(),
            )
        })
        .collect();
    assert_eq!(
        changes,
        [
            ("open".to_string(), "addressed".to_string()),
            ("addressed".to_string(), "resolved".to_string()),
            ("resolved".to_string(), "open".to_string()),
        ],
        "one event per status change, in order"
    );
}
