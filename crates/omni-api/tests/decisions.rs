//! US-037: decisions, through the API. Run with `--features stack-tests` in
//! the Compose test container, which reaches Postgres at `postgres:5432` and
//! RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. Several
//! agents share the key and tell themselves apart by their identity headers.
//! The CLI side is in crates/omni-cli/tests/decisions.rs and
//! stack_decisions.rs.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; threads, phases and posts
//! are those of US-033 to US-036, tests/threads.rs, tests/posts.rs,
//! tests/sealed_phases.rs and tests/thread_waits.rs, change sets and their
//! approval those of US-016 and US-025, tests/changesets.rs and
//! tests/approve_merge.rs):
//!
//! - `POST /threads/{thread_id}/decisions` (agent) proposes a decision for the
//!   thread: 201 `{"decision": Decision}`. Its body is the decision file of
//!   `omni decision propose`, whose schema `docs/formats/decision.md`
//!   documents:
//!   - `direction`: the chosen direction, a string that is not blank.
//!   - `chosen`: the `name` of the option that won.
//!   - `criteria`: the criteria the options were scored on, a list of at
//!     least one string, none blank and no two the same.
//!   - `options`: the options considered, a list of at least one
//!     `{"name", "scores", "lost_because"}`. Names are not blank and no two
//!     are the same. `scores` is an object with one number for each
//!     criterion and nothing else. `lost_because` says why the option lost: a
//!     string that is not blank for every option but the chosen one, and
//!     `null` (or left out) for the chosen one.
//!   - `assumptions` and `open_questions`: lists of strings.
//!   - `dissent`: a list of `{"by", "text"}`, both strings that are not
//!     blank: who disagrees and why.
//!   - `posts`: the posts the decision relies on, a list of IDs of posts of
//!     the same thread that the proposing agent identity may see (US-035).
//!     Another thread's post, an unknown post and a post hidden from the
//!     proposer are all refused alike.
//!
//!   Every field is required, and a field the schema does not have, at the top
//!   or in an option, is refused. A body that breaks any of this is 400
//!   `{"error": "invalid_request"}`, records nothing and emits nothing. The
//!   human does not propose (403 `forbidden`); an unknown thread is 404.
//! - A Decision is `{"id" (UUIDv7), "project_id", "thread_id", "changeset_id"
//!   (the thread's change set), "status", "author", "direction", "chosen",
//!   "criteria", "options", "assumptions", "open_questions", "dissent",
//!   "posts", "rejection_reason", "created_at"}`, with the proposed fields as
//!   they were sent. `author` is the proposer as for posts (`{"kind":
//!   "agent", "run", "role", "model", "harness"}`). `rejection_reason` is
//!   `null` until the decision is rejected, and kept after that.
//! - `GET /decisions/{decision_id}` (both) answers 200 `{"decision":
//!   Decision}` for every decision of the project, a superseded one included;
//!   an unknown one is 404 `not_found`.
//! - Statuses: a new decision is `proposed`. Only the human moves it on:
//!   `POST /decisions/{decision_id}/approve` (human) takes `{}` and makes it
//!   `approved`; `POST /decisions/{decision_id}/reject` (human) takes
//!   `{"reason"}`, a string that is not blank (400 `invalid_request`
//!   without one), and makes it `rejected`. Both answer 200 `{"decision":
//!   Decision}`. An API key is 403 `forbidden`, a decision that is not
//!   `proposed` is 409 `{"error": "invalid_transition"}`, and an unknown one
//!   is 404; none of these changes anything.
//! - A thread has at most one current decision: the newest one. A new
//!   proposal supersedes the thread's earlier decision, whatever its status
//!   (`proposed`, `approved` or `rejected`), which becomes `superseded`. Two
//!   proposals made at the same time still leave exactly one decision that is
//!   not superseded. `GET /threads/{thread_id}` answers the current Decision
//!   in the Thread's `decision` (`null` without one, as in US-036), the very
//!   object that `GET /decisions/{decision_id}` answers.
//! - What an agent reads of a decision keeps to the sealed phases (US-035):
//!   in the decision and thread reads, an ID in `posts` that names a post the
//!   reader may not see is left out, and the hidden post's ID appears nowhere
//!   in the answer. The human sees every ID. A decision proposed in a sealed
//!   phase is itself hidden from an agent that has not posted in that phase
//!   (US-056, tests/sealed_phases.rs): its read is 404 and the thread's
//!   `decision` is `null`.
//! - `POST /changesets/{changeset_id}/decisions/{decision_id}` (agent) links
//!   the change set to a decision of the project that it carries out (from
//!   any thread of the project): 200 `{"changeset": Changeset}`. Every
//!   Changeset body (show and the answers of every transition) then carries
//!   `"decisions": [{"id", "thread_id", "status"}, ...]`, the linked
//!   decisions in the order they were linked, each with its status now, and
//!   `[]` while the change set has none. Linking a decision again leaves it
//!   in the list once. An unknown decision, or one of another project, is
//!   404. The human does not link (403 `forbidden`).
//! - Approving a change set (US-025) while any linked decision is not
//!   `approved` is 409 `{"error": "decision_not_approved", "decision_id"}`,
//!   naming such a decision, and changes nothing: the change set stays
//!   `in_review` with no approval recorded and no event. Once every linked
//!   decision is approved, approving works as before.
//! - Proposing emits `decision.proposed`, a new proposal emits
//!   `decision.superseded` for the decision it supersedes, and approving and
//!   rejecting emit `decision.approved` and `decision.rejected`. Each event's
//!   `data` has at least `{"decision_id", "thread_id", "changeset_id",
//!   "status"}`. A refused call emits nothing.
//! - Storage: decisions live in the tenant table `decisions` (the API role
//!   keeps its UPDATE privilege there, since the status changes), whose
//!   columns without a default are `workspace_id`, `thread_id`, `content`
//!   (jsonb, the decision as proposed) and `author` (jsonb); its status and
//!   rejection reason are columns of it with a default or nullable. Links live
//!   in the append-only tenant table `changeset_decisions`, whose columns
//!   without a default are `workspace_id`, `changeset_id` and `decision_id`
//!   (crates/omni-core/tests/tenancy.rs lists it in `APPEND_ONLY_TABLES`).
//!   The story adds no other tenant table; tests/workspace_delete.rs and the
//!   isolation suite seed both.

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

const TEST_DB: &str = "omni_us037_decisions";
const TEST_BUCKET: &str = "omni-us037-decisions";
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
    format!("us037-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

fn agent_as<'a>(f: &'a Fixture, run: &'a str, role: &'a str) -> Auth<'a> {
    Auth::Agent {
        key: &f.key,
        run,
        role,
    }
}

/// The agent that starts threads and proposes decisions: `run-1/moderator`.
fn moderator(f: &Fixture) -> Auth<'_> {
    agent_as(f, "run-1", "moderator")
}

/// Committee agents that post in the threads: `run-1/critic`, `run-1/scout`.
fn critic(f: &Fixture) -> Auth<'_> {
    agent_as(f, "run-1", "critic")
}

fn scout(f: &Fixture) -> Auth<'_> {
    agent_as(f, "run-1", "scout")
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

fn thread_path(f: &Fixture, project: &str, thread: &str) -> String {
    format!("{}/threads/{thread}", project_base(f, project))
}

fn decision_path(f: &Fixture, project: &str, decision: &str) -> String {
    format!("{}/decisions/{decision}", project_base(f, project))
}

fn changeset_path(f: &Fixture, project: &str, changeset: &str) -> String {
    format!("{}/changesets/{changeset}", project_base(f, project))
}

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &moderator(f),
                Some(&json!({ "name": unique("decisions-project") })),
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
                &moderator(f),
                Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
            ),
            201,
            "create a change set",
        )["changeset"],
    )
}

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
                &moderator(f),
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

/// A project with a change set and an open thread: (project, change set,
/// thread).
fn thread_in_new_project(f: &Fixture, client: &Client) -> (String, String, String) {
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let thread = new_thread(f, client, &project, &changeset, false);
    (project, changeset, thread)
}

/// Posts a position as `auth`; returns the post's ID.
fn post(f: &Fixture, client: &Client, auth: &Auth, project: &str, thread: &str) -> String {
    let request = json!({
        "type": "position",
        "summary": unique("A position"),
        "body": unique("Why this position holds")
    });
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/posts", thread_path(f, project, thread)),
                auth,
                Some(&request),
            ),
            201,
            "post in the thread",
        )["post"],
    )
}

/// A decision file that keeps to the schema, relying on `posts`. Its
/// direction is unique, so a test can tell proposals apart.
fn proposal(posts: &[&str]) -> Value {
    json!({
        "direction": unique("Use the split hero: copy on the left, product shot on the right"),
        "chosen": "Split hero",
        "criteria": ["Readable at 320 px", "Fits above the fold"],
        "options": [
            {
                "name": "Split hero",
                "scores": { "Readable at 320 px": 8.5, "Fits above the fold": 9 },
                "lost_because": null
            },
            {
                "name": "Full-bleed photo",
                "scores": { "Readable at 320 px": 5, "Fits above the fold": 7 },
                "lost_because": "Text over the photo fails contrast at 320 px."
            },
            {
                "name": "Carousel",
                "scores": { "Readable at 320 px": 4, "Fits above the fold": 6 },
                "lost_because": "It hides two of the three messages."
            }
        ],
        "assumptions": ["Most visitors arrive on phones."],
        "open_questions": ["Is there a product shot for dark mode?"],
        "dissent": [{ "by": "run-1/scout", "text": "The carousel did better in last year's study." }],
        "posts": posts
    })
}

fn propose_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/decisions", thread_path(f, project, thread)),
        auth,
        Some(body),
    )
}

/// The moderator proposes `body`; returns the Decision of the answer.
fn propose(f: &Fixture, client: &Client, project: &str, thread: &str, body: &Value) -> Value {
    must(
        propose_response(f, client, &moderator(f), project, thread, body),
        201,
        "propose a decision",
    )["decision"]
        .clone()
}

fn act_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    decision: &str,
    action: &str,
    body: &Value,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/{action}", decision_path(f, project, decision)),
        auth,
        Some(body),
    )
}

/// The human approves the decision; returns the Decision of the answer.
fn approve_decision(f: &Fixture, client: &Client, project: &str, decision: &str) -> Value {
    must(
        act_response(
            f,
            client,
            &human(f),
            project,
            decision,
            "approve",
            &json!({}),
        ),
        200,
        "the human approves the decision",
    )["decision"]
        .clone()
}

/// The human rejects the decision for `reason`; returns the Decision of the
/// answer.
fn reject_decision(
    f: &Fixture,
    client: &Client,
    project: &str,
    decision: &str,
    reason: &str,
) -> Value {
    must(
        act_response(
            f,
            client,
            &human(f),
            project,
            decision,
            "reject",
            &json!({ "reason": reason }),
        ),
        200,
        "the human rejects the decision",
    )["decision"]
        .clone()
}

fn decision_of(f: &Fixture, client: &Client, auth: &Auth, project: &str, decision: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &decision_path(f, project, decision),
            auth,
            None,
        ),
        200,
        "read the decision",
    )["decision"]
        .clone()
}

fn status_of(f: &Fixture, client: &Client, project: &str, decision: &str) -> String {
    decision_of(f, client, &moderator(f), project, decision)["status"]
        .as_str()
        .unwrap_or_else(|| panic!("the decision {decision} has a status"))
        .to_string()
}

/// The Thread's `decision` as `auth` reads it.
fn current_decision(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
) -> Value {
    let answer = must(
        call(client, "GET", &thread_path(f, project, thread), auth, None),
        200,
        "read the thread",
    );
    answer["thread"]
        .get("decision")
        .unwrap_or_else(|| panic!("the thread has `decision`: {answer}"))
        .clone()
}

fn link_response(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    changeset: &str,
    decision: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!(
            "{}/decisions/{decision}",
            changeset_path(f, project, changeset)
        ),
        auth,
        None,
    )
}

/// The moderator links the change set to the decision; returns the
/// Changeset of the answer.
fn link(f: &Fixture, client: &Client, project: &str, changeset: &str, decision: &str) -> Value {
    must(
        link_response(f, client, &moderator(f), project, changeset, decision),
        200,
        "link the change set to the decision",
    )["changeset"]
        .clone()
}

fn changeset_of(f: &Fixture, client: &Client, project: &str, changeset: &str) -> Value {
    must(
        call(
            client,
            "GET",
            &changeset_path(f, project, changeset),
            &moderator(f),
            None,
        ),
        200,
        "show the change set",
    )["changeset"]
        .clone()
}

/// The linked decisions of a Changeset as (id, thread_id, status).
fn linked(changeset: &Value) -> Vec<(String, String, String)> {
    changeset["decisions"]
        .as_array()
        .unwrap_or_else(|| panic!("a change set carries `decisions`: {changeset}"))
        .iter()
        .map(|entry| {
            let field = |name: &str| {
                entry[name]
                    .as_str()
                    .unwrap_or_else(|| panic!("a linked decision has `{name}`: {entry}"))
                    .to_string()
            };
            (field("id"), field("thread_id"), field("status"))
        })
        .collect()
}

/// A change set of `project` with one saved version of a new doc, submitted
/// and so `in_review`.
fn changeset_in_review(f: &Fixture, client: &Client, project: &str) -> String {
    let changeset = new_changeset(f, client, project);
    let base = changeset_path(f, project, &changeset);
    let doc = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/docs"),
                &moderator(f),
                Some(&json!({ "kind": "html", "pillar": "spec", "title": unique("Hero PRD") })),
            ),
            201,
            "create a doc",
        )["doc"],
    );
    must(
        call(
            client,
            "POST",
            &format!("{base}/docs/{doc}/ops"),
            &moderator(f),
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
    must(
        call(
            client,
            "POST",
            &format!("{base}/docs/{doc}/versions"),
            &moderator(f),
            Some(&json!({})),
        ),
        201,
        "save a version",
    );
    let submitted = must(
        call(
            client,
            "POST",
            &format!("{base}/submit"),
            &moderator(f),
            None,
        ),
        200,
        "submit the change set",
    );
    assert_eq!(submitted["changeset"]["status"], "in_review", "{submitted}");
    changeset
}

fn approve_changeset_response(
    f: &Fixture,
    client: &Client,
    project: &str,
    changeset: &str,
) -> Response {
    call(
        client,
        "POST",
        &format!("{}/approve", changeset_path(f, project, changeset)),
        &human(f),
        Some(&json!({})),
    )
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

/// Every live event of a project whose name starts with `decision.`, as
/// (event, data).
fn decision_events(project: &str) -> Vec<(String, Value)> {
    block(async {
        let mut conn = admin_connection().await;
        let rows = sqlx::query_as::<_, (String, String)>(
            "select event, data::jsonb::text from live_events \
             where project_id = $1::uuid and event like 'decision.%' order by seq",
        )
        .bind(project)
        .fetch_all(&mut conn)
        .await
        .unwrap_or_else(|e| panic!("read the live events: {e}"));
        rows.into_iter()
            .map(|(event, data)| (event, parse(&data)))
            .collect()
    })
}

/// The decisions the thread has in the table `decisions`, as the admin sees
/// them.
fn stored_decisions(thread: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        let exists: bool = sqlx::query_scalar("select to_regclass('public.decisions') is not null")
            .fetch_one(&mut conn)
            .await
            .expect("look for the decisions table");
        if !exists {
            return 0;
        }
        sqlx::query_scalar("select count(*) from decisions where thread_id = $1::uuid")
            .bind(thread)
            .fetch_one(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("count the decisions: {e}"))
    })
}

// ---------------------------------------------------------------------------
// Proposing
// ---------------------------------------------------------------------------

#[test]
fn proposing_records_the_decision_with_every_field_of_the_schema() {
    let (f, client) = ready();
    let (project, changeset, thread) = thread_in_new_project(f, client);
    let first = post(f, client, &critic(f), &project, &thread);
    let second = post(f, client, &scout(f), &project, &thread);
    let body = proposal(&[&first, &second]);

    let decision = propose(f, client, &project, &thread, &body);
    let id = id_of(&decision);
    assert_eq!(
        id.chars().nth(14),
        Some('7'),
        "the decision's ID is a UUIDv7: {decision}"
    );
    for field in [
        "direction",
        "chosen",
        "criteria",
        "options",
        "assumptions",
        "open_questions",
        "dissent",
        "posts",
    ] {
        assert_eq!(
            decision[field], body[field],
            "the decision keeps `{field}` as proposed: {decision}"
        );
    }
    assert_eq!(decision["status"], "proposed", "{decision}");
    assert_eq!(decision["rejection_reason"], Value::Null, "{decision}");
    assert_eq!(decision["project_id"], project.as_str(), "{decision}");
    assert_eq!(decision["thread_id"], thread.as_str(), "{decision}");
    assert_eq!(decision["changeset_id"], changeset.as_str(), "{decision}");
    assert!(decision["created_at"].is_string(), "{decision}");
    let author = &decision["author"];
    assert_eq!(author["kind"], "agent", "{decision}");
    assert_eq!(author["run"], "run-1", "{decision}");
    assert_eq!(author["role"], "moderator", "{decision}");

    for (auth, reader) in [(moderator(f), "an agent"), (human(f), "the human")] {
        let read = decision_of(f, client, &auth, &project, &id);
        assert_eq!(read, decision, "{reader} reads the decision as proposed");
        assert_eq!(
            current_decision(f, client, &auth, &project, &thread),
            decision,
            "{reader}: the thread's decision is the one proposed"
        );
    }
}

#[test]
fn a_proposal_that_breaks_the_schema_is_refused_and_records_nothing() {
    let (f, client) = ready();
    let (project, _, thread) = thread_in_new_project(f, client);
    let mine = post(f, client, &moderator(f), &project, &thread);
    let valid = proposal(&[&mine]);

    type Mutation = fn(&mut Value);
    let broken: Vec<(&str, Mutation)> = vec![
        ("no direction", |d| {
            d.as_object_mut().unwrap().remove("direction");
        }),
        ("a blank direction", |d| d["direction"] = json!("   ")),
        ("a direction that is no string", |d| {
            d["direction"] = json!(7)
        }),
        ("no chosen option", |d| {
            d.as_object_mut().unwrap().remove("chosen");
        }),
        ("a chosen option that is not among the options", |d| {
            d["chosen"] = json!("Video hero")
        }),
        ("no criteria", |d| d["criteria"] = json!([])),
        ("a blank criterion", |d| {
            d["criteria"] = json!(["Readable at 320 px", " "])
        }),
        ("no options", |d| d["options"] = json!([])),
        ("two options with one name", |d| {
            d["options"][2]["name"] = json!("Full-bleed photo")
        }),
        ("a losing option without why it lost", |d| {
            d["options"][1]["lost_because"] = Value::Null
        }),
        ("a losing option with a blank reason", |d| {
            d["options"][2]["lost_because"] = json!("")
        }),
        ("a chosen option that says why it lost", |d| {
            d["options"][0]["lost_because"] = json!("It did not lose.")
        }),
        ("an option without the score of a criterion", |d| {
            d["options"][1]["scores"]
                .as_object_mut()
                .unwrap()
                .remove("Fits above the fold");
        }),
        ("a score that is no number", |d| {
            d["options"][1]["scores"]["Fits above the fold"] = json!("high")
        }),
        ("a score for a criterion that is not listed", |d| {
            d["options"][0]["scores"]["Loads fast"] = json!(3)
        }),
        ("assumptions that are no list of strings", |d| {
            d["assumptions"] = json!("Most visitors arrive on phones.")
        }),
        ("no open questions", |d| {
            d.as_object_mut().unwrap().remove("open_questions");
        }),
        ("dissent without who disagrees", |d| {
            d["dissent"] = json!([{ "text": "The carousel did better." }])
        }),
        ("dissent with a blank text", |d| {
            d["dissent"] = json!([{ "by": "run-1/scout", "text": "" }])
        }),
        ("posts that are no list", |d| {
            d["posts"] = json!("all of them")
        }),
        ("a field the schema does not have", |d| {
            d["verdict"] = json!("ship it")
        }),
        ("an option field the schema does not have", |d| {
            d["options"][0]["colour"] = json!("blue")
        }),
    ];
    for (what, mutate) in broken {
        let mut body = valid.clone();
        mutate(&mut body);
        let response = propose_response(f, client, &moderator(f), &project, &thread, &body);
        assert_error(&response, 400, "invalid_request", what);
    }
    let response = call(
        client,
        "POST",
        &format!("{}/decisions", thread_path(f, &project, &thread)),
        &moderator(f),
        Some(&json!(["not", "an", "object"])),
    );
    assert_error(
        &response,
        400,
        "invalid_request",
        "a body that is no object",
    );

    assert_eq!(
        current_decision(f, client, &moderator(f), &project, &thread),
        Value::Null,
        "a refused proposal records no decision"
    );
    assert_eq!(stored_decisions(&thread), 0, "nothing is stored");
    assert!(
        events_of(&project, "decision.proposed").is_empty(),
        "a refused proposal emits nothing"
    );
    // The valid file itself is taken, so the refusals above were the rules.
    propose(f, client, &project, &thread, &valid);
}

#[test]
fn a_proposal_relies_only_on_posts_of_its_thread_that_the_proposer_may_see() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let thread = new_thread(f, client, &project, &changeset, true);
    let other_thread = new_thread(f, client, &project, &changeset, false);
    let elsewhere = post(f, client, &critic(f), &project, &other_thread);
    // The scout's post in the sealed phase is hidden from the moderator, who
    // has not posted in it.
    let hidden = post(f, client, &scout(f), &project, &thread);

    for (what, posts) in [
        ("a post of another thread", vec![elsewhere.as_str()]),
        ("a post that does not exist", vec![UNKNOWN_ID]),
        ("an ID that is no UUID", vec!["not-a-post"]),
        ("a post hidden from the proposer", vec![hidden.as_str()]),
    ] {
        let response = propose_response(
            f,
            client,
            &moderator(f),
            &project,
            &thread,
            &proposal(&posts),
        );
        assert_error(&response, 400, "invalid_request", what);
    }
    assert_eq!(stored_decisions(&thread), 0, "nothing is stored");

    // Once the moderator has posted in the sealed phase it sees the scout's
    // post, and may rely on it and on its own.
    let own = post(f, client, &moderator(f), &project, &thread);
    let decision = propose(f, client, &project, &thread, &proposal(&[&own, &hidden]));
    assert_eq!(decision["posts"], json!([own, hidden]), "{decision}");
}

#[test]
fn the_human_does_not_propose_and_an_unknown_thread_is_not_found() {
    let (f, client) = ready();
    let (project, _, thread) = thread_in_new_project(f, client);
    let response = propose_response(f, client, &human(f), &project, &thread, &proposal(&[]));
    assert_error(&response, 403, "forbidden", "the human proposes");
    let response = propose_response(
        f,
        client,
        &moderator(f),
        &project,
        UNKNOWN_ID,
        &proposal(&[]),
    );
    assert_error(&response, 404, "not_found", "a proposal for no thread");
    assert_eq!(stored_decisions(&thread), 0, "nothing is stored");
    assert_eq!(
        current_decision(f, client, &moderator(f), &project, &thread),
        Value::Null
    );
}

// ---------------------------------------------------------------------------
// Statuses
// ---------------------------------------------------------------------------

#[test]
fn only_the_human_rejects_and_a_rejection_needs_a_reason_that_is_kept() {
    let (f, client) = ready();
    let (project, _, thread) = thread_in_new_project(f, client);
    let id = id_of(&propose(f, client, &project, &thread, &proposal(&[])));

    let response = act_response(
        f,
        client,
        &moderator(f),
        &project,
        &id,
        "reject",
        &json!({ "reason": "An agent may not reject." }),
    );
    assert_error(&response, 403, "forbidden", "an agent rejects");
    for (what, body) in [
        ("without a reason", json!({})),
        ("with a blank reason", json!({ "reason": "  " })),
        ("with a reason that is no string", json!({ "reason": 42 })),
    ] {
        let response = act_response(f, client, &human(f), &project, &id, "reject", &body);
        assert_error(&response, 400, "invalid_request", what);
    }
    assert_eq!(status_of(f, client, &project, &id), "proposed");

    let reason = unique("The fold is lower on the phones we sell to");
    let rejected = reject_decision(f, client, &project, &id, &reason);
    assert_eq!(rejected["status"], "rejected", "{rejected}");
    assert_eq!(rejected["rejection_reason"], reason.as_str(), "{rejected}");
    for (auth, reader) in [(moderator(f), "an agent"), (human(f), "the human")] {
        let read = decision_of(f, client, &auth, &project, &id);
        assert_eq!(read["status"], "rejected", "{reader}: {read}");
        assert_eq!(
            read["rejection_reason"],
            reason.as_str(),
            "{reader}: {read}"
        );
        let current = current_decision(f, client, &auth, &project, &thread);
        assert_eq!(
            current["rejection_reason"],
            reason.as_str(),
            "{reader}: {current}"
        );
    }
}

#[test]
fn a_decision_is_approved_or_rejected_once() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let changeset = new_changeset(f, client, &project);
    let approved_thread = new_thread(f, client, &project, &changeset, false);
    let rejected_thread = new_thread(f, client, &project, &changeset, false);
    let approved = id_of(&propose(
        f,
        client,
        &project,
        &approved_thread,
        &proposal(&[]),
    ));
    let rejected = id_of(&propose(
        f,
        client,
        &project,
        &rejected_thread,
        &proposal(&[]),
    ));
    approve_decision(f, client, &project, &approved);
    reject_decision(f, client, &project, &rejected, "Not this one.");

    let again = [
        (&approved, "approve", json!({}), "approved"),
        (
            &approved,
            "reject",
            json!({ "reason": "Too late." }),
            "approved",
        ),
        (&rejected, "approve", json!({}), "rejected"),
        (
            &rejected,
            "reject",
            json!({ "reason": "Twice." }),
            "rejected",
        ),
    ];
    for (id, action, body, status) in again {
        let response = act_response(f, client, &human(f), &project, id, action, &body);
        assert_error(
            &response,
            409,
            "invalid_transition",
            &format!("{action} a decision that is {status}"),
        );
        let read = decision_of(f, client, &human(f), &project, id);
        assert_eq!(read["status"], status, "{read}");
    }
    let read = decision_of(f, client, &human(f), &project, &rejected);
    assert_eq!(read["rejection_reason"], "Not this one.", "{read}");

    for action in ["approve", "reject"] {
        let response = act_response(
            f,
            client,
            &human(f),
            &project,
            UNKNOWN_ID,
            action,
            &json!({ "reason": "No such decision." }),
        );
        assert_error(
            &response,
            404,
            "not_found",
            &format!("{action} no decision"),
        );
    }
    let response = call(
        client,
        "GET",
        &decision_path(f, &project, UNKNOWN_ID),
        &moderator(f),
        None,
    );
    assert_error(&response, 404, "not_found", "read no decision");
}

// ---------------------------------------------------------------------------
// Superseding
// ---------------------------------------------------------------------------

#[test]
fn a_new_proposal_supersedes_a_proposal_still_proposed() {
    let (f, client) = ready();
    let (project, _, thread) = thread_in_new_project(f, client);
    let first = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    let second = propose(f, client, &project, &thread, &proposal(&[]));
    let second_id = id_of(&second);
    assert_ne!(first, second_id);

    assert_eq!(status_of(f, client, &project, &first), "superseded");
    assert_eq!(second["status"], "proposed", "{second}");
    let current = current_decision(f, client, &human(f), &project, &thread);
    assert_eq!(
        current["id"],
        second_id.as_str(),
        "the newest proposal is the thread's one current decision: {current}"
    );

    for action in ["approve", "reject"] {
        let response = act_response(
            f,
            client,
            &human(f),
            &project,
            &first,
            action,
            &json!({ "reason": "Too late." }),
        );
        assert_error(
            &response,
            409,
            "invalid_transition",
            &format!("{action} a superseded decision"),
        );
    }
    assert_eq!(status_of(f, client, &project, &first), "superseded");
    assert_eq!(status_of(f, client, &project, &second_id), "proposed");
}

#[test]
fn a_new_proposal_supersedes_a_rejected_or_approved_decision_and_one_stays_current() {
    let (f, client) = ready();
    let (project, _, thread) = thread_in_new_project(f, client);
    let rejected = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    reject_decision(f, client, &project, &rejected, "Try the split hero.");

    let approved = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    let read = decision_of(f, client, &human(f), &project, &rejected);
    assert_eq!(read["status"], "superseded", "{read}");
    assert_eq!(
        read["rejection_reason"], "Try the split hero.",
        "the reason of a rejection is kept: {read}"
    );
    approve_decision(f, client, &project, &approved);

    let latest = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    assert_eq!(status_of(f, client, &project, &approved), "superseded");
    let statuses: Vec<String> = [&rejected, &approved, &latest]
        .into_iter()
        .map(|id| status_of(f, client, &project, id))
        .collect();
    assert_eq!(
        statuses,
        ["superseded", "superseded", "proposed"],
        "every decision but the newest is superseded"
    );
    assert_eq!(
        current_decision(f, client, &moderator(f), &project, &thread)["id"],
        latest.as_str()
    );
}

#[test]
fn two_proposals_at_once_leave_one_current_decision() {
    let (f, client) = ready();
    let (project, _, thread) = thread_in_new_project(f, client);
    let earlier = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    let made: Vec<String> = std::thread::scope(|scope| {
        let runs: Vec<_> = (0..4)
            .map(|_| {
                let (project, thread) = (&project, &thread);
                scope.spawn(move || id_of(&propose(f, client, project, thread, &proposal(&[]))))
            })
            .collect();
        runs.into_iter()
            .map(|run| run.join().expect("a proposal"))
            .collect()
    });
    let all: Vec<&String> = std::iter::once(&earlier).chain(made.iter()).collect();
    let live: Vec<&&String> = all
        .iter()
        .filter(|id| status_of(f, client, &project, id) != "superseded")
        .collect();
    assert_eq!(
        live.len(),
        1,
        "exactly one of the thread's decisions is not superseded: {all:?}"
    );
    assert_eq!(
        current_decision(f, client, &moderator(f), &project, &thread)["id"],
        live[0].as_str(),
        "the thread's decision is that one"
    );
}

// ---------------------------------------------------------------------------
// Linking change sets
// ---------------------------------------------------------------------------

#[test]
fn linking_a_decision_lists_it_on_the_change_set_with_its_status() {
    let (f, client) = ready();
    let (project, thread_changeset, thread) = thread_in_new_project(f, client);
    let decision = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    // The change set that carries the decision out is another one.
    let changeset = new_changeset(f, client, &project);
    assert_eq!(
        linked(&changeset_of(f, client, &project, &changeset)),
        Vec::<(String, String, String)>::new(),
        "a change set without links has `decisions: []`"
    );

    let answer = link(f, client, &project, &changeset, &decision);
    let expected = vec![(decision.clone(), thread.clone(), "proposed".to_string())];
    assert_eq!(linked(&answer), expected, "{answer}");
    assert_eq!(answer["id"], changeset.as_str(), "{answer}");
    assert_eq!(
        linked(&changeset_of(f, client, &project, &changeset)),
        expected
    );

    let again = link(f, client, &project, &changeset, &decision);
    assert_eq!(linked(&again), expected, "a decision is linked once");

    approve_decision(f, client, &project, &decision);
    assert_eq!(
        linked(&changeset_of(f, client, &project, &changeset)),
        vec![(decision.clone(), thread.clone(), "approved".to_string())],
        "a linked decision shows its status now"
    );

    // A second decision is listed after the first.
    let other_thread = new_thread(f, client, &project, &thread_changeset, false);
    let second = id_of(&propose(f, client, &project, &other_thread, &proposal(&[])));
    let answer = link(f, client, &project, &changeset, &second);
    assert_eq!(
        linked(&answer),
        vec![
            (decision, thread, "approved".to_string()),
            (second, other_thread, "proposed".to_string()),
        ],
        "{answer}"
    );
}

#[test]
fn linking_needs_a_decision_of_the_project_and_an_agent() {
    let (f, client) = ready();
    let (project, _, thread) = thread_in_new_project(f, client);
    let decision = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    let changeset = new_changeset(f, client, &project);
    let (elsewhere, _, other_thread) = thread_in_new_project(f, client);
    let foreign = id_of(&propose(
        f,
        client,
        &elsewhere,
        &other_thread,
        &proposal(&[]),
    ));

    let response = link_response(f, client, &moderator(f), &project, &changeset, UNKNOWN_ID);
    assert_error(&response, 404, "not_found", "link no decision");
    let response = link_response(f, client, &moderator(f), &project, &changeset, &foreign);
    assert_error(
        &response,
        404,
        "not_found",
        "link another project's decision",
    );
    let response = link_response(f, client, &moderator(f), &project, UNKNOWN_ID, &decision);
    assert_error(&response, 404, "not_found", "link no change set");
    let response = link_response(f, client, &human(f), &project, &changeset, &decision);
    assert_error(&response, 403, "forbidden", "the human links");

    assert_eq!(
        linked(&changeset_of(f, client, &project, &changeset)),
        Vec::<(String, String, String)>::new(),
        "the refused links linked nothing"
    );
}

// ---------------------------------------------------------------------------
// Approving change sets (Q5)
// ---------------------------------------------------------------------------

#[test]
fn approving_a_change_set_whose_linked_decision_is_proposed_is_409_naming_it() {
    let (f, client) = ready();
    let (project, _, thread) = thread_in_new_project(f, client);
    let decision = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    let changeset = changeset_in_review(f, client, &project);
    link(f, client, &project, &changeset, &decision);

    let response = approve_changeset_response(f, client, &project, &changeset);
    assert_eq!(response.status, 409, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "decision_not_approved", "decision_id": decision }),
        "the answer names the decision"
    );
    let shown = changeset_of(f, client, &project, &changeset);
    assert_eq!(shown["status"], "in_review", "nothing changed: {shown}");
    assert_eq!(shown["approved_versions"], json!([]), "{shown}");
    assert!(
        events_of(&project, "changeset.approved")
            .iter()
            .all(|data| data["changeset_id"] != changeset.as_str()),
        "a refused approval emits nothing"
    );
}

#[test]
fn approving_a_change_set_names_a_rejected_linked_decision_next_to_an_approved_one() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let thread_changeset = new_changeset(f, client, &project);
    let first_thread = new_thread(f, client, &project, &thread_changeset, false);
    let second_thread = new_thread(f, client, &project, &thread_changeset, false);
    let approved = id_of(&propose(f, client, &project, &first_thread, &proposal(&[])));
    let rejected = id_of(&propose(
        f,
        client,
        &project,
        &second_thread,
        &proposal(&[]),
    ));
    approve_decision(f, client, &project, &approved);
    reject_decision(f, client, &project, &rejected, "Not yet.");
    let changeset = changeset_in_review(f, client, &project);
    link(f, client, &project, &changeset, &approved);
    link(f, client, &project, &changeset, &rejected);

    let response = approve_changeset_response(f, client, &project, &changeset);
    assert_eq!(response.status, 409, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({ "error": "decision_not_approved", "decision_id": rejected }),
        "the answer names the decision that is not approved"
    );
    assert_eq!(
        changeset_of(f, client, &project, &changeset)["status"],
        "in_review"
    );
}

#[test]
fn once_every_linked_decision_is_approved_the_change_set_is_approved_and_merges() {
    let (f, client) = ready();
    let (project, _, thread) = thread_in_new_project(f, client);
    let decision = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    let changeset = changeset_in_review(f, client, &project);
    link(f, client, &project, &changeset, &decision);
    let response = approve_changeset_response(f, client, &project, &changeset);
    assert_eq!(response.status, 409, "{}", response.text());

    approve_decision(f, client, &project, &decision);
    let approved = must(
        approve_changeset_response(f, client, &project, &changeset),
        200,
        "approve the change set",
    );
    assert_eq!(approved["changeset"]["status"], "merged", "{approved}");
    assert_eq!(
        linked(&approved["changeset"]),
        vec![(decision, thread, "approved".to_string())],
        "{approved}"
    );
}

// ---------------------------------------------------------------------------
// Live events
// ---------------------------------------------------------------------------

fn assert_event_data(data: &Value, decision: &str, thread: &str, changeset: &str, status: &str) {
    assert_eq!(data["decision_id"], decision, "{data}");
    assert_eq!(data["thread_id"], thread, "{data}");
    assert_eq!(data["changeset_id"], changeset, "{data}");
    assert_eq!(data["status"], status, "{data}");
}

#[test]
fn proposing_superseding_approving_and_rejecting_emit_their_events() {
    let (f, client) = ready();
    let (project, changeset, thread) = thread_in_new_project(f, client);
    let first = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    let second = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    reject_decision(f, client, &project, &second, "Not this one either.");
    let third = id_of(&propose(f, client, &project, &thread, &proposal(&[])));
    approve_decision(f, client, &project, &third);

    let events = decision_events(&project);
    let of = |name: &str, decision: &str| -> Vec<Value> {
        events
            .iter()
            .filter(|(event, data)| event == name && data["decision_id"] == decision)
            .map(|(_, data)| data.clone())
            .collect()
    };
    for (name, decision, status) in [
        ("decision.proposed", &first, "proposed"),
        ("decision.proposed", &second, "proposed"),
        ("decision.proposed", &third, "proposed"),
        ("decision.superseded", &first, "superseded"),
        ("decision.rejected", &second, "rejected"),
        ("decision.superseded", &second, "superseded"),
        ("decision.approved", &third, "approved"),
    ] {
        let found = of(name, decision);
        assert_eq!(found.len(), 1, "one {name} for {decision}: {events:?}");
        assert_event_data(&found[0], decision, &thread, &changeset, status);
    }
    assert!(
        of("decision.superseded", &third).is_empty(),
        "the current decision is not superseded: {events:?}"
    );
    assert_eq!(events.len(), 7, "no other decision events: {events:?}");
}
