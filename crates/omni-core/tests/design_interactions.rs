//! US-17: the `interactions` attribute on layers. A layer lists what it does
//! when the person using a prototype touches it, written in the tag format
//! like `fill`, with FigCraft's names.

use omni_core::design::proto::{
    Action, Easing, OverlayAnchor, OverlayPosition, Transition, Trigger,
};
use omni_core::design::proto::{Overflow, ScrollPosition};
use omni_core::design::tags::{self, TagError};
use omni_core::design::{Node, NodeData, NodeKind};
use serde_json::{Value, json};

const CARD: &str = r#"<Frame id="aaaaaaaa" name="Card" width={100} height={40} interactions="#;

fn frame_with(interactions: &str) -> String {
    format!("{CARD}{{{interactions}}}/>")
}

fn existing(source: &str) -> Node {
    tags::parse(source)
        .unwrap_or_else(|error| panic!("these tags must parse, but: {error}\n{source}"))
        .assign_ids(|| panic!("every tag has an `id`"))
}

/// Parses `source`, prints the node and parses that again: the same node and
/// the same tags. Returns the node and the printed tags.
fn round_trip(source: &str) -> (Node, String) {
    let node = existing(source);
    let printed = tags::print(&node);
    let back = existing(&printed);
    assert!(back == node, "printing lost something:\n{printed}");
    assert_eq!(tags::print(&back), printed, "printing is not stable");
    (node, printed)
}

fn error_of(source: &str) -> TagError {
    match tags::parse(source) {
        Ok(tree) => panic!("these tags must be rejected, but they parsed as {tree:?}"),
        Err(error) => error,
    }
}

#[test]
fn a_frame_reads_back_its_interaction_with_the_same_values() {
    let (node, printed) = round_trip(&frame_with(
        r#"[{trigger: {type: "onClick"}, actions: [{type: "navigate", dest: "k3f9x2qa", preserveScroll: true}], transition: {type: "slideIn", direction: "left"}, durationMs: 300, easing: "easeOut"}]"#,
    ));
    let [interaction] = node.interactions() else {
        panic!("one interaction")
    };
    assert_eq!(interaction.trigger, Trigger::OnClick);
    assert_eq!(
        interaction.actions,
        [Action::Navigate {
            dest: "k3f9x2qa".to_string(),
            preserve_scroll: true
        }]
    );
    assert!(matches!(interaction.transition, Transition::SlideIn { .. }));
    assert!(printed.contains("slideIn") && printed.contains("preserveScroll: true"));
    assert!(!printed.contains("durationMs"), "{printed}");
    assert!(!printed.contains("easing"), "{printed}");
}

#[test]
fn every_trigger_action_transition_and_easing_round_trips() {
    let triggers = [
        r#"{type: "onClick"}"#,
        r#"{type: "onHover"}"#,
        r#"{type: "onPress"}"#,
        r#"{type: "onDrag"}"#,
        r#"{type: "mouseEnter"}"#,
        r#"{type: "mouseLeave"}"#,
        r#"{type: "mouseDown"}"#,
        r#"{type: "mouseUp"}"#,
        r#"{type: "afterDelay", ms: 600}"#,
        r#"{type: "keyPress", key: "Shift+K"}"#,
    ];
    let actions = [
        r#"{type: "navigate", dest: "b1"}"#,
        r#"{type: "back"}"#,
        r#"{type: "scrollTo", dest: "b1", offset: [0, 20]}"#,
        r##"{type: "openOverlay", dest: "b1", position: "manual", offset: [12, 40], anchor: "triggerOutside", background: "#00000033", closeOnOutside: false}"##,
        r#"{type: "swapOverlay", dest: "b1"}"#,
        r#"{type: "closeOverlay"}"#,
        r#"{type: "openUrl", url: "https://example.com", newTab: false}"#,
        r#"{type: "changeTo", dest: "c1"}"#,
        r#"{type: "setVariable", variable: "count", value: 1}"#,
        r#"{type: "setVariable", variable: "count", expr: {op: "add", args: [{variable: "count"}, {value: 1}]}}"#,
        r#"{type: "setVariableMode", collection: "color", mode: "dark"}"#,
        r#"{type: "conditional", blocks: [{condition: {op: "equals", args: [{variable: "a"}, {value: 1}]}, actions: [{type: "back"}]}, {actions: [{type: "closeOverlay"}]}]}"#,
    ];
    let mut transitions = vec![
        r#"{type: "instant"}"#.to_string(),
        r#"{type: "dissolve"}"#.to_string(),
        r#"{type: "smartAnimate"}"#.to_string(),
    ];
    for kind in ["moveIn", "moveOut", "push", "slideIn", "slideOut"] {
        for direction in ["left", "right", "top", "bottom"] {
            transitions.push(format!(r#"{{type: "{kind}", direction: "{direction}"}}"#));
        }
    }
    let mut easings: Vec<String> = [
        "linear",
        "easeIn",
        "easeOut",
        "easeInOut",
        "easeInBack",
        "easeOutBack",
        "easeInOutBack",
        "gentle",
        "quick",
        "bouncy",
        "slow",
    ]
    .iter()
    .map(|name| format!("\"{name}\""))
    .collect();
    easings.push(r#"{type: "customBezier", x1: 0.2, y1: 0, x2: 0, y2: 1}"#.to_string());
    easings.push(r#"{type: "customSpring", mass: 1, stiffness: 100, damping: 15}"#.to_string());

    let mut items = Vec::new();
    for trigger in triggers {
        items.push(format!(
            "{{trigger: {trigger}, actions: [{}]}}",
            actions.join(", ")
        ));
    }
    for transition in &transitions {
        items.push(format!(
            "{{actions: [{{type: \"back\"}}], transition: {transition}}}"
        ));
    }
    for easing in &easings {
        items.push(format!(
            "{{actions: [{{type: \"back\"}}], easing: {easing}}}"
        ));
    }
    let (node, _) = round_trip(&frame_with(&format!("[{}]", items.join(", "))));
    assert_eq!(node.interactions().len(), items.len());
    assert!(matches!(
        node.interactions().last().map(|i| i.easing),
        Some(Easing::CustomSpring { .. })
    ));
}

#[test]
fn an_interaction_with_only_actions_reads_back_with_the_defaults_and_prints_none_of_them() {
    let (node, printed) = round_trip(&frame_with(
        r#"[{trigger: "onClick", actions: [{type: "navigate", dest: "k3f9x2qa"}]}]"#,
    ));
    let interaction = &node.interactions()[0];
    assert_eq!(interaction.trigger, Trigger::OnClick);
    assert_eq!(interaction.transition, Transition::Instant);
    assert_eq!(interaction.duration_ms, 300);
    assert_eq!(interaction.easing, Easing::EaseOut);
    assert_eq!(interaction.delay_ms, 0);
    for field in ["trigger", "transition", "durationMs", "easing", "delayMs"] {
        assert!(!printed.contains(field), "{field} printed: {printed}");
    }
    let (_, printed) = round_trip(&frame_with(
        r#"[{trigger: "onHover", actions: [{type: "back"}]}]"#,
    ));
    assert!(
        printed.contains(r#"trigger: {type: "onHover"}"#),
        "{printed}"
    );
}

#[test]
fn every_layer_tag_takes_interactions_and_a_page_does_not() {
    let list = r#"interactions={[{actions: [{type: "back"}]}]}"#;
    let sources = [
        format!(r#"<Board id="aaaaaaaa" name="B" width={{10}} height={{10}} {list}/>"#),
        format!(
            r#"<Text id="aaaaaaaa" name="T" content="x" fontFamily="Inter" fontSize={{12}} width="hug" height="hug" {list}/>"#
        ),
        format!(
            r#"<Image id="aaaaaaaa" name="I" image="attachment:01a1ad2e-d5dc-7f82-a750-fbf6e9721e15" fit="cover" width={{10}} height={{10}} {list}/>"#
        ),
        format!(
            r#"<Vector id="aaaaaaaa" name="V" path="M0 0 L1 1" width={{10}} height={{10}} {list}/>"#
        ),
        format!(r#"<Instance id="aaaaaaaa" name="N" component="bbbbbbbb" {list}/>"#),
        format!(r#"<Component id="aaaaaaaa" name="C" width={{10}} height={{10}} {list}/>"#),
    ];
    for source in sources {
        let (node, _) = round_trip(&source);
        assert_eq!(node.interactions().len(), 1, "{source}");
    }
    assert_eq!(
        existing(&frame_with(r#"[{actions: []}]"#)).kind(),
        NodeKind::Frame
    );
    let error = error_of(&format!(r#"<Page id="aaaaaaaa" name="P" {list}/>"#));
    assert!(
        error
            .message()
            .contains("`interactions` is not an attribute of `<Page>`"),
        "{error}"
    );
}

#[test]
fn set_adds_changes_and_removes_interactions() {
    let mut node = existing(r#"<Frame id="aaaaaaaa" name="Card" width={100} height={40}/>"#);
    let set = |node: &mut Node, value: Value| {
        let properties = json!({"interactions": value});
        tags::set_properties(node, None, properties.as_object().unwrap())
    };
    set(
        &mut node,
        json!([{"actions": [{"type": "navigate", "dest": "k3f9x2qa"}]}]),
    )
    .expect("a list of interactions is accepted");
    assert_eq!(node.interactions().len(), 1);
    set(&mut node, Value::Null).expect("null removes interactions");
    assert!(node.interactions().is_empty());
}

#[test]
fn errors_name_the_problem_and_the_fix() {
    let cases = [
        (
            r#"[{trigger: {type: "tap"}, actions: []}]"#,
            "`tap` is not a trigger",
            "write one of onClick, onHover, onPress, onDrag, mouseEnter, mouseLeave, mouseDown, mouseUp, afterDelay, keyPress",
        ),
        (
            r#"[{actions: [{type: "navigate"}]}]"#,
            "`dest` is required for navigate",
            "dest",
        ),
        (
            r#"[{actions: [], durationMs: -1}]"#,
            "`durationMs` is a whole number of milliseconds",
            "write 0 or more",
        ),
        (
            r#"[{actions: [{type: "teleport"}]}]"#,
            "`teleport` is not an action",
            "navigate",
        ),
        (
            r#"[{actions: [], transition: {type: "slideIn"}}]"#,
            "`direction` is required for slideIn",
            "left",
        ),
        (
            r#"[{actions: [], easing: "bumpy"}]"#,
            "`bumpy` is not an easing",
            "customSpring",
        ),
        (
            r#"[{trigger: "click", actions: []}]"#,
            "`click` is not a trigger",
            "onClick",
        ),
    ];
    for (interactions, message, fix) in cases {
        let error = error_of(&frame_with(interactions));
        assert!(error.message().contains(message), "{interactions}: {error}");
        assert!(error.fix().contains(fix), "{interactions}: {error}");
    }
}

#[test]
fn actions_left_without_their_optional_fields_take_figcraft_defaults_and_print_none_of_them() {
    let (node, printed) = round_trip(&frame_with(
        r#"[{actions: [{type: "navigate", dest: "b1"}, {type: "scrollTo", dest: "b1"}, {type: "openOverlay", dest: "b1"}, {type: "openUrl", url: "https://example.com"}]}]"#,
    ));
    let [navigate, scroll, overlay, url] = &node.interactions()[0].actions[..] else {
        panic!("four actions")
    };
    assert_eq!(
        *navigate,
        Action::Navigate {
            dest: "b1".to_string(),
            preserve_scroll: false
        }
    );
    assert_eq!(
        *scroll,
        Action::ScrollTo {
            dest: "b1".to_string(),
            offset: [0.0, 0.0]
        }
    );
    let Action::OpenOverlay {
        position,
        close_on_outside,
        anchor,
        ..
    } = overlay
    else {
        panic!("an openOverlay action: {overlay:?}")
    };
    assert_eq!(*position, OverlayPosition::Center);
    assert!(*close_on_outside);
    assert_eq!(*anchor, OverlayAnchor::Screen);
    assert_eq!(
        *url,
        Action::OpenUrl {
            url: "https://example.com".to_string(),
            new_tab: true
        }
    );
    for field in [
        "preserveScroll",
        "offset",
        "position",
        "closeOnOutside",
        "anchor",
        "newTab",
    ] {
        assert!(!printed.contains(field), "{field} printed: {printed}");
    }
}

#[test]
fn a_plain_easing_written_as_an_object_is_printed_as_its_name() {
    let (node, printed) = round_trip(&frame_with(
        r#"[{actions: [{type: "back"}], easing: {type: "linear"}}]"#,
    ));
    assert_eq!(node.interactions()[0].easing, Easing::Linear);
    assert!(printed.contains(r#"easing: "linear""#), "{printed}");
}

#[test]
fn a_missing_required_field_is_named_with_its_type() {
    let cases = [
        (
            r#"[{trigger: {type: "afterDelay"}, actions: []}]"#,
            "`ms` is required for afterDelay",
        ),
        (
            r#"[{trigger: {type: "keyPress"}, actions: []}]"#,
            "`key` is required for keyPress",
        ),
        (
            r#"[{actions: [{type: "openUrl"}]}]"#,
            "`url` is required for openUrl",
        ),
        (
            r#"[{actions: [], easing: {type: "customBezier", x1: 0.2, y1: 0, x2: 0}}]"#,
            "`y2` is required for customBezier",
        ),
        (
            r#"[{actions: [{type: "setVariable", variable: "count"}]}]"#,
            "`value` is required for setVariable",
        ),
    ];
    for (interactions, message) in cases {
        let error = error_of(&frame_with(interactions));
        assert!(error.message().contains(message), "{interactions}: {error}");
    }
}

const SCROLL_BOARD: &str = r#"<Board id="bbbbbbbb" name="Home" width={390} height={844} dir="col""#;

#[test]
fn overflow_on_a_board_and_a_frame_round_trips_and_defaults_to_none() {
    let (board, printed) = round_trip(&format!(
        r#"{SCROLL_BOARD} overflow="both"><Frame id="aaaaaaaa" name="List" width={{100}} height={{400}} dir="col" overflow="vertical"/><Frame id="cccccccc" name="Plain" width={{100}} height={{40}} dir="col"/></Board>"#
    ));
    assert!(printed.contains(r#"overflow="both""#) && printed.contains(r#"overflow="vertical""#));
    assert_eq!(printed.matches("overflow").count(), 2, "{printed}");
    let kinds: Vec<Overflow> = board.children().iter().map(frame_overflow).collect();
    assert_eq!(kinds, [Overflow::Vertical, Overflow::None]);
    assert_eq!(frame_overflow(&board), Overflow::Both);
}

fn frame_overflow(node: &Node) -> Overflow {
    match node.data() {
        NodeData::Board(frame) | NodeData::Frame(frame) => frame.overflow,
        other => panic!("a frame, not {other:?}"),
    }
}

#[test]
fn an_unknown_overflow_names_the_keywords_there_are() {
    let error = error_of(&format!(r#"{SCROLL_BOARD} overflow="sideways"/>"#));
    assert!(error.message().contains("`sideways`"), "{error}");
    for word in ["none", "horizontal", "vertical", "both"] {
        assert!(error.fix().contains(word), "{error}");
    }
}

#[test]
fn a_component_takes_no_overflow() {
    let error = error_of(
        r#"<Component id="dddddddd" name="Card" width={10} height={10} dir="col" overflow="both"/>"#,
    );
    assert!(error.message().contains("overflow"), "{error}");
}

#[test]
fn scroll_position_on_layers_in_a_board_round_trips_and_defaults_to_scrolls() {
    let (board, printed) = round_trip(&format!(
        r#"{SCROLL_BOARD}><Frame id="aaaaaaaa" name="Bar" width={{100}} height={{40}} dir="col" scrollPosition="fixed"/><Text id="cccccccc" name="Title" content="Hi" fontFamily="Inter" fontSize={{16}} scrollPosition="sticky"/><Text id="eeeeeeee" name="Body" content="Hi" fontFamily="Inter" fontSize={{16}}/></Board>"#
    ));
    let positions: Vec<ScrollPosition> = board
        .children()
        .iter()
        .map(|node| node.scroll_position())
        .collect();
    assert_eq!(
        positions,
        [
            ScrollPosition::Fixed,
            ScrollPosition::Sticky,
            ScrollPosition::Scrolls
        ]
    );
    assert_eq!(printed.matches("scrollPosition").count(), 2, "{printed}");
}

#[test]
fn scroll_position_is_refused_outside_the_layers_of_a_board() {
    let sources = [
        format!(r#"{SCROLL_BOARD} scrollPosition="fixed"/>"#),
        r#"<Page id="aaaaaaaa" name="Flows" scrollPosition="fixed"/>"#.to_string(),
        r#"<Component id="dddddddd" name="Card" width={10} height={10} dir="col" scrollPosition="fixed"/>"#.to_string(),
        r#"<Component id="dddddddd" name="Card" width={10} height={10} dir="col"><Text id="cccccccc" name="T" content="Hi" fontFamily="Inter" fontSize={16} scrollPosition="fixed"/></Component>"#.to_string(),
    ];
    for source in sources {
        let error = error_of(&source);
        assert_eq!(
            error.message(),
            "`scrollPosition` is for layers inside a board",
            "{source}"
        );
    }
}

#[test]
fn an_unknown_scroll_position_names_the_keywords_there_are() {
    let error = error_of(&format!(
        r#"{SCROLL_BOARD}><Frame name="Bar" width={{100}} height={{40}} dir="col" scrollPosition="x"/></Board>"#
    ));
    assert!(error.message().contains("`x`"), "{error}");
    for word in ["scrolls", "fixed", "sticky"] {
        assert!(error.fix().contains(word), "{error}");
    }
}

#[test]
fn a_layer_written_alone_takes_a_scroll_position() {
    let (node, _) = round_trip(
        r#"<Frame id="aaaaaaaa" name="Bar" width={100} height={40} dir="col" scrollPosition="fixed"/>"#,
    );
    assert_eq!(node.scroll_position(), ScrollPosition::Fixed);
}

#[test]
fn set_refuses_a_scroll_position_on_a_layer_of_a_component_and_takes_it_in_a_board() {
    let text = r#"<Text id="cccccccc" name="T" content="Hi" fontFamily="Inter" fontSize={16}/>"#;
    let component = existing(&format!(
        r#"<Component id="dddddddd" name="Card" width={{10}} height={{10}} dir="col">{text}</Component>"#
    ));
    let board = existing(&format!("{SCROLL_BOARD}>{text}</Board>"));
    let properties = json!({"scrollPosition": "fixed"});
    let properties = properties.as_object().unwrap();

    let mut in_component = component.children()[0].clone();
    let error = tags::set_properties(
        &mut in_component,
        Some(&tags::Host::of(&component)),
        properties,
    )
    .expect_err("a layer of a component takes no scrollPosition");
    assert!(
        error.contains("`scrollPosition` is for layers inside a board"),
        "{error}"
    );
    assert_eq!(in_component.scroll_position(), ScrollPosition::Scrolls);

    let mut in_board = board.children()[0].clone();
    tags::set_properties(&mut in_board, Some(&tags::Host::of(&board)), properties)
        .expect("a layer of a board takes scrollPosition");
    assert_eq!(in_board.scroll_position(), ScrollPosition::Fixed);
}
