//! US-036: what the API tells about a thread so that `omni wait --thread` can
//! wait for others: who has posted in each phase, and the thread's decision.
//! Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port. Several
//! agents share the key and tell themselves apart by their identity headers.
//! The CLI side is in crates/omni-cli/tests/thread_waits.rs and digests.rs.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; threads, phases and posts
//! are those of US-033 to US-035, tests/threads.rs, tests/posts.rs and
//! tests/sealed_phases.rs):
//!
//! - `GET /threads/{thread_id}` (both) answers `{"thread": Thread}`, where
//!   each entry of the Thread's `phases` also has `posted_by`: the distinct
//!   agent identities that have posted in that phase, each `{"run", "role"}`,
//!   in the order of their first post there. Two runs with the same role are
//!   two identities; an identity that posted twice is there once; the human
//!   is never in it. Nothing is cached: the next read after a post shows it.
//! - `posted_by` is told to every caller, also in a sealed phase where the
//!   caller may not read the posts yet (US-035 leaves this open): it says who
//!   has posted, never what. The answer still holds nothing of a hidden post:
//!   not its ID, its summary or its body.
//! - The Thread has `decision`: `null` while the thread has no decision.
//!   US-037 makes it the thread's current decision, with at least `id` and
//!   `status`, which `omni wait --until decision:approved|rejected` reads.
//! - The story adds no route and no tenant table.

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

const TEST_DB: &str = "omni_us036_thread_waits";
const TEST_BUCKET: &str = "omni-us036-thread-waits";

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
    format!("us036-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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

/// The agent that starts threads and phases: `run-1/moderator`.
fn moderator(f: &Fixture) -> Auth<'_> {
    agent_as(f, "run-1", "moderator")
}

/// The agent whose posts the others must not see too early: `run-1/critic`.
fn critic(f: &Fixture) -> Auth<'_> {
    agent_as(f, "run-1", "critic")
}

/// The agent that reads: `run-1/scout`.
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

fn new_project(f: &Fixture, client: &Client) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("/api/v1/w/{}/projects", f.workspace),
                &moderator(f),
                Some(&json!({ "name": unique("waits-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

/// A project with a change set and a thread whose first phase is sealed or
/// open: (project, thread).
fn thread_in_new_project(f: &Fixture, client: &Client, sealed: bool) -> (String, String) {
    let project = new_project(f, client);
    let changeset = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/changesets", project_base(f, &project)),
                &moderator(f),
                Some(&json!({ "title": unique("change"), "run_tag": "run-1" })),
            ),
            201,
            "create a change set",
        )["changeset"],
    );
    let thread = id_of(
        &must(
            call(
                client,
                "POST",
                &format!(
                    "{}/changesets/{changeset}/threads",
                    project_base(f, &project)
                ),
                &moderator(f),
                Some(&json!({
                    "question": unique("Which hero layout?"),
                    "criteria": ["Readable at 320 px"],
                    "sealed": sealed
                })),
            ),
            201,
            "start a thread",
        )["thread"],
    );
    (project, thread)
}

fn start_phase(f: &Fixture, client: &Client, project: &str, thread: &str, sealed: bool) {
    must(
        call(
            client,
            "POST",
            &format!("{}/phases", thread_path(f, project, thread)),
            &moderator(f),
            Some(&json!({ "sealed": sealed })),
        ),
        201,
        "start the next phase",
    );
}

/// What a test knows of a post it made: its ID and the texts that must not
/// show to an agent that may not see it.
#[derive(Clone)]
struct Made {
    id: String,
    summary: String,
    body: String,
}

/// Posts a position as `auth` with a unique summary and body.
fn post(
    f: &Fixture,
    client: &Client,
    auth: &Auth,
    project: &str,
    thread: &str,
    label: &str,
) -> Made {
    let summary = unique(&format!("{label} summary"));
    let body = unique(&format!("{label} body"));
    let request = json!({ "type": "position", "summary": summary, "body": body });
    let posted = must(
        call(
            client,
            "POST",
            &format!("{}/posts", thread_path(f, project, thread)),
            auth,
            Some(&request),
        ),
        201,
        &format!("post {request}"),
    );
    Made {
        id: id_of(&posted["post"]),
        summary,
        body,
    }
}

fn read_thread(f: &Fixture, client: &Client, auth: &Auth, project: &str, thread: &str) -> Response {
    call(client, "GET", &thread_path(f, project, thread), auth, None)
}

/// The posts of a thread read, as the caller sees them.
fn thread_posts(answer: &Value) -> Vec<Value> {
    answer["thread"]["posts"]
        .as_array()
        .unwrap_or_else(|| panic!("the thread has a list of `posts`: {answer}"))
        .clone()
}

/// Neither the ID, the summary nor the body of `hidden` is anywhere in `text`.
fn assert_nothing_of(text: &str, hidden: &Made, what: &str) {
    for (name, marker) in [
        ("ID", &hidden.id),
        ("summary", &hidden.summary),
        ("body", &hidden.body),
    ] {
        assert!(
            !text.contains(marker.as_str()),
            "{what}: the {name} of a hidden post shows: {text}"
        );
    }
}

/// The identities that phase `number` says have posted in it, as
/// (run, role).
fn posted_by(answer: &Value, number: u64) -> Vec<(String, String)> {
    let phases = answer["thread"]["phases"]
        .as_array()
        .unwrap_or_else(|| panic!("the thread has `phases`: {answer}"));
    let phase = phases
        .iter()
        .find(|p| p["number"] == number)
        .unwrap_or_else(|| panic!("the thread has phase {number}: {answer}"));
    phase["posted_by"]
        .as_array()
        .unwrap_or_else(|| panic!("phase {number} has a list `posted_by`: {phase}"))
        .iter()
        .map(|who| {
            (
                who["run"].as_str().unwrap_or("").to_string(),
                who["role"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

fn who(run: &str, role: &str) -> (String, String) {
    (run.to_string(), role.to_string())
}

fn read(f: &Fixture, client: &Client, auth: &Auth, project: &str, thread: &str) -> Value {
    must(
        read_thread(f, client, auth, project, thread),
        200,
        "read the thread",
    )
}

// ---------------------------------------------------------------------------
// posted_by
// ---------------------------------------------------------------------------

#[test]
fn each_phase_tells_which_agent_identities_have_posted_in_it() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, false);
    let other_critic = agent_as(f, "run-2", "critic");
    post(f, client, &critic(f), &project, &thread, "critic");
    post(f, client, &scout(f), &project, &thread, "scout");
    post(f, client, &critic(f), &project, &thread, "critic again");
    post(f, client, &human(f), &project, &thread, "human");
    post(f, client, &other_critic, &project, &thread, "other critic");

    let expected = vec![
        who("run-1", "critic"),
        who("run-1", "scout"),
        who("run-2", "critic"),
    ];
    for (auth, reader) in [
        (moderator(f), "the moderator"),
        (critic(f), "a critic"),
        (human(f), "the human"),
    ] {
        let answer = read(f, client, &auth, &project, &thread);
        assert_eq!(
            posted_by(&answer, 1),
            expected,
            "{reader}: every identity once, in the order of its first post, \
             and never the human: {answer}"
        );
    }
}

#[test]
fn posted_by_belongs_to_the_phase_the_posts_were_made_in() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, false);
    post(f, client, &critic(f), &project, &thread, "phase 1");
    post(f, client, &scout(f), &project, &thread, "phase 1");
    start_phase(f, client, &project, &thread, true);

    let answer = read(f, client, &moderator(f), &project, &thread);
    assert_eq!(
        posted_by(&answer, 1),
        vec![who("run-1", "critic"), who("run-1", "scout")],
        "{answer}"
    );
    assert_eq!(
        posted_by(&answer, 2),
        Vec::<(String, String)>::new(),
        "nobody has posted in the new phase yet: {answer}"
    );

    post(f, client, &scout(f), &project, &thread, "phase 2");
    let answer = read(f, client, &moderator(f), &project, &thread);
    assert_eq!(
        posted_by(&answer, 2),
        vec![who("run-1", "scout")],
        "the next read shows the new post's identity: {answer}"
    );
    assert_eq!(
        posted_by(&answer, 1),
        vec![who("run-1", "critic"), who("run-1", "scout")],
        "phase 1 is as it was: {answer}"
    );
}

#[test]
fn a_sealed_phase_tells_an_agent_who_has_posted_but_not_what() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, true);
    let first = post(f, client, &critic(f), &project, &thread, "sealed critic");
    let second = post(
        f,
        client,
        &agent_as(f, "run-2", "critic"),
        &project,
        &thread,
        "sealed other critic",
    );

    // The moderator has not posted in the sealed phase.
    let response = read_thread(f, client, &moderator(f), &project, &thread);
    let text = response.text();
    let answer = must(response, 200, "the moderator reads the thread");
    assert_eq!(
        posted_by(&answer, 1),
        vec![who("run-1", "critic"), who("run-2", "critic")],
        "the moderator learns who has posted: {answer}"
    );
    assert!(
        thread_posts(&answer).is_empty(),
        "but reads none of the posts: {answer}"
    );
    assert_nothing_of(&text, &first, "the moderator's thread read");
    assert_nothing_of(&text, &second, "the moderator's thread read");
}

// ---------------------------------------------------------------------------
// decision
// ---------------------------------------------------------------------------

#[test]
fn a_thread_without_a_decision_has_a_null_decision() {
    let (f, client) = ready();
    let (project, thread) = thread_in_new_project(f, client, false);
    post(f, client, &critic(f), &project, &thread, "critic");
    for (auth, reader) in [(moderator(f), "an agent"), (human(f), "the human")] {
        let answer = read(f, client, &auth, &project, &thread);
        assert_eq!(
            answer["thread"].get("decision"),
            Some(&Value::Null),
            "{reader}: the thread has `decision`, null without one: {answer}"
        );
    }
}
