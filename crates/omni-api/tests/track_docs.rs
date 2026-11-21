//! US-061: docs in a track (Q51). Run with `--features stack-tests` in the
//! Compose test container, which reaches Postgres at `postgres:5432` and RustFS
//! at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with one member
//! (alice) and a live session, mint an API key through the API, create a
//! scratch RustFS bucket and serve the API's router on a local port.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}/projects/{project_id}`; tracks are those of
//! tests/tracks.rs, the doc routes those of tests/live_docs.rs):
//!
//! - `docs.track_id` is nullable (null is a global doc of the project) with
//!   the foreign key `(workspace_id, project_id, track_id)` to `tracks
//!   (workspace_id, project_id, id)`.
//! - A Doc (`DocBody`) carries `track_id`, and so does the `data` of
//!   `doc.created`.
//! - A design doc in a track resolves its instances against the project's
//!   design system.
//! - `GET /docs` (both) answers `{"docs": [Doc, ...]}`, every doc of the
//!   project oldest first; `?track=<id>` lists that track's docs only and
//!   `?track=global` the global ones only. Another value is 400, an unknown
//!   project 404 `not_found`.
//! - US-308: `POST /docs` takes `feature_id` for a doc on a feature, which
//!   `?feature=<id>` lists and the track and global lists leave out. A design
//!   may sit on a feature, a design system may not; a feature and a track
//!   together are 422 `one_scope_only`.
//! - US-328: `POST /docs` takes `part_id` for a doc on a part; more than one
//!   of `part_id`, `feature_id` and `track_id` is 422 `one_scope_only`.
//! - US-314: only a track's owner (with a session or a key the owner made)
//!   adds a story to it; another member's is 403 `{"error":
//!   "not_track_owner"}`. Every member reads the board and comments on the
//!   track's docs.

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

const TEST_DB: &str = "omni_us061_track_docs";
const TEST_BUCKET: &str = "omni-us061-track-docs";
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
        // Heads keep their content in RustFS, so the API gets a real bucket.
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
    /// A key that alice minted with her session.
    key: String,
    /// bob, a second member of the workspace, with a key he minted.
    bob_token: String,
    bob_key: String,
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
            for name in ["alice", "bob"] {
                let user = text(
                    "insert into users (email) values ($1) returning id::text",
                    &[&format!("{name}@example.test")],
                )
                .await;
                text(
                    "insert into sessions (user_id, token_hash, expires_at) \
                     values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
                     returning id::text",
                    &[&user, &session_token(name)],
                )
                .await;
                text(
                    "insert into memberships (workspace_id, user_id) \
                     values ($1::uuid, $2::uuid) returning id::text",
                    &[&workspace, &user],
                )
                .await;
            }
            workspace
        });
        let server = TestServer::start(&config());
        let mint = |name: &str| -> String {
            let response = call(
                &server.client,
                "POST",
                "/api/v1/api-keys",
                &Auth::Session(&session_token(name)),
                Some(&json!({})),
            );
            assert_eq!(response.status, 201, "mint a key: {}", response.text());
            response.json()["secret"]
                .as_str()
                .expect("the new key has a secret")
                .to_string()
        };
        let key = mint("alice");
        let bob_key = mint("bob");
        Fixture {
            workspace,
            alice_token: session_token("alice"),
            key,
            bob_token: session_token("bob"),
            bob_key,
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

/// Asserts that `value` has the key `track_id` and that it is `expected`
/// (`Value::Null` for a global doc).
fn assert_track_id(value: &Value, expected: &Value, what: &str) {
    assert!(
        value
            .as_object()
            .is_some_and(|o| o.contains_key("track_id")),
        "{what} carries `track_id`: {value}"
    );
    assert_eq!(&value["track_id"], expected, "{what}: {value}");
}

fn project_base(f: &Fixture, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", f.workspace)
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
                Some(&json!({ "name": unique("track-docs-project") })),
            ),
            201,
            "create a project",
        )["project"],
    )
}

/// Creates a part of the project for a track to sit on (US-305); returns its ID.
fn new_part(f: &Fixture, client: &Client, project: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/parts", project_base(f, project)),
                &agent(f),
                Some(&json!({ "name": unique("part"), "kind": "app" })),
            ),
            201,
            "create a part",
        )["part"],
    )
}

/// Creates a track as the agent; returns its ID.
fn new_track(f: &Fixture, client: &Client, project: &str, name: &str) -> String {
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/tracks", project_base(f, project)),
                &agent(f),
                Some(&json!({ "name": name, "part_id": new_part(f, client, project) })),
            ),
            201,
            "create a track",
        )["track"],
    )
}

/// Creates a doc as the agent, in `track` or at project level without one;
/// returns the doc of the answer.
fn create_live(
    f: &Fixture,
    client: &Client,
    project: &str,
    track: Option<&str>,
    kind: &str,
    pillar: &str,
) -> Value {
    let path = match track {
        None => docs_path(f, project),
        Some(track) => format!("{}/tracks/{track}/docs", project_base(f, project)),
    };
    must(
        call(
            client,
            "POST",
            &path,
            &agent(f),
            Some(&json!({ "kind": kind, "pillar": pillar, "title": unique(pillar) })),
        ),
        201,
        "create a doc",
    )["doc"]
        .clone()
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
fn a_doc_takes_the_track_it_is_created_in() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let track = new_track(f, client, &project, "Checkout");

    for (scope, expected) in [(Some(track.as_str()), json!(track)), (None, Value::Null)] {
        for (kind, pillar) in [
            ("html", "spec"),
            ("html", "architecture"),
            ("html", "db"),
            ("design", "ui"),
        ] {
            let doc = create_live(f, client, &project, scope, kind, pillar);
            assert_track_id(&doc, &expected, &format!("a new {kind} {pillar} doc"));
            let id = id_of(&doc);
            for auth in [agent(f), human(f)] {
                let shown = show_doc(f, client, &auth, &project, &id);
                assert_track_id(&shown, &expected, "the shown doc");
            }
            let event = events_of(&project, "doc.created")
                .into_iter()
                .find(|data| data["doc_id"] == id.as_str())
                .unwrap_or_else(|| panic!("doc.created for {id}"));
            assert_track_id(&event, &expected, "the data of doc.created");
        }
    }
}

#[test]
fn the_docs_table_ties_a_doc_to_a_track_of_its_own_project() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let other = new_project(f, client);
    let own = new_track(f, client, &project, "Checkout");
    let foreign = new_track(f, client, &other, "Checkout");
    let workspace = f.workspace.as_str();
    let insert = "insert into docs (workspace_id, project_id, kind, pillar, title, track_id) \
                  values ($1::uuid, $2::uuid, 'html', 'spec', 'Direct', $3::uuid)";
    assert!(
        accepted(insert, &[workspace, &project, &own]),
        "a doc may belong to a track of its project"
    );
    assert!(
        !accepted(insert, &[workspace, &project, &foreign]),
        "a doc cannot belong to a track of another project"
    );
    assert!(
        !accepted(insert, &[workspace, &project, UNKNOWN_ID]),
        "a doc cannot belong to a track that does not exist"
    );
    assert!(
        accepted(
            "insert into docs (workspace_id, project_id, kind, pillar, title) \
             values ($1::uuid, $2::uuid, 'html', 'spec', 'Global')",
            &[workspace, &project]
        ),
        "a doc without a track is global"
    );
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn a_design_in_a_track_resolves_its_instances_against_the_project_design_system() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let track = new_track(f, client, &project, "Checkout");
    let base = project_base(f, &project);

    // The project's design system, with one component.
    create_live(f, client, &project, None, "design-system", "ui");
    let tags = r#"<Component name="Cart" width={16} height={16}>
  <Vector name="Cart glyph" x={0} y={0} width={16} height={16} path="M0 0 L16 16"/>
</Component>
"#;
    let imported = must(
        call(
            client,
            "PUT",
            &format!("{base}/components"),
            &agent(f),
            Some(&json!({ "tags": tags })),
        ),
        200,
        "import the components",
    );
    let cart = id_of(&imported["components"][0]);

    // A design doc of the track draws an instance of the project's
    // component.
    let design = create_live(f, client, &project, Some(&track), "design", "ui");
    assert_track_id(&design, &json!(track), "the track's design");
    let doc = format!("{base}/docs/{}", id_of(&design));
    let head = must(
        call(client, "GET", &format!("{doc}/html"), &agent(f), None),
        200,
        "read the design's head",
    );
    let page = id_of(&head["pages"][0]);
    let pay_board = |component: &str| {
        json!({ "ops": [{
            "op": "insert",
            "parent": page,
            "position": { "after": null },
            "content": format!(
                r#"<Board name="Pay" x={{0}} y={{0}} width={{375}} height={{812}} dir="col">
  <Instance name="Cart icon" component="{component}"/>
</Board>"#
            )
        }] })
    };
    let unknown = call(
        client,
        "POST",
        &format!("{doc}/ops"),
        &agent(f),
        Some(&pay_board("zzzzzzzz")),
    );
    assert_eq!(
        unknown.status,
        400,
        "an instance of a component the project does not have is refused: {}",
        unknown.text()
    );
    assert_eq!(
        unknown.json()["error"],
        "invalid_operation",
        "{}",
        unknown.text()
    );
    must(
        call(
            client,
            "POST",
            &format!("{doc}/ops"),
            &agent(f),
            Some(&pay_board(&cart)),
        ),
        200,
        "add a board with an instance of the project's component",
    );

    let head = must(
        call(client, "GET", &format!("{doc}/html"), &agent(f), None),
        200,
        "read the design's head",
    );
    let components = head["components"]
        .as_array()
        .unwrap_or_else(|| panic!("the head lists its components: {head}"));
    assert!(
        components.iter().any(|c| c["id"] == cart.as_str()),
        "the instance resolves to the project's Cart: {head}"
    );
}

// ---------------------------------------------------------------------------
// The list of docs
// ---------------------------------------------------------------------------

#[test]
fn the_doc_list_has_every_doc_of_the_project_and_filters_by_track_or_global() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let checkout = new_track(f, client, &project, "Checkout");
    let search = new_track(f, client, &project, "Search");
    let first = id_of(&create_live(f, client, &project, None, "html", "spec"));
    let second = id_of(&create_live(
        f,
        client,
        &project,
        Some(&checkout),
        "html",
        "spec",
    ));
    let third = id_of(&create_live(
        f,
        client,
        &project,
        Some(&search),
        "design",
        "ui",
    ));
    let fourth = id_of(&create_live(
        f,
        client,
        &project,
        None,
        "html",
        "architecture",
    ));

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
            assert_track_id(listed, &expected, "a listed doc");
        }

        let of_checkout = list_docs(client, &auth, &format!("{path}?track={checkout}"));
        assert_eq!(
            ids(&of_checkout),
            [second.clone()],
            "?track=<id> lists that track's docs only"
        );
        let global_only = list_docs(client, &auth, &format!("{path}?track=global"));
        assert_eq!(
            ids(&global_only),
            [first.clone(), fourth.clone()],
            "?track=global lists the global docs only"
        );
    }

    let response = call(
        client,
        "GET",
        &format!("{path}?track=not-a-track"),
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
// Docs on features (US-308)
// ---------------------------------------------------------------------------

/// Creates a part and the feature "Analyze" on it; returns the feature's ID.
fn new_feature(f: &Fixture, client: &Client, project: &str) -> String {
    let part = new_part(f, client, project);
    id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{}/parts/{part}/features", project_base(f, project)),
                &agent(f),
                Some(&json!({ "name": "Analyze", "description": "Charts per channel" })),
            ),
            201,
            "create a feature",
        )["feature"],
    )
}

fn create_on_feature(
    f: &Fixture,
    client: &Client,
    project: &str,
    feature: &str,
    kind: &str,
    pillar: &str,
) -> Response {
    call(
        client,
        "POST",
        &docs_path(f, project),
        &agent(f),
        Some(&json!({
            "kind": kind,
            "pillar": pillar,
            "title": "How Analyze works",
            "feature_id": feature,
        })),
    )
}

#[test]
fn a_doc_on_a_feature_is_listed_by_its_feature_and_not_by_a_track_or_global() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let track = new_track(f, client, &project, "Wision");
    let analyze = new_feature(f, client, &project);
    let global = id_of(&create_live(f, client, &project, None, "html", "spec"));
    let in_track = id_of(&create_live(
        f,
        client,
        &project,
        Some(&track),
        "html",
        "spec",
    ));
    let created = must(
        create_on_feature(f, client, &project, &analyze, "html", "architecture"),
        201,
        "create a doc on the feature",
    )["doc"]
        .clone();
    let on_feature = id_of(&created);
    assert_eq!(created["feature_id"], analyze.as_str(), "{created}");
    assert_track_id(&created, &Value::Null, "a doc on a feature");

    let path = docs_path(f, &project);
    for auth in [agent(f), human(f)] {
        assert_eq!(
            ids(&list_docs(
                client,
                &auth,
                &format!("{path}?feature={analyze}")
            )),
            [on_feature.clone()],
            "?feature=<id> lists that feature's docs only"
        );
        assert_eq!(
            ids(&list_docs(client, &auth, &format!("{path}?track={track}"))),
            [in_track.clone()],
            "a track's list leaves out the feature's doc"
        );
        assert_eq!(
            ids(&list_docs(client, &auth, &format!("{path}?track=global"))),
            [global.clone()],
            "the global list leaves out the feature's doc"
        );
    }
}

#[test]
fn a_design_doc_may_sit_on_a_feature_and_a_design_system_may_not() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let analyze = new_feature(f, client, &project);
    let design = must(
        create_on_feature(f, client, &project, &analyze, "design", "ui"),
        201,
        "create a design on the feature",
    )["doc"]
        .clone();
    assert_eq!(design["feature_id"], analyze.as_str(), "{design}");

    let response = create_on_feature(f, client, &project, &analyze, "design-system", "ui");
    assert_error(&response, 422, "design_system_is_global");
}

/// US-318: the web hides a service track's UI step, but the API never refuses
/// a design on one, in the track's docs or in the project's docs.
#[test]
fn a_design_doc_is_created_on_a_track_of_a_service_part() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let base = project_base(f, &project);
    let iot = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/parts"),
                &agent(f),
                Some(&json!({ "name": unique("iot-service"), "kind": "service" })),
            ),
            201,
            "create a service part",
        )["part"],
    );
    let track = id_of(
        &must(
            call(
                client,
                "POST",
                &format!("{base}/tracks"),
                &agent(f),
                Some(&json!({ "name": "Ingest", "part_id": iot })),
            ),
            201,
            "create a track on the service part",
        )["track"],
    );

    let in_track = create_live(f, client, &project, Some(&track), "design", "ui");
    let in_project = must(
        call(
            client,
            "POST",
            &docs_path(f, &project),
            &agent(f),
            Some(&json!({ "kind": "design", "pillar": "ui", "title": unique("ui"), "track_id": track })),
        ),
        201,
        "create a design on the service track through the project's docs",
    )["doc"]
        .clone();
    for doc in [in_track, in_project] {
        assert_track_id(&doc, &json!(track), "a design on a service track");
        let shown = show_doc(f, client, &human(f), &project, &id_of(&doc));
        assert_eq!(shown["kind"], "design", "{shown}");
        assert_track_id(&shown, &json!(track), "the shown design");
    }
}

#[test]
fn a_doc_with_both_a_feature_and_a_track_is_422_one_scope_only() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let track = new_track(f, client, &project, "Wision");
    let analyze = new_feature(f, client, &project);
    let response = call(
        client,
        "POST",
        &docs_path(f, &project),
        &agent(f),
        Some(&json!({
            "kind": "html",
            "pillar": "architecture",
            "title": "How Analyze works",
            "feature_id": analyze,
            "track_id": track,
        })),
    );
    assert_error(&response, 422, "one_scope_only");
    assert!(
        list_docs(client, &agent(f), &docs_path(f, &project)).is_empty(),
        "the refused doc was not created"
    );
}

// ---------------------------------------------------------------------------
// Docs on parts (US-328)
// ---------------------------------------------------------------------------

#[test]
fn a_doc_with_a_part_and_a_feature_or_a_track_is_422_one_scope_only() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let part = new_part(f, client, &project);
    let track = new_track(f, client, &project, "Wision");
    let analyze = new_feature(f, client, &project);
    let scopes = [
        json!({ "part_id": part, "feature_id": analyze }),
        json!({ "part_id": part, "track_id": track }),
        json!({ "part_id": part, "feature_id": analyze, "track_id": track }),
    ];
    for scope in scopes {
        let mut body = json!({
            "kind": "html",
            "pillar": "architecture",
            "title": "Architecture",
        });
        body.as_object_mut()
            .expect("an object")
            .extend(scope.as_object().expect("an object").clone());
        let response = call(
            client,
            "POST",
            &docs_path(f, &project),
            &agent(f),
            Some(&body),
        );
        assert_error(&response, 422, "one_scope_only");
    }
    assert!(
        list_docs(client, &agent(f), &docs_path(f, &project)).is_empty(),
        "no refused doc was created"
    );
}

// ---------------------------------------------------------------------------
// Only the owner changes a track (US-314)
// ---------------------------------------------------------------------------

/// Adds a paragraph to an HTML doc's head as the agent; returns its node ID.
fn paragraph(f: &Fixture, client: &Client, project: &str, doc: &str) -> String {
    let answer = must(
        call(
            client,
            "POST",
            &format!("{}/docs/{doc}/ops", project_base(f, project)),
            &agent(f),
            Some(&json!({ "ops": [{
                "op": "insert",
                "parent": null,
                "position": { "after": null },
                "content": "<p>Orders ship in two days.</p>",
            }] })),
        ),
        200,
        "apply a batch to the head",
    );
    answer["results"][0]["node"]
        .as_str()
        .unwrap_or_else(|| panic!("an insert names its node: {answer}"))
        .to_string()
}

#[test]
fn only_the_owner_adds_a_story_and_every_member_reads_the_board_and_comments() {
    let (f, client) = ready();
    let project = new_project(f, client);
    // Made with alice's key, so alice owns the track.
    let track = new_track(f, client, &project, "Long windows");
    let doc = id_of(&create_live(
        f,
        client,
        &project,
        Some(&track),
        "html",
        "spec",
    ));
    let node = paragraph(f, client, &project, &doc);
    let stories = format!("{}/tracks/{track}/stories", project_base(f, &project));
    let story = |title: &str| json!({ "title": title, "text": "", "topic": "", "criteria": [] });

    for (auth, who) in [
        (Auth::Session(&f.bob_token), "bob's session"),
        (Auth::Agent(&f.bob_key), "bob's key"),
    ] {
        let response = call(client, "POST", &stories, &auth, Some(&story("Not mine")));
        assert_error(&response, 403, "not_track_owner");
        let board = must(
            call(client, "GET", &stories, &auth, None),
            200,
            &format!("{who} reads the board"),
        );
        assert_eq!(board["stories"], json!([]), "{who} added no story: {board}");
        must(
            call(
                client,
                "POST",
                &format!("{}/docs/{doc}/comments", project_base(f, &project)),
                &auth,
                Some(&json!({ "node": node, "text": format!("A comment from {who}") })),
            ),
            201,
            &format!("{who} comments on the track's doc"),
        );
    }

    for (auth, who) in [(human(f), "alice's session"), (agent(f), "alice's key")] {
        must(
            call(client, "POST", &stories, &auth, Some(&story(who))),
            201,
            &format!("the owner adds a story with {who}"),
        );
    }
}
