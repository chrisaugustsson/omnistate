//! US-023: claims through the API. Run with `--features stack-tests` in the
//! Compose test container, which reaches Postgres at `postgres:5432` and
//! RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint two API keys through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. The tests
//! of expiry use a second router over the same database whose claims last a
//! few seconds (`AppState::with_claim_ttl`), and the tests of the background
//! sweep a router of their own each, which no other test calls; every test
//! talks to one router only, since each keeps its own drafts in memory. Some
//! tests hold a row of the scratch database with an admin transaction to stop
//! a request of the API halfway through, at a known point.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; docs, drafts and
//! operations are those of US-022, tests/docs.rs):
//!
//! - A claim locks a board (design docs) or a section (HTML docs, the
//!   `<section>` block) of one draft for one agent identity. The agent
//!   identity of a request, and so the holder of a claim, is its `X-Omni-Run`
//!   and `X-Omni-Role` headers only, not its API key: two agents that share a
//!   key but not a run and role are two identities, and an agent with the same
//!   role but another run, or the same run but another role, is another
//!   identity. `X-Omni-Model` and `X-Omni-Harness` are recorded (the Claim's
//!   `agent`) but do not tell holders apart: the same run and role with another
//!   or a missing model or harness is the holder for its edits, its release,
//!   the renewal of its claim, and claiming the node again (which answers the
//!   one claim, never a second).
//! - `POST /changesets/{changeset_id}/claims` (agent) takes `{"node"}` and
//!   claims that node in the change set's draft that has it (node IDs are
//!   unique within the project, so the node names the draft): 201 `{"claim":
//!   Claim}`. Claiming a node that the same identity already holds answers 200
//!   or 201 with the same claim. A Claim is `{"id" (UUIDv7), "doc_id",
//!   "draft_id", "changeset_id", "node", "agent": {"run", "role", "model",
//!   "harness"}, "taken_at", "expires_at"}`.
//!   - A node that no draft of the change set has is 404 `{"error":
//!     "not_found"}`. A node that is neither a board nor a section is 400
//!     `"error": "invalid_request"`. Neither claims anything.
//!   - If another identity holds a claim on the node, on one of its ancestors
//!     or on one of its descendants, it is 409 `{"error": "claimed",
//!     "message", "claim"}`: `claim` is the claim in the way and `message`
//!     names its holder (their run and role). Nothing is claimed.
//! - `POST /changesets/{changeset_id}/claims/release` (agent) takes `{"node"}`
//!   and ends the caller's claim on that node: 200 `{"claim": Claim}`. A node
//!   without a claim is 404 `not_found`; another identity's claim is 409
//!   `"error": "claimed"` and stays.
//! - While a claim is held, a batch of operations from another identity that
//!   touches the claimed subtree (changes, moves or deletes a node in it, or
//!   inserts or moves a node into it) is 409 `{"error": "claimed", "index",
//!   "message", "claim"}`: `index` is the operation's, `message` names the
//!   holder (their run and role), and nothing changes or is logged. The
//!   holder's own operations, and everyone's outside the subtree, apply.
//! - Replacing or deleting an ancestor of a claimed node takes the claimed
//!   subtree along, so it is refused for other identities the same way.
//! - A claim expires when its holder makes no operation in that draft for the
//!   claim TTL: 10 minutes, or what `AppState::with_claim_ttl` sets.
//!   `expires_at` is the holder's last operation in the draft (or the claim's
//!   `taken_at`) plus the TTL. An expired claim blocks nothing and is no longer
//!   listed. Only the holder's own applied operations renew its claim; other
//!   agents' operations in the draft never do.
//! - An expired claim never comes back to life. A batch of the holder renews
//!   its claim when the batch commits, before the draft's lock is let go, and
//!   only if the claim is still live at that moment: a batch that commits
//!   after the claim's `expires_at` renews nothing, and once another identity
//!   has taken the subtree, nothing the old holder does restores its claim.
//! - Claims expire on time without any request: a background sweep, started
//!   with the API's state or router (built inside a Tokio runtime, as these
//!   tests build theirs), ends the claims whose time is up every few seconds
//!   (at most 5) and emits their `claim.expired`, whose `data` also carries
//!   the claim's `expires_at`.
//! - Releasing a claim holds the draft's lock until the release has
//!   committed: an operation sent meanwhile waits for it, and if the release
//!   fails, the claim stays and still stops that operation.
//! - `GET /docs/{doc_id}` lists the doc's live claims, as Claims, in
//!   `"claims"`.
//! - Taking, releasing and the expiry of a claim emit `claim.taken`,
//!   `claim.released` and `claim.expired`, each with `data` `{"claim_id",
//!   "doc_id", "draft_id", "changeset_id", "node", "agent"}`; that of
//!   `claim.expired` also has `"expires_at"`.
//! - The human never claims: a session on the claim routes is 403 `forbidden`.
//! - Claims may live in memory, or in the tenant table `claims` whose only
//!   columns without a default are `workspace_id`, `draft_id`, `node` and
//!   `agent` (tests/workspace_delete.rs seeds it when it exists).

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::routes::{Access, registry};
use omni_api::{AppState, Config};
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const TEST_DB: &str = "omni_us023_claims";
const TEST_BUCKET: &str = "omni-us023-claims";
/// A node ID in the right format that no node has.
const UNKNOWN_NODE: &str = "zzzzzzzz";
/// How long a claim lasts on the short router.
const SHORT_TTL: Duration = Duration::from_secs(3);

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
    /// Two API keys of the workspace.
    key: String,
    other_key: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us023-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
        let mint = || -> String {
            let response = call(
                &server.client,
                "POST",
                &format!("/api/v1/w/{workspace}/api-keys"),
                &Auth::Session(&session_token("alice")),
                Some(&json!({})),
            );
            assert_eq!(response.status, 201, "mint a key: {}", response.text());
            response.json()["secret"]
                .as_str()
                .expect("the new key has a secret")
                .to_string()
        };
        let key = mint();
        let other_key = mint();
        Fixture {
            workspace,
            alice_token: session_token("alice"),
            key,
            other_key,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

/// A second router over the scratch database whose claims last `SHORT_TTL`.
struct ShortServer {
    client: Client,
    /// Keeps the router's runtime, and so the router, alive.
    _runtime: tokio::runtime::Runtime,
}

fn short() -> (&'static Fixture, &'static Client) {
    static SHORT: OnceLock<ShortServer> = OnceLock::new();
    let f = fixture();
    let server = SHORT.get_or_init(serve_short);
    (f, &server.client)
}

/// A router of its own whose claims last `SHORT_TTL`, for a test that must be
/// the only one whose requests reach it: no request of another test can end
/// its expired claims, so only the API itself does.
fn quiet() -> (&'static Fixture, ShortServer) {
    (fixture(), serve_short())
}

fn serve_short() -> ShortServer {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind a local port");
    let port = listener.local_addr().unwrap().port();
    let router = {
        // The connection pool needs a runtime to be created in.
        let _guard = runtime.enter();
        let state = AppState::new(&config())
            .expect("build the API state")
            .with_claim_ttl(SHORT_TTL);
        omni_api::router(state)
    };
    runtime.spawn(async move {
        axum::serve(listener, router).await.expect("serve the API");
    });
    ShortServer {
        client: Client::local(port),
        _runtime: runtime,
    }
}

/// Who sends a request: an agent (an API key with its run and role, and the
/// test model and harness), an agent that declares its model and harness
/// itself (or leaves them out), or the human (a session cookie).
#[derive(Clone, Copy)]
enum Auth<'a> {
    Agent {
        key: &'a str,
        run: &'a str,
        role: &'a str,
    },
    Declared {
        key: &'a str,
        run: &'a str,
        role: &'a str,
        model: Option<&'a str>,
        harness: Option<&'a str>,
    },
    Session(&'a str),
}

/// The agent that takes the claims in these tests.
fn holder(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-1",
        role: "writer",
    }
}

/// Another agent, with a key of its own.
fn other(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.other_key,
        run: "run-2",
        role: "reviewer",
    }
}

/// Another agent that shares the holder's API key, as agents on one machine
/// that ran `omni login` once do.
fn same_key_other_agent(f: &Fixture) -> Auth<'_> {
    Auth::Agent {
        key: &f.key,
        run: "run-3",
        role: "editor",
    }
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let headers: Vec<(&str, String)> = match auth {
        Auth::Agent { key, run, role } => vec![
            ("Authorization", format!("Bearer {key}")),
            ("X-Omni-Run", run.to_string()),
            ("X-Omni-Role", role.to_string()),
            ("X-Omni-Model", "test-model".to_string()),
            ("X-Omni-Harness", "test-harness".to_string()),
        ],
        Auth::Declared {
            key,
            run,
            role,
            model,
            harness,
        } => {
            let mut headers = vec![
                ("Authorization", format!("Bearer {key}")),
                ("X-Omni-Run", run.to_string()),
                ("X-Omni-Role", role.to_string()),
            ];
            if let Some(model) = model {
                headers.push(("X-Omni-Model", model.to_string()));
            }
            if let Some(harness) = harness {
                headers.push(("X-Omni-Harness", harness.to_string()));
            }
            headers
        }
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

fn assert_uuid_v7(id: &Value) {
    let id = id
        .as_str()
        .unwrap_or_else(|| panic!("an ID is a string: {id}"));
    let bytes = id.as_bytes();
    assert!(
        bytes.len() == 36
            && bytes[14] == b'7'
            && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
            && id.chars().all(|c| c == '-' || c.is_ascii_hexdigit()),
        "{id:?} is not a UUIDv7"
    );
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

/// One draft of an HTML doc in a change set, as the tests use it.
struct Draft {
    project: String,
    changeset: String,
    doc: String,
    draft: String,
}

impl Draft {
    fn base(&self, f: &Fixture) -> String {
        format!("/api/v1/w/{}/projects/{}", f.workspace, self.project)
    }

    fn claims_path(&self, f: &Fixture) -> String {
        format!("{}/changesets/{}/claims", self.base(f), self.changeset)
    }

    fn draft_path(&self, f: &Fixture) -> String {
        format!(
            "{}/changesets/{}/docs/{}",
            self.base(f),
            self.changeset,
            self.doc
        )
    }

    fn doc_path(&self, f: &Fixture) -> String {
        format!("{}/docs/{}", self.base(f), self.doc)
    }
}

/// A new project with an open change set and an HTML PRD started in it.
fn new_draft(f: &Fixture, client: &Client) -> Draft {
    let project = must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects", f.workspace),
            &holder(f),
            Some(&json!({ "name": unique("claims-project") })),
        ),
        201,
        "create a project",
    )["project"]["id"]
        .as_str()
        .expect("a project id")
        .to_string();
    let changeset = new_changeset(f, client, &project);
    let (doc, draft) = new_doc(f, client, &project, &changeset);
    Draft {
        project,
        changeset,
        doc,
        draft,
    }
}

fn new_changeset(f: &Fixture, client: &Client, project: &str) -> String {
    must(
        call(
            client,
            "POST",
            &format!("/api/v1/w/{}/projects/{project}/changesets", f.workspace),
            &holder(f),
            Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
        ),
        201,
        "create a change set",
    )["changeset"]["id"]
        .as_str()
        .expect("a change set id")
        .to_string()
}

/// Starts an HTML PRD in the change set; returns (doc ID, draft ID).
fn new_doc(f: &Fixture, client: &Client, project: &str, changeset: &str) -> (String, String) {
    let answer = must(
        call(
            client,
            "POST",
            &format!(
                "/api/v1/w/{}/projects/{project}/changesets/{changeset}/docs",
                f.workspace
            ),
            &holder(f),
            Some(&json!({ "kind": "html", "pillar": "spec", "title": unique("Checkout PRD") })),
        ),
        201,
        "create a doc",
    );
    let doc = answer["doc"]["id"].as_str().expect("a doc id").to_string();
    let draft = answer["draft"]["id"]
        .as_str()
        .expect("a draft id")
        .to_string();
    (doc, draft)
}

fn edit(f: &Fixture, client: &Client, auth: &Auth, draft: &Draft, ops: &Value) -> Response {
    call(
        client,
        "POST",
        &format!("{}/ops", draft.draft_path(f)),
        auth,
        Some(&json!({ "ops": ops })),
    )
}

/// Inserts `content` at the top of the draft as the holder; returns the IDs
/// of the new nodes in document order.
fn insert(f: &Fixture, client: &Client, draft: &Draft, content: &str) -> Vec<String> {
    let answer = must(
        edit(
            f,
            client,
            &holder(f),
            draft,
            &json!([{ "op": "insert", "parent": null, "position": { "after": null }, "content": content }]),
        ),
        200,
        "insert a block",
    );
    answer["results"][0]["created"]
        .as_array()
        .expect("the IDs of the new nodes")
        .iter()
        .map(|id| id.as_str().expect("a node ID").to_string())
        .collect()
}

/// A section with a heading and a paragraph: (section, heading, paragraph).
fn add_section(
    f: &Fixture,
    client: &Client,
    draft: &Draft,
    title: &str,
) -> (String, String, String) {
    let ids = insert(
        f,
        client,
        draft,
        &format!("<section><h2>{title}</h2><p>{title} text</p></section>"),
    );
    assert_eq!(ids.len(), 3, "a section, its heading and its paragraph");
    (ids[0].clone(), ids[1].clone(), ids[2].clone())
}

fn claim(f: &Fixture, client: &Client, auth: &Auth, draft: &Draft, node: &str) -> Response {
    call(
        client,
        "POST",
        &draft.claims_path(f),
        auth,
        Some(&json!({ "node": node })),
    )
}

fn release(f: &Fixture, client: &Client, auth: &Auth, draft: &Draft, node: &str) -> Response {
    call(
        client,
        "POST",
        &format!("{}/release", draft.claims_path(f)),
        auth,
        Some(&json!({ "node": node })),
    )
}

/// The doc's claims, as `GET /docs/{doc}` lists them.
fn claims_of(f: &Fixture, client: &Client, draft: &Draft) -> Vec<Value> {
    let shown = must(
        call(client, "GET", &draft.doc_path(f), &holder(f), None),
        200,
        "show the doc",
    );
    shown["doc"]["claims"]
        .as_array()
        .unwrap_or_else(|| panic!("the doc lists its claims: {shown}"))
        .clone()
}

fn draft_html(f: &Fixture, client: &Client, draft: &Draft) -> String {
    must(
        call(client, "GET", &draft.draft_path(f), &holder(f), None),
        200,
        "read the draft",
    )["draft"]["html"]
        .as_str()
        .expect("the draft has its html")
        .to_string()
}

/// The number of rows in the draft's log.
fn log_len(draft: &Draft) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar("select count(*) from draft_ops where draft_id = $1::uuid")
            .bind(&draft.draft)
            .fetch_one(&mut conn)
            .await
            .expect("count the draft's log")
    })
}

/// The live events of a project named `event`, oldest first, as their `data`.
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

/// Seconds since the epoch of an RFC 3339 time, as Postgres reads it.
fn epoch(time: &Value) -> f64 {
    let time = time
        .as_str()
        .unwrap_or_else(|| panic!("a time is a string: {time}"));
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar::<_, f64>("select extract(epoch from $1::text::timestamptz)::float8")
            .bind(time)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("{time:?} is not a time: {e}"))
    })
}

/// Asserts that a refusal names the holder: their run and role.
fn assert_names_holder(answer: &Value, run: &str, role: &str) {
    let message = answer["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal has a message: {answer}"));
    assert!(
        message.contains(run) && message.contains(role),
        "the message names the holder {run}/{role}: {message:?}"
    );
    assert_eq!(answer["claim"]["agent"]["run"], run, "{answer}");
    assert_eq!(answer["claim"]["agent"]["role"], role, "{answer}");
}

/// Asserts that `response` is a refusal because `node` is claimed by the
/// holder (run-1/writer).
fn assert_claimed_by_holder(response: &Response, node: &str) -> Value {
    assert_eq!(response.status, 409, "body: {}", response.text());
    let answer = response.json();
    assert_eq!(answer["error"], "claimed", "{answer}");
    assert_names_holder(&answer, "run-1", "writer");
    assert_eq!(answer["claim"]["node"], node, "{answer}");
    answer
}

fn set_html(node: &str, rev: u64, html: &str) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": { "html": html } })
}

/// The `claim.expired` events of a project, oldest first: each event's `data`
/// and the time it was written, in seconds since the epoch.
fn expiry_events(project: &str) -> Vec<(Value, f64)> {
    block(async {
        let mut conn = admin_connection().await;
        let rows = sqlx::query_as::<_, (String, f64)>(
            "select data::jsonb::text, extract(epoch from created_at)::float8 from live_events \
             where project_id = $1::uuid and event = 'claim.expired' order by seq",
        )
        .bind(project)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the live events: {e}"));
        rows.into_iter()
            .map(|(text, at)| (parse(&text), at))
            .collect()
    })
}

/// Seconds since the epoch, as the database's clock says.
fn db_now() -> f64 {
    block(async {
        let mut conn = admin_connection().await;
        sqlx::query_scalar::<_, f64>("select extract(epoch from clock_timestamp())::float8")
            .fetch_one(&mut conn)
            .await
            .expect("read the clock")
    })
}

/// Waits, making no API call, until the project has a `claim.expired` event or
/// `within` has passed, and returns the events.
fn wait_for_expiry_event(project: &str, within: Duration) -> Vec<(Value, f64)> {
    let start = std::time::Instant::now();
    loop {
        let events = expiry_events(project);
        if !events.is_empty() || start.elapsed() >= within {
            return events;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// An admin transaction that holds one row with `FOR UPDATE`, which stops
/// every transaction of the API that changes the row or adds a row that refers
/// to it (a foreign key check) until the lock is let go. The row is held by a
/// thread of its own; dropping the lock lets it go too.
struct RowLock {
    /// The admin transaction's backend.
    pid: i32,
    release: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RowLock {
    /// Holds the row of `table` (a table with an `id` column) whose ID is `id`.
    fn hold(table: &'static str, id: &str) -> RowLock {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let id = id.to_string();
        let thread = std::thread::spawn(move || {
            block(async move {
                let mut conn = admin_connection().await;
                let mut tx = conn.begin().await.expect("begin the admin transaction");
                let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
                    .fetch_one(&mut *tx)
                    .await
                    .expect("the admin backend");
                sqlx::query(&format!(
                    "select 1 from {table} where id = $1::uuid for update"
                ))
                .bind(&id)
                .execute(&mut *tx)
                .await
                .unwrap_or_else(|e| panic!("lock the row of {table}: {e}"));
                ready_tx.send(pid).expect("report the lock");
                // Nothing else runs on this thread's runtime meanwhile.
                let _ = release_rx.recv();
                let _ = tx.rollback().await;
            })
        });
        let pid = ready_rx.recv().expect("the row is held");
        RowLock {
            pid,
            release: Some(release_tx),
            thread: Some(thread),
        }
    }

    /// The backends that wait for the row.
    fn waiting(&self) -> Vec<i32> {
        block(async {
            let mut conn = admin_connection().await;
            sqlx::query_scalar::<_, i32>(
                "select pid from pg_stat_activity \
                 where $1 = any(pg_blocking_pids(pid)) order by backend_start, pid",
            )
            .bind(self.pid)
            .fetch_all(&mut conn)
            .await
            .expect("read the waiting backends")
        })
    }

    /// Waits until at least `count` backends wait for the row, or `within`
    /// passed; returns the waiting backends.
    fn wait_for_waiters(&self, count: usize, within: Duration) -> Vec<i32> {
        let start = std::time::Instant::now();
        loop {
            let waiting = self.waiting();
            if waiting.len() >= count || start.elapsed() >= within {
                return waiting;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Lets the row go and waits until the admin transaction has ended.
    fn let_go(mut self) {
        self.end();
    }

    fn end(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RowLock {
    fn drop(&mut self) {
        self.end();
    }
}

/// Ends the backend `pid`, so that its transaction fails.
fn terminate(pid: i32) {
    block(async {
        let mut conn = admin_connection().await;
        let ended: bool = sqlx::query_scalar("select pg_terminate_backend($1)")
            .bind(pid)
            .fetch_one(&mut conn)
            .await
            .expect("terminate the backend");
        assert!(ended, "backend {pid} was terminated");
    })
}

// ---------------------------------------------------------------------------
// The routes
// ---------------------------------------------------------------------------

#[test]
fn the_claim_routes_are_in_the_registry_as_agent_routes() {
    let routes = registry();
    for path in [
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/claims",
        "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/claims/release",
    ] {
        let entry = routes
            .iter()
            .find(|r| r.method == "POST" && r.path == path)
            .unwrap_or_else(|| panic!("the registry has no POST {path}"));
        assert_eq!(entry.access, Access::Agent, "POST {path}");
    }
}

// ---------------------------------------------------------------------------
// omni claim and omni release
// ---------------------------------------------------------------------------

#[test]
fn an_agent_claims_a_section_of_its_draft_and_the_doc_lists_the_claim() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, _, _) = add_section(f, client, &draft, "Pricing");

    let answer = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let claim = &answer["claim"];
    assert_uuid_v7(&claim["id"]);
    assert_eq!(claim["node"], section.as_str(), "{claim}");
    assert_eq!(claim["doc_id"], draft.doc.as_str(), "{claim}");
    assert_eq!(
        claim["draft_id"],
        draft.draft.as_str(),
        "the claim belongs to the change set's draft: {claim}"
    );
    assert_eq!(claim["changeset_id"], draft.changeset.as_str(), "{claim}");
    assert_eq!(
        claim["agent"],
        json!({ "run": "run-1", "role": "writer", "model": "test-model", "harness": "test-harness" }),
        "{claim}"
    );
    let ttl = epoch(&claim["expires_at"]) - epoch(&claim["taken_at"]);
    assert!(
        (595.0..=605.0).contains(&ttl),
        "a claim lasts 10 minutes by default, not {ttl} s: {claim}"
    );

    let listed = claims_of(f, client, &draft);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["id"], claim["id"], "{listed:?}");
    assert_eq!(listed[0]["node"], section.as_str(), "{listed:?}");
    assert_eq!(listed[0]["agent"]["run"], "run-1", "{listed:?}");
    assert_eq!(listed[0]["agent"]["role"], "writer", "{listed:?}");
}

#[test]
fn claiming_a_node_twice_keeps_the_one_claim() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, _, _) = add_section(f, client, &draft, "Pricing");
    let first = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let again = claim(f, client, &holder(f), &draft, &section);
    assert!(
        matches!(again.status, 200 | 201),
        "the holder claims its own node again: {}",
        again.text()
    );
    assert_eq!(again.json()["claim"]["id"], first["claim"]["id"]);
    assert_eq!(claims_of(f, client, &draft).len(), 1);
}

#[test]
fn the_holder_releases_its_claim() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, _, paragraph) = add_section(f, client, &draft, "Pricing");
    let taken = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let released = must(
        release(f, client, &holder(f), &draft, &section),
        200,
        "release the claim",
    );
    assert_eq!(released["claim"]["id"], taken["claim"]["id"], "{released}");
    assert_eq!(released["claim"]["node"], section.as_str(), "{released}");
    assert_eq!(claims_of(f, client, &draft), Vec::<Value>::new());

    // Released, the section is free for anyone again.
    must(
        edit(
            f,
            client,
            &other(f),
            &draft,
            &json!([set_html(&paragraph, 1, "Changed by another agent")]),
        ),
        200,
        "edit the released section",
    );
    must(
        claim(f, client, &other(f), &draft, &section),
        201,
        "another agent claims the released section",
    );
}

#[test]
fn releasing_needs_a_claim_of_ones_own() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, _, _) = add_section(f, client, &draft, "Pricing");
    let (free, _, _) = add_section(f, client, &draft, "Shipping");
    must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );

    let response = release(f, client, &other(f), &draft, &section);
    assert_claimed_by_holder(&response, &section);
    assert_eq!(
        claims_of(f, client, &draft).len(),
        1,
        "another agent cannot end the holder's claim"
    );

    let response = release(f, client, &holder(f), &draft, &free);
    assert_error(&response, 404, "not_found");
}

#[test]
fn only_sections_of_a_draft_of_the_change_set_can_be_claimed() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (_, heading, paragraph) = add_section(f, client, &draft, "Pricing");

    // Not a section.
    for node in [&heading, &paragraph] {
        let response = claim(f, client, &holder(f), &draft, node);
        assert_eq!(
            response.status,
            400,
            "{node} is not a board or a section: {}",
            response.text()
        );
        assert_eq!(response.json()["error"], "invalid_request");
    }

    // No such node in the change set's drafts.
    let response = claim(f, client, &holder(f), &draft, UNKNOWN_NODE);
    assert_error(&response, 404, "not_found");

    // A body without a node ID.
    for body in [json!({}), json!({ "node": "not a node" })] {
        let response = call(
            client,
            "POST",
            &draft.claims_path(f),
            &holder(f),
            Some(&body),
        );
        assert_eq!(response.status, 400, "{body}: {}", response.text());
        assert_eq!(response.json()["error"], "invalid_request");
    }
    assert_eq!(claims_of(f, client, &draft), Vec::<Value>::new());
}

#[test]
fn a_claim_belongs_to_the_draft_of_its_change_set() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, _, _) = add_section(f, client, &draft, "Pricing");

    // Another change set of the project has no draft with that node.
    let other_changeset = new_changeset(f, client, &draft.project);
    new_doc(f, client, &draft.project, &other_changeset);
    let elsewhere = Draft {
        project: draft.project.clone(),
        changeset: other_changeset,
        doc: draft.doc.clone(),
        draft: draft.draft.clone(),
    };
    let response = claim(f, client, &holder(f), &elsewhere, &section);
    assert_error(&response, 404, "not_found");

    let answer = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section in its own change set",
    );
    assert_eq!(answer["claim"]["draft_id"], draft.draft.as_str());
    assert_eq!(answer["claim"]["changeset_id"], draft.changeset.as_str());
}

#[test]
fn the_human_never_claims() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, _, _) = add_section(f, client, &draft, "Pricing");
    let human = Auth::Session(&f.alice_token);
    let response = claim(f, client, &human, &draft, &section);
    assert_error(&response, 403, "forbidden");
    must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let response = release(f, client, &human, &draft, &section);
    assert_error(&response, 403, "forbidden");
    assert_eq!(claims_of(f, client, &draft).len(), 1);
}

// ---------------------------------------------------------------------------
// Claims in the way of other claims
// ---------------------------------------------------------------------------

/// A section inside a section: (outer, inner, the inner one's paragraph).
fn nested_sections(f: &Fixture, client: &Client, draft: &Draft) -> (String, String, String) {
    let ids = insert(
        f,
        client,
        draft,
        "<section><h2>Outer</h2><section><h3>Inner</h3><p>Inner text</p></section></section>",
    );
    assert_eq!(ids.len(), 5, "{ids:?}");
    (ids[0].clone(), ids[2].clone(), ids[4].clone())
}

#[test]
fn a_claim_fails_on_a_node_that_another_agent_holds() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, _, _) = add_section(f, client, &draft, "Pricing");
    must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    for auth in [other(f), same_key_other_agent(f)] {
        let response = claim(f, client, &auth, &draft, &section);
        assert_claimed_by_holder(&response, &section);
    }
    let listed = claims_of(f, client, &draft);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["agent"]["run"], "run-1", "{listed:?}");
}

#[test]
fn a_claim_fails_under_a_claim_on_an_ancestor() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (outer, inner, _) = nested_sections(f, client, &draft);
    must(
        claim(f, client, &holder(f), &draft, &outer),
        201,
        "claim the outer section",
    );
    let response = claim(f, client, &other(f), &draft, &inner);
    assert_claimed_by_holder(&response, &outer);
    assert_eq!(claims_of(f, client, &draft).len(), 1);
}

#[test]
fn a_claim_fails_over_a_claim_on_a_descendant() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (outer, inner, _) = nested_sections(f, client, &draft);
    must(
        claim(f, client, &holder(f), &draft, &inner),
        201,
        "claim the inner section",
    );
    let response = claim(f, client, &other(f), &draft, &outer);
    assert_claimed_by_holder(&response, &inner);
    assert_eq!(claims_of(f, client, &draft).len(), 1);
}

#[test]
fn agents_claim_different_sections_of_one_draft() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (pricing, _, _) = add_section(f, client, &draft, "Pricing");
    let (shipping, _, _) = add_section(f, client, &draft, "Shipping");
    must(
        claim(f, client, &holder(f), &draft, &pricing),
        201,
        "the holder claims one section",
    );
    must(
        claim(f, client, &other(f), &draft, &shipping),
        201,
        "another agent claims another section",
    );
    let listed = claims_of(f, client, &draft);
    let mut held: Vec<(String, String)> = listed
        .iter()
        .map(|c| {
            (
                c["node"].as_str().unwrap_or("?").to_string(),
                c["agent"]["run"].as_str().unwrap_or("?").to_string(),
            )
        })
        .collect();
    held.sort();
    let mut wanted = vec![
        (pricing.clone(), "run-1".to_string()),
        (shipping.clone(), "run-2".to_string()),
    ];
    wanted.sort();
    assert_eq!(held, wanted, "{listed:?}");
}

// ---------------------------------------------------------------------------
// Operations under a claim
// ---------------------------------------------------------------------------

#[test]
fn operations_inside_a_claimed_section_from_another_agent_are_rejected_naming_the_holder() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (outside, _, outside_paragraph) = add_section(f, client, &draft, "Shipping");
    let (section, heading, paragraph) = add_section(f, client, &draft, "Pricing");
    must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let html = draft_html(f, client, &draft);
    let logged = log_len(&draft);

    let inside_ops = [
        // A change of a node in the subtree.
        set_html(&paragraph, 1, "Changed by another agent"),
        json!({ "op": "replace", "node": heading, "rev": 1, "content": "<h2>Rewritten</h2>" }),
        // An insert into the subtree.
        json!({ "op": "insert", "parent": section, "position": { "after": heading }, "content": "<p>Sneaked in</p>" }),
        // A move into, within and out of the subtree.
        json!({ "op": "move", "node": outside_paragraph, "rev": 1, "parent": section, "position": { "after": null } }),
        json!({ "op": "move", "node": paragraph, "rev": 1, "parent": outside, "position": { "after": null } }),
        // A delete in the subtree, and of the claimed node itself.
        json!({ "op": "delete", "node": paragraph, "rev": 1 }),
        json!({ "op": "delete", "node": section, "rev": 1 }),
    ];
    for op in inside_ops {
        for auth in [other(f), same_key_other_agent(f)] {
            let response = edit(f, client, &auth, &draft, &json!([op.clone()]));
            let answer = assert_claimed_by_holder(&response, &section);
            assert_eq!(answer["index"], 0, "{op}: {answer}");
        }
    }
    assert_eq!(
        draft_html(f, client, &draft),
        html,
        "a refused operation changes nothing"
    );
    assert_eq!(log_len(&draft), logged, "a refused operation logs nothing");
}

#[test]
fn a_batch_with_one_operation_inside_a_claim_is_refused_whole() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (_, _, outside_paragraph) = add_section(f, client, &draft, "Shipping");
    let (section, _, paragraph) = add_section(f, client, &draft, "Pricing");
    must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let html = draft_html(f, client, &draft);
    let logged = log_len(&draft);
    let response = edit(
        f,
        client,
        &other(f),
        &draft,
        &json!([
            set_html(&outside_paragraph, 1, "Fine on its own"),
            set_html(&paragraph, 1, "Changed by another agent")
        ]),
    );
    let answer = assert_claimed_by_holder(&response, &section);
    assert_eq!(answer["index"], 1, "the second operation: {answer}");
    assert_eq!(draft_html(f, client, &draft), html);
    assert_eq!(log_len(&draft), logged);
    assert!(
        events_of(&draft.project, "op.applied").len() as i64 == logged,
        "a refused batch emits nothing"
    );
}

#[test]
fn the_holder_and_everyone_outside_the_claim_still_edit() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (_, _, outside_paragraph) = add_section(f, client, &draft, "Shipping");
    let (section, _, paragraph) = add_section(f, client, &draft, "Pricing");
    must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    must(
        edit(
            f,
            client,
            &holder(f),
            &draft,
            &json!([
                set_html(&paragraph, 1, "The holder's change"),
                { "op": "insert", "parent": section, "position": { "after": paragraph }, "content": "<p>The holder's new paragraph</p>" }
            ]),
        ),
        200,
        "the holder edits inside its claim",
    );
    must(
        edit(
            f,
            client,
            &other(f),
            &draft,
            &json!([
                set_html(&outside_paragraph, 1, "Another agent's change"),
                { "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>Another agent's top paragraph</p>" }
            ]),
        ),
        200,
        "another agent edits outside the claim",
    );
    let html = draft_html(f, client, &draft);
    for text in [
        "The holder's change",
        "The holder's new paragraph",
        "Another agent's change",
        "Another agent's top paragraph",
    ] {
        assert!(html.contains(text), "{text:?} is in the draft: {html}");
    }
}

// ---------------------------------------------------------------------------
// Expiry
// ---------------------------------------------------------------------------

#[test]
fn the_claim_ttl_is_configurable() {
    let (f, client) = short();
    let draft = new_draft(f, client);
    let (section, _, _) = add_section(f, client, &draft, "Pricing");
    let answer = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let ttl = epoch(&answer["claim"]["expires_at"]) - epoch(&answer["claim"]["taken_at"]);
    assert!(
        (ttl - SHORT_TTL.as_secs_f64()).abs() < 0.5,
        "the claim lasts the configured {SHORT_TTL:?}, not {ttl} s: {answer}"
    );
}

#[test]
fn a_claim_expires_without_an_operation_from_its_holder() {
    let (f, client) = short();
    let draft = new_draft(f, client);
    let (section, _, paragraph) = add_section(f, client, &draft, "Pricing");
    let taken = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let response = edit(
        f,
        client,
        &other(f),
        &draft,
        &json!([set_html(&paragraph, 1, "Too early")]),
    );
    assert_claimed_by_holder(&response, &section);

    std::thread::sleep(SHORT_TTL + Duration::from_millis(700));

    must(
        edit(
            f,
            client,
            &other(f),
            &draft,
            &json!([set_html(&paragraph, 1, "After the claim expired")]),
        ),
        200,
        "another agent edits once the claim expired",
    );
    assert_eq!(
        claims_of(f, client, &draft),
        Vec::<Value>::new(),
        "an expired claim is no longer listed"
    );
    let expired = events_of(&draft.project, "claim.expired");
    assert_eq!(expired.len(), 1, "{expired:?}");
    assert_eq!(
        expired[0]["claim_id"], taken["claim"]["id"],
        "{}",
        expired[0]
    );
    assert_eq!(expired[0]["node"], section.as_str(), "{}", expired[0]);
    assert_eq!(expired[0]["agent"]["run"], "run-1", "{}", expired[0]);
    must(
        claim(f, client, &other(f), &draft, &section),
        201,
        "another agent claims the section once the claim expired",
    );
}

#[test]
fn an_operation_from_the_holder_in_the_draft_keeps_the_claim_alive() {
    let (f, client) = short();
    let draft = new_draft(f, client);
    let (section, heading, paragraph) = add_section(f, client, &draft, "Pricing");
    let taken = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let first_expiry = epoch(&taken["claim"]["expires_at"]);

    std::thread::sleep(Duration::from_millis(2000));
    must(
        edit(
            f,
            client,
            &holder(f),
            &draft,
            &json!([set_html(&heading, 1, "Still working")]),
        ),
        200,
        "the holder edits inside its claim",
    );
    let listed = claims_of(f, client, &draft);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(
        epoch(&listed[0]["expires_at"]) >= first_expiry + 1.5,
        "the holder's operation moved the expiry on: {listed:?} (first {first_expiry})"
    );

    // Past the first expiry, the claim still holds.
    std::thread::sleep(Duration::from_millis(2000));
    let response = edit(
        f,
        client,
        &other(f),
        &draft,
        &json!([set_html(&paragraph, 1, "Too early")]),
    );
    assert_claimed_by_holder(&response, &section);

    // A TTL after the holder's last operation, it is gone.
    std::thread::sleep(Duration::from_millis(1800));
    must(
        edit(
            f,
            client,
            &other(f),
            &draft,
            &json!([set_html(&paragraph, 1, "After the claim expired")]),
        ),
        200,
        "another agent edits once the claim expired",
    );
}

#[test]
fn an_operation_of_the_holder_in_another_draft_does_not_keep_the_claim_alive() {
    let (f, client) = short();
    let draft = new_draft(f, client);
    let (section, _, paragraph) = add_section(f, client, &draft, "Pricing");
    // Another doc of the same change set: another draft.
    let (other_doc, other_draft_id) = new_doc(f, client, &draft.project, &draft.changeset);
    let other_draft = Draft {
        project: draft.project.clone(),
        changeset: draft.changeset.clone(),
        doc: other_doc,
        draft: other_draft_id,
    };
    must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );

    std::thread::sleep(Duration::from_millis(2000));
    insert(f, client, &other_draft, "<p>Busy elsewhere</p>");

    std::thread::sleep(Duration::from_millis(1700));
    must(
        edit(
            f,
            client,
            &other(f),
            &draft,
            &json!([set_html(&paragraph, 1, "After the claim expired")]),
        ),
        200,
        "the claim expired although its holder worked in another draft",
    );
}

// ---------------------------------------------------------------------------
// Live events
// ---------------------------------------------------------------------------

#[test]
fn taking_and_releasing_a_claim_emit_claim_taken_and_claim_released() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, _, _) = add_section(f, client, &draft, "Pricing");
    let taken = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let claim_id = taken["claim"]["id"].clone();

    // A refused claim emits nothing.
    let response = claim(f, client, &other(f), &draft, &section);
    assert_eq!(response.status, 409, "{}", response.text());

    must(
        release(f, client, &holder(f), &draft, &section),
        200,
        "release the claim",
    );
    for name in ["claim.taken", "claim.released"] {
        let events = events_of(&draft.project, name);
        assert_eq!(events.len(), 1, "{name}: {events:?}");
        let data = &events[0];
        assert_eq!(data["claim_id"], claim_id, "{name}: {data}");
        assert_eq!(data["doc_id"], draft.doc.as_str(), "{name}: {data}");
        assert_eq!(data["draft_id"], draft.draft.as_str(), "{name}: {data}");
        assert_eq!(
            data["changeset_id"],
            draft.changeset.as_str(),
            "{name}: {data}"
        );
        assert_eq!(data["node"], section.as_str(), "{name}: {data}");
        assert_eq!(data["agent"]["run"], "run-1", "{name}: {data}");
        assert_eq!(data["agent"]["role"], "writer", "{name}: {data}");
    }
    assert!(
        events_of(&draft.project, "claim.expired").is_empty(),
        "a released claim does not also expire"
    );
}

// ---------------------------------------------------------------------------
// The holder is its run and role
// ---------------------------------------------------------------------------

/// The holder's run and role with a model and harness of their own, or none.
fn holder_as<'a>(key: &'a str, model: Option<&'a str>, harness: Option<&'a str>) -> Auth<'a> {
    Auth::Declared {
        key,
        run: "run-1",
        role: "writer",
        model,
        harness,
    }
}

#[test]
fn the_holder_is_its_run_and_role_whatever_model_or_harness_it_sends() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, heading, paragraph) = add_section(f, client, &draft, "Pricing");
    let taken = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let id = taken["claim"]["id"].clone();

    let variants = [
        (
            holder_as(&f.key, Some("another-model"), Some("another-harness")),
            &heading,
        ),
        // Without a model or harness, and with another key.
        (holder_as(&f.other_key, None, None), &paragraph),
    ];
    for (auth, node) in &variants {
        must(
            edit(
                f,
                client,
                auth,
                &draft,
                &json!([set_html(node, 1, "The holder's change")]),
            ),
            200,
            "the holder edits inside its claim with another or no model and harness",
        );
        let again = claim(f, client, auth, &draft, &section);
        assert!(
            matches!(again.status, 200 | 201),
            "the holder claims its own node again with another or no model: {}",
            again.text()
        );
        assert_eq!(
            again.json()["claim"]["id"],
            id,
            "the same run and role get their one claim back, never a second"
        );
        let listed = claims_of(f, client, &draft);
        assert_eq!(listed.len(), 1, "{listed:?}");
    }

    let released = must(
        release(f, client, &holder_as(&f.key, None, None), &draft, &section),
        200,
        "the holder releases its claim without a model or harness",
    );
    assert_eq!(released["claim"]["id"], id, "{released}");
    assert_eq!(claims_of(f, client, &draft), Vec::<Value>::new());
}

#[test]
fn an_agent_with_the_same_role_and_another_run_or_the_same_run_and_another_role_is_another_agent() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, _, paragraph) = add_section(f, client, &draft, "Pricing");
    must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let html = draft_html(f, client, &draft);
    let strangers = [
        Auth::Agent {
            key: &f.key,
            run: "run-9",
            role: "writer",
        },
        Auth::Agent {
            key: &f.key,
            run: "run-1",
            role: "reviewer",
        },
    ];
    for auth in &strangers {
        let response = claim(f, client, auth, &draft, &section);
        assert_claimed_by_holder(&response, &section);
        let response = edit(
            f,
            client,
            auth,
            &draft,
            &json!([set_html(&paragraph, 1, "Changed by another agent")]),
        );
        let answer = assert_claimed_by_holder(&response, &section);
        assert_eq!(answer["index"], 0, "{answer}");
        let response = release(f, client, auth, &draft, &section);
        assert_claimed_by_holder(&response, &section);
    }
    let listed = claims_of(f, client, &draft);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["agent"]["run"], "run-1", "{listed:?}");
    assert_eq!(listed[0]["agent"]["role"], "writer", "{listed:?}");
    assert_eq!(draft_html(f, client, &draft), html);
}

#[test]
fn the_holder_renews_its_claim_whatever_model_it_sends() {
    let (f, client) = short();
    let draft = new_draft(f, client);
    let (section, heading, paragraph) = add_section(f, client, &draft, "Pricing");
    let taken = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let first_expiry = epoch(&taken["claim"]["expires_at"]);

    std::thread::sleep(Duration::from_millis(2000));
    must(
        edit(
            f,
            client,
            &holder_as(&f.key, Some("another-model"), None),
            &draft,
            &json!([set_html(&heading, 1, "Still working")]),
        ),
        200,
        "the holder edits inside its claim with another model",
    );
    let listed = claims_of(f, client, &draft);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(
        epoch(&listed[0]["expires_at"]) >= first_expiry + 1.5,
        "the holder's operation with another model moved the expiry on: {listed:?} \
         (first {first_expiry})"
    );

    // Past the first expiry, the claim still holds.
    std::thread::sleep(Duration::from_millis(2000));
    let response = edit(
        f,
        client,
        &other(f),
        &draft,
        &json!([set_html(&paragraph, 1, "Too early")]),
    );
    assert_claimed_by_holder(&response, &section);
}

// ---------------------------------------------------------------------------
// Ancestors of a claim
// ---------------------------------------------------------------------------

#[test]
fn replacing_or_deleting_an_ancestor_of_a_claimed_section_is_refused_for_other_agents() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (outer, inner, _) = nested_sections(f, client, &draft);
    must(
        claim(f, client, &holder(f), &draft, &inner),
        201,
        "claim the inner section",
    );
    let html = draft_html(f, client, &draft);
    let logged = log_len(&draft);
    let ancestor_ops = [
        json!({ "op": "replace", "node": outer, "rev": 1, "content": "<section><h2>Replaced</h2></section>" }),
        json!({ "op": "delete", "node": outer, "rev": 1 }),
    ];
    for op in ancestor_ops {
        for auth in [other(f), same_key_other_agent(f)] {
            let response = edit(f, client, &auth, &draft, &json!([op.clone()]));
            let answer = assert_claimed_by_holder(&response, &inner);
            assert_eq!(answer["index"], 0, "{op}: {answer}");
        }
    }
    assert_eq!(
        draft_html(f, client, &draft),
        html,
        "the claimed section is still there"
    );
    assert_eq!(log_len(&draft), logged, "a refused operation logs nothing");
    assert_eq!(claims_of(f, client, &draft).len(), 1);
}

// ---------------------------------------------------------------------------
// Expiry on time, and only the holder renews
// ---------------------------------------------------------------------------

/// Asserts that `events` is the one `claim.expired` of `taken`'s claim,
/// written on time for a claim that expires at `expires_at`.
fn assert_expired_on_time(events: &[(Value, f64)], taken: &Value, expires_at: f64) {
    assert_eq!(
        events.len(),
        1,
        "the API itself emits claim.expired, with no request after the claim's time is up: \
         {events:?}"
    );
    let (data, written) = &events[0];
    assert_eq!(data["claim_id"], taken["claim"]["id"], "{data}");
    assert_eq!(data["node"], taken["claim"]["node"], "{data}");
    assert!(
        data["expires_at"].is_string(),
        "claim.expired carries the expiry time: {data}"
    );
    assert!(
        (epoch(&data["expires_at"]) - expires_at).abs() < 0.01,
        "the event's expires_at is the claim's ({expires_at}): {data}"
    );
    assert!(
        *written >= expires_at - 0.05 && *written <= expires_at + 6.0,
        "claim.expired is written within a few seconds of the expiry ({expires_at}), \
         not at {written}"
    );
}

#[test]
fn a_claim_expires_on_time_without_any_request() {
    let (f, server) = quiet();
    let client = &server.client;
    let draft = new_draft(f, client);
    let (section, _, _) = add_section(f, client, &draft, "Pricing");
    let taken = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let expires_at = epoch(&taken["claim"]["expires_at"]);

    // No API call from here on.
    let events = wait_for_expiry_event(&draft.project, SHORT_TTL + Duration::from_secs(8));
    assert_expired_on_time(&events, &taken, expires_at);
    assert_eq!(claims_of(f, client, &draft), Vec::<Value>::new());
}

#[test]
fn operations_of_other_agents_in_the_draft_do_not_renew_a_claim() {
    let (f, server) = quiet();
    let client = &server.client;
    let draft = new_draft(f, client);
    let (_, _, outside_paragraph) = add_section(f, client, &draft, "Shipping");
    let (section, _, _) = add_section(f, client, &draft, "Pricing");
    let taken = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let expires_at = epoch(&taken["claim"]["expires_at"]);

    std::thread::sleep(Duration::from_millis(1000));
    must(
        edit(
            f,
            client,
            &other(f),
            &draft,
            &json!([{ "op": "insert", "parent": null, "position": { "after": null }, "content": "<p>Another agent's paragraph</p>" }]),
        ),
        200,
        "another agent inserts outside the claim",
    );
    std::thread::sleep(Duration::from_millis(1000));
    must(
        edit(
            f,
            client,
            &other(f),
            &draft,
            &json!([set_html(&outside_paragraph, 1, "Another agent's change")]),
        ),
        200,
        "another agent edits outside the claim",
    );
    let listed = claims_of(f, client, &draft);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(
        (epoch(&listed[0]["expires_at"]) - expires_at).abs() < 0.01,
        "another agent's operations do not move the expiry on: {listed:?} (taken {expires_at})"
    );

    // No API call from here on.
    let events = wait_for_expiry_event(&draft.project, SHORT_TTL + Duration::from_secs(8));
    assert_expired_on_time(&events, &taken, expires_at);
}

// ---------------------------------------------------------------------------
// An expired claim stays expired
// ---------------------------------------------------------------------------

#[test]
fn a_batch_that_commits_after_the_claim_expired_does_not_bring_it_back() {
    let (f, server) = quiet();
    let client = &server.client;
    let draft = new_draft(f, client);
    let (_, _, outside_paragraph) = add_section(f, client, &draft, "Shipping");
    let (section, _, paragraph) = add_section(f, client, &draft, "Pricing");
    let taken = must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let expires_at = epoch(&taken["claim"]["expires_at"]);

    // The holder's batch gets stuck writing its log while the claim is live,
    // and commits only once the claim has expired.
    let late = std::thread::scope(|scope| {
        let lock = RowLock::hold("drafts", &draft.draft);
        let batch = scope.spawn(|| {
            edit(
                f,
                client,
                &holder(f),
                &draft,
                &json!([set_html(&outside_paragraph, 1, "The holder's late change")]),
            )
        });
        let waiting = lock.wait_for_waiters(1, Duration::from_secs(2));
        assert_eq!(
            waiting.len(),
            1,
            "the holder's batch waits for the draft's row: {waiting:?}"
        );
        assert!(
            db_now() < expires_at,
            "the holder's batch started while its claim was live"
        );
        while db_now() < expires_at + 0.7 {
            std::thread::sleep(Duration::from_millis(100));
        }
        lock.let_go();
        batch.join().expect("the batch's thread")
    });
    let late = must(late, 200, "the holder's late batch applies");

    let response = claim(f, client, &other(f), &draft, &section);
    assert_eq!(
        response.status,
        201,
        "the claim expired before the holder's batch committed, so the batch did not \
         renew it and another agent takes the section: {}",
        response.text()
    );

    // Whatever the old holder does next does not restore its claim.
    let revision = late["results"][0]["revision"]
        .as_u64()
        .expect("the revision of the changed paragraph");
    must(
        edit(
            f,
            client,
            &holder(f),
            &draft,
            &json!([set_html(
                &outside_paragraph,
                revision,
                "The holder works on"
            )]),
        ),
        200,
        "the old holder edits outside the section",
    );
    let response = claim(f, client, &holder(f), &draft, &section);
    assert_eq!(response.status, 409, "{}", response.text());
    assert_names_holder(&response.json(), "run-2", "reviewer");
    let response = edit(
        f,
        client,
        &holder(f),
        &draft,
        &json!([set_html(&paragraph, 1, "The old holder's change")]),
    );
    assert_eq!(response.status, 409, "{}", response.text());
    assert_names_holder(&response.json(), "run-2", "reviewer");
    let listed = claims_of(f, client, &draft);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["agent"]["run"], "run-2", "{listed:?}");
    must(
        edit(
            f,
            client,
            &other(f),
            &draft,
            &json!([set_html(&paragraph, 1, "The new holder's change")]),
        ),
        200,
        "the new holder edits inside its claim",
    );
}

// ---------------------------------------------------------------------------
// Releasing holds the draft's lock
// ---------------------------------------------------------------------------

#[test]
fn an_operation_sent_during_a_release_waits_for_it_and_a_failed_release_keeps_the_claim() {
    let (f, client) = ready();
    let draft = new_draft(f, client);
    let (section, _, paragraph) = add_section(f, client, &draft, "Pricing");
    must(
        claim(f, client, &holder(f), &draft, &section),
        201,
        "claim the section",
    );
    let html = draft_html(f, client, &draft);
    let logged = log_len(&draft);

    // The release gets stuck writing its event; meanwhile another agent edits
    // inside the section; then the release's transaction fails.
    let (released, edited) = std::thread::scope(|scope| {
        let lock = RowLock::hold("projects", &draft.project);
        let releasing = scope.spawn(|| release(f, client, &holder(f), &draft, &section));
        let waiting = lock.wait_for_waiters(1, Duration::from_secs(5));
        assert_eq!(
            waiting.len(),
            1,
            "the release waits to write its event: {waiting:?}"
        );
        let release_backend = waiting[0];
        let editing = scope.spawn(|| {
            edit(
                f,
                client,
                &other(f),
                &draft,
                &json!([set_html(&paragraph, 1, "Changed during the release")]),
            )
        });
        // An edit that does not wait for the release gets as far as its own
        // event, and waits for the project's row too.
        lock.wait_for_waiters(2, Duration::from_millis(1500));
        terminate(release_backend);
        lock.let_go();
        (
            releasing.join().expect("the release's thread"),
            editing.join().expect("the edit's thread"),
        )
    });
    assert!(
        released.status >= 500,
        "the release whose transaction failed fails: {} {}",
        released.status,
        released.text()
    );
    let answer = assert_claimed_by_holder(&edited, &section);
    assert_eq!(answer["index"], 0, "{answer}");
    let listed = claims_of(f, client, &draft);
    assert_eq!(
        listed.len(),
        1,
        "the failed release keeps the claim: {listed:?}"
    );
    assert_eq!(listed[0]["agent"]["run"], "run-1", "{listed:?}");
    assert_eq!(draft_html(f, client, &draft), html);
    assert_eq!(log_len(&draft), logged);
    assert!(
        events_of(&draft.project, "claim.released").is_empty(),
        "a failed release emits nothing"
    );
}
