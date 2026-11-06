//! US-42: the converter writes Penpot's prototype interactions and flows onto
//! the design (docs/formats/penpot-export.md, "Left out, and reported"), on
//! small exports that the tests build.
//!
//! - A `destination` resolves to the node of a top-level board of any page of
//!   the file once every page is converted. A nested frame, a left-out shape or
//!   an unknown id drops the interaction with the entry "prototype link to
//!   <destination> was not imported".
//! - `open-overlay` keeps its position (`center` when Penpot names none), a
//!   manual `overlayPosition` as `offset`, `closeClickOutside` (off unless set)
//!   and `backgroundOverlay` as `#00000033`; `toggle-overlay` opens an overlay
//!   and is listed. Unknown event or action types are dropped and listed.
//! - A shape keeps its first 64 interactions and a page its first 8 flows, cut
//!   from Penpot's raw list, and the cut is listed once with how many.
//! - A flow is written on the page design of its starting board, named `Flow`
//!   when it has no name; one whose board was not imported, or whose name that
//!   board already has, is dropped and listed under `flows`.

mod common;

use std::collections::HashMap;

use omni_core::design::Node;
use omni_core::design::proto::{
    Action, Direction, Easing, Interaction, OverlayAnchor, OverlayPosition, Transition, Trigger,
};
use omni_penpot::import::{Ids, import_file};
use omni_penpot::{Conversion, ReportEntry, convert_file};
use serde_json::{Value, json};

use common::convert::*;
use common::{SyntheticPage, export_entries, stored_zip, write_temp};

/// A page numbered `n` with `top_level` on it.
fn numbered_page(n: u32, top_level: Vec<S>) -> SyntheticPage {
    let mut found = page(top_level);
    found.id = format!("a0000000-0000-4000-8000-{n:012x}");
    found.name = format!("Page {n}");
    found.index = n - 1;
    found
}

/// The path of an export of `pages`, each with the `flows` Penpot keeps on the
/// page, keyed by flow id.
fn write_pages(test: &str, pages: &[(SyntheticPage, Value)]) -> std::path::PathBuf {
    let synthetic: Vec<SyntheticPage> = pages
        .iter()
        .map(|(page, _)| SyntheticPage {
            id: page.id.clone(),
            name: page.name.clone(),
            index: page.index,
            top_level: page.top_level.clone(),
            shapes: page.shapes.clone(),
        })
        .collect();
    let mut entries = export_entries(FILE_ID, "Synthetic", &synthetic);
    for (page, flows) in pages {
        let name = format!("files/{FILE_ID}/pages/{}.json", page.id);
        let entry = entries
            .iter_mut()
            .find(|(entry, _)| *entry == name)
            .expect("the page entry");
        entry.1 = json!({ "id": page.id, "name": page.name, "index": page.index, "flows": flows })
            .to_string()
            .into_bytes();
    }
    write_temp(
        &format!("convert-proto-{test}.penpot"),
        &stored_zip(&entries),
    )
}

/// Converts an export of `pages`, checking that every design validates.
fn convert_pages(test: &str, pages: &[(SyntheticPage, Value)]) -> Conversion {
    let path = write_pages(test, pages);
    let conversion = convert_file(&path).expect("the export converts");
    for board in &conversion.boards {
        assert_validates(&board.design, &board.name);
    }
    conversion
}

/// The interactions written on the node called `name` of the board `board`.
fn interactions_of<'a>(
    conversion: &'a Conversion,
    board_name: &str,
    name: &str,
) -> &'a [Interaction] {
    node(board_node(board(conversion, board_name)), name).interactions()
}

/// The ID of the node of the board called `name`.
fn board_id(conversion: &Conversion, name: &str) -> String {
    board_node(board(conversion, name)).id().to_string()
}

/// The `(name, start)` of each flow written on the design of the board `name`.
fn flows_of(conversion: &Conversion, name: &str) -> Vec<(String, String)> {
    json_of(board(conversion, name))["flows"]
        .as_array()
        .map(|flows| {
            flows
                .iter()
                .map(|flow| (text(&flow["name"]), text(&flow["start"])))
                .collect()
        })
        .unwrap_or_default()
}

fn text(value: &Value) -> String {
    value.as_str().expect("a string").to_string()
}

/// The entries of the report under `property` for the shape `shape_id`.
fn listed<'a>(conversion: &'a Conversion, shape_id: &str, property: &str) -> Vec<&'a ReportEntry> {
    entries_of(conversion, shape_id)
        .into_iter()
        .filter(|entry| entry.property.as_deref() == Some(property))
        .collect()
}

/// An `onClick` interaction that runs `action` with no animation.
fn instant(action: Action) -> Interaction {
    Interaction {
        trigger: Trigger::OnClick,
        actions: vec![action],
        transition: Transition::Instant,
        duration_ms: 0,
        easing: Easing::Linear,
        delay_ms: 0,
    }
}

fn navigate(dest: &str) -> Action {
    Action::Navigate {
        dest: dest.to_string(),
        preserve_scroll: false,
    }
}

/// A Penpot click that navigates to `destination`.
fn click_to(destination: &str) -> Value {
    json!({ "eventType": "click", "actionType": "navigate", "destination": destination })
}

/// A rect numbered `n` called `name` with the Penpot `interactions`.
fn trigger_rect(n: u32, name: &str, y: f64, interactions: Value) -> S {
    shape(n, "rect", name, [0.0, y, 50.0, 20.0]).with(json!({ "interactions": interactions }))
}

#[test]
fn a_link_to_a_board_on_another_page_names_that_boards_node() {
    let first = numbered_page(
        1,
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![trigger_rect(
                2,
                "Next",
                0.0,
                json!([click_to(&uid(10))]),
            )]),
        ],
    );
    let second = numbered_page(
        2,
        vec![shape(10, "frame", "Settings", [0.0, 0.0, 400.0, 300.0])],
    );
    let conversion = convert_pages("other-page", &[(first, json!({})), (second, json!({}))]);

    let settings = board_id(&conversion, "Settings");
    assert_eq!(
        interactions_of(&conversion, "Home", "Next"),
        [instant(navigate(&settings))]
    );
    assert!(
        entries_of(&conversion, &uid(2)).is_empty(),
        "{:#?}",
        conversion.report.entries
    );
}

#[test]
fn a_link_to_a_nested_frame_or_an_unknown_id_is_dropped_and_listed() {
    let top_level = vec![
        shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
            trigger_rect(2, "To nested", 0.0, json!([click_to(&uid(11))])),
            trigger_rect(3, "To nowhere", 30.0, json!([click_to(&uid(99))])),
        ]),
        shape(10, "frame", "Details", [500.0, 0.0, 400.0, 300.0]).kids(vec![shape(
            11,
            "frame",
            "Card",
            [500.0, 0.0, 100.0, 100.0],
        )]),
    ];
    let conversion = convert_pages("not-imported", &[(page(top_level), json!({}))]);

    for (n, name, destination) in [(2, "To nested", uid(11)), (3, "To nowhere", uid(99))] {
        assert!(
            interactions_of(&conversion, "Home", name).is_empty(),
            "{name} links nowhere"
        );
        let entries = listed(&conversion, &uid(n), "interactions");
        assert_eq!(entries.len(), 1, "{:#?}", conversion.report.entries);
        assert_eq!(entries[0].name, name);
        assert_eq!(entries[0].board, "Home");
        assert_eq!(
            entries[0].reason,
            format!("prototype link to {destination} was not imported")
        );
    }
}

#[test]
fn an_overlay_keeps_its_position_offset_close_and_background_and_a_toggle_is_listed() {
    let items = json!([
        {
            "eventType": "click", "actionType": "open-overlay", "destination": uid(10),
            "overlayPosType": "manual", "overlayPosition": { "x": 12, "y": 40 },
            "closeClickOutside": true, "backgroundOverlay": true
        },
        { "eventType": "click", "actionType": "toggle-overlay", "destination": uid(10) }
    ]);
    let top_level = vec![
        shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0])
            .kids(vec![trigger_rect(2, "Menu", 0.0, items)]),
        shape(10, "frame", "Sheet", [500.0, 0.0, 200.0, 100.0]),
    ];
    let conversion = convert_pages("overlay", &[(page(top_level), json!({}))]);

    let sheet = board_id(&conversion, "Sheet");
    assert_eq!(
        interactions_of(&conversion, "Home", "Menu"),
        [
            instant(Action::OpenOverlay {
                dest: sheet.clone(),
                position: OverlayPosition::Manual,
                close_on_outside: true,
                background: Some("#00000033".to_string()),
                offset: Some([12.0, 40.0]),
                anchor: OverlayAnchor::Screen,
            }),
            instant(Action::OpenOverlay {
                dest: sheet,
                position: OverlayPosition::Center,
                close_on_outside: false,
                background: None,
                offset: None,
                anchor: OverlayAnchor::Screen,
            }),
        ]
    );
    let entries = listed(&conversion, &uid(2), "interactions");
    assert_eq!(entries.len(), 1, "{:#?}", conversion.report.entries);
    assert!(
        entries[0]
            .reason
            .contains("toggle-overlay imported as open overlay"),
        "{:?}",
        entries[0].reason
    );
}

#[test]
fn each_penpot_event_type_becomes_its_trigger() {
    let items = json!([
        { "eventType": "click", "actionType": "close-overlay" },
        { "eventType": "mouse-press", "actionType": "close-overlay" },
        { "eventType": "mouse-over", "actionType": "close-overlay" },
        { "eventType": "mouse-enter", "actionType": "close-overlay" },
        { "eventType": "mouse-leave", "actionType": "close-overlay" },
        { "eventType": "after-delay", "delay": 250, "actionType": "close-overlay" },
        { "eventType": "after-delay", "actionType": "close-overlay" }
    ]);
    let top_level = vec![
        shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0])
            .kids(vec![trigger_rect(2, "Area", 0.0, items)]),
    ];
    let conversion = convert_pages("triggers", &[(page(top_level), json!({}))]);

    let triggers: Vec<&Trigger> = interactions_of(&conversion, "Home", "Area")
        .iter()
        .map(|interaction| &interaction.trigger)
        .collect();
    assert_eq!(
        triggers,
        [
            &Trigger::OnClick,
            &Trigger::OnPress,
            &Trigger::OnHover,
            &Trigger::MouseEnter,
            &Trigger::MouseLeave,
            &Trigger::AfterDelay { ms: 250 },
            &Trigger::AfterDelay { ms: 600 },
        ]
    );
}

#[test]
fn each_penpot_action_type_becomes_its_action() {
    let items = json!([
        { "eventType": "click", "actionType": "navigate", "destination": uid(10), "preserveScroll": true },
        { "eventType": "click", "actionType": "close-overlay" },
        { "eventType": "click", "actionType": "prev-screen" },
        { "eventType": "click", "actionType": "open-url", "url": "https://example.com/help" }
    ]);
    let top_level = vec![
        shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0])
            .kids(vec![trigger_rect(2, "Area", 0.0, items)]),
        shape(10, "frame", "Details", [500.0, 0.0, 400.0, 300.0]),
    ];
    let conversion = convert_pages("actions", &[(page(top_level), json!({}))]);

    let actions: Vec<&Action> = interactions_of(&conversion, "Home", "Area")
        .iter()
        .flat_map(|interaction| &interaction.actions)
        .collect();
    assert_eq!(
        actions,
        [
            &Action::Navigate {
                dest: board_id(&conversion, "Details"),
                preserve_scroll: true,
            },
            &Action::CloseOverlay,
            &Action::Back,
            &Action::OpenUrl {
                url: "https://example.com/help".to_string(),
                new_tab: true,
            },
        ]
    );
}

#[test]
fn unknown_event_and_action_types_are_dropped_and_listed_with_the_type() {
    let items = json!([
        { "eventType": "mouse-wheel", "actionType": "close-overlay" },
        { "eventType": "click", "actionType": "scroll-to" },
        { "eventType": "click", "actionType": "prev-screen" }
    ]);
    let top_level = vec![
        shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0])
            .kids(vec![trigger_rect(2, "Area", 0.0, items)]),
    ];
    let conversion = convert_pages("unknown", &[(page(top_level), json!({}))]);

    assert_eq!(
        interactions_of(&conversion, "Home", "Area"),
        [instant(Action::Back)]
    );
    let entries = listed(&conversion, &uid(2), "interactions");
    assert_eq!(entries.len(), 2, "{:#?}", conversion.report.entries);
    for (entry, kind) in entries.iter().zip(["mouse-wheel", "scroll-to"]) {
        assert_eq!(entry.name, "Area");
        assert!(
            entry.reason.contains(kind),
            "{:?} names {kind}",
            entry.reason
        );
    }
}

#[test]
fn animations_become_transitions_with_their_duration_and_easing() {
    let animated = |animation: Value| json!({ "eventType": "click", "actionType": "prev-screen", "animation": animation });
    let items = json!([
        animated(json!({ "animationType": "dissolve", "duration": 100, "easing": "ease-in" })),
        animated(json!({ "animationType": "slide", "way": "out", "direction": "right", "duration": 200, "easing": "ease-out" })),
        animated(json!({ "animationType": "slide", "way": "in", "direction": "up", "duration": 300, "easing": "ease-in-out" })),
        animated(json!({ "animationType": "push", "direction": "down", "duration": 400, "easing": "linear" })),
        animated(json!({ "animationType": "dissolve", "duration": 500, "easing": "bounce-wildly" })),
        { "eventType": "click", "actionType": "prev-screen" }
    ]);
    let top_level = vec![
        shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0])
            .kids(vec![trigger_rect(2, "Back", 0.0, items)]),
    ];
    let conversion = convert_pages("animations", &[(page(top_level), json!({}))]);

    let played: Vec<(Transition, u32, Easing)> = interactions_of(&conversion, "Home", "Back")
        .iter()
        .map(|interaction| {
            (
                interaction.transition,
                interaction.duration_ms,
                interaction.easing,
            )
        })
        .collect();
    assert_eq!(
        played,
        [
            (Transition::Dissolve, 100, Easing::EaseIn),
            (
                Transition::SlideOut {
                    direction: Direction::Right
                },
                200,
                Easing::EaseOut
            ),
            (
                Transition::SlideIn {
                    direction: Direction::Top
                },
                300,
                Easing::EaseInOut
            ),
            (
                Transition::Push {
                    direction: Direction::Bottom
                },
                400,
                Easing::Linear
            ),
            (Transition::Dissolve, 500, Easing::Linear),
            (Transition::Instant, 0, Easing::Linear),
        ]
    );
}

#[test]
fn a_shape_keeps_the_first_64_of_its_raw_interactions_and_lists_how_many_were_cut() {
    // 66 interactions, the first of an unknown type: the cap takes 64 of the
    // raw list, so 63 are written and 2 are cut.
    let mut over: Vec<Value> =
        vec![json!({ "eventType": "mouse-wheel", "actionType": "prev-screen" })];
    over.extend(std::iter::repeat_n(
        json!({ "eventType": "click", "actionType": "prev-screen" }),
        65,
    ));
    let at_cap = vec![json!({ "eventType": "click", "actionType": "prev-screen" }); 64];
    let top_level = vec![
        shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![
            trigger_rect(2, "Over", 0.0, Value::Array(over)),
            trigger_rect(3, "At cap", 30.0, Value::Array(at_cap)),
        ]),
    ];
    let conversion = convert_pages("cap", &[(page(top_level), json!({}))]);

    assert_eq!(interactions_of(&conversion, "Home", "Over").len(), 63);
    let entries = listed(&conversion, &uid(2), "interactions");
    assert_eq!(
        entries.len(),
        2,
        "the unknown type and the cut: {entries:#?}"
    );
    let cut: Vec<_> = entries
        .iter()
        .filter(|entry| !entry.reason.contains("mouse-wheel"))
        .collect();
    assert_eq!(cut.len(), 1, "{entries:#?}");
    assert!(
        cut[0].reason.contains('2'),
        "the entry says 2 were cut: {:?}",
        cut[0].reason
    );

    assert_eq!(interactions_of(&conversion, "Home", "At cap").len(), 64);
    assert!(
        entries_of(&conversion, &uid(3)).is_empty(),
        "{:#?}",
        conversion.report.entries
    );
}

/// A Penpot flow called `name` that starts at the shape `start`.
fn flow(id: u32, name: &str, start: &str) -> (String, Value) {
    (
        uid(id),
        json!({ "id": uid(id), "name": name, "startingFrame": start }),
    )
}

fn flows(list: Vec<(String, Value)>) -> Value {
    Value::Object(list.into_iter().collect())
}

#[test]
fn flows_go_on_the_design_of_their_starting_board_up_to_8_a_page() {
    let top_level = vec![
        shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]),
        shape(10, "frame", "Details", [500.0, 0.0, 400.0, 300.0]),
    ];
    let page_flows = flows(vec![
        flow(30, "", &uid(1)),
        flow(31, "Main", &uid(1)),
        flow(32, "Side", &uid(10)),
        flow(33, "F4", &uid(10)),
        flow(34, "F5", &uid(10)),
        flow(35, "F6", &uid(10)),
        flow(36, "F7", &uid(10)),
        flow(37, "F8", &uid(10)),
        flow(38, "Cut", &uid(10)),
    ]);
    let conversion = convert_pages("flows", &[(page(top_level), page_flows)]);

    let home = board_id(&conversion, "Home");
    let details = board_id(&conversion, "Details");
    assert_eq!(
        flows_of(&conversion, "Home"),
        [
            ("Flow".to_string(), home.clone()),
            ("Main".to_string(), home)
        ]
    );
    let names: Vec<String> = ["Side", "F4", "F5", "F6", "F7", "F8"]
        .map(String::from)
        .into();
    assert_eq!(
        flows_of(&conversion, "Details"),
        names
            .into_iter()
            .map(|name| (name, details.clone()))
            .collect::<Vec<_>>()
    );
    let cut = listed(&conversion, PAGE_ID, "flows");
    assert_eq!(cut.len(), 1, "{:#?}", conversion.report.entries);
    assert!(
        cut[0].reason.contains('1'),
        "the entry says 1 was cut: {:?}",
        cut[0].reason
    );
}

#[test]
fn a_flow_from_a_board_not_imported_or_with_a_name_its_board_has_is_dropped_and_listed() {
    let top_level = vec![shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0])];
    let page_flows = flows(vec![
        flow(30, "Main", &uid(1)),
        flow(31, "Main", &uid(1)),
        flow(32, "Lost", &uid(99)),
    ]);
    let conversion = convert_pages("dropped-flows", &[(page(top_level), page_flows)]);

    let home = board_id(&conversion, "Home");
    assert_eq!(flows_of(&conversion, "Home"), [("Main".to_string(), home)]);
    assert_eq!(
        listed(&conversion, &uid(1), "flows").len(),
        1,
        "the second \"Main\" is listed: {:#?}",
        conversion.report.entries
    );
    let lost = listed(&conversion, &uid(99), "flows");
    assert_eq!(lost.len(), 1, "{:#?}", conversion.report.entries);
    assert_eq!(
        lost[0].reason,
        "the prototype flow \"Lost\" starts at a board that was not imported; the flow is left out"
    );
}

/// The `id` of the node called `name` in the JSON `tree`.
fn id_named(tree: &Value, name: &str) -> Option<String> {
    if tree["name"] == name {
        return tree["id"].as_str().map(str::to_string);
    }
    tree["children"]
        .as_array()?
        .iter()
        .find_map(|child| id_named(child, name))
}

#[test]
fn the_import_sets_a_flow_on_its_own_page_and_links_by_the_imported_board_ids() {
    let first = numbered_page(
        1,
        vec![
            shape(1, "frame", "Home", [0.0, 0.0, 400.0, 300.0]).kids(vec![trigger_rect(
                2,
                "Next",
                0.0,
                json!([click_to(&uid(20))]),
            )]),
            shape(10, "frame", "Checkout", [500.0, 0.0, 400.0, 300.0]),
        ],
    );
    let second = numbered_page(
        2,
        vec![shape(20, "frame", "Settings", [0.0, 0.0, 400.0, 300.0])],
    );
    let path = write_pages(
        "import",
        &[
            (first, flows(vec![flow(30, "Pay", &uid(10))])),
            (second, json!({})),
        ],
    );
    let import = import_file(&path).expect("the export imports");
    let parts = import
        .design_parts(&Ids::default())
        .expect("the pages as tags");

    assert_eq!(parts.len(), 2);
    assert_eq!(
        parts[0].flows,
        [("Pay".to_string(), 1)],
        "the flow starts at Checkout"
    );
    assert!(parts[1].flows.is_empty());
    let pages: Vec<Value> = import
        .pages
        .iter()
        .map(|page: &Node| serde_json::to_value(page).expect("a page is JSON"))
        .collect();
    let settings = id_named(&pages[1], "Settings").expect("Settings is imported");
    assert_eq!(parts[1].board_ids, [settings.clone()]);
    assert_eq!(
        parts[0].interactions.len(),
        1,
        "Next's link waits for the boards"
    );
    let link = &parts[0].interactions[0];
    assert_eq!(link.board, 0, "Next is on Home");
    let servers = HashMap::from([(settings, "srv00001".to_string())]);
    let written: Value =
        serde_json::from_str(&link.json(&servers).expect("the link resolves")).expect("JSON");
    assert_eq!(written[0]["actions"][0]["dest"], "srv00001");
}
