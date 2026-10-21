//! US-051: `omni import penpot <file.penpot> --project <id>` against the real
//! API. Run with `--features stack-tests` in the Compose test container, which
//! reaches Postgres at `postgres:5432` and RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key (the
//! agent) and a user with a live session (the human), create a scratch RustFS
//! bucket and serve the API's router on a local port against both. The built
//! `omni` runs against that address with the key in its config file. The
//! human's approval is a plain HTTP request with the session cookie, because
//! the CLI has no command for it. The rules of the import itself are in
//! crates/omni-penpot/tests/import_synthetic.rs; the export these tests write
//! follows the same Penpot fields (crates/omni-penpot/tests/common/import.rs).
//!
//! The contract these tests pin down:
//!
//! - `omni import penpot <file> --project <id>` imports the export through a
//!   new change set of the project: the design doc (its pages and boards) and
//!   the project's design system doc (the tokens, with Penpot's themes Mörkt and
//!   Ljust as modes, and the components) each get a version in it, and the
//!   change set is left `open` or `in_review`, to be reviewed and merged like
//!   any other. Once the human approves it, it merges and both docs have a
//!   mainline version.
//! - With `--json` it prints one JSON object with `changeset_id`,
//!   `design_doc_id`, `design_system_doc_id` and `report`, the list of what
//!   could not be imported, each entry with `page`, `board`, `shape_id`,
//!   `name` and `reason`. Without `--json` it prints the change set's ID.
//! - The images of the export are uploaded as workspace attachments, and the
//!   design refers to them as `attachment:<id>`.
//! - A file that is not a Penpot export fails with one line on standard error
//!   that names the file, and leaves no change set behind.
//! - An import into a project whose design system doc already has a mainline
//!   version (a second import, after the first merged) adds its tokens and
//!   components to that design system and keeps the ones it already has.
//! - fixtures/penpot/tirebud-ordrar.penpot, the export with components,
//!   variants and swaps, imports through the API with every page, board,
//!   component, copy and override that the converter makes of it, and merges.

mod common;

use common::*;
use omni_api::Config;
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::future::Future;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us051_cli";
const TEST_BUCKET: &str = "omni-us051-cli";
const KEY_PREFIX: &str = "omniustest51";
const HUMAN_TOKEN: &str = "us051-cli-human-session-0123456789abcdef0123456789abcdef0123456789";
const AGENT: &str = "run-51/importer";

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
        s3_endpoint: std::env::var("OMNI_S3_ENDPOINT")
            .unwrap_or_else(|_| "http://rustfs:9000".to_string()),
        s3_access_key: std::env::var("OMNI_S3_ACCESS_KEY")
            .unwrap_or_else(|_| "omni-dev".to_string()),
        s3_secret_key: std::env::var("OMNI_S3_SECRET_KEY")
            .unwrap_or_else(|_| "omni-dev-secret".to_string()),
        s3_bucket: TEST_BUCKET.to_string(),
    }
}

async fn admin_connection() -> PgConnection {
    PgConnection::connect_with(&admin_options().database(TEST_DB))
        .await
        .expect("connect to the scratch database as the admin role")
}

struct Fixture {
    url: String,
    port: u16,
    workspace: String,
    secret: String,
    /// Keeps the API's runtime, and so the server, alive.
    _runtime: tokio::runtime::Runtime,
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
        });
        let secret = format!("{KEY_PREFIX}_{}", "0123456789abcdef".repeat(4));
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
                "insert into users (email) values ('hanna@example.test') returning id::text",
                &[],
            )
            .await;
            text(
                "insert into sessions (user_id, token_hash, expires_at) \
                 values ($1::uuid, sha256(convert_to($2, 'UTF8')), now() + interval '1 day') \
                 returning id::text",
                &[&user, HUMAN_TOKEN],
            )
            .await;
            text(
                "insert into memberships (workspace_id, user_id) \
                 values ($1::uuid, $2::uuid) returning id::text",
                &[&workspace, &user],
            )
            .await;
            text(
                "insert into api_keys (workspace_id, prefix, key_hash, scopes) \
                 values ($1::uuid, $2, encode(sha256(convert_to($3, 'UTF8')), 'hex'), '{*}') \
                 returning id::text",
                &[&workspace, KEY_PREFIX, &secret],
            )
            .await;
            workspace
        });

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
            let state = omni_api::AppState::new(&config()).expect("build the API state");
            runtime
                .block_on(state.ensure_bucket(&config(), 30))
                .expect("create the scratch bucket");
            omni_api::router(state)
        };
        runtime.spawn(async move {
            axum::serve(listener, router).await.expect("serve the API");
        });
        Fixture {
            url: format!("http://127.0.0.1:{port}"),
            port,
            workspace,
            secret,
            _runtime: runtime,
        }
    })
}

/// A home directory whose config file holds the agent's key, as `omni login`
/// leaves it.
fn agent_home(f: &Fixture) -> Scratch {
    let home = Scratch::new("agent");
    write_config(&home.path, &f.url, &f.workspace, &f.secret);
    home
}

fn run(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path).args(args).args(&["--as", AGENT]).run()
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn json_of(out: &Out, what: &str) -> Value {
    assert_eq!(out.code, 0, "{what}: {}", out.stderr);
    out.json()
}

/// A new project; returns its ID.
fn new_project(home: &Scratch) -> String {
    let created = json_of(
        &run(home, &["project", "create", &unique("butik"), "--json"]),
        "project create",
    );
    created["project"]["id"]
        .as_str()
        .expect("a project id")
        .to_string()
}

/// `omni api GET <path>` under the project; returns the answer.
fn get(f: &Fixture, home: &Scratch, project: &str, path: &str) -> Value {
    let full = format!("/api/v1/w/{}/projects/{project}{path}", f.workspace);
    json_of(&run(home, &["api", "GET", &full]), &format!("GET {full}"))
}

/// The answer of a request: its status, headers and body.
struct Fetched {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Fetched {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        let text = String::from_utf8_lossy(&self.body);
        // The API may send the body chunked; the JSON object is what lies
        // between the first `{` and the last `}`.
        match (text.find('{'), text.rfind('}')) {
            (Some(start), Some(end)) => {
                serde_json::from_str(&text[start..=end]).unwrap_or(Value::Null)
            }
            _ => Value::Null,
        }
    }
}

/// One request to the API with `auth` as its credentials header.
fn request(f: &Fixture, method: &str, path: &str, auth: &str, body: Option<&Value>) -> Fetched {
    let body = body.map(Value::to_string).unwrap_or_default();
    let mut stream = TcpStream::connect(("127.0.0.1", f.port)).expect("connect to the API");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n{auth}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("send request");
    let mut raw = Vec::new();
    // A reset after the response is complete is not an error for these tests.
    let _ = stream.read_to_end(&mut raw);
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no HTTP response: {:?}", String::from_utf8_lossy(&raw)));
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .expect("status line");
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
        .collect();
    Fetched {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

/// One request with the human's session cookie.
fn as_human(f: &Fixture, method: &str, path: &str, body: &Value) -> Fetched {
    request(
        f,
        method,
        path,
        &format!("Cookie: omni_session={HUMAN_TOKEN}"),
        Some(body),
    )
}

/// `GET path` with the agent's key; the body comes back as bytes.
fn fetch_as_agent(f: &Fixture, path: &str) -> Fetched {
    request(
        f,
        "GET",
        path,
        &format!(
            "Authorization: Bearer {}\r\nX-Omni-Run: run-51\r\nX-Omni-Role: importer",
            f.secret
        ),
        None,
    )
}

// The export ------------------------------------------------------------------------------

const FILE_ID: &str = "f0000000-0000-4000-8000-000000000051";
const ROOT_ID: &str = "00000000-0000-0000-0000-000000000000";
const PAGE_K: &str = "a0000000-0000-4000-8000-00000000005a";
const PAGE_S: &str = "a0000000-0000-4000-8000-00000000005b";
const C_NEUTRAL: &str = "c0000000-0000-4000-8000-000000000051";
const C_ACCENT: &str = "c0000000-0000-4000-8000-000000000052";
const V_BADGE: &str = "d0000000-0000-4000-8000-000000000051";
const MEDIA: &str = "e0000000-0000-4000-8000-000000000051";
const STORED: &str = "e0000000-0000-4000-8000-000000000052";

/// A 2 by 2 PNG: the first column red, the second blue.
const PNG: [u8; 74] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d,
    0x24, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x42, 0xff, 0x19, 0x60, 0x0c, 0x00, 0x43, 0xce, 0x07, 0xf9, 0x58, 0xed, 0x66, 0xa0, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// The id of the shape number `n`.
fn uid(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

/// A shape of the export, with the shapes inside it.
struct Shape {
    json: Value,
    kids: Vec<Shape>,
}

fn shape(n: u32, kind: &str, name: &str, [x, y, width, height]: [f64; 4], extra: Value) -> Shape {
    let mut json = json!({
        "id": uid(n),
        "name": name,
        "type": kind,
        "x": x,
        "y": y,
        "width": width,
        "height": height,
        "rotation": 0,
        "selrect": {
            "x": x, "y": y, "width": width, "height": height,
            "x1": x, "y1": y, "x2": x + width, "y2": y + height
        },
        "strokes": [],
        "fills": []
    });
    for (key, value) in extra.as_object().expect("an object") {
        json[key] = value.clone();
    }
    Shape {
        json,
        kids: Vec::new(),
    }
}

impl Shape {
    fn kids(mut self, kids: Vec<Shape>) -> Shape {
        self.kids = kids;
        self
    }
}

fn solid(hex: &str) -> Value {
    json!([{ "fillColor": hex, "fillOpacity": 1 }])
}

/// A text of one run of `text` in Inter 14, filled `hex`.
fn text(n: u32, name: &str, bounds: [f64; 4], text: &str, extra: Value) -> Shape {
    let run = json!({
        "fontId": "gfont-inter", "fontFamily": "Inter", "fontSize": "14",
        "fontWeight": "400", "fontVariantId": "regular", "fontStyle": "normal",
        "lineHeight": "1.2", "letterSpacing": "0", "textAlign": "left",
        "textDecoration": "none", "textTransform": "none", "textDirection": "ltr",
        "fills": [{ "fillColor": "#F4F4F5", "fillOpacity": 1 }],
        "text": text
    });
    let mut paragraph = run.clone();
    paragraph.as_object_mut().unwrap().remove("text");
    paragraph["type"] = json!("paragraph");
    paragraph["children"] = json!([run]);
    let mut all = json!({
        "growType": "fixed",
        "content": {
            "type": "root",
            "children": [{ "type": "paragraph-set", "children": [paragraph] }]
        }
    });
    for (key, value) in extra.as_object().expect("an object") {
        all[key] = value.clone();
    }
    shape(n, "text", name, bounds, all)
}

fn flatten(shape: Shape, parent: &str, out: &mut Vec<Value>) {
    let Shape { mut json, kids } = shape;
    json["parentId"] = json!(parent);
    json["frameId"] = json!(parent);
    let id = json["id"].as_str().unwrap().to_string();
    if !kids.is_empty() {
        json["shapes"] = json!(
            kids.iter()
                .map(|kid| kid.json["id"].clone())
                .collect::<Vec<_>>()
        );
    }
    out.push(json);
    for kid in kids {
        flatten(kid, &id, out);
    }
}

/// The archive entries of a page of the file `file` with `top_level` directly
/// on it.
fn page_entries(
    file: &str,
    id: &str,
    name: &str,
    index: u32,
    top_level: Vec<Shape>,
) -> Vec<(String, Vec<u8>)> {
    let dir = format!("files/{file}/pages/{id}");
    let mut entries = vec![(
        format!("{dir}.json"),
        json!({ "id": id, "name": name, "index": index })
            .to_string()
            .into_bytes(),
    )];
    let ids: Vec<Value> = top_level
        .iter()
        .map(|shape| shape.json["id"].clone())
        .collect();
    let mut root = shape(0, "frame", "Root Frame", [0.0, 0.0, 0.01, 0.01], json!({}));
    root.json["id"] = json!(ROOT_ID);
    root.json["parentId"] = json!(ROOT_ID);
    root.json["frameId"] = json!(ROOT_ID);
    root.json["shapes"] = json!(ids);
    entries.push((
        format!("{dir}/{ROOT_ID}.json"),
        root.json.to_string().into_bytes(),
    ));
    let mut shapes = Vec::new();
    for shape in top_level {
        flatten(shape, ROOT_ID, &mut shapes);
    }
    for shape in shapes {
        let id = shape["id"].as_str().unwrap().to_string();
        entries.push((format!("{dir}/{id}.json"), shape.to_string().into_bytes()));
    }
    entries
}

/// The main instance of a Badge variant, numbered `n` with its Label `n + 1`.
fn badge_main(n: u32, component: &str, variant: &str, fill: &str, x: f64) -> Shape {
    shape(
        n,
        "frame",
        "Badge",
        [x, 10.0, 80.0, 24.0],
        json!({
            "fills": solid(fill),
            "componentRoot": true,
            "mainInstance": true,
            "componentId": component,
            "componentFile": FILE_ID,
            "variantId": V_BADGE,
            "variantName": variant,
            "touched": []
        }),
    )
    .kids(vec![text(
        n + 1,
        "Label",
        [x + 8.0, 14.0, 64.0, 16.0],
        "Standard",
        json!({}),
    )])
}

/// The synthetic export: the variant set Badge (Neutral and Accent) on the
/// page Komponenter, and on the page Start the board Hem, filled with the
/// token `color.surface`, holding the text Rubrik (the token `color.text`), the
/// copy Märke of Badge / Accent whose Label says "Ny", the image Bild and the
/// boolean shape Union, which cannot be imported; the text Anteckning sits on
/// the page outside every board.
fn export_bytes() -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        (
            "manifest.json".to_string(),
            json!({
                "type": "penpot/export-files",
                "version": 1,
                "generatedBy": "penpot/2.19.0-RC1",
                "referer": "penpot",
                "files": [{ "id": FILE_ID, "name": "Butik", "features": ["components/v2", "variants/v1", "design-tokens/v1"] }],
                "relations": []
            })
            .to_string()
            .into_bytes(),
        ),
        (
            format!("files/{FILE_ID}.json"),
            json!({ "id": FILE_ID, "name": "Butik", "version": 67 }).to_string().into_bytes(),
        ),
    ];
    let markers = shape(
        1,
        "frame",
        "Märken",
        [0.0, 0.0, 300.0, 100.0],
        json!({ "fills": solid("#131316") }),
    )
    .kids(vec![
        shape(
            2,
            "frame",
            "Badge",
            [0.0, 0.0, 200.0, 44.0],
            json!({ "isVariantContainer": true }),
        )
        .kids(vec![
            badge_main(3, C_NEUTRAL, "Neutral", "#36363E", 10.0),
            badge_main(6, C_ACCENT, "Accent", "#01B597", 100.0),
        ]),
    ]);
    entries.extend(page_entries(
        FILE_ID,
        PAGE_K,
        "Komponenter",
        0,
        vec![markers],
    ));
    let copy = shape(
        12,
        "frame",
        "Märke",
        [20.0, 60.0, 80.0, 24.0],
        json!({
            "fills": solid("#01B597"),
            "componentRoot": true,
            "componentId": C_ACCENT,
            "componentFile": FILE_ID,
            "shapeRef": uid(6),
            "touched": []
        }),
    )
    .kids(vec![text(
        13,
        "Label",
        [28.0, 64.0, 64.0, 16.0],
        "Ny",
        json!({ "shapeRef": uid(7), "touched": ["text-content-text"] }),
    )]);
    let hem = shape(
        10,
        "frame",
        "Hem",
        [0.0, 0.0, 400.0, 300.0],
        json!({ "fills": solid("#1C1C21"), "appliedTokens": { "fill": "color.surface" } }),
    )
    .kids(vec![
        text(
            11,
            "Rubrik",
            [20.0, 20.0, 200.0, 24.0],
            "Välkommen",
            json!({ "appliedTokens": { "fill": "color.text" } }),
        ),
        copy,
        shape(
            14,
            "rect",
            "Bild",
            [220.0, 60.0, 100.0, 100.0],
            json!({ "fills": [{
                "fillImage": {
                    "id": MEDIA, "name": "picture.png", "width": 2, "height": 2,
                    "mtype": "image/png", "keepAspectRatio": true
                },
                "fillOpacity": 1
            }] }),
        ),
        shape(
            15,
            "bool",
            "Union",
            [220.0, 180.0, 40.0, 40.0],
            json!({ "fills": solid("#FF0000") }),
        ),
    ]);
    let note = text(
        20,
        "Anteckning",
        [500.0, 20.0, 120.0, 16.0],
        "Utkast",
        json!({}),
    );
    entries.extend(page_entries(FILE_ID, PAGE_S, "Start", 1, vec![hem, note]));
    let tokens = json!({
        "Bas": { "space": { "4": { "$value": "16", "$type": "spacing", "$description": "" } } },
        "Mörkt": { "color": {
            "surface": { "$value": "#1C1C21", "$type": "color", "$description": "" },
            "text": { "$value": "#F4F4F5", "$type": "color", "$description": "" }
        } },
        "Ljust": { "color": {
            "surface": { "$value": "#FFFFFF", "$type": "color", "$description": "" },
            "text": { "$value": "#18181B", "$type": "color", "$description": "" }
        } },
        "$themes": [
            { "id": "f1000000-0000-4000-8000-000000000051", "name": "Mörkt", "group": "Tema",
              "description": "", "isSource": false,
              "selectedTokenSets": { "Mörkt": "enabled", "Bas": "enabled" } },
            { "id": "f1000000-0000-4000-8000-000000000052", "name": "Ljust", "group": "Tema",
              "description": "", "isSource": false,
              "selectedTokenSets": { "Ljust": "enabled", "Bas": "enabled" } }
        ],
        "$metadata": {
            "tokenSetOrder": ["Bas", "Mörkt", "Ljust"],
            "activeThemes": ["Tema/Mörkt"],
            "activeSets": ["Mörkt", "Bas"]
        }
    });
    entries.push((
        format!("files/{FILE_ID}/tokens.json"),
        tokens.to_string().into_bytes(),
    ));
    for (component, main, value) in [(C_NEUTRAL, 3, "Neutral"), (C_ACCENT, 6, "Accent")] {
        entries.push((
            format!("files/{FILE_ID}/components/{component}.json"),
            json!({
                "id": component,
                "name": "Badge",
                "path": "",
                "mainInstanceId": uid(main),
                "mainInstancePage": PAGE_K,
                "variantId": V_BADGE,
                "variantProperties": [{ "name": "Typ", "value": value }]
            })
            .to_string()
            .into_bytes(),
        ));
    }
    entries.push((
        format!("files/{FILE_ID}/media/{MEDIA}.json"),
        json!({
            "id": MEDIA, "name": "picture.png", "width": 2, "height": 2,
            "mtype": "image/png", "mediaId": STORED
        })
        .to_string()
        .into_bytes(),
    ));
    entries.push((
        format!("objects/{STORED}.json"),
        json!({ "id": STORED, "bucket": "file-media-object", "contentType": "image/png", "size": PNG.len() })
            .to_string()
            .into_bytes(),
    ));
    entries.push((format!("objects/{STORED}.png"), PNG.to_vec()));
    stored_zip(&entries)
}

/// Writes the synthetic export into `home` and returns its path.
fn write_export(home: &Scratch) -> PathBuf {
    let path = home.join("butik.penpot");
    std::fs::write(&path, export_bytes()).expect("write the export");
    path
}

const FILE_KASSA: &str = "f0000000-0000-4000-8000-000000000052";
const PAGE_KASSA: &str = "a0000000-0000-4000-8000-00000000005c";
const C_KNAPP: &str = "c0000000-0000-4000-8000-000000000053";

/// A second synthetic export, of another file of the same product: the
/// component Knapp on the board Knappar, and the board Kassa, filled with the
/// token `color.border`, holding the copy Köpknapp whose Label says "Betala".
/// Its tokens have the same themes as the first export's and the same
/// `color.surface`, but a new token `color.border` and no `color.text`.
fn kassa_export_bytes() -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        (
            "manifest.json".to_string(),
            json!({
                "type": "penpot/export-files",
                "version": 1,
                "generatedBy": "penpot/2.19.0-RC1",
                "referer": "penpot",
                "files": [{ "id": FILE_KASSA, "name": "Kassa", "features": ["components/v2", "variants/v1", "design-tokens/v1"] }],
                "relations": []
            })
            .to_string()
            .into_bytes(),
        ),
        (
            format!("files/{FILE_KASSA}.json"),
            json!({ "id": FILE_KASSA, "name": "Kassa", "version": 67 })
                .to_string()
                .into_bytes(),
        ),
    ];
    let buttons = shape(
        30,
        "frame",
        "Knappar",
        [0.0, 0.0, 200.0, 100.0],
        json!({ "fills": solid("#131316") }),
    )
    .kids(vec![
        shape(
            31,
            "frame",
            "Knapp",
            [10.0, 10.0, 120.0, 40.0],
            json!({
                "fills": solid("#01B597"),
                "componentRoot": true,
                "mainInstance": true,
                "componentId": C_KNAPP,
                "componentFile": FILE_KASSA,
                "touched": []
            }),
        )
        .kids(vec![text(
            32,
            "Label",
            [18.0, 22.0, 104.0, 16.0],
            "Köp",
            json!({}),
        )]),
    ]);
    let checkout = shape(
        33,
        "frame",
        "Kassa",
        [300.0, 0.0, 400.0, 300.0],
        json!({ "fills": solid("#3F3F46"), "appliedTokens": { "fill": "color.border" } }),
    )
    .kids(vec![
        shape(
            34,
            "frame",
            "Köpknapp",
            [320.0, 20.0, 120.0, 40.0],
            json!({
                "fills": solid("#01B597"),
                "componentRoot": true,
                "componentId": C_KNAPP,
                "componentFile": FILE_KASSA,
                "shapeRef": uid(31),
                "touched": []
            }),
        )
        .kids(vec![text(
            35,
            "Label",
            [328.0, 32.0, 104.0, 16.0],
            "Betala",
            json!({ "shapeRef": uid(32), "touched": ["text-content-text"] }),
        )]),
    ]);
    entries.extend(page_entries(
        FILE_KASSA,
        PAGE_KASSA,
        "Kassa",
        0,
        vec![buttons, checkout],
    ));
    let tokens = json!({
        "Bas": { "space": { "4": { "$value": "16", "$type": "spacing", "$description": "" } } },
        "Mörkt": { "color": {
            "surface": { "$value": "#1C1C21", "$type": "color", "$description": "" },
            "border": { "$value": "#3F3F46", "$type": "color", "$description": "" }
        } },
        "Ljust": { "color": {
            "surface": { "$value": "#FFFFFF", "$type": "color", "$description": "" },
            "border": { "$value": "#E4E4E7", "$type": "color", "$description": "" }
        } },
        "$themes": [
            { "id": "f1000000-0000-4000-8000-000000000051", "name": "Mörkt", "group": "Tema",
              "description": "", "isSource": false,
              "selectedTokenSets": { "Mörkt": "enabled", "Bas": "enabled" } },
            { "id": "f1000000-0000-4000-8000-000000000052", "name": "Ljust", "group": "Tema",
              "description": "", "isSource": false,
              "selectedTokenSets": { "Ljust": "enabled", "Bas": "enabled" } }
        ],
        "$metadata": {
            "tokenSetOrder": ["Bas", "Mörkt", "Ljust"],
            "activeThemes": ["Tema/Mörkt"],
            "activeSets": ["Mörkt", "Bas"]
        }
    });
    entries.push((
        format!("files/{FILE_KASSA}/tokens.json"),
        tokens.to_string().into_bytes(),
    ));
    entries.push((
        format!("files/{FILE_KASSA}/components/{C_KNAPP}.json"),
        json!({
            "id": C_KNAPP,
            "name": "Knapp",
            "path": "",
            "mainInstanceId": uid(31),
            "mainInstancePage": PAGE_KASSA
        })
        .to_string()
        .into_bytes(),
    ));
    stored_zip(&entries)
}

/// Writes the second synthetic export into `home` and returns its path.
fn write_kassa_export(home: &Scratch) -> PathBuf {
    let path = home.join("kassa.penpot");
    std::fs::write(&path, kassa_export_bytes()).expect("write the export");
    path
}

/// A zip archive of `entries` with every file stored uncompressed.
fn stored_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in entries {
        let offset = archive.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        // Local file header.
        archive.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        archive.extend_from_slice(&20u16.to_le_bytes());
        archive.extend_from_slice(&[0; 2 + 2 + 2]);
        archive.extend_from_slice(&33u16.to_le_bytes());
        archive.extend_from_slice(&crc.to_le_bytes());
        archive.extend_from_slice(&size.to_le_bytes());
        archive.extend_from_slice(&size.to_le_bytes());
        archive.extend_from_slice(&(name.len() as u16).to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(data);
        // Central directory header.
        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&[0; 2 + 2 + 2]);
        directory.extend_from_slice(&33u16.to_le_bytes());
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&[0; 2 + 2 + 2 + 2 + 4]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let directory_offset = archive.len() as u32;
    let directory_size = directory.len() as u32;
    archive.extend_from_slice(&directory);
    // End of central directory record.
    archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    archive.extend_from_slice(&[0; 2 + 2]);
    archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    archive.extend_from_slice(&directory_size.to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

// The import ------------------------------------------------------------------------------

/// What `omni import penpot --json` printed.
struct Imported {
    project: String,
    changeset: String,
    design_doc: String,
    design_system_doc: String,
    answer: Value,
}

fn import(home: &Scratch, project: &str, file: &Path) -> Imported {
    let out = run(
        home,
        &[
            "import",
            "penpot",
            file.to_str().expect("a path"),
            "--project",
            project,
            "--json",
        ],
    );
    let answer = json_of(&out, "omni import penpot");
    let text = |key: &str| {
        answer[key]
            .as_str()
            .unwrap_or_else(|| panic!("the answer has {key}: {answer}"))
            .to_string()
    };
    Imported {
        project: project.to_string(),
        changeset: text("changeset_id"),
        design_doc: text("design_doc_id"),
        design_system_doc: text("design_system_doc_id"),
        answer,
    }
}

fn status(home: &Scratch, imported: &Imported) -> String {
    let shown = json_of(
        &run(
            home,
            &[
                "changeset",
                "show",
                &imported.changeset,
                "--project",
                &imported.project,
                "--json",
            ],
        ),
        "changeset show",
    );
    shown["changeset"]["status"]
        .as_str()
        .expect("a status")
        .to_string()
}

/// The change set's draft of the design doc: its pages and the components
/// its instances use.
fn design_draft(f: &Fixture, home: &Scratch, imported: &Imported) -> Value {
    get(
        f,
        home,
        &imported.project,
        &format!(
            "/changesets/{}/docs/{}",
            imported.changeset, imported.design_doc
        ),
    )["draft"]
        .clone()
}

/// The JSON node named `name` in `root` or below it.
fn find<'a>(root: &'a Value, name: &str) -> Option<&'a Value> {
    if root["name"] == name {
        return Some(root);
    }
    root["children"]
        .as_array()?
        .iter()
        .find_map(|child| find(child, name))
}

fn page<'a>(pages: &'a Value, name: &str) -> &'a Value {
    pages
        .as_array()
        .and_then(|pages| pages.iter().find(|page| page["name"] == name))
        .unwrap_or_else(|| panic!("no page {name}: {pages}"))
}

fn board_names(page: &Value) -> Vec<&str> {
    page["children"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|board| board["name"].as_str())
        .collect()
}

/// Every `attachment:<id>` that a JSON value mentions.
fn attachment_ids(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if let Some(id) = text.strip_prefix("attachment:") {
                found.push(id.to_string());
            }
        }
        Value::Array(items) => items.iter().for_each(|item| attachment_ids(item, found)),
        Value::Object(map) => map.values().for_each(|item| attachment_ids(item, found)),
        _ => {}
    }
}

fn lowercase(value: &Value) -> String {
    value.as_str().unwrap_or("").to_lowercase()
}

#[test]
fn import_penpot_puts_the_design_and_its_design_system_into_one_change_set() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let imported = import(&home, &project, &write_export(&home));
    assert!(
        ["open", "in_review"].contains(&status(&home, &imported).as_str()),
        "the import waits for review"
    );

    let tokens = get(
        f,
        &home,
        &project,
        &format!("/changesets/{}/tokens", imported.changeset),
    );
    assert_eq!(
        tokens["doc_id"],
        imported.design_system_doc.as_str(),
        "{tokens}"
    );
    assert_eq!(tokens["modes"], json!(["Mörkt", "Ljust"]), "{tokens}");
    let surface = &tokens["tokens"]["color"]["surface"]["$extensions"]["omnistate"]["modes"];
    assert_eq!(lowercase(&surface["Mörkt"]), "#1c1c21", "{tokens}");
    assert_eq!(lowercase(&surface["Ljust"]), "#ffffff", "{tokens}");

    let draft = design_draft(f, &home, &imported);
    let start = page(&draft["pages"], "Start");
    assert_eq!(board_names(start), ["Hem", "Unplaced"], "{start}");
    assert_eq!(
        board_names(page(&draft["pages"], "Komponenter")),
        ["Märken"]
    );
    let hem = find(start, "Hem").expect("the board Hem");
    assert_eq!(hem["fills"][0]["color"], "$color.surface", "{hem}");
    assert_eq!(find(hem, "Rubrik").expect("Rubrik")["color"], "$color.text");
    let badge = find(hem, "Märke").expect("the copy Märke");
    assert_eq!(badge["type"], "instance", "{badge}");
    let component = badge["component"].as_str().expect("a component");
    let components = draft["components"]
        .as_array()
        .expect("the components it uses");
    assert!(
        components
            .iter()
            .any(|known| known["id"] == component && known["name"] == "Badge"),
        "{badge} places a Badge of the design system: {components:?}"
    );
    let overrides = badge["overrides"].as_object().expect("overrides");
    assert!(
        overrides.values().any(|change| change["content"] == "Ny"),
        "the text of the copy: {badge}"
    );
}

#[test]
fn the_import_is_reviewed_and_merged_like_any_other_change() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let imported = import(&home, &project, &write_export(&home));
    if status(&home, &imported) == "open" {
        let submitted = run(
            &home,
            &[
                "changeset",
                "submit",
                &imported.changeset,
                "--project",
                &project,
            ],
        );
        assert_eq!(submitted.code, 0, "submit: {}", submitted.stderr);
    }
    assert_eq!(status(&home, &imported), "in_review");
    let approved = as_human(
        f,
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/changesets/{}/approve",
            f.workspace, imported.changeset
        ),
        &json!({}),
    );
    assert_eq!(
        approved.status,
        200,
        "the human approves: {:?}",
        String::from_utf8_lossy(&approved.body)
    );
    assert_eq!(status(&home, &imported), "merged", "{}", approved.json());

    let tokens = get(f, &home, &project, "/tokens");
    assert!(tokens["version_id"].is_string(), "{tokens}");
    assert_eq!(tokens["modes"], json!(["Mörkt", "Ljust"]), "{tokens}");
    let doc = get(
        f,
        &home,
        &project,
        &format!("/docs/{}", imported.design_doc),
    );
    let version = doc["doc"]["mainline_version_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the design doc has a mainline version: {doc}"));
    let version = &get(f, &home, &project, &format!("/versions/{version}"))["version"];
    assert!(version["design_system_version_id"].is_string(), "{version}");
    assert_eq!(
        board_names(page(&version["pages"], "Start")),
        ["Hem", "Unplaced"]
    );
    let system = get(
        f,
        &home,
        &project,
        &format!("/docs/{}", imported.design_system_doc),
    );
    assert!(
        system["doc"]["mainline_version_id"].is_string(),
        "the design system doc has a mainline version: {system}"
    );
}

#[test]
fn images_of_the_export_become_workspace_attachments() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let imported = import(&home, &project, &write_export(&home));
    let draft = design_draft(f, &home, &imported);
    let picture = find(page(&draft["pages"], "Start"), "Bild").expect("the image Bild");
    let mut ids = Vec::new();
    attachment_ids(picture, &mut ids);
    assert_eq!(ids.len(), 1, "{picture}");
    let served = fetch_as_agent(
        f,
        &format!("/api/v1/w/{}/attachments/{}", f.workspace, ids[0]),
    );
    assert_eq!(
        served.status,
        200,
        "{:?}",
        String::from_utf8_lossy(&served.body)
    );
    assert_eq!(served.body, PNG, "the image of the export, byte for byte");
    assert_eq!(served.header("content-type"), Some("image/png"));
}

#[test]
fn what_cannot_be_imported_is_listed_in_the_report() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let imported = import(&home, &project, &write_export(&home));
    let report = imported.answer["report"].as_array().expect("a report");
    assert!(
        report
            .iter()
            .any(|entry| entry["shape_id"] == uid(15).as_str()
                && entry["page"] == "Start"
                && entry["board"] == "Hem"
                && entry["name"] == "Union"
                && !entry["reason"].as_str().unwrap_or("").is_empty()),
        "the boolean shape Union: {report:?}"
    );

    // Without --json it prints the change set to review.
    let out = run(
        &home,
        &[
            "import",
            "penpot",
            write_export(&home).to_str().unwrap(),
            "--project",
            &project,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let listed = json_of(
        &run(
            &home,
            &["changeset", "list", "--project", &project, "--json"],
        ),
        "changeset list",
    );
    let ids: Vec<&str> = listed["changesets"]
        .as_array()
        .expect("change sets")
        .iter()
        .filter_map(|changeset| changeset["id"].as_str())
        .filter(|id| *id != imported.changeset)
        .collect();
    assert_eq!(ids.len(), 1, "a second change set: {listed}");
    assert!(out.stdout.contains(ids[0]), "{:?}", out.stdout);
}

#[test]
fn a_file_that_is_no_penpot_export_leaves_no_change_set() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let file = home.join("anteckning.penpot");
    std::fs::write(&file, "inte en export").unwrap();
    let out = run(
        &home,
        &[
            "import",
            "penpot",
            file.to_str().unwrap(),
            "--project",
            &project,
        ],
    );
    assert_eq!(out.code, 1, "{:?}", out.stdout);
    assert_eq!(out.stdout, "");
    let lines = out.stderr_lines();
    assert_eq!(lines.len(), 1, "{:?}", out.stderr);
    assert!(lines[0].contains("anteckning.penpot"), "{:?}", lines[0]);
    let listed = json_of(
        &run(
            &home,
            &["changeset", "list", "--project", &project, "--json"],
        ),
        "changeset list",
    );
    assert_eq!(listed["changesets"], json!([]), "{listed}");
}

#[test]
fn the_tirebud_idag_export_imports() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/penpot/tirebud-idag.penpot");
    if !path.is_file() {
        eprintln!("SKIPPED: fixtures/penpot/tirebud-idag.penpot is not there");
        return;
    }
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let imported = import(&home, &project, &path);
    let tokens = get(
        f,
        &home,
        &project,
        &format!("/changesets/{}/tokens", imported.changeset),
    );
    assert_eq!(tokens["modes"], json!(["Mörkt", "Ljust"]));
    let draft = design_draft(f, &home, &imported);
    let boards = board_names(page(&draft["pages"], "Idag"));
    assert_eq!(boards.len(), 16, "15 boards and Unplaced: {boards:?}");
    for name in ["Idag / Dag / Desktop", "Idag / Dag / Telefon", "Unplaced"] {
        assert!(boards.contains(&name), "{name} in {boards:?}");
    }
}

/// Submits the imported change set if it is still open, has the human approve
/// it and checks that it merged.
fn merge(f: &Fixture, home: &Scratch, imported: &Imported) {
    if status(home, imported) == "open" {
        let submitted = run(
            home,
            &[
                "changeset",
                "submit",
                &imported.changeset,
                "--project",
                &imported.project,
            ],
        );
        assert_eq!(submitted.code, 0, "submit: {}", submitted.stderr);
    }
    let approved = as_human(
        f,
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{}/changesets/{}/approve",
            f.workspace, imported.project, imported.changeset
        ),
        &json!({}),
    );
    assert_eq!(
        approved.status,
        200,
        "the human approves: {:?}",
        String::from_utf8_lossy(&approved.body)
    );
    assert_eq!(status(home, imported), "merged", "{}", approved.json());
}

/// The components of the design system version that the import saved.
fn saved_components(f: &Fixture, home: &Scratch, imported: &Imported) -> Vec<Value> {
    let version = imported.answer["design_system_version_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the import saved the design system: {}", imported.answer));
    get(f, home, &imported.project, &format!("/versions/{version}"))["version"]["components"]
        .as_array()
        .unwrap_or_else(|| panic!("the version {version} lists its components"))
        .clone()
}

/// The values of the token at `path` in each mode of a token tree.
fn token_modes<'a>(tokens: &'a Value, path: &[&str]) -> &'a Value {
    let mut token = &tokens["tokens"];
    for part in path {
        token = &token[*part];
    }
    &token["$extensions"]["omnistate"]["modes"]
}

#[test]
fn a_second_import_keeps_the_design_system_that_the_project_has() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let first = import(&home, &project, &write_export(&home));
    merge(f, &home, &first);

    // The design system doc now has a mainline version; the second import adds
    // to it and takes nothing of it away.
    let second = import(&home, &project, &write_kassa_export(&home));
    assert_eq!(
        second.design_system_doc, first.design_system_doc,
        "a project has one design system doc"
    );
    let tokens = get(
        f,
        &home,
        &project,
        &format!("/changesets/{}/tokens", second.changeset),
    );
    assert_eq!(tokens["modes"], json!(["Mörkt", "Ljust"]), "{tokens}");
    let kept = token_modes(&tokens, &["color", "text"]);
    assert_eq!(
        (lowercase(&kept["Mörkt"]), lowercase(&kept["Ljust"])),
        ("#f4f4f5".to_string(), "#18181b".to_string()),
        "the token color.text of the first import stays: {tokens}"
    );
    let added = token_modes(&tokens, &["color", "border"]);
    assert_eq!(
        (lowercase(&added["Mörkt"]), lowercase(&added["Ljust"])),
        ("#3f3f46".to_string(), "#e4e4e7".to_string()),
        "the token color.border of the second import: {tokens}"
    );

    let components = saved_components(f, &home, &second);
    let named = |name: &str| {
        components
            .iter()
            .filter(|component| component["name"] == name)
            .count()
    };
    assert_eq!(
        (named("Badge"), named("Knapp")),
        (2, 1),
        "the two Badge variants of the first import stay beside Knapp: {components:?}"
    );

    let draft = design_draft(f, &home, &second);
    let kassa = page(&draft["pages"], "Kassa");
    // The page is named Kassa too, so look the board up among its children.
    let board = kassa["children"]
        .as_array()
        .and_then(|boards| boards.iter().find_map(|child| find(child, "Kassa")))
        .expect("the board Kassa");
    assert_eq!(board["fills"][0]["color"], "$color.border", "{board}");
    let copy = find(board, "Köpknapp").expect("the copy Köpknapp");
    assert_eq!(copy["type"], "instance", "{copy}");
    assert!(
        components
            .iter()
            .any(|known| known["id"] == copy["component"] && known["name"] == "Knapp"),
        "{copy} places Knapp: {components:?}"
    );

    merge(f, &home, &second);
    let mainline = get(f, &home, &project, "/tokens");
    for path in [["color", "text"], ["color", "border"], ["color", "surface"]] {
        assert!(
            token_modes(&mainline, &path)["Ljust"].is_string(),
            "{path:?} is on the mainline: {mainline}"
        );
    }
    let system = get(
        f,
        &home,
        &project,
        &format!("/docs/{}", first.design_system_doc),
    );
    let version = system["doc"]["mainline_version_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the design system has a mainline version: {system}"));
    let on_mainline =
        &get(f, &home, &project, &format!("/versions/{version}"))["version"]["components"];
    let names: Vec<&str> = on_mainline
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|component| component["name"].as_str())
        .collect();
    assert_eq!(
        (
            names.iter().filter(|name| **name == "Badge").count(),
            names.iter().filter(|name| **name == "Knapp").count()
        ),
        (2, 1),
        "the merged design system: {names:?}"
    );
}

/// Every node of type `kind` in `value` and below it.
fn nodes_of<'a>(value: &'a Value, kind: &str, found: &mut Vec<&'a Value>) {
    if value["type"] == kind {
        found.push(value);
    }
    for child in value["children"].as_array().into_iter().flatten() {
        nodes_of(child, kind, found);
    }
}

/// The override kinds and how many overrides the instances in `pages` have.
fn overrides_of(instances: &[&Value]) -> (std::collections::BTreeSet<String>, usize) {
    let mut kinds = std::collections::BTreeSet::new();
    let mut count = 0;
    for instance in instances {
        for change in instance["overrides"]
            .as_object()
            .into_iter()
            .flat_map(|overrides| overrides.values())
        {
            count += 1;
            for field in change.as_object().into_iter().flat_map(|c| c.keys()) {
                kinds.insert(field.clone());
            }
        }
    }
    (kinds, count)
}

#[test]
fn the_tirebud_ordrar_export_imports_with_its_components_and_merges() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/penpot/tirebud-ordrar.penpot");
    if !path.is_file() {
        eprintln!("SKIPPED: fixtures/penpot/tirebud-ordrar.penpot is not there");
        return;
    }
    // What the converter makes of the export, to compare the API's drafts with.
    let library = omni_penpot::import::import_file(&path).expect("the converter reads ordrar");
    let library_pages: Vec<Value> = library
        .pages
        .iter()
        .map(|page| serde_json::to_value(page).expect("a page as JSON"))
        .collect();
    let mut library_instances = Vec::new();
    for page in &library_pages {
        nodes_of(page, "instance", &mut library_instances);
    }
    let (library_kinds, library_overrides) = overrides_of(&library_instances);
    assert_eq!(
        library_kinds,
        ["component", "content", "fills", "visible"]
            .map(String::from)
            .into_iter()
            .collect(),
        "text, fills, visibility and swapped components"
    );

    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let imported = import(&home, &project, &path);
    let report = imported.answer["report"].as_array().expect("a report");
    assert_eq!(
        report.len(),
        library.report.len(),
        "the report lists all the converter leaves out"
    );
    let tokens = get(
        f,
        &home,
        &project,
        &format!("/changesets/{}/tokens", imported.changeset),
    );
    assert_eq!(tokens["modes"], json!(["Mörkt", "Ljust"]));

    let components = saved_components(f, &home, &imported);
    assert_eq!(
        components.len(),
        library.component_count(),
        "every component of the export"
    );
    let variants = |all: &[Value]| {
        all.iter()
            .filter(|component| {
                component["props"]
                    .as_object()
                    .is_some_and(|props| props.values().any(|def| def["type"] == "variant"))
            })
            .count()
    };
    assert_eq!(
        variants(&components),
        variants(
            library.design_system["$components"]
                .as_array()
                .expect("the converter's components")
        ),
        "the variants keep their properties"
    );
    let known: std::collections::HashSet<&str> = components
        .iter()
        .filter_map(|component| component["id"].as_str())
        .collect();

    let draft = design_draft(f, &home, &imported);
    let pages = draft["pages"].as_array().expect("pages");
    assert_eq!(
        pages
            .iter()
            .map(|page| (page["name"].clone(), board_names(page).len()))
            .collect::<Vec<_>>(),
        library_pages
            .iter()
            .map(|page| (page["name"].clone(), board_names(page).len()))
            .collect::<Vec<_>>(),
        "every page with every board"
    );
    let mut instances = Vec::new();
    for page in pages {
        nodes_of(page, "instance", &mut instances);
    }
    assert_eq!(instances.len(), library_instances.len(), "every copy");
    let (kinds, overrides) = overrides_of(&instances);
    assert_eq!(kinds, library_kinds);
    assert_eq!(overrides, library_overrides, "every override of the copies");
    for instance in &instances {
        let component = instance["component"].as_str().unwrap_or("");
        assert!(
            known.contains(component),
            "{} places a component of the design system",
            instance["id"]
        );
        for change in instance["overrides"]
            .as_object()
            .into_iter()
            .flat_map(|overrides| overrides.values())
        {
            if let Some(swapped) = change.get("component") {
                assert!(
                    known.contains(swapped.as_str().unwrap_or("")),
                    "{} swaps in a component of the design system: {change}",
                    instance["id"]
                );
            }
        }
    }

    merge(f, &home, &imported);
    let doc = get(
        f,
        &home,
        &project,
        &format!("/docs/{}", imported.design_doc),
    );
    let version = doc["doc"]["mainline_version_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the design doc has a mainline version: {doc}"));
    let version = &get(f, &home, &project, &format!("/versions/{version}"))["version"];
    assert!(version["design_system_version_id"].is_string(), "{version}");
}
