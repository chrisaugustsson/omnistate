//! US-22: `omni design flows --doc <id> [--page <id>]`, tested as the built
//! binary against a mock HTTP server that answers the outline
//! (`GET /docs/{doc}/html?view=outline`) and the board route
//! (`GET /docs/{doc}/boards/{board}`), where the interactions come from.
//!
//! The doc has three pages:
//!
//! - Shop: flows `Checkout → Cart` and `Returns → Returns`; Cart has `Pay
//!   button` navigating to Confirmation and `Promo` navigating to a board the
//!   doc does not have; Settings is reached by nothing.
//! - Account: a flow whose start board is gone, and an interaction pointing at
//!   Cart on the Shop page.
//! - Graph: no flows; A navigates to B and opens C as an overlay, B swaps to D
//!   and E navigates only to itself.

mod common;

use common::*;
use serde_json::{Value, json};

const PROJECT: &str = "01900000-0000-7000-8000-0000000000b1";
const DOC: &str = "01900000-0000-7000-8000-0000000000d1";
/// A doc that answers the outline with 400, as a doc that is not a design.
const NOT_A_DESIGN: &str = "01900000-0000-7000-8000-0000000000d2";
/// A doc whose outline answer has no `outline`.
const NO_OUTLINE: &str = "01900000-0000-7000-8000-0000000000d3";

const SHOP: &str = "pg000001";
const ACCOUNT: &str = "pg000002";
const GRAPH: &str = "pg000003";

const CART: &str = "crt00001";
const CONFIRMATION: &str = "cnf00002";
const RETURNS: &str = "rtn00003";
const SETTINGS: &str = "stg00004";
const PROFILE: &str = "prf00005";
const A: &str = "grfa0001";
const B: &str = "grfb0002";
const C: &str = "grfc0003";
const D: &str = "grfd0004";
const E: &str = "grfe0005";

fn base() -> String {
    format!("/api/v1/w/{WORKSPACE}/projects/{PROJECT}")
}

fn outline_path(doc: &str) -> String {
    format!("{}/docs/{doc}/html?view=outline", base())
}

fn board_path(board: &str) -> String {
    format!("{}/docs/{DOC}/boards/{board}", base())
}

fn slide(actions: Value) -> Value {
    json!({
        "trigger": {"type": "onClick"},
        "actions": actions,
        "transition": {"type": "slideIn", "direction": "left"},
        "durationMs": 300,
        "easing": "easeOut"
    })
}

fn board_entry(id: &str, name: &str) -> Value {
    json!({"id": id, "name": name, "type": "board"})
}

fn outline() -> Value {
    json!({
        "outline": [
            {
                "id": SHOP,
                "name": "Shop",
                "flows": [
                    {"name": "Checkout", "start": CART},
                    {"name": "Returns", "start": RETURNS}
                ],
                "children": [
                    board_entry(CART, "Cart"),
                    board_entry(CONFIRMATION, "Confirmation"),
                    board_entry(RETURNS, "Returns"),
                    board_entry(SETTINGS, "Settings")
                ]
            },
            {
                "id": ACCOUNT,
                "name": "Account",
                "flows": [{"name": "Checkout", "start": "gne00009"}],
                "children": [board_entry(PROFILE, "Profile")]
            },
            {
                "id": GRAPH,
                "name": "Graph",
                "flows": [],
                "children": [
                    board_entry(A, "A"),
                    board_entry(B, "B"),
                    board_entry(C, "C"),
                    board_entry(D, "D"),
                    board_entry(E, "E")
                ]
            }
        ]
    })
}

fn board(id: &str) -> Option<Value> {
    let node = match id {
        CART => json!({
            "id": CART,
            "name": "Cart",
            "children": [
                {
                    "id": "pay00001",
                    "name": "Pay button",
                    "interactions": [slide(json!([{"type": "navigate", "dest": CONFIRMATION}]))]
                },
                {
                    "id": "prm00002",
                    "name": "Promo",
                    "interactions": [slide(json!([{"type": "navigate", "dest": "h2q9d7ma"}]))]
                }
            ]
        }),
        CONFIRMATION => json!({"id": CONFIRMATION, "name": "Confirmation"}),
        RETURNS => json!({"id": RETURNS, "name": "Returns"}),
        SETTINGS => json!({"id": SETTINGS, "name": "Settings"}),
        PROFILE => json!({
            "id": PROFILE,
            "name": "Profile",
            "children": [{
                "id": "edt00006",
                "name": "Edit",
                "interactions": [slide(json!([{"type": "navigate", "dest": CART}]))]
            }]
        }),
        A => json!({
            "id": A,
            "name": "A",
            "interactions": [slide(json!([
                {"type": "navigate", "dest": B},
                {"type": "openOverlay", "dest": C}
            ]))]
        }),
        B => json!({
            "id": B,
            "name": "B",
            "interactions": [slide(json!([{"type": "swapOverlay", "dest": D}]))]
        }),
        C => json!({"id": C, "name": "C"}),
        D => json!({"id": D, "name": "D"}),
        E => json!({
            "id": E,
            "name": "E",
            "interactions": [slide(json!([{"type": "navigate", "dest": E}]))]
        }),
        _ => return None,
    };
    Some(json!({"doc_id": DOC, "board_id": id, "node": node}))
}

fn mock() -> MockServer {
    MockServer::start(move |request| {
        let path = request.path.as_str();
        if request.method != "GET" {
            return Reply::json(404, &json!({ "error": "not_found" }));
        }
        if path == outline_path(DOC) {
            return Reply::json(200, &outline());
        }
        if path == outline_path(NOT_A_DESIGN) {
            return Reply::json(400, &json!({ "error": "not_a_design" }));
        }
        if path == outline_path(NO_OUTLINE) {
            return Reply::json(200, &json!({ "doc_id": NO_OUTLINE }));
        }
        let prefix = format!("{}/docs/{DOC}/boards/", base());
        match path.strip_prefix(prefix.as_str()).and_then(board) {
            Some(answer) => Reply::json(200, &answer),
            None => Reply::json(404, &json!({ "error": "not_found" })),
        }
    })
}

fn logged_in(server: &str) -> Scratch {
    let home = Scratch::new("home");
    write_config(&home.path, server, WORKSPACE, API_KEY);
    home
}

fn flows(home: &Scratch, args: &[&str]) -> Out {
    omni(&home.path)
        .args(&["design", "flows"])
        .args(args)
        .args(&["--project", PROJECT, "--as", "run-22/designer"])
        .run()
}

fn paths(server: &MockServer) -> Vec<String> {
    server.requests().into_iter().map(|r| r.path).collect()
}

fn assert_error(out: &Out, line: &str) {
    assert_eq!(
        out.code, 2,
        "stdout: {:?}; stderr: {:?}",
        out.stdout, out.stderr
    );
    assert_eq!(out.stdout, "", "an error prints nothing on standard output");
    let lines = out.stderr_lines();
    assert_eq!(
        lines.len(),
        1,
        "one line on standard error: {:?}",
        out.stderr
    );
    assert!(
        lines[0].contains(line),
        "{:?} should contain {line:?}",
        lines[0]
    );
}

#[test]
fn a_page_prints_its_flows_interactions_unreachable_boards_and_dangling_targets() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = flows(&home, &["--doc", DOC, "--page", SHOP]);
    assert_eq!(out.code, 0, "a dangling target is no error: {}", out.stderr);
    assert_eq!(
        out.stdout.trim_end(),
        "Page Shop (pg000001)
  flows: Checkout → Cart (crt00001); Returns → Returns (rtn00003)
  Cart (crt00001): 2 interactions
    Pay button (pay00001) onClick → navigate Confirmation (cnf00002) · slideIn left · 300 ms · easeOut
    Promo (prm00002) onClick → navigate h2q9d7ma (missing) · slideIn left · 300 ms · easeOut
  Confirmation (cnf00002): no interactions
  Returns (rtn00003): no interactions
  Settings (stg00004): no interactions
  unreachable: Settings (stg00004)
  dangling: Cart/Promo (prm00002) onClick → navigate h2q9d7ma (missing)"
    );
}

#[test]
fn a_board_is_reached_only_by_a_flow_start_or_a_board_action_on_another_board() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = flows(&home, &["--doc", DOC, "--page", GRAPH]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(lines.contains(&"  flows: none"), "{lines:#?}");
    assert!(
        lines.contains(&"    A (grfa0001) onClick → navigate B (grfb0002), openOverlay C (grfc0003) · slideIn left · 300 ms · easeOut"),
        "{lines:#?}"
    );
    assert!(
        lines.contains(&"  unreachable: A (grfa0001); E (grfe0005)"),
        "B by navigate, C by open overlay and D by swap overlay are reached; E points only at itself: {lines:#?}"
    );
    assert!(lines.contains(&"  dangling: none"), "{lines:#?}");
}

#[test]
fn a_target_on_another_page_is_named_and_a_flow_start_that_is_gone_is_missing_but_not_dangling() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = flows(&home, &["--doc", DOC, "--page", ACCOUNT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(
        lines.contains(&"  flows: Checkout → gne00009 (missing)"),
        "{lines:#?}"
    );
    assert!(
        lines.contains(&"    Edit (edt00006) onClick → navigate Cart (crt00001) · slideIn left · 300 ms · easeOut"),
        "{lines:#?}"
    );
    assert!(lines.contains(&"  dangling: none"), "{lines:#?}");
}

#[test]
fn the_command_reads_the_outline_once_and_only_the_boards_of_the_pages_it_prints() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = flows(&home, &["--doc", DOC, "--page", ACCOUNT]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.starts_with("Page Account (pg000002)\n"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("Page Shop"), "{}", out.stdout);
    assert_eq!(paths(&server), [outline_path(DOC), board_path(PROFILE)]);

    let server = mock();
    let home = logged_in(&server.url);
    let out = flows(&home, &["--doc", DOC]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout
            .contains("(missing)\n\nPage Account (pg000002)\n"),
        "pages in outline order with a blank line between: {}",
        out.stdout
    );
    let mut expected = vec![outline_path(DOC)];
    expected.extend(
        [
            CART,
            CONFIRMATION,
            RETURNS,
            SETTINGS,
            PROFILE,
            A,
            B,
            C,
            D,
            E,
        ]
        .map(board_path),
    );
    assert_eq!(paths(&server), expected);
}

#[test]
fn a_page_the_design_does_not_have_exits_2_and_reads_no_board() {
    let server = mock();
    let home = logged_in(&server.url);
    let out = flows(&home, &["--doc", DOC, "--page", "zzzzzzzz"]);
    assert_error(&out, "zzzzzzzz is not a page of the design");
    assert_eq!(paths(&server), [outline_path(DOC)]);
}

#[test]
fn a_doc_that_is_not_a_design_exits_2() {
    let server = mock();
    let home = logged_in(&server.url);
    assert_error(
        &flows(&home, &["--doc", NOT_A_DESIGN]),
        &format!("{NOT_A_DESIGN} is not a design"),
    );
    assert_error(
        &flows(&home, &["--doc", NO_OUTLINE]),
        &format!("{NO_OUTLINE} is not a design"),
    );
}
