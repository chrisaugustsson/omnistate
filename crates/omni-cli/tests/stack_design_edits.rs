//! US-031: `omni design edit`, `omni claim` and `omni release` on boards,
//! against the real API. Run with `--features stack-tests` in the Compose test
//! container, which reaches Postgres at `postgres:5432` and RustFS at
//! `rustfs:9000`.
//!
//! The tests migrate a scratch database, seed a workspace with an API key (the
//! agents, told apart by `--as`), create a scratch RustFS bucket and serve the
//! API's router on a local port against both. The built `omni` runs against
//! that address with the key in its config file. Each design is imported with
//! `omni api PUT .../design` (US-029). The contract of the CLI is in
//! tests/design_edits.rs and tests/claims.rs, and the API's in
//! crates/omni-api/tests/design_edits.rs.

mod common;

use common::*;
use omni_api::Config;
use serde_json::{Value, json};
use sqlx::Connection;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use std::collections::HashMap;
use std::future::Future;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const TEST_DB: &str = "omni_us031_cli";
const TEST_BUCKET: &str = "omni-us031-cli";
const KEY_PREFIX: &str = "omniustest31";
/// The agent that builds the designs and claims boards.
const HOLDER: &str = "run-1/designer";
/// Another agent on the same machine, with the same key.
const OTHER: &str = "run-2/illustrator";

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
            let workspace: String = sqlx::query_scalar(
                "insert into workspaces (name) values ('Alpha') returning id::text",
            )
            .fetch_one(&mut conn)
            .await
            .expect("seed a workspace");
            sqlx::query(
                "insert into api_keys (workspace_id, prefix, key_hash, scopes) \
                 values ($1::uuid, $2, encode(sha256(convert_to($3, 'UTF8')), 'hex'), '{*}')",
            )
            .bind(&workspace)
            .bind(KEY_PREFIX)
            .bind(&secret)
            .execute(&mut conn)
            .await
            .expect("seed an API key");
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
            workspace,
            secret,
            _runtime: runtime,
        }
    })
}

/// A home directory whose config file holds the agents' key, as `omni login`
/// leaves it.
fn agent_home(f: &Fixture) -> Scratch {
    let home = Scratch::new("agent");
    write_config(&home.path, &f.url, &f.workspace, &f.secret);
    home
}

fn run_as(home: &Scratch, agent: &str, args: &[&str], stdin: &[u8]) -> Out {
    omni(&home.path)
        .args(args)
        .args(&["--as", agent])
        .stdin(stdin)
        .run()
}

fn unique(label: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("{label}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn json_of(out: &Out, what: &str) -> Value {
    assert_eq!(out.code, 0, "{what}: {}", out.stderr);
    out.json()
}

/// Node IDs by node name. Every name in a design is used once.
type Ids = HashMap<String, String>;

fn collect_ids(nodes: &Value, ids: &mut Ids) {
    for node in nodes.as_array().unwrap_or(&Vec::new()) {
        if let (Some(name), Some(id)) = (node["name"].as_str(), node["id"].as_str()) {
            ids.insert(name.to_string(), id.to_string());
        }
        collect_ids(&node["children"], ids);
    }
}

/// One page, Checkout, with the boards Cart and Payment.
const DESIGN: &str = r##"<Page name="Checkout">
  <Board name="Cart" x={0} y={0} width={360} height={640} dir="col" gap={8}>
    <Frame name="Summary" dir="col" gap={4}>
      <Text name="Total" content="Summa 10 kr" fontFamily="Inter" fontSize={16}/>
    </Frame>
  </Board>
  <Board name="Payment" x={400} y={0} width={360} height={640} dir="col">
    <Text name="Pay" content="Betala nu" fontFamily="Inter" fontSize={16}/>
  </Board>
</Page>
"##;

/// A project (by name) with a design doc started in an open change set and
/// `DESIGN` imported into its draft.
struct Design {
    project: String,
    changeset: String,
    doc: String,
    ids: Ids,
}

impl Design {
    fn id(&self, name: &str) -> String {
        self.ids
            .get(name)
            .unwrap_or_else(|| panic!("the node {name} has an ID: {:?}", self.ids))
            .clone()
    }

    /// `--project <name> --changeset <id>`, and `--doc <id>` too with `doc`.
    fn target(&self, doc: bool) -> Vec<String> {
        let mut target = vec![
            "--project".to_string(),
            self.project.clone(),
            "--changeset".to_string(),
            self.changeset.clone(),
        ];
        if doc {
            target.extend(["--doc".to_string(), self.doc.clone()]);
        }
        target
    }
}

fn new_design(f: &Fixture, home: &Scratch) -> Design {
    let project = unique("checkout");
    let created = json_of(
        &run_as(
            home,
            HOLDER,
            &["project", "create", &project, "--json"],
            b"",
        ),
        "project create",
    );
    let project_id = created["project"]["id"]
        .as_str()
        .expect("a project id")
        .to_string();
    let out = run_as(
        home,
        HOLDER,
        &[
            "changeset",
            "create",
            "--project",
            &project,
            "--title",
            "Screens",
            "--json",
        ],
        b"",
    );
    let changeset = json_of(&out, "changeset create")["changeset"]["id"]
        .as_str()
        .expect("a change set id")
        .to_string();
    let out = run_as(
        home,
        HOLDER,
        &[
            "doc",
            "create",
            "--project",
            &project,
            "--changeset",
            &changeset,
            "--kind",
            "design",
            "--pillar",
            "ui",
            "--title",
            "Checkout screens",
            "--json",
        ],
        b"",
    );
    let doc = json_of(&out, "doc create")["doc"]["id"]
        .as_str()
        .expect("a doc id")
        .to_string();
    let path = format!(
        "/api/v1/w/{}/projects/{project_id}/changesets/{changeset}/docs/{doc}/design",
        f.workspace
    );
    let body = json!({ "tags": DESIGN }).to_string();
    let out = run_as(
        home,
        HOLDER,
        &["api", "PUT", &path, "--body-file", "-"],
        body.as_bytes(),
    );
    let imported = json_of(&out, "import the design");
    let mut ids = Ids::new();
    collect_ids(&imported["pages"], &mut ids);
    assert_eq!(ids.len(), 6, "{imported}");
    Design {
        project,
        changeset,
        doc,
        ids,
    }
}

/// `omni design edit` as `agent` with the batch on standard input.
fn design_edit(home: &Scratch, agent: &str, design: &Design, extra: &[&str], ops: &Value) -> Out {
    let target = design.target(true);
    let mut args: Vec<&str> = vec!["design", "edit"];
    args.extend(target.iter().map(String::as_str));
    args.extend(extra);
    run_as(home, agent, &args, ops.to_string().as_bytes())
}

/// `omni design read` of one board of the draft.
fn read_board(home: &Scratch, design: &Design, board: &str) -> String {
    let target = design.target(true);
    let mut args: Vec<&str> = vec!["design", "read", "--board", board];
    args.extend(target.iter().map(String::as_str));
    let out = run_as(home, HOLDER, &args, b"");
    assert_eq!(out.code, 0, "design read: {}", out.stderr);
    out.stdout
}

/// `omni claim` or `omni release` of a node as `agent`.
fn claim_command(home: &Scratch, agent: &str, design: &Design, command: &str, node: &str) -> Out {
    let target = design.target(false);
    let mut args: Vec<&str> = vec![command, node];
    args.extend(target.iter().map(String::as_str));
    run_as(home, agent, &args, b"")
}

fn set_content(node: &str, rev: u64, content: &str) -> Value {
    json!({ "op": "set", "node": node, "rev": rev, "properties": { "content": content } })
}

fn text_tag(name: &str, content: &str) -> String {
    format!(r#"<Text name="{name}" content="{content}" fontFamily="Inter" fontSize={{14}}/>"#)
}

#[test]
fn an_agent_edits_a_design_from_the_command_line_and_gets_the_new_ids() {
    let f = fixture();
    let home = agent_home(f);
    let design = new_design(f, &home);
    let board = format!(
        "<Board name=\"Receipt\" x={{800}} y={{0}} width={{360}} height={{640}} dir=\"col\">\n  {}\n</Board>",
        text_tag("Thanks", "Tack för ditt köp")
    );
    let ops = json!([
        set_content(&design.id("Total"), 1, "Summa 12 kr"),
        {
            "op": "insert",
            "parent": design.id("Checkout"),
            "position": { "after": design.id("Payment") },
            "content": board
        }
    ]);
    let out = design_edit(&home, HOLDER, &design, &["--json"], &ops);
    let answer = json_of(&out, "design edit");
    let results = answer["results"].as_array().expect("results");
    assert_eq!(results.len(), 2, "{answer}");
    assert_eq!(results[0]["revision"], 2, "{answer}");
    let created: Vec<String> = results[1]["created"]
        .as_array()
        .expect("created")
        .iter()
        .map(|id| id.as_str().expect("an ID").to_string())
        .collect();
    assert_eq!(created.len(), 2, "{answer}");

    let receipt = read_board(&home, &design, &created[0]);
    assert!(
        receipt.contains(&format!(r#"<Board id="{}""#, created[0])),
        "{receipt}"
    );
    assert!(
        receipt.contains(&format!(r#"id="{}""#, created[1])),
        "{receipt}"
    );
    let cart = read_board(&home, &design, &design.id("Cart"));
    assert!(cart.contains("Summa 12 kr"), "{cart}");

    let ops = json!([{ "op": "delete", "node": "zzzzzzzz", "rev": 1 }]);
    let out = design_edit(&home, HOLDER, &design, &[], &ops);
    assert_ne!(out.code, 0, "an unknown ID is an error: {}", out.stdout);
    assert!(
        out.stderr.contains("zzzzzzzz"),
        "the error names the unknown ID: {:?}",
        out.stderr
    );
}

#[test]
fn omni_claim_and_release_work_on_boards() {
    let f = fixture();
    let home = agent_home(f);
    let design = new_design(f, &home);
    let cart = design.id("Cart");
    let out = claim_command(&home, HOLDER, &design, "claim", &cart);
    assert_eq!(out.code, 0, "claim the board: {}", out.stderr);
    assert!(out.stdout.contains(&cart), "{}", out.stdout);

    let theirs = claim_command(&home, OTHER, &design, "claim", &cart);
    assert_ne!(theirs.code, 0, "{}", theirs.stdout);
    assert!(
        theirs.stderr.contains("run-1") && theirs.stderr.contains("designer"),
        "the refusal names the holder: {:?}",
        theirs.stderr
    );
    let ops = json!([set_content(&design.id("Total"), 1, "Summa 12 kr")]);
    let refused = design_edit(&home, OTHER, &design, &[], &ops);
    assert_ne!(refused.code, 0, "{}", refused.stdout);
    assert!(
        refused.stderr.contains("run-1") && refused.stderr.contains("designer"),
        "the refusal names the holder: {:?}",
        refused.stderr
    );
    let elsewhere = design_edit(
        &home,
        OTHER,
        &design,
        &[],
        &json!([set_content(&design.id("Pay"), 1, "Betala")]),
    );
    assert_eq!(elsewhere.code, 0, "another board: {}", elsewhere.stderr);

    let out = claim_command(&home, HOLDER, &design, "release", &cart);
    assert_eq!(out.code, 0, "release the board: {}", out.stderr);
    let out = design_edit(&home, OTHER, &design, &[], &ops);
    assert_eq!(out.code, 0, "the board is free again: {}", out.stderr);
}

#[test]
fn two_agents_edit_different_boards_of_one_draft_from_the_command_line_at_once() {
    let f = fixture();
    let home = agent_home(f);
    let design = new_design(f, &home);
    const ROUNDS: usize = 5;
    let barrier = std::sync::Barrier::new(2);
    let jobs = [
        (HOLDER, design.id("Summary"), "A"),
        (OTHER, design.id("Payment"), "B"),
    ];
    let outs: Vec<Out> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(agent, parent, label)| {
                let (barrier, home, design) = (&barrier, &home, &design);
                scope.spawn(move || {
                    barrier.wait();
                    (0..ROUNDS)
                        .map(|round| {
                            let name = format!("{label}{round}");
                            let ops = json!([{
                                "op": "insert",
                                "parent": parent,
                                "position": { "after": null },
                                "content": text_tag(&name, &name)
                            }]);
                            design_edit(home, agent, design, &[], &ops)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("an agent's thread"))
            .collect()
    });
    for out in &outs {
        assert_eq!(out.code, 0, "every edit applies: {}", out.stderr);
    }
    let cart = read_board(&home, &design, &design.id("Cart"));
    let payment = read_board(&home, &design, &design.id("Payment"));
    for round in 0..ROUNDS {
        assert!(cart.contains(&format!(r#"name="A{round}""#)), "{cart}");
        assert!(
            payment.contains(&format!(r#"name="B{round}""#)),
            "{payment}"
        );
    }
}
