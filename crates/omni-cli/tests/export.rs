//! US-120: `omni export`, tested as the built `omni` against a mock HTTP server.

mod common;

use common::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const FEATURE: &str = "01900000-0000-7000-8000-0000000000f1";
const APP_SPEC: &str = "01900000-0000-7000-8000-0000000000d1";
const SYSTEM: &str = "01900000-0000-7000-8000-0000000000d2";
const FEATURE_SPEC: &str = "01900000-0000-7000-8000-0000000000d3";
const FEATURE_DESIGN: &str = "01900000-0000-7000-8000-0000000000d4";
const BOARD: &str = "b0ardcar";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn doc(id: &str, kind: &str, pillar: &str, title: &str) -> Value {
    json!({"id": id, "kind": kind, "pillar": pillar, "title": title})
}

fn app_docs() -> Vec<Value> {
    vec![
        doc(APP_SPEC, "html", "spec", "Product overview"),
        doc(SYSTEM, "design-system", "ui", "Design system"),
    ]
}

fn feature_docs() -> Vec<Value> {
    vec![
        doc(FEATURE_SPEC, "html", "spec", "Checkout flow"),
        doc(FEATURE_DESIGN, "design", "ui", "Checkout screens"),
    ]
}

fn seq_of(id: &str) -> u64 {
    match id {
        APP_SPEC => 4,
        SYSTEM => 3,
        FEATURE_SPEC => 7,
        FEATURE_DESIGN => 12,
        _ => unreachable!(),
    }
}

fn html_of(id: &str) -> String {
    format!("<section data-omni-id=\"s1\"><p>head of {id}</p></section>")
}

const BOARD_TAGS: &str = "<board id=\"b0ardcar\" name=\"Cart\" w=\"390\" h=\"844\"></board>";

fn two_mode_tree() -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "$type": "color",
            "ink": {"$value": "#0f172a"},
            "paper": {
                "$value": "#ffffff",
                "$extensions": {"omnistate": {"modes": {"light": "#ffffff", "dark": "#0b1120"}}}
            },
            "text": {
                "$value": "{color.ink}",
                "$extensions": {"omnistate": {"modes": {"light": "{color.ink}", "dark": "{color.paper}"}}}
            },
            "shadow": {"$value": "#0f172a33"}
        },
        "typography": {
            "$type": "typography",
            "body": {
                "$value": {"fontFamily": "Inter", "fontSize": "16px", "lineHeight": 1.5},
                "$extensions": {"omnistate": {"modes": {
                    "light": {"fontFamily": "Inter", "fontSize": "16px", "lineHeight": 1.5},
                    "dark": {"fontFamily": "Inter", "fontSize": "18px", "lineHeight": 1.5}
                }}}
            }
        },
        "shadow": {
            "$type": "shadow",
            "card": {"$value": {"color": "{color.shadow}", "offsetX": "0px", "offsetY": "4px", "blur": "12px"}}
        }
    })
}

fn tokens_answer(tree: Value) -> Value {
    let modes = tree["$extensions"]["omnistate"]["modes"].clone();
    json!({"project_id": PROJECT, "doc_id": SYSTEM, "modes": modes, "tokens": tree})
}

/// An API whose project has `app` docs at the app level and `feature` docs in
/// `FEATURE`, and answers `GET /tokens` with `tokens`.
fn mock(app: Vec<Value>, feature: Vec<Value>, tokens: Value) -> MockServer {
    MockServer::start(move |request| answer(request, &app, &feature, &tokens))
}

/// What the API of `mock` answers `request` with.
fn answer(request: &Recorded, app: &[Value], feature: &[Value], tokens: &Value) -> Reply {
    {
        let not_found = || Reply::json(404, &json!({"error": "not_found"}));
        if request.method != "GET" {
            return not_found();
        }
        let Some(rest) = request.path.strip_prefix(&base()) else {
            return not_found();
        };
        if rest == "/docs?feature=global" {
            return Reply::json(200, &json!({"docs": app}));
        }
        if rest == format!("/docs?feature={FEATURE}") {
            return Reply::json(200, &json!({"docs": feature}));
        }
        if rest == "/tokens" {
            return Reply::json(200, tokens);
        }
        let Some(rest) = rest.strip_prefix("/docs/") else {
            return not_found();
        };
        let (id, tail) = rest.split_at(36);
        match tail {
            "/activity?limit=1" => Reply::json(
                200,
                &json!({"entries": [{"seq": seq_of(id), "kind": "write"}]}),
            ),
            "/html" => Reply::json(200, &json!({"html": html_of(id)})),
            "/html?view=outline" if id == FEATURE_DESIGN => Reply::json(
                200,
                &json!({"outline": [{"id": "p4ge0001", "name": "Main", "children": [
                    {"id": BOARD, "name": "Cart", "placeholder": true}
                ]}]}),
            ),
            "/boards/b0ardcar" if id == FEATURE_DESIGN => {
                Reply::json(200, &json!({"tags": BOARD_TAGS}))
            }
            _ => not_found(),
        }
    }
}

fn export(server: &MockServer, out: &Path, scope: &[&str]) -> Out {
    let home = Scratch::new("home");
    write_config(&home.path, &server.url, WORKSPACE, API_KEY);
    omni(&home.path)
        .args(&[
            "export",
            "--project",
            PROJECT,
            "--out",
            &out.display().to_string(),
        ])
        .args(scope)
        .args(&["--as", "run-120/builder"])
        .run()
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} was written ({e})", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} holds JSON ({e}): {text}", path.display()))
}

/// The manifest's entries, each without its path.
fn entries_without_path(out: &Path) -> BTreeSet<String> {
    read_json(&out.join("manifest.json"))["files"]
        .as_array()
        .expect("manifest.json has a files array")
        .iter()
        .map(|entry| {
            let mut entry = entry.clone();
            entry.as_object_mut().unwrap().remove("path");
            entry.to_string()
        })
        .collect()
}

/// The path the manifest gives the file whose entry has `field` = `value`.
fn path_where(out: &Path, field: &str, value: &str) -> String {
    let manifest = read_json(&out.join("manifest.json"));
    let found: Vec<&Value> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry[field] == value)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "one entry with {field} = {value}: {manifest}"
    );
    found[0]["path"].as_str().unwrap().to_string()
}

/// The blocks of a stylesheet: each selector with its declarations.
fn css_blocks(css: &str) -> Vec<(String, BTreeMap<String, String>)> {
    css.split('}')
        .filter(|block| !block.trim().is_empty())
        .map(|block| {
            let (selector, body) = block.split_once('{').expect("a block has a body");
            let declarations = body
                .split(';')
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    let (name, value) = line.split_once(':').expect("a declaration");
                    (name.trim().to_string(), value.trim().to_string())
                })
                .collect();
            (selector.trim().to_string(), declarations)
        })
        .collect()
}

fn tokens_css_blocks(out: &Path) -> Vec<(String, BTreeMap<String, String>)> {
    let path = out.join("tokens.css");
    let css = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} was written ({e})", path.display()));
    css_blocks(&css)
}

fn declarations(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

fn entry(id: &str, pillar: &str, title: &str, seq: u64) -> String {
    json!({"doc_id": id, "pillar": pillar, "title": title, "seq": seq}).to_string()
}

#[test]
fn a_feature_export_lists_the_feature_docs_and_the_app_tokens_each_at_its_seq() {
    let server = mock(app_docs(), feature_docs(), tokens_answer(two_mode_tree()));
    let out = Scratch::new("out");

    let run = export(&server, &out.path, &["--feature", FEATURE]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let board = json!({
        "doc_id": FEATURE_DESIGN, "pillar": "ui", "title": "Checkout screens", "seq": 12,
        "board_id": BOARD, "board": "Cart"
    });
    let expected: BTreeSet<String> = [
        entry(FEATURE_SPEC, "spec", "Checkout flow", 7),
        board.to_string(),
        entry(SYSTEM, "ui", "Design system", 3),
    ]
    .into_iter()
    .collect();
    assert_eq!(entries_without_path(&out.path), expected);
}

#[test]
fn a_project_export_writes_the_app_docs_and_tokens_but_no_feature_doc_and_no_design_system_board() {
    let server = mock(app_docs(), feature_docs(), tokens_answer(two_mode_tree()));
    let out = Scratch::new("out");

    let run = export(&server, &out.path, &[]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let expected: BTreeSet<String> = [
        entry(APP_SPEC, "spec", "Product overview", 4),
        entry(SYSTEM, "ui", "Design system", 3),
    ]
    .into_iter()
    .collect();
    assert_eq!(entries_without_path(&out.path), expected);
    assert_eq!(
        path_where(&out.path, "doc_id", APP_SPEC)
            .rsplit_once('.')
            .unwrap()
            .1,
        "html"
    );
    let system_paths: BTreeSet<String> = read_json(&out.path.join("manifest.json"))["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["doc_id"] == SYSTEM)
        .map(|entry| entry["path"].as_str().unwrap().to_string())
        .collect();
    let tokens_files: BTreeSet<String> =
        ["tokens.css".to_string(), "tokens.json".to_string()].into();
    assert_eq!(system_paths, tokens_files);
}

#[test]
fn the_exported_files_hold_the_html_the_board_tags_and_the_token_tree() {
    let server = mock(app_docs(), feature_docs(), tokens_answer(two_mode_tree()));
    let out = Scratch::new("out");

    let run = export(&server, &out.path, &["--feature", FEATURE]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let html = path_where(&out.path, "doc_id", FEATURE_SPEC);
    assert!(html.starts_with("spec/"), "{html}");
    assert!(html.ends_with(".html"), "{html}");
    assert_eq!(
        std::fs::read_to_string(out.path.join(&html)).unwrap(),
        html_of(FEATURE_SPEC)
    );
    let board = path_where(&out.path, "board_id", BOARD);
    assert!(board.starts_with("ui/"), "{board}");
    assert_eq!(
        std::fs::read_to_string(out.path.join(&board)).unwrap(),
        BOARD_TAGS
    );
    assert_eq!(read_json(&out.path.join("tokens.json")), two_mode_tree());
}

#[test]
fn tokens_css_root_names_variables_by_path_keeps_references_and_splits_typography_and_shadow() {
    let server = mock(app_docs(), feature_docs(), tokens_answer(two_mode_tree()));
    let out = Scratch::new("out");

    let run = export(&server, &out.path, &[]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let blocks = tokens_css_blocks(&out.path);
    assert_eq!(blocks[0].0, ":root");
    assert_eq!(
        blocks[0].1,
        declarations(&[
            ("--color-ink", "#0f172a"),
            ("--color-paper", "#ffffff"),
            ("--color-text", "var(--color-ink)"),
            ("--color-shadow", "#0f172a33"),
            ("--typography-body-font-family", "\"Inter\""),
            ("--typography-body-font-size", "16px"),
            ("--typography-body-line-height", "1.5"),
            ("--shadow-card", "0px 4px 12px 0px var(--color-shadow)"),
        ])
    );
}

#[test]
fn tokens_css_gives_each_other_mode_a_block_with_only_what_it_changes() {
    let server = mock(app_docs(), feature_docs(), tokens_answer(two_mode_tree()));
    let out = Scratch::new("out");

    let run = export(&server, &out.path, &[]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let blocks = tokens_css_blocks(&out.path);
    let selectors: Vec<&str> = blocks.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(selectors, [":root", "[data-mode=\"dark\"]"]);
    assert_eq!(
        blocks[1].1,
        declarations(&[
            ("--color-paper", "#0b1120"),
            ("--color-text", "var(--color-paper)"),
            ("--typography-body-font-size", "18px"),
        ])
    );
}

#[test]
fn two_tokens_that_join_to_one_variable_fail_the_export_naming_both() {
    let tree = json!({
        "$extensions": {"omnistate": {"modes": ["light"]}},
        "color": {
            "$type": "color",
            "brand-500": {"$value": "#2563eb"},
            "brand": {"500": {"$value": "#1d4ed8"}}
        }
    });
    let server = mock(app_docs(), feature_docs(), tokens_answer(tree));
    let out = Scratch::new("out");

    let run = export(&server, &out.path, &[]);

    assert_ne!(run.code, 0, "stdout: {}", run.stdout);
    let lines = run.stderr_lines();
    assert_eq!(
        lines.len(),
        1,
        "one line on standard error: {:?}",
        run.stderr
    );
    assert!(lines[0].contains("color.brand-500"), "{}", lines[0]);
    assert!(lines[0].contains("color.brand.500"), "{}", lines[0]);
    assert!(!out.path.join("manifest.json").exists());
}

#[test]
fn a_design_system_without_a_token_tree_gets_no_token_files() {
    let empty = json!({"project_id": PROJECT, "doc_id": SYSTEM, "modes": [], "tokens": null});
    let server = mock(app_docs(), feature_docs(), empty);
    let out = Scratch::new("out");

    let run = export(&server, &out.path, &[]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(!out.path.join("tokens.json").exists());
    assert!(!out.path.join("tokens.css").exists());
}

#[test]
fn a_project_without_a_design_system_gets_no_token_files() {
    let app = vec![doc(APP_SPEC, "html", "spec", "Product overview")];
    let server = mock(app, feature_docs(), json!({"error": "unexpected"}));
    let out = Scratch::new("out");

    let run = export(&server, &out.path, &["--feature", FEATURE]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(!out.path.join("tokens.json").exists());
    assert!(!out.path.join("tokens.css").exists());
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.path.ends_with("/tokens")),
        "no token read without a design system"
    );
}

#[test]
fn exporting_into_a_used_folder_overwrites_same_names_and_deletes_nothing_else() {
    let server = mock(app_docs(), feature_docs(), tokens_answer(two_mode_tree()));
    let out = Scratch::new("out");
    std::fs::create_dir_all(out.path.join("spec")).unwrap();
    std::fs::write(out.path.join("spec/renamed-doc.html"), "<p>old</p>").unwrap();
    std::fs::write(out.path.join("tokens.json"), "{}").unwrap();

    let run = export(&server, &out.path, &[]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(
        std::fs::read_to_string(out.path.join("spec/renamed-doc.html")).unwrap(),
        "<p>old</p>"
    );
    assert_eq!(read_json(&out.path.join("tokens.json")), two_mode_tree());
}

#[test]
fn a_board_whose_picture_the_api_refuses_is_named_on_stderr_and_the_other_files_are_written() {
    const WALL: &str = "b0ard2nd";
    const REFUSAL: &str =
        "the board's picture would be more than 16384 px on a side; 16384 px is the limit";
    let server = MockServer::start(|request| {
        let design = format!("{}/docs/{FEATURE_DESIGN}", base());
        match request.path.strip_prefix(&design).unwrap_or_default() {
            "/html?view=outline" => Reply::json(
                200,
                &json!({"outline": [{"id": "p4ge0001", "name": "Main", "children": [
                    {"id": BOARD, "name": "Cart", "placeholder": true},
                    {"id": WALL, "name": "Wall", "placeholder": true}
                ]}]}),
            ),
            "/boards/b0ard2nd" => {
                Reply::json(200, &json!({"tags": "<board id=\"b0ard2nd\"></board>"}))
            }
            "/boards/b0ardcar/png" => Reply {
                status: 200,
                headers: vec![("Content-Type".to_string(), "image/png".to_string())],
                body: "picture of Cart".to_string(),
            },
            "/boards/b0ard2nd/png" => Reply::json(
                422,
                &json!({"error": "board_too_large", "message": REFUSAL}),
            ),
            _ => answer(
                request,
                &app_docs(),
                &feature_docs(),
                &tokens_answer(two_mode_tree()),
            ),
        }
    });
    let out = Scratch::new("out");

    let run = export(&server, &out.path, &["--feature", FEATURE]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let cart = path_where(&out.path, "board_id", BOARD);
    let cart_png = out.path.join(cart.replace(".tags", ".png"));
    assert!(cart.ends_with(".tags"), "{cart}");
    assert_eq!(
        std::fs::read_to_string(&cart_png).unwrap(),
        "picture of Cart"
    );
    let wall = path_where(&out.path, "board_id", WALL);
    assert!(
        out.path.join(&wall).is_file(),
        "the refused board's tags are written"
    );
    assert!(!out.path.join(wall.replace(".tags", ".png")).exists());
    let lines = run.stderr_lines();
    assert_eq!(lines.len(), 1, "stderr: {}", run.stderr);
    for part in [WALL, FEATURE_DESIGN, REFUSAL] {
        assert!(lines[0].contains(part), "{part} in {}", lines[0]);
    }
}
