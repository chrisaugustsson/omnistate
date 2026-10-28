//! US-051, US-105: `omni import penpot <file.penpot> --project <id> [--feature
//! <name>]` against the real API. Run with `--features stack-tests` in the
//! Compose test container, which reaches Postgres at `postgres:5432` and
//! RustFS at `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key (the
//! agent) and a user with a live session (the human), create a scratch RustFS
//! bucket and serve the API's router on a local port against both. The built
//! `omni` runs against that address with the key in its config file. The rules
//! of the import itself are in crates/omni-penpot/tests/import_synthetic.rs;
//! the export these tests write follows the same Penpot fields
//! (crates/omni-penpot/tests/common/import.rs).
//!
//! The contract these tests pin down (US-105: the import writes the live docs,
//! as an agent's edits do since US-102..US-104):
//!
//! - `omni import penpot <file> --project <id>` writes the export straight into
//!   the project, with no change set: the project has no change set after it.
//!   The design doc (kind `design`, pillar `ui`, at the app level) holds the
//!   pages and boards in its head, and the project's design system doc (kind
//!   `design-system`, pillar `ui`, always at the app level) holds the tokens,
//!   with Penpot's themes Mörkt and Ljust as modes, and the components in its
//!   head. Both read back at once: `GET /docs/{id}/html` of the design answers
//!   its pages and the components its copies place, every board of it reads
//!   with `GET /docs/{id}/boards/{board}`, `GET /tokens` answers the design
//!   system's tokens, and the design system's head read (`GET
//!   /docs/{id}/html`) answers its head text, the token tree with the
//!   components under `$components` (docs/formats/tokens.md).
//! - With `--json` it prints one JSON object with `design_doc_id`,
//!   `design_system_doc_id` and `report`, the list of what could not be
//!   imported, each entry with `page`, `board`, `shape_id`, `name` and
//!   `reason`, and no `changeset_id`. Without `--json` it prints the design
//!   doc's ID.
//! - The images of the export are uploaded as workspace attachments, and the
//!   design refers to them as `attachment:<id>`.
//! - A file that is not a Penpot export fails with one line on standard error
//!   that names the file, and leaves no doc and no change set behind.
//! - An import into a project that already has a design system (one an earlier
//!   import wrote, or one an agent made with `PUT /tokens` and `PUT
//!   /components`) writes into that same doc: its tokens and components stay,
//!   with the IDs they had, the export's are added, and a token of the same
//!   path takes the export's value.
//! - With `--feature <name>` the design doc lands in that feature's UI pillar,
//!   and the tokens and components land in the project's app-level design
//!   system.
//! - The fonts its texts use, when their files sit beside the export in
//!   `fonts/<family>/` (US-057), are uploaded as workspace attachments too, and
//!   the design system's typography tokens of that family list them as
//!   `attachment:<id>` in `$extensions.omnistate.fontFiles`, where the design
//!   viewer finds them.
//! - fixtures/penpot/tirebud-idag.penpot imports into an empty project and
//!   every one of its boards reads back; fixtures/penpot/tirebud-ordrar.penpot,
//!   the export with components, variants and swaps, imports with every page,
//!   board, component, copy and override that the converter makes of it.

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

/// A real font file, which stands for Geist in the export with fonts.
const FONT: &[u8] = include_bytes!("../../omni-render/fonts/Inter-Regular.ttf");

/// An export whose board Typsnitt holds the text Brödtext in Geist, with the
/// library typography Body in Geist, written into its own folder in `home`
/// with `fonts/Geist/Geist-Regular.ttf` ([`FONT`]) beside it.
fn write_font_export(home: &Scratch) -> PathBuf {
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        (
            "manifest.json".to_string(),
            json!({
                "type": "penpot/export-files",
                "version": 1,
                "generatedBy": "penpot/2.19.0-RC1",
                "referer": "penpot",
                "files": [{ "id": FILE_ID, "name": "Typsnitt", "features": ["components/v2", "variants/v1", "design-tokens/v1"] }],
                "relations": []
            })
            .to_string()
            .into_bytes(),
        ),
        (
            format!("files/{FILE_ID}.json"),
            json!({ "id": FILE_ID, "name": "Typsnitt", "version": 67 }).to_string().into_bytes(),
        ),
    ];
    let mut body_text = text(
        31,
        "Brödtext",
        [20.0, 20.0, 200.0, 24.0],
        "Dina däck",
        json!({}),
    );
    // The paragraph and its run are in Geist.
    let paragraph = &mut body_text.json["content"]["children"][0]["children"][0];
    paragraph["fontFamily"] = json!("Geist");
    paragraph["fontId"] = json!("gfont-geist");
    paragraph["children"][0]["fontFamily"] = json!("Geist");
    paragraph["children"][0]["fontId"] = json!("gfont-geist");
    let board = shape(
        30,
        "frame",
        "Typsnitt",
        [0.0, 0.0, 300.0, 100.0],
        json!({ "fills": solid("#FFFFFF") }),
    )
    .kids(vec![body_text]);
    entries.extend(page_entries(FILE_ID, PAGE_S, "Start", 0, vec![board]));
    let body = "b2000000-0000-4000-8000-000000000051";
    entries.push((
        format!("files/{FILE_ID}/typographies/{body}.json"),
        json!({
            "lineHeight": "1.2",
            "path": "TireBud",
            "fontStyle": "normal",
            "fontId": "gfont-geist",
            "fontSize": "16",
            "fontWeight": "400",
            "name": "Body",
            "modifiedAt": "2026-10-02T10:37:00.964Z",
            "fontVariantId": "regular",
            "id": body,
            "letterSpacing": "0",
            "fontFamily": "Geist",
            "textTransform": "none"
        })
        .to_string()
        .into_bytes(),
    ));
    let dir = home.join("typsnitt");
    std::fs::create_dir_all(dir.join("fonts/Geist")).expect("the font folder");
    std::fs::write(dir.join("fonts/Geist/Geist-Regular.ttf"), FONT).expect("write the font file");
    let path = dir.join("typsnitt.penpot");
    std::fs::write(&path, stored_zip(&entries)).expect("write the export");
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
    design_doc: String,
    design_system_doc: String,
    answer: Value,
}

fn import_with(home: &Scratch, project: &str, file: &Path, extra: &[&str]) -> Imported {
    let mut args = vec![
        "import",
        "penpot",
        file.to_str().expect("a path"),
        "--project",
        project,
        "--json",
    ];
    args.extend_from_slice(extra);
    let out = run(home, &args);
    let answer = json_of(&out, "omni import penpot");
    let text = |key: &str| {
        answer[key]
            .as_str()
            .unwrap_or_else(|| panic!("the answer has {key}: {answer}"))
            .to_string()
    };
    Imported {
        project: project.to_string(),
        design_doc: text("design_doc_id"),
        design_system_doc: text("design_system_doc_id"),
        answer,
    }
}

fn import(home: &Scratch, project: &str, file: &Path) -> Imported {
    import_with(home, project, file, &[])
}

/// The IDs of the project's change sets.
fn changesets(home: &Scratch, project: &str) -> Vec<String> {
    let listed = json_of(
        &run(home, &["changeset", "list", "--project", project, "--json"]),
        "changeset list",
    );
    listed["changesets"]
        .as_array()
        .unwrap_or_else(|| panic!("change sets: {listed}"))
        .iter()
        .filter_map(|changeset| changeset["id"].as_str().map(str::to_string))
        .collect()
}

/// Checks that the import wrote the live docs: no change set in the project,
/// and none named in the answer.
fn assert_no_change_set(home: &Scratch, imported: &Imported) {
    assert!(
        imported
            .answer
            .get("changeset_id")
            .is_none_or(Value::is_null),
        "the import names no change set: {}",
        imported.answer
    );
    assert_eq!(
        changesets(home, &imported.project),
        Vec::<String>::new(),
        "the import leaves no change set in the project"
    );
}

/// The doc's record: `GET /docs/{doc}`.
fn doc_of(f: &Fixture, home: &Scratch, project: &str, doc: &str) -> Value {
    get(f, home, project, &format!("/docs/{doc}"))["doc"].clone()
}

/// The head read of a doc: `GET /docs/{doc}/html`.
fn head(f: &Fixture, home: &Scratch, project: &str, doc: &str) -> Value {
    get(f, home, project, &format!("/docs/{doc}/html"))
}

/// The components of the design system's head: its head text holds the token
/// tree with the components under `$components` (docs/formats/tokens.md).
fn system_components(f: &Fixture, home: &Scratch, imported: &Imported) -> Vec<Value> {
    let read = head(f, home, &imported.project, &imported.design_system_doc);
    let text = read["html"]
        .as_str()
        .unwrap_or_else(|| panic!("the design system's head read has its text: {read}"));
    let json: Value = serde_json::from_str(text)
        .unwrap_or_else(|e| panic!("the design system's head is JSON ({e}): {text:?}"));
    json["$components"]
        .as_array()
        .unwrap_or_else(|| panic!("the design system's head lists its components: {json}"))
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

/// Reads every board of every page of the design's head with `GET
/// /docs/{doc}/boards/{board}`; returns how many it read.
fn read_every_board(f: &Fixture, home: &Scratch, project: &str, doc: &str, pages: &Value) -> usize {
    let mut read = 0;
    for page in pages.as_array().expect("pages") {
        let page_id = page["id"].as_str().expect("a page id");
        for board in page["children"].as_array().into_iter().flatten() {
            let id = board["id"].as_str().expect("a board id");
            let answer = get(f, home, project, &format!("/docs/{doc}/boards/{id}"));
            assert_eq!(answer["board_id"], id, "{answer}");
            assert_eq!(answer["page_id"], page_id, "{answer}");
            let tags = answer["tags"].as_str().unwrap_or("");
            assert!(
                tags.starts_with("<Board") && tags.contains(&format!("id=\"{id}\"")),
                "the board {} reads in the tag format: {tags:?}",
                board["name"]
            );
            read += 1;
        }
    }
    read
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

/// The values of the token at `path` in each mode of a token tree.
fn token_modes<'a>(tokens: &'a Value, path: &[&str]) -> &'a Value {
    let mut token = &tokens["tokens"];
    for part in path {
        token = &token[*part];
    }
    &token["$extensions"]["omnistate"]["modes"]
}

/// The IDs of the components named `name`, sorted.
fn ids_named(components: &[Value], name: &str) -> Vec<String> {
    let mut ids: Vec<String> = components
        .iter()
        .filter(|component| component["name"] == name)
        .filter_map(|component| component["id"].as_str().map(str::to_string))
        .collect();
    ids.sort();
    ids
}

#[test]
fn import_penpot_writes_the_design_and_its_design_system_with_no_change_set() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let imported = import(&home, &project, &write_export(&home));
    assert_no_change_set(&home, &imported);

    let design = doc_of(f, &home, &project, &imported.design_doc);
    assert_eq!(
        (&design["kind"], &design["pillar"], &design["feature_id"]),
        (&json!("design"), &json!("ui"), &Value::Null),
        "a design doc of the app level's UI pillar: {design}"
    );
    let system = doc_of(f, &home, &project, &imported.design_system_doc);
    assert_eq!(
        (&system["kind"], &system["pillar"], &system["feature_id"]),
        (&json!("design-system"), &json!("ui"), &Value::Null),
        "the project's design system: {system}"
    );

    let tokens = get(f, &home, &project, "/tokens");
    assert_eq!(
        tokens["doc_id"],
        imported.design_system_doc.as_str(),
        "{tokens}"
    );
    assert_eq!(tokens["modes"], json!(["Mörkt", "Ljust"]), "{tokens}");
    let surface = token_modes(&tokens, &["color", "surface"]);
    assert_eq!(lowercase(&surface["Mörkt"]), "#1c1c21", "{tokens}");
    assert_eq!(lowercase(&surface["Ljust"]), "#ffffff", "{tokens}");

    let read = head(f, &home, &project, &imported.design_doc);
    let start = page(&read["pages"], "Start");
    assert_eq!(board_names(start), ["Hem", "Unplaced"], "{start}");
    assert_eq!(board_names(page(&read["pages"], "Komponenter")), ["Märken"]);
    let hem = find(start, "Hem").expect("the board Hem");
    assert_eq!(hem["fills"][0]["color"], "$color.surface", "{hem}");
    assert_eq!(find(hem, "Rubrik").expect("Rubrik")["color"], "$color.text");
    let badge = find(hem, "Märke").expect("the copy Märke");
    assert_eq!(badge["type"], "instance", "{badge}");
    let component = badge["component"].as_str().expect("a component");
    let used = read["components"]
        .as_array()
        .unwrap_or_else(|| panic!("the head read lists the components it uses: {read}"));
    assert!(
        used.iter()
            .any(|known| known["id"] == component && known["name"] == "Badge"),
        "{badge} places a Badge of the design system: {used:?}"
    );
    let overrides = badge["overrides"].as_object().expect("overrides");
    assert!(
        overrides.values().any(|change| change["content"] == "Ny"),
        "the text of the copy: {badge}"
    );
    let components = system_components(f, &home, &imported);
    assert_eq!(
        ids_named(&components, "Badge").len(),
        2,
        "both Badge variants are in the design system's head: {components:?}"
    );
    assert!(
        ids_named(&components, "Badge").contains(&component.to_string()),
        "the copy places a component of the design system's head"
    );

    assert_eq!(
        read_every_board(f, &home, &project, &imported.design_doc, &read["pages"]),
        3,
        "Märken, Hem and Unplaced read back"
    );
}

#[test]
fn images_of_the_export_become_workspace_attachments() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let imported = import(&home, &project, &write_export(&home));
    let read = head(f, &home, &project, &imported.design_doc);
    let picture = find(page(&read["pages"], "Start"), "Bild").expect("the image Bild");
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
}

#[test]
fn the_tirebud_idag_export_imports_into_an_empty_project_and_every_board_reads() {
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
    assert_no_change_set(&home, &imported);
    let system = doc_of(f, &home, &project, &imported.design_system_doc);
    assert_eq!(system["kind"], "design-system", "{system}");
    let tokens = get(f, &home, &project, "/tokens");
    assert_eq!(tokens["doc_id"], imported.design_system_doc.as_str());
    assert_eq!(tokens["modes"], json!(["Mörkt", "Ljust"]));

    let read = head(f, &home, &project, &imported.design_doc);
    let boards = board_names(page(&read["pages"], "Idag"));
    assert_eq!(boards.len(), 16, "15 boards and Unplaced: {boards:?}");
    for name in ["Idag / Dag / Desktop", "Idag / Dag / Telefon", "Unplaced"] {
        assert!(boards.contains(&name), "{name} in {boards:?}");
    }
    // The converter's pages, to compare the head with.
    let library = omni_penpot::import::import_file(&path).expect("the converter reads idag");
    let expected: usize = library.pages.iter().map(|page| page.children().len()).sum();
    assert_eq!(
        read_every_board(f, &home, &project, &imported.design_doc, &read["pages"]),
        expected,
        "every board of the export reads back"
    );
}

#[test]
fn a_second_import_merges_into_the_design_system_of_the_first() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let first = import(&home, &project, &write_export(&home));
    let badges = ids_named(&system_components(f, &home, &first), "Badge");
    assert_eq!(badges.len(), 2, "the first import's Badge variants");

    // The project has a design system now; the second import adds to it and
    // takes nothing of it away.
    let second = import(&home, &project, &write_kassa_export(&home));
    assert_no_change_set(&home, &second);
    assert_eq!(
        second.design_system_doc, first.design_system_doc,
        "a project has one design system doc"
    );
    assert_ne!(
        second.design_doc, first.design_doc,
        "another file is another design"
    );
    let tokens = get(f, &home, &project, "/tokens");
    assert_eq!(tokens["doc_id"], first.design_system_doc.as_str());
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
    assert!(
        token_modes(&tokens, &["color", "surface"])["Ljust"].is_string(),
        "{tokens}"
    );

    let components = system_components(f, &home, &second);
    assert_eq!(
        ids_named(&components, "Badge"),
        badges,
        "the two Badge variants of the first import stay, with their IDs: {components:?}"
    );
    let knapp = ids_named(&components, "Knapp");
    assert_eq!(knapp.len(), 1, "Knapp is added: {components:?}");

    let read = head(f, &home, &project, &second.design_doc);
    let kassa = page(&read["pages"], "Kassa");
    // The page is named Kassa too, so look the board up among its children.
    let board = kassa["children"]
        .as_array()
        .and_then(|boards| boards.iter().find_map(|child| find(child, "Kassa")))
        .expect("the board Kassa");
    assert_eq!(board["fills"][0]["color"], "$color.border", "{board}");
    let copy = find(board, "Köpknapp").expect("the copy Köpknapp");
    assert_eq!(copy["type"], "instance", "{copy}");
    assert_eq!(copy["component"], knapp[0].as_str(), "{copy} places Knapp");

    // The first design still resolves its copy against the design system.
    let first_read = head(f, &home, &project, &first.design_doc);
    let badge = find(page(&first_read["pages"], "Start"), "Märke").expect("the copy Märke");
    let component = badge["component"].as_str().expect("a component");
    assert!(badges.contains(&component.to_string()), "{badge}");
    assert!(
        first_read["components"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|known| known["id"] == component && known["name"] == "Badge"),
        "{first_read}"
    );
}

#[test]
fn with_feature_the_design_lands_in_the_feature_and_the_design_system_at_the_app_level() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    let name = unique("Kassaflöde");
    let created = json_of(
        &run(
            &home,
            &["feature", "create", &name, "--project", &project, "--json"],
        ),
        "feature create",
    );
    let feature = created["feature"]["id"]
        .as_str()
        .expect("a feature id")
        .to_string();

    let imported = import_with(&home, &project, &write_export(&home), &["--feature", &name]);
    assert_no_change_set(&home, &imported);
    let design = doc_of(f, &home, &project, &imported.design_doc);
    assert_eq!(
        (&design["kind"], &design["pillar"], &design["feature_id"]),
        (&json!("design"), &json!("ui"), &json!(feature)),
        "the design is in the feature's UI pillar: {design}"
    );
    let system = doc_of(f, &home, &project, &imported.design_system_doc);
    assert_eq!(
        (&system["kind"], &system["pillar"], &system["feature_id"]),
        (&json!("design-system"), &json!("ui"), &Value::Null),
        "the design system is the app level's: {system}"
    );

    let tokens = get(f, &home, &project, "/tokens");
    assert_eq!(tokens["doc_id"], imported.design_system_doc.as_str());
    assert_eq!(tokens["modes"], json!(["Mörkt", "Ljust"]), "{tokens}");
    let badges = ids_named(&system_components(f, &home, &imported), "Badge");
    assert_eq!(badges.len(), 2, "the components are in the design system");

    let read = head(f, &home, &project, &imported.design_doc);
    let badge = find(page(&read["pages"], "Start"), "Märke").expect("the copy Märke");
    assert_eq!(badge["type"], "instance", "{badge}");
    assert!(
        badges.contains(&badge["component"].as_str().unwrap_or("").to_string()),
        "the feature's design places a component of the app level's design system: {badge}"
    );
    assert_eq!(
        read_every_board(f, &home, &project, &imported.design_doc, &read["pages"]),
        3
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
fn the_tirebud_ordrar_export_imports_with_its_components() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/penpot/tirebud-ordrar.penpot");
    if !path.is_file() {
        eprintln!("SKIPPED: fixtures/penpot/tirebud-ordrar.penpot is not there");
        return;
    }
    // What the converter makes of the export, to compare the API's heads with.
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
    assert_no_change_set(&home, &imported);
    let report = imported.answer["report"].as_array().expect("a report");
    assert_eq!(
        report.len(),
        library.report.len(),
        "the report lists all the converter leaves out"
    );
    let tokens = get(f, &home, &project, "/tokens");
    assert_eq!(tokens["modes"], json!(["Mörkt", "Ljust"]));

    let components = system_components(f, &home, &imported);
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

    let read = head(f, &home, &project, &imported.design_doc);
    let pages = read["pages"].as_array().expect("pages");
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
}

#[test]
fn fonts_beside_the_export_become_font_files_of_its_typography_tokens() {
    let f = fixture();
    let home = agent_home(f);
    let project = new_project(&home);
    import(&home, &project, &write_font_export(&home));
    let tokens = get(f, &home, &project, "/tokens");
    let mut typographies = Vec::new();
    typographies_in(&tokens["tokens"], &mut typographies);
    let geist: Vec<&Value> = typographies
        .into_iter()
        .filter(|token| token["$value"]["fontFamily"] == "Geist")
        .collect();
    assert!(!geist.is_empty(), "a typography token in Geist: {tokens}");
    for token in geist {
        let files = token["$extensions"]["omnistate"]["fontFiles"]
            .as_array()
            .unwrap_or_else(|| panic!("the token lists its font files: {token}"));
        assert_eq!(files.len(), 1, "{token}");
        let mut ids = Vec::new();
        attachment_ids(&files[0], &mut ids);
        assert_eq!(ids.len(), 1, "an attachment:<id>: {token}");
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
        assert_eq!(
            served.body, FONT,
            "the font file beside the export, byte for byte"
        );
    }
}

/// Every typography token below `node` of a DTCG tree.
fn typographies_in<'a>(node: &'a Value, found: &mut Vec<&'a Value>) {
    let Some(map) = node.as_object() else { return };
    if map.contains_key("$value") {
        if node["$value"]["fontFamily"].is_string() {
            found.push(node);
        }
        return;
    }
    for (name, child) in map {
        if !name.starts_with('$') {
            typographies_in(child, found);
        }
    }
}
