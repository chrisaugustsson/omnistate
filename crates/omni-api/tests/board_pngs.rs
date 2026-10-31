//! US-121: `GET /docs/{doc_id}/boards/{board_id}/png`, the picture of a board
//! that `omni export` writes. Run with `--features stack-tests` in the Compose
//! test container, which reaches Postgres at `postgres:5432` and RustFS at
//! `rustfs:9000`.

mod common;

use common::{Client, Response, TestServer, cookie};
use omni_api::Config;
use omni_core::design::Node;
use omni_render::paint::Assets;
use omni_render::system::DesignSystem;
use omni_render::view::board_png;
use s3::creds::Credentials;
use s3::{Bucket, BucketConfiguration, Region};
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us121_board_pngs";
const TEST_BUCKET: &str = "omni-us121-board-pngs";
/// A node ID in the right format that no node has.
const UNKNOWN_NODE: &str = "zzzzzzzz";

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

struct Fixture {
    workspace: String,
    key: String,
    server: TestServer,
}

fn session_token() -> String {
    "us121-session-alice-0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
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
            let mut conn = PgConnection::connect_with(&admin_options().database(TEST_DB))
                .await
                .expect("connect to the scratch database as the admin role");
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
                &[&user, &session_token()],
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
        let response = server.client.request(
            "POST",
            &format!("/api/v1/w/{workspace}/api-keys"),
            &[("Cookie", &cookie(&session_token()))],
            Some("{}"),
        );
        assert_eq!(response.status, 201, "mint a key: {}", response.text());
        let key = response.json()["secret"]
            .as_str()
            .expect("the new key has a secret")
            .to_string();
        Fixture {
            workspace,
            key,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

fn agent_headers(f: &Fixture) -> Vec<(&'static str, String)> {
    vec![
        ("Authorization", format!("Bearer {}", f.key)),
        ("X-Omni-Run", "run-121".to_string()),
        ("X-Omni-Role", "builder".to_string()),
        ("X-Omni-Model", "test-model".to_string()),
        ("X-Omni-Harness", "test-harness".to_string()),
    ]
}

fn call(f: &Fixture, client: &Client, method: &str, path: &str, body: Option<&Value>) -> Response {
    let headers = agent_headers(f);
    let headers: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
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

fn new_project(f: &Fixture, client: &Client) -> String {
    let path = format!("/api/v1/w/{}/projects", f.workspace);
    let body = json!({ "name": unique("pngs-project") });
    id_of(
        &must(
            call(f, client, "POST", &path, Some(&body)),
            201,
            "create a project",
        )["project"],
    )
}

fn new_doc(f: &Fixture, client: &Client, project: &str, kind: &str) -> String {
    let path = format!("{}/docs", project_base(f, project));
    let body = json!({ "kind": kind, "pillar": "ui", "title": unique(kind) });
    id_of(
        &must(
            call(f, client, "POST", &path, Some(&body)),
            201,
            "create a doc",
        )["doc"],
    )
}

/// A design doc in `project` whose first page holds the board in `tags`;
/// returns the doc and the board's ID.
fn design_with_board(f: &Fixture, client: &Client, project: &str, tags: &str) -> (String, String) {
    let doc = new_doc(f, client, project, "design");
    let html = format!("{}/docs/{doc}/html", project_base(f, project));
    let page = id_of(&must(call(f, client, "GET", &html, None), 200, "read the head")["pages"][0]);
    let ops = json!({ "ops": [
        { "op": "insert", "parent": page, "position": { "after": null }, "content": tags }
    ]});
    let ops_path = format!("{}/docs/{doc}/ops", project_base(f, project));
    must(
        call(f, client, "POST", &ops_path, Some(&ops)),
        200,
        "insert the board",
    );
    let head = must(call(f, client, "GET", &html, None), 200, "read the head");
    (doc, id_of(&head["pages"][0]["children"][0]))
}

fn png_response(f: &Fixture, client: &Client, project: &str, doc: &str, board: &str) -> Response {
    let path = format!("{}/docs/{doc}/boards/{board}/png", project_base(f, project));
    call(f, client, "GET", &path, None)
}

/// The width and height in a PNG's header.
fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "the body is a PNG");
    let at = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
    (at(16), at(20))
}

fn upload(f: &Fixture, client: &Client, bytes: &[u8], filename: &str) -> String {
    let headers = agent_headers(f);
    let mut headers: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
    headers.push(("Content-Type", "application/octet-stream"));
    headers.push(("X-Omni-Filename", filename));
    let response = client.request_bytes(
        "POST",
        &format!("/api/v1/w/{}/attachments", f.workspace),
        &headers,
        Some(bytes),
    );
    id_of(&must(response, 201, "upload a file")["attachment"])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn text_in_a_family_the_design_system_supplies_is_drawn_in_its_font_files() {
    let font_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/penpot/fonts/Geist/Geist-Regular.ttf");
    let Ok(geist) = std::fs::read(&font_path) else {
        eprintln!("SKIPPED: {} is not there", font_path.display());
        return;
    };
    let (f, client) = ready();
    let project = new_project(f, client);
    new_doc(f, client, &project, "design-system");
    let font = upload(f, client, &geist, "Geist-Regular.ttf");
    let tokens = json!({
        "$extensions": {"omnistate": {"modes": ["light"]}},
        "typography": {
            "$type": "typography",
            "display": {
                "$value": {"fontFamily": "Geist", "fontSize": "40px", "fontWeight": 400, "lineHeight": 1.2, "letterSpacing": "0px"},
                "$extensions": {"omnistate": {"fontFiles": [format!("attachment:{font}")]}}
            }
        }
    });
    let tokens_path = format!("{}/tokens", project_base(f, &project));
    must(
        call(
            f,
            client,
            "PUT",
            &tokens_path,
            Some(&json!({ "tokens": tokens })),
        ),
        200,
        "import the tokens",
    );
    let (doc, board) = design_with_board(
        f,
        client,
        &project,
        r#"<Board name="Quiz" x={0} y={0} width={400} height={80}>
  <Text name="Title" fontFamily="Geist" fontSize={40} content="Wagon Quiz 123"/>
</Board>"#,
    );

    let response = png_response(f, client, &project, &doc, &board);

    assert_eq!(response.status, 200, "{}", response.text());
    let board_path = format!("{}/docs/{doc}/boards/{board}", project_base(f, &project));
    let node: Node = serde_json::from_value(
        must(
            call(f, client, "GET", &board_path, None),
            200,
            "read the board",
        )["node"]
            .clone(),
    )
    .expect("the board is a node");
    let mut viewer = Assets::new();
    viewer.add_font("Geist", &geist).expect("Geist is a font");
    let system = DesignSystem::from_json(&tokens).expect("the tokens are a design system");
    let expected = board_png(node, &viewer, Some(&system), 2.0).expect("the board is drawn");
    assert!(
        response.body == expected,
        "the picture is drawn with the Geist font files of the design system"
    );
}

#[test]
fn a_board_whose_side_at_2x_passes_16384_px_is_refused_naming_the_limit() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, at_limit) = design_with_board(
        f,
        client,
        &project,
        r##"<Board name="At limit" x={0} y={0} width={8192} height={1} fill="#0000ff"/>"##,
    );
    let (wide_doc, too_wide) = design_with_board(
        f,
        client,
        &project,
        r##"<Board name="Too wide" x={0} y={0} width={8192.5} height={1} fill="#0000ff"/>"##,
    );

    let refused = png_response(f, client, &project, &wide_doc, &too_wide);
    let drawn = png_response(f, client, &project, &doc, &at_limit);

    assert_eq!(refused.status, 422, "{}", refused.text());
    let message = refused.json()["message"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        message.contains("16384"),
        "the message names the limit: {message}"
    );
    assert_eq!(drawn.status, 200, "{}", drawn.text());
    assert_eq!(drawn.header("content-type"), Some("image/png"));
    assert_eq!(png_size(&drawn.body), (16384, 2));
}

#[test]
fn an_unknown_board_is_not_found() {
    let (f, client) = ready();
    let project = new_project(f, client);
    let (doc, _) = design_with_board(
        f,
        client,
        &project,
        r#"<Board name="Known" x={0} y={0} width={40} height={40}/>"#,
    );

    let response = png_response(f, client, &project, &doc, UNKNOWN_NODE);

    assert_eq!(response.status, 404, "{}", response.text());
}
