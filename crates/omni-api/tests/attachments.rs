//! US-019: attachments. Run with `--features stack-tests` in the Compose test
//! container, which reaches Postgres at `postgres:5432` and RustFS at
//! `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed two workspaces (A and B) with one
//! member each (alice in A, bob in B) and a live session for each, mint an API
//! key per workspace through the API, create a scratch RustFS bucket and serve
//! the API's router on a local port against both.
//!
//! The contract these tests pin down (all paths are under
//! `/api/v1/w/{workspace_id}`; the workspace is only ever taken from the path):
//!
//! - `POST /attachments` (agent) uploads one file. The body is the file's raw
//!   bytes; `X-Omni-Filename` is its name (percent-encoded UTF-8) and
//!   `Content-Type` is what the sender claims, which the API never trusts. The
//!   answer is 201 `{"attachment": {"id", "ref", "filename", "kind",
//!   "content_type", "size", "created_at"}}`. `id` is a UUIDv7, `ref` is
//!   `attachment:<id>` (how every doc, design and post refers to the file),
//!   `size` is the number of bytes. `kind` is `image` for a PNG, JPEG, WebP or
//!   GIF, `font` for a WOFF2, TTF or OTF, and `file` for anything else; it and
//!   an image's `content_type` (`image/png`, `image/jpeg`, `image/webp`,
//!   `image/gif`) come from the bytes alone, never from the name or the claimed
//!   type. A file over 20 MB is 413 `{"error": "file_too_large"}` and nothing
//!   is stored. (The tests accept 20,000,000 bytes and refuse 20 MiB plus one
//!   byte, so both readings of "20 MB" pass.)
//! - `GET /attachments/{attachment_id}` (both) answers the stored bytes. An
//!   image whose bytes confirm its type comes back inline with that type;
//!   everything else (fonts, other files, and a file that only claims to be an
//!   image) comes back with `Content-Disposition: attachment`. Every response
//!   of the API sends `X-Content-Type-Options: nosniff` and `Content-Security-
//!   Policy: sandbox`. An unknown, malformed or other workspace's ID is 404
//!   `not_found`.
//! - The file is stored in RustFS at `ws/{workspace_id}/attachments/{id}`. The
//!   metadata is a row of the tenant table `attachments` (`id`, `workspace_id`,
//!   `filename` text, `content_type` text, `size_bytes` bigint; every other
//!   column is nullable or has a default), so the database enforces the workspace as for any other
//!   tenant record.

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

const TEST_DB: &str = "omni_us019_attachments";
const TEST_BUCKET: &str = "omni-us019-attachments";
/// An ID that no row has.
const UNKNOWN_ID: &str = "01900000-0000-7000-8000-000000000000";
/// What the tests call 20 MB when they must succeed with it, and the byte count
/// just over 20 MiB when they must be refused.
const TWENTY_MB: usize = 20_000_000;
const OVER_TWENTY_MIB: usize = 20 * 1024 * 1024 + 1;

const PNG: &[u8] = include_bytes!("fixtures/pixel.png");
const JPEG: &[u8] = include_bytes!("fixtures/pixel.jpg");
const GIF: &[u8] = include_bytes!("fixtures/pixel.gif");
const WEBP: &[u8] = include_bytes!("fixtures/pixel.webp");

/// A file that starts like a font of the given kind; the rest is padding.
fn font(magic: &[u8]) -> Vec<u8> {
    let mut bytes = magic.to_vec();
    bytes.extend((0..200u32).map(|n| (n % 251) as u8));
    bytes
}

fn woff2() -> Vec<u8> {
    font(b"wOF2")
}

fn ttf() -> Vec<u8> {
    font(&[0x00, 0x01, 0x00, 0x00])
}

fn otf() -> Vec<u8> {
    font(b"OTTO")
}

/// A page of script that a browser would run if it were served as HTML or SVG.
const SCRIPT_PAGE: &[u8] = b"<html><body><script>fetch('/api/v1/auth/me')</script></body></html>";
const SVG_WITH_SCRIPT: &[u8] =
    b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script><circle r=\"4\"/></svg>";

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

fn region() -> Region {
    Region::Custom {
        region: "us-east-1".to_string(),
        endpoint: s3_endpoint(),
    }
}

fn credentials() -> Credentials {
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
    Bucket::new(TEST_BUCKET, region(), credentials())
        .expect("bucket settings")
        .with_path_style()
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

async fn admin_connection() -> PgConnection {
    PgConnection::connect_with(&admin_options().database(TEST_DB))
        .await
        .expect("connect to the scratch database as the admin role")
}

struct Fixture {
    a: String,
    /// A workspace that no test stores a file in, for the tests that check that
    /// a refused upload stored nothing while other tests upload to A and B.
    c: String,
    alice_token: String,
    carol_token: String,
    key_a: String,
    key_c: String,
    server: TestServer,
}

fn session_token(name: &str) -> String {
    format!("us019-session-{name}-0123456789abcdef0123456789abcdef0123456789abcdef")
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
            if !bucket().exists().await.unwrap_or(false) {
                Bucket::create_with_path_style(
                    TEST_BUCKET,
                    region(),
                    credentials(),
                    BucketConfiguration::default(),
                )
                .await
                .unwrap_or_else(|e| panic!("cannot create the scratch bucket: {e}"));
            }
            // The bucket outlives the test run; what an earlier run left in it
            // (a file of 20 MB among it) is not wanted.
            let b = bucket();
            for key in keys_under("ws/").await {
                b.delete_object(&key)
                    .await
                    .unwrap_or_else(|e| panic!("cannot clear {key}: {e}"));
            }
        });
        let (a, _, c) = block(async {
            let mut conn = admin_connection().await;
            let mut text = async |sql: &str, binds: &[&str]| -> String {
                let mut query = sqlx::query_scalar::<_, String>(sql);
                for bind in binds {
                    query = query.bind(*bind);
                }
                query.fetch_one(&mut conn).await.expect("seed")
            };
            let a = text(
                "insert into workspaces (name) values ('Alpha') returning id::text",
                &[],
            )
            .await;
            let b = text(
                "insert into workspaces (name) values ('Beta') returning id::text",
                &[],
            )
            .await;
            let c = text(
                "insert into workspaces (name) values ('Gamma') returning id::text",
                &[],
            )
            .await;
            for (name, workspace) in [("alice", &a), ("bob", &b), ("carol", &c)] {
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
                    &[workspace, &user],
                )
                .await;
            }
            (a, b, c)
        });
        let server = TestServer::start(&config());
        let mint = |token: &str, workspace: &str| -> String {
            let response = call(
                &server.client,
                "POST",
                &format!("/api/v1/w/{workspace}/api-keys"),
                &Auth::Session(token),
                Some(&json!({})),
            );
            assert_eq!(response.status, 201, "mint a key: {}", response.text());
            response.json()["secret"]
                .as_str()
                .expect("the new key has a secret")
                .to_string()
        };
        let key_a = mint(&session_token("alice"), &a);
        let key_c = mint(&session_token("carol"), &c);
        Fixture {
            a,
            c,
            alice_token: session_token("alice"),
            carol_token: session_token("carol"),
            key_a,
            key_c,
            server,
        }
    })
}

fn ready() -> (&'static Fixture, &'static Client) {
    let fixture = fixture();
    (fixture, &fixture.server.client)
}

/// How a request authenticates: an API key (an agent), a session cookie (the
/// human) or nothing.
#[derive(Clone, Copy)]
enum Auth<'a> {
    Key(&'a str),
    Session(&'a str),
    Nobody,
}

impl Auth<'_> {
    fn headers(&self) -> Vec<(&'static str, String)> {
        match self {
            Auth::Key(secret) => vec![("Authorization", format!("Bearer {secret}"))],
            Auth::Session(token) => vec![("Cookie", cookie(token))],
            Auth::Nobody => vec![],
        }
    }
}

fn call(client: &Client, method: &str, path: &str, auth: &Auth, body: Option<&Value>) -> Response {
    let owned = auth.headers();
    let headers: Vec<(&str, &str)> = owned.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let body = body.map(Value::to_string);
    client.request(method, path, &headers, body.as_deref())
}

fn attachments_path(workspace: &str) -> String {
    format!("/api/v1/w/{workspace}/attachments")
}

fn attachment_path(workspace: &str, id: &str) -> String {
    format!("{}/{id}", attachments_path(workspace))
}

/// Uploads `bytes` as `filename`, claiming to be of type `claimed`.
fn upload(
    client: &Client,
    auth: &Auth,
    workspace: &str,
    bytes: &[u8],
    filename: &str,
    claimed: &str,
) -> Response {
    let owned = auth.headers();
    let mut headers: Vec<(&str, &str)> = owned.iter().map(|(n, v)| (*n, v.as_str())).collect();
    headers.push(("Content-Type", claimed));
    headers.push(("X-Omni-Filename", filename));
    client.request_bytes("POST", &attachments_path(workspace), &headers, Some(bytes))
}

/// Uploads to workspace A as its agent and returns the attachment object.
fn upload_ok(f: &Fixture, client: &Client, bytes: &[u8], filename: &str, claimed: &str) -> Value {
    let response = upload(client, &Auth::Key(&f.key_a), &f.a, bytes, filename, claimed);
    assert_eq!(
        response.status,
        201,
        "upload {filename}: {}",
        String::from_utf8_lossy(&response.body[..response.body.len().min(300)])
    );
    response.json()["attachment"].clone()
}

fn id_of(attachment: &Value) -> String {
    attachment["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the attachment has an `id`: {attachment}"))
        .to_string()
}

/// Reads an attachment of workspace A with its agent.
fn download(f: &Fixture, client: &Client, id: &str) -> Response {
    call(
        client,
        "GET",
        &attachment_path(&f.a, id),
        &Auth::Key(&f.key_a),
        None,
    )
}

fn assert_error(response: &Response, status: u16, code: &str) {
    assert_eq!(
        response.status,
        status,
        "body: {}",
        String::from_utf8_lossy(&response.body[..response.body.len().min(300)])
    );
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

/// The two headers every response of the API sends (FR-46).
fn assert_hardened(response: &Response, what: &str) {
    assert_eq!(
        response
            .header("x-content-type-options")
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("nosniff"),
        "{what}: X-Content-Type-Options"
    );
    let policy = response.header("content-security-policy").unwrap_or("");
    assert!(
        policy
            .split(';')
            .any(|directive| directive.trim().eq_ignore_ascii_case("sandbox")),
        "{what}: Content-Security-Policy should be `sandbox`, got {policy:?}"
    );
}

/// The lowercase `Content-Disposition` of a response, or "" without one.
fn disposition(response: &Response) -> String {
    response
        .header("content-disposition")
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn is_download(response: &Response) -> bool {
    disposition(response).starts_with("attachment")
}

fn is_inline(response: &Response) -> bool {
    let disposition = disposition(response);
    disposition.is_empty() || disposition.starts_with("inline")
}

/// The number of attachment rows of a workspace (0 while the table is missing,
/// so a test fails on what it asserts and not on this lookup).
fn attachment_rows(workspace: &str) -> i64 {
    block(async {
        let mut conn = admin_connection().await;
        let exists: bool =
            sqlx::query_scalar("select to_regclass('public.attachments') is not null")
                .fetch_one(&mut conn)
                .await
                .expect("look for the attachments table");
        if !exists {
            return 0;
        }
        sqlx::query_scalar("select count(*) from attachments where workspace_id = $1::uuid")
            .bind(workspace)
            .fetch_one(&mut conn)
            .await
            .expect("count the attachment rows")
    })
}

async fn keys_under(prefix: &str) -> Vec<String> {
    let pages = bucket()
        .list(prefix.to_string(), None)
        .await
        .unwrap_or_else(|e| panic!("cannot list {prefix}: {e}"));
    let mut keys: Vec<String> = pages
        .into_iter()
        .flat_map(|page| page.contents)
        .map(|object| object.key)
        .collect();
    keys.sort();
    keys
}

// ---------------------------------------------------------------------------
// omni attach: uploading, and the ID that comes back
// ---------------------------------------------------------------------------

#[test]
fn an_agent_uploads_a_file_and_gets_an_id_to_refer_to_it_by() {
    let (f, client) = ready();
    let response = upload(
        client,
        &Auth::Key(&f.key_a),
        &f.a,
        PNG,
        "logo.png",
        "image/png",
    );
    assert_eq!(response.status, 201, "{}", response.text());
    let attachment = response.json()["attachment"].clone();
    assert_uuid_v7(&attachment["id"]);
    let id = id_of(&attachment);
    assert_eq!(
        attachment["ref"],
        format!("attachment:{id}"),
        "the file is referred to everywhere as attachment:<id>"
    );
    assert_eq!(attachment["filename"], "logo.png");
    assert_eq!(attachment["kind"], "image");
    assert_eq!(attachment["content_type"], "image/png");
    assert_eq!(attachment["size"], PNG.len());
}

#[test]
fn the_file_name_arrives_percent_encoded_in_x_omni_filename() {
    let (f, client) = ready();
    let attachment = upload_ok(
        f,
        client,
        b"quarterly numbers",
        "my%20q3%20report%20%C3%A5.txt",
        "text/plain",
    );
    assert_eq!(attachment["filename"], "my q3 report \u{e5}.txt");
}

#[test]
fn only_an_agent_of_the_workspace_can_upload() {
    let (f, client) = ready();
    let response = upload(
        client,
        &Auth::Session(&f.carol_token),
        &f.c,
        PNG,
        "a.png",
        "image/png",
    );
    assert_error(&response, 403, "forbidden");
    assert_eq!(attachment_rows(&f.c), 0, "a refused upload stored a row");
    assert_eq!(
        block(keys_under(&format!("ws/{}/", f.c))),
        Vec::<String>::new(),
        "a refused upload stored a file"
    );
}

// ---------------------------------------------------------------------------
// Limits and accepted types
// ---------------------------------------------------------------------------

#[test]
fn a_file_of_20_million_bytes_is_accepted_and_comes_back_whole() {
    let (f, client) = ready();
    let bytes: Vec<u8> = (0..TWENTY_MB).map(|n| (n % 251) as u8).collect();
    let attachment = upload_ok(f, client, &bytes, "big.bin", "application/octet-stream");
    assert_eq!(attachment["size"], TWENTY_MB);
    let response = download(f, client, &id_of(&attachment));
    assert_eq!(response.status, 200);
    assert!(response.body == bytes, "the downloaded bytes differ");
}

#[test]
fn a_file_over_20_mib_is_refused_with_413_and_nothing_is_stored() {
    let (f, client) = ready();
    let bytes = vec![0x61u8; OVER_TWENTY_MIB];
    let response = upload(
        client,
        &Auth::Key(&f.key_c),
        &f.c,
        &bytes,
        "huge.bin",
        "application/octet-stream",
    );
    assert_error(&response, 413, "file_too_large");
    assert_hardened(&response, "the 413");
    assert_eq!(attachment_rows(&f.c), 0, "a refused file left a row");
    assert_eq!(
        block(keys_under(&format!("ws/{}/", f.c))),
        Vec::<String>::new(),
        "a refused file left an object in RustFS"
    );
}

// ---------------------------------------------------------------------------
// Serving
// ---------------------------------------------------------------------------

#[test]
fn an_image_is_served_inline_once_its_bytes_confirm_the_type() {
    let (f, client) = ready();
    let images: [(&[u8], &str); 4] = [
        (PNG, "image/png"),
        (JPEG, "image/jpeg"),
        (GIF, "image/gif"),
        (WEBP, "image/webp"),
    ];
    for (bytes, content_type) in images {
        let attachment = upload_ok(f, client, bytes, "pic", "application/octet-stream");
        let response = download(f, client, &id_of(&attachment));
        assert_eq!(response.status, 200, "{content_type}");
        assert_eq!(response.content_type(), content_type);
        assert!(
            is_inline(&response),
            "{content_type} should be shown inline: {:?}",
            response.header("content-disposition")
        );
        assert!(response.body == bytes, "{content_type}: the bytes differ");
        assert_hardened(&response, content_type);
    }
}

#[test]
fn a_file_that_only_claims_to_be_an_image_is_never_served_inline() {
    let (f, client) = ready();
    let disguises: [(&str, &[u8], &str); 3] = [
        ("evil.png", SCRIPT_PAGE, "image/png"),
        ("evil.html", SCRIPT_PAGE, "text/html"),
        // SVG is not one of the accepted images: it can carry script.
        ("logo.svg", SVG_WITH_SCRIPT, "image/svg+xml"),
    ];
    for (name, bytes, claimed) in disguises {
        let attachment = upload_ok(f, client, bytes, name, claimed);
        assert_eq!(attachment["kind"], "file", "{name}");
        let response = download(f, client, &id_of(&attachment));
        assert_eq!(response.status, 200, "{name}");
        assert!(
            is_download(&response),
            "{name} must be a download: {:?}",
            response.header("content-disposition")
        );
        let served = response.content_type().to_ascii_lowercase();
        assert!(
            !served.starts_with("image/") && !served.contains("html") && !served.contains("xml"),
            "{name} is served as {served:?}, which a browser could render"
        );
        assert!(response.body == bytes, "{name}: the bytes differ");
        assert_hardened(&response, name);
    }
}

#[test]
fn fonts_and_other_files_are_served_as_downloads() {
    let (f, client) = ready();
    let files: [(&str, Vec<u8>, &str); 5] = [
        ("notes.txt", b"plain notes".to_vec(), "text/plain"),
        ("brand.woff2", woff2(), "font/woff2"),
        ("brand.ttf", ttf(), "font/ttf"),
        ("brand.otf", otf(), "font/otf"),
        ("a.pdf", b"%PDF-1.7 not really".to_vec(), "application/pdf"),
    ];
    for (name, bytes, claimed) in files {
        let attachment = upload_ok(f, client, &bytes, name, claimed);
        let response = download(f, client, &id_of(&attachment));
        assert_eq!(response.status, 200, "{name}");
        assert!(
            is_download(&response),
            "{name} must be a download: {:?}",
            response.header("content-disposition")
        );
        assert!(
            disposition(&response).contains(name),
            "the download is named {name}: {:?}",
            response.header("content-disposition")
        );
        assert!(response.body == bytes, "{name}: the bytes differ");
        assert_hardened(&response, name);
    }
}

#[test]
fn every_response_sends_nosniff_and_csp_sandbox() {
    let (f, client) = ready();
    let image = id_of(&upload_ok(f, client, PNG, "h.png", "image/png"));
    let text = id_of(&upload_ok(f, client, b"text", "h.txt", "text/plain"));
    let key = Auth::Key(&f.key_a);
    let human = Auth::Session(&f.alice_token);

    let mut responses: Vec<(String, Response)> = vec![
        (
            "upload".to_string(),
            upload(client, &key, &f.a, PNG, "h.png", "image/png"),
        ),
        (
            "upload without credentials".to_string(),
            upload(client, &Auth::Nobody, &f.a, PNG, "h.png", "image/png"),
        ),
        (
            "an inline image".to_string(),
            call(client, "GET", &attachment_path(&f.a, &image), &key, None),
        ),
        (
            "a download".to_string(),
            call(client, "GET", &attachment_path(&f.a, &text), &key, None),
        ),
        (
            "an image for the human".to_string(),
            call(client, "GET", &attachment_path(&f.a, &image), &human, None),
        ),
        (
            "an unknown attachment".to_string(),
            call(
                client,
                "GET",
                &attachment_path(&f.a, UNKNOWN_ID),
                &key,
                None,
            ),
        ),
        (
            "a malformed attachment ID".to_string(),
            call(
                client,
                "GET",
                &attachment_path(&f.a, "not-a-uuid"),
                &key,
                None,
            ),
        ),
        (
            "a download without credentials".to_string(),
            call(
                client,
                "GET",
                &attachment_path(&f.a, &text),
                &Auth::Nobody,
                None,
            ),
        ),
    ];
    // Not only the attachment routes: FR-46 says every response.
    for path in [
        "/api/health".to_string(),
        "/api/openapi.json".to_string(),
        format!("/api/v1/w/{}/whoami", f.a),
        format!("/api/v1/w/{}/projects", f.a),
        "/api/v1/nothing-here".to_string(),
    ] {
        responses.push((
            format!("GET {path}"),
            call(client, "GET", &path, &key, None),
        ));
    }
    responses.push((
        "a failed login".to_string(),
        call(
            client,
            "POST",
            "/api/v1/auth/login",
            &Auth::Nobody,
            Some(&json!({ "email": "alice@example.test", "password": "wrong" })),
        ),
    ));
    for (what, response) in &responses {
        assert_hardened(response, what);
    }
}

// ---------------------------------------------------------------------------
// Who can read what
// ---------------------------------------------------------------------------

#[test]
fn unknown_and_malformed_attachment_ids_are_404() {
    let (f, client) = ready();
    for path in [
        attachment_path(&f.a, UNKNOWN_ID),
        attachment_path(&f.a, "not-a-uuid"),
    ] {
        let response = call(client, "GET", &path, &Auth::Key(&f.key_a), None);
        assert_error(&response, 404, "not_found");
    }
}
