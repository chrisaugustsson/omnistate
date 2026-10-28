//! US-029: components and instances (Q27, FR-30). The design system doc
//! defines components with properties, and instances in design docs place
//! them, storing only their component, their property values and their
//! overrides.
//!
//! The contract these tests pin down (the tag format is that of US-027, and
//! the JSON form is the node model's):
//!
//! - `<Component>` defines a component. It takes every attribute of
//!   `<Frame>` that sizes, lays out and styles it (`width`, `height`, `dir`,
//!   `gap`, `pad`, `fill`, `radius`, ...) and `props`, and its children are
//!   the component's tree. In JSON it is a node of type `component` with its
//!   `props`. A component sits at the top of the design system doc only: a
//!   `<Component>` inside a page, a board, a frame or another component is
//!   an error at its tag name.
//! - `props` maps each property name to its definition, written the same in
//!   tags and in JSON: `{type: "variant", values: [...], default}` (a fixed
//!   set of values, the default among them), `{type: "boolean", default}`,
//!   `{type: "text", default}` and `{type: "slot", default: "<componentId>"}`.
//!   Any other type is an error at the `props` value that names it, and its
//!   fix lists the four kinds.
//! - A layer of the component takes a property with `bind`, written the same
//!   in tags and in JSON: `bind={{visible: "<boolean prop>"}}` shows or hides
//!   the layer, `bind={{content: "<text prop>"}}` replaces the text of a
//!   `<Text>`, and `bind={{component: "<slot prop>"}}` swaps the component of
//!   an `<Instance>`. A bind to a property that the component does not
//!   define, to a property of the wrong kind, or of `content` on a layer that
//!   is no text is an error at the `bind` value that names the property or
//!   the field. `bind` outside a component is an error.
//! - `<Instance component="..." props={...} overrides={...}>` places a
//!   component. In JSON an instance holds its `component`, its `props` (each
//!   property's value: text for variant, text and slot properties, `true` or
//!   `false` for boolean ones) and its `overrides`, and nothing else besides
//!   its identity, placement and size. `overrides` maps the ID of a node of
//!   the component to what the instance changes on it: `content` (text),
//!   `fills` (as a node's fills), `visible` and `component` (the contents of
//!   a slot). An override key that is no node ID, another override field, an
//!   invalid fill or a property value that is a list or an object is an
//!   error at the value that names it.
//! - The node inside an instance is addressed as `<instanceId>/<componentNodeId>`:
//!   `omni_core::design::NodeAddress` is that address, shared by overrides,
//!   comments and the renderer.
//! - Components, instances with their props and overrides, and binds
//!   round-trip through tags and through JSON without loss.
//! - `docs/formats/design-tags.md` documents components, their properties,
//!   binds, instances with props and overrides, and the address form, with
//!   examples that parse.

mod common;

use common::{assert_rejected, assert_round_trips, first_difference, to_json};
use omni_core::design::tags::{self, TagError};
use omni_core::design::{Node, NodeAddress, NodeId};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The PRD's four kinds of property.
const KINDS: [&str; 4] = ["variant", "boolean", "text", "slot"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn id(text: &str) -> NodeId {
    text.parse().expect("a valid node ID")
}

/// The node `source` describes, when every tag in it carries an `id`.
fn existing(source: &str) -> Node {
    tags::parse(source)
        .unwrap_or_else(|error| panic!("these tags must parse, but: {error}\n{source}"))
        .assign_ids(|| panic!("every tag has an `id`, so no node is new"))
}

/// The line and column, counting from 1 and in characters, of the start of
/// the `nth` (from 0) occurrence of `needle` in `source`.
fn position(source: &str, needle: &str, nth: usize) -> (usize, usize) {
    let (offset, _) = source
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("`{needle}` occurs fewer than {} times", nth + 1));
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    (line, source[line_start..offset].chars().count() + 1)
}

/// The position of the value of the `nth` attribute `name` in `source`: its
/// opening `"` or `{`.
fn value_of(source: &str, name: &str, nth: usize) -> (usize, usize) {
    let (line, column) = position(source, &format!(" {name}="), nth);
    (line, column + name.chars().count() + 2)
}

fn rejected(source: &str) -> TagError {
    match tags::parse(source) {
        Ok(tree) => panic!("these tags must be rejected, but they parsed as {tree:?}:\n{source}"),
        Err(error) => error,
    }
}

/// Asserts that `source` is rejected with an error at `at`, whose message
/// names each of `names` and whose fix mentions each of `fixes`.
fn assert_error_at(source: &str, at: (usize, usize), names: &[&str], fixes: &[&str]) -> TagError {
    let error = rejected(source);
    assert_eq!(
        (error.line(), error.column()),
        at,
        "the error must point at line {}, column {}; it says: {error}\n{source}",
        at.0,
        at.1
    );
    for name in names {
        assert!(
            error.message().contains(name),
            "the message must name `{name}`; it says: {}",
            error.message()
        );
    }
    assert!(
        !error.fix().trim().is_empty(),
        "the error must say how to fix it; it says: {error}"
    );
    for fix in fixes {
        assert!(
            error.fix().contains(fix),
            "the fix must mention `{fix}`; it says: {}",
            error.fix()
        );
    }
    error
}

/// Asserts that printing `node` and parsing the tags back gives the same
/// node, and printing that again the same tags. Returns the tags.
fn assert_tags_round_trip(node: &Node) -> String {
    let printed = tags::print(node);
    let back = tags::parse(&printed)
        .unwrap_or_else(|error| panic!("printed tags must parse again, but: {error}\n{printed}"))
        .assign_ids(|| panic!("printed tags carry the ID of every node, so no node is new"));
    let (expected, actual) = (to_json(node), to_json(&back));
    if let Some(difference) = first_difference(&expected, &actual, "")
        .or_else(|| first_difference(&actual, &expected, ""))
    {
        panic!("printing and parsing the tags lost or changed a property: {difference}\n{printed}");
    }
    assert!(&back == node, "the tags must parse back to the same node");
    assert_eq!(
        tags::print(&back),
        printed,
        "printing the parsed node again must give the same tags"
    );
    printed
}

/// The children of `json`, by name.
fn child<'a>(json: &'a Value, name: &str) -> &'a Value {
    json["children"]
        .as_array()
        .unwrap_or_else(|| panic!("the node has children: {json}"))
        .iter()
        .find(|child| child["name"] == name)
        .unwrap_or_else(|| panic!("the node has a child named {name}: {json:#}"))
}

/// A button with a property of each kind: a variant for its style, a boolean
/// that shows or hides its icon, a text property for its label, and a slot
/// that swaps the icon's component.
const BUTTON: &str = r##"<Component id="btn00001" name="Button" dir="row" gap={8} pad={12} fill="#2563eb" radius={8} props={{style: {type: "variant", values: ["primary", "secondary"], default: "primary"}, showIcon: {type: "boolean", default: true}, label: {type: "text", default: "Buy"}, icon: {type: "slot", default: "cart0001"}}}>
  <Instance id="gfx00001" name="Icon" component="cart0001" bind={{visible: "showIcon", component: "icon"}}/>
  <Text id="txt00001" name="Label" content="Buy" fontFamily="Inter" fontSize={14} color="#ffffff" bind={{content: "label"}}/>
  <Frame id="bdg00001" name="Badge" width={8} height={8} fill="#dc2626"/>
</Component>"##;

fn button_props() -> Value {
    json!({
        "style": {"type": "variant", "values": ["primary", "secondary"], "default": "primary"},
        "showIcon": {"type": "boolean", "default": true},
        "label": {"type": "text", "default": "Buy"},
        "icon": {"type": "slot", "default": "cart0001"}
    })
}

/// A page whose board places the button, with a value for each property and
/// an override of each kind.
const CHECKOUT: &str = r##"<Page id="page0001" name="Home">
  <Board id="brd00001" name="Checkout" x={0} y={0} width={375} height={812} dir="col" gap={16}>
    <Instance id="pay00001" name="Pay button" component="btn00001" props={{style: "secondary", showIcon: false, label: "Pay", icon: "hrt00001"}} overrides={{txt00001: {content: "Pay now"}, bdg00001: {fills: [{type: "solid", color: "#16a34a"}]}, gfx00001: {visible: false, component: "hrt00001"}}}/>
  </Board>
</Page>"##;

fn pay_button_props() -> Value {
    json!({"style": "secondary", "showIcon": false, "label": "Pay", "icon": "hrt00001"})
}

fn pay_button_overrides() -> Value {
    json!({
        "txt00001": {"content": "Pay now"},
        "bdg00001": {"fills": [{"type": "solid", "color": "#16a34a"}]},
        "gfx00001": {"visible": false, "component": "hrt00001"}
    })
}

/// `BUTTON` with `props` replaced.
fn button_with_props(props: &str) -> String {
    let start = BUTTON.find(" props=").expect("the button has props");
    let end = BUTTON.find("}}>\n").expect("the props end the tag") + 2;
    format!("{} props={props}{}", &BUTTON[..start], &BUTTON[end..])
}

/// `CHECKOUT` with the pay button's tag replaced.
fn checkout_with_instance(instance: &str) -> String {
    let start = CHECKOUT
        .find("<Instance")
        .expect("the board has an instance");
    let end = CHECKOUT[start..]
        .find("/>")
        .expect("the instance closes itself")
        + start
        + 2;
    format!("{}{instance}{}", &CHECKOUT[..start], &CHECKOUT[end..])
}

// ---------------------------------------------------------------------------
// <Component> tags define components with four kinds of property
// ---------------------------------------------------------------------------

#[test]
fn a_component_tag_defines_a_component_with_properties_of_four_kinds() {
    let button = existing(BUTTON);
    let json = to_json(&button);
    assert_eq!(json["type"], "component", "{json:#}");
    assert_eq!(json["id"], "btn00001");
    assert_eq!(json["name"], "Button");
    assert_eq!(json["props"], button_props(), "{json:#}");
    // The component is laid out and styled like a frame.
    assert_eq!(json["layout"]["mode"], "flex", "{json:#}");
    assert_eq!(json["layout"]["direction"], "row", "{json:#}");
    assert_eq!(json["fills"][0]["color"], "#2563eb", "{json:#}");
    let names: Vec<&str> = json["children"]
        .as_array()
        .expect("the component's tree is its children")
        .iter()
        .map(|child| child["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Icon", "Label", "Badge"]);
}

#[test]
fn layers_take_the_component_properties_with_bind() {
    let json = to_json(&existing(BUTTON));
    let icon = child(&json, "Icon");
    assert_eq!(icon["type"], "instance");
    assert_eq!(
        icon["bind"],
        json!({"visible": "showIcon", "component": "icon"}),
        "a boolean shows or hides a layer, and a slot swaps an instance's component: {icon}"
    );
    let label = child(&json, "Label");
    assert_eq!(
        label["bind"],
        json!({"content": "label"}),
        "a text property replaces a label: {label}"
    );
    assert!(
        child(&json, "Badge").get("bind").is_none(),
        "a layer without a bind has none"
    );
}

#[test]
fn a_component_round_trips_through_tags_and_json() {
    let button = existing(BUTTON);
    let printed = assert_tags_round_trip(&button);
    assert!(
        printed.starts_with("<Component"),
        "a component is written as `<Component>`: {printed}"
    );
    for attribute in ["props=", "bind="] {
        assert!(
            printed.contains(attribute),
            "{attribute} is written: {printed}"
        );
    }
    assert_round_trips(&to_json(&button));
}

#[test]
fn a_property_of_another_kind_is_rejected_and_the_fix_lists_the_four_kinds() {
    let source = button_with_props(
        r#"{{style: {type: "number", default: 1}, showIcon: {type: "boolean", default: true}, label: {type: "text", default: "Buy"}, icon: {type: "slot", default: "cart0001"}}}"#,
    );
    assert_error_at(&source, value_of(&source, "props", 0), &["number"], &KINDS);
}

#[test]
fn a_variant_default_outside_its_values_is_rejected() {
    let source = button_with_props(
        r#"{{style: {type: "variant", values: ["primary", "secondary"], default: "ghost"}, showIcon: {type: "boolean", default: true}, label: {type: "text", default: "Buy"}, icon: {type: "slot", default: "cart0001"}}}"#,
    );
    assert_error_at(&source, value_of(&source, "props", 0), &["ghost"], &[]);
}

#[test]
fn a_slot_default_that_is_no_component_id_is_rejected() {
    let source = button_with_props(
        r#"{{style: {type: "variant", values: ["primary", "secondary"], default: "primary"}, showIcon: {type: "boolean", default: true}, label: {type: "text", default: "Buy"}, icon: {type: "slot", default: "Cart icon"}}}"#,
    );
    assert_error_at(&source, value_of(&source, "props", 0), &["Cart icon"], &[]);
}

#[test]
fn a_bind_to_a_property_the_component_does_not_define_is_rejected() {
    let source = BUTTON.replace(
        r#"bind={{content: "label"}}"#,
        r#"bind={{content: "title"}}"#,
    );
    assert_error_at(&source, value_of(&source, "bind", 1), &["title"], &[]);
}

#[test]
fn a_bind_to_a_property_of_the_wrong_kind_is_rejected() {
    // `showIcon` is a boolean: it cannot replace a label.
    let source = BUTTON.replace(
        r#"bind={{content: "label"}}"#,
        r#"bind={{content: "showIcon"}}"#,
    );
    assert_error_at(&source, value_of(&source, "bind", 1), &["showIcon"], &[]);
    // `label` is a text property: it cannot show or hide a layer.
    let source = BUTTON.replace(
        r#"bind={{visible: "showIcon", component: "icon"}}"#,
        r#"bind={{visible: "label", component: "icon"}}"#,
    );
    assert_error_at(&source, value_of(&source, "bind", 0), &["label"], &[]);
    // `style` is a variant: it swaps no component.
    let source = BUTTON.replace(
        r#"bind={{visible: "showIcon", component: "icon"}}"#,
        r#"bind={{visible: "showIcon", component: "style"}}"#,
    );
    assert_error_at(&source, value_of(&source, "bind", 0), &["style"], &[]);
}

#[test]
fn a_text_property_binds_to_texts_and_a_slot_to_instances_only() {
    let source = BUTTON.replace(
        r##"fill="#dc2626"/>"##,
        r##"fill="#dc2626" bind={{content: "label"}}/>"##,
    );
    assert_error_at(&source, value_of(&source, "bind", 2), &["content"], &[]);
    let source = BUTTON.replace(
        r##"fill="#dc2626"/>"##,
        r##"fill="#dc2626" bind={{component: "icon"}}/>"##,
    );
    assert_error_at(&source, value_of(&source, "bind", 2), &["component"], &[]);
}

#[test]
fn a_component_sits_at_the_top_of_the_design_system_doc_only() {
    let in_a_board = r##"<Page id="page0001" name="Home">
  <Board id="brd00001" name="Checkout" x={0} y={0} width={375} height={812}>
    <Component id="btn00001" name="Button" fill="#2563eb"/>
  </Board>
</Page>"##;
    assert_error_at(
        in_a_board,
        position(in_a_board, "Component", 0),
        &["Component"],
        &[],
    );
    let in_a_component = r##"<Component id="btn00001" name="Button" fill="#2563eb">
  <Component id="btn00002" name="Inner" fill="#dc2626"/>
</Component>"##;
    assert_error_at(
        in_a_component,
        position(in_a_component, "Component", 1),
        &["Component"],
        &[],
    );
}

#[test]
fn bind_outside_a_component_is_rejected() {
    let source = r##"<Page id="page0001" name="Home">
  <Board id="brd00001" name="Checkout" x={0} y={0} width={375} height={812}>
    <Text id="txt00009" name="Title" content="Kassa" fontFamily="Inter" fontSize={24} bind={{content: "label"}}/>
  </Board>
</Page>"##;
    let error = rejected(source);
    assert_eq!(
        error.line(),
        3,
        "the error is on the line of the bind: {error}"
    );
    assert!(
        error.message().contains("bind"),
        "the message names `bind`: {}",
        error.message()
    );
}

#[test]
fn component_json_is_checked_like_its_tags() {
    let json = to_json(&existing(BUTTON));
    let mut other_kind = json.clone();
    other_kind["props"]["style"] = json!({"type": "number", "default": 1});
    let message = assert_rejected(&other_kind, "a property of another kind is rejected");
    assert!(message.contains("number"), "{message}");
    let mut unknown_prop = json.clone();
    unknown_prop["children"][1]["bind"] = json!({"content": "title"});
    let message = assert_rejected(
        &unknown_prop,
        "a bind to a property the component does not define is rejected",
    );
    assert!(message.contains("title"), "{message}");
}

#[test]
fn a_component_json_node_is_not_a_child_of_a_page_or_board() {
    let component = to_json(&existing(BUTTON));
    let page = json!({
        "id": "page0001",
        "type": "page",
        "name": "Home",
        "children": [component]
    });
    assert_rejected(&page, "a component is no board of a page");
}

// ---------------------------------------------------------------------------
// <Instance component props overrides>
// ---------------------------------------------------------------------------

#[test]
fn an_instance_stores_only_its_component_its_property_values_and_its_overrides() {
    let page = to_json(&existing(CHECKOUT));
    let instance = child(child(&page, "Checkout"), "Pay button");
    assert_eq!(instance["type"], "instance");
    assert_eq!(instance["component"], "btn00001");
    assert_eq!(instance["props"], pay_button_props(), "{instance:#}");
    assert_eq!(
        instance["overrides"],
        pay_button_overrides(),
        "{instance:#}"
    );
    let keys: BTreeSet<&str> = instance
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let allowed: BTreeSet<&str> = [
        "id",
        "type",
        "name",
        "visible",
        "component",
        "props",
        "overrides",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        keys, allowed,
        "an instance in a flex flow stores its identity, its component, its property values and its overrides, and nothing of the component's tree or style"
    );
}

#[test]
fn an_instance_with_props_and_overrides_round_trips_through_tags_and_json() {
    let page = existing(CHECKOUT);
    let printed = assert_tags_round_trip(&page);
    for attribute in ["props=", "overrides="] {
        assert!(
            printed.contains(attribute),
            "{attribute} is written: {printed}"
        );
    }
    assert_round_trips(&to_json(&page));
}

#[test]
fn an_instance_takes_no_style_and_no_children() {
    let styled = checkout_with_instance(
        r##"<Instance id="pay00001" name="Pay button" component="btn00001" fill="#16a34a"/>"##,
    );
    let error = rejected(&styled);
    assert!(error.message().contains("fill"), "{error}");
    let with_children = checkout_with_instance(
        r##"<Instance id="pay00001" name="Pay button" component="btn00001"><Text id="txt00009" name="Extra" content="x" fontFamily="Inter" fontSize={12}/></Instance>"##,
    );
    rejected(&with_children);
}

#[test]
fn an_override_key_must_be_the_id_of_a_component_node() {
    let source = checkout_with_instance(
        r#"<Instance id="pay00001" name="Pay button" component="btn00001" overrides={{"Label": {content: "Pay now"}}}/>"#,
    );
    assert_error_at(&source, value_of(&source, "overrides", 0), &["Label"], &[]);
}

#[test]
fn an_override_changes_only_text_fills_visibility_and_slot_contents() {
    let source = checkout_with_instance(
        r#"<Instance id="pay00001" name="Pay button" component="btn00001" overrides={{txt00001: {fontSize: 20}}}/>"#,
    );
    assert_error_at(
        &source,
        value_of(&source, "overrides", 0),
        &["fontSize"],
        &["content", "fills", "visible", "component"],
    );
    let source = checkout_with_instance(
        r##"<Instance id="pay00001" name="Pay button" component="btn00001" overrides={{bdg00001: {fills: [{type: "solid", color: "#12"}]}}}/>"##,
    );
    assert_error_at(&source, value_of(&source, "overrides", 0), &["#12"], &[]);
    let source = checkout_with_instance(
        r#"<Instance id="pay00001" name="Pay button" component="btn00001" overrides={{gfx00001: {component: "Heart"}}}/>"#,
    );
    assert_error_at(&source, value_of(&source, "overrides", 0), &["Heart"], &[]);
}

#[test]
fn a_property_value_is_text_or_true_or_false() {
    let source = checkout_with_instance(
        r#"<Instance id="pay00001" name="Pay button" component="btn00001" props={{style: {value: "secondary"}}}/>"#,
    );
    assert_error_at(&source, value_of(&source, "props", 0), &["style"], &[]);
    let source = checkout_with_instance(
        r#"<Instance id="pay00001" name="Pay button" component="btn00001" props={{label: ["Pay"]}}/>"#,
    );
    assert_error_at(&source, value_of(&source, "props", 0), &["label"], &[]);
}

#[test]
fn instance_json_is_checked_like_its_tags() {
    let page = to_json(&existing(CHECKOUT));
    let instance = child(child(&page, "Checkout"), "Pay button").clone();
    let mut other_field = instance.clone();
    other_field["overrides"]["txt00001"] = json!({"fontSize": 20});
    let message = assert_rejected(&other_field, "an override of another field is rejected");
    assert!(message.contains("fontSize"), "{message}");
    let mut bad_key = instance.clone();
    bad_key["overrides"] = json!({"Label": {"content": "Pay now"}});
    let message = assert_rejected(&bad_key, "an override key that is no node ID is rejected");
    assert!(message.contains("Label"), "{message}");
    let mut styled = instance;
    styled["fills"] = json!([{"type": "solid", "color": "#16a34a"}]);
    assert_rejected(&styled, "an instance stores no fills of its own");
}

// ---------------------------------------------------------------------------
// Node addresses: <instanceId>/<componentNodeId>
// ---------------------------------------------------------------------------

#[test]
fn a_node_inside_an_instance_is_addressed_as_the_instance_and_the_component_node() {
    let plain: NodeAddress = "pay00001".parse().expect("a node ID is an address");
    assert_eq!(plain.instance(), None);
    assert_eq!(plain.node(), &id("pay00001"));
    assert_eq!(plain.to_string(), "pay00001");

    let inside: NodeAddress = "pay00001/txt00001"
        .parse()
        .expect("<instanceId>/<componentNodeId> is an address");
    assert_eq!(inside.instance(), Some(&id("pay00001")));
    assert_eq!(inside.node(), &id("txt00001"));
    assert_eq!(inside.to_string(), "pay00001/txt00001");
    assert_eq!(
        inside,
        NodeAddress::new(Some(id("pay00001")), id("txt00001"))
    );
    assert_ne!(inside, plain);
}

#[test]
fn text_that_is_no_node_address_is_rejected() {
    for bad in [
        "",
        "/",
        "pay0001",
        "pay00001/",
        "/txt00001",
        "pay00001/txt00001/bdg00001",
        "PAY00001",
        "pay00001 txt00001",
        "pay00001/Label",
    ] {
        assert!(
            bad.parse::<NodeAddress>().is_err(),
            "`{bad}` is no node address"
        );
    }
}

#[test]
fn every_override_key_of_an_instance_addresses_a_node_inside_it() {
    let page = to_json(&existing(CHECKOUT));
    let instance = child(child(&page, "Checkout"), "Pay button");
    let instance_id = instance["id"].as_str().unwrap();
    for key in instance["overrides"].as_object().unwrap().keys() {
        let address: NodeAddress = format!("{instance_id}/{key}")
            .parse()
            .unwrap_or_else(|_| panic!("{instance_id}/{key} is a node address"));
        assert_eq!(address.instance(), Some(&id(instance_id)));
        assert_eq!(address.node().as_str(), key);
    }
}

// ---------------------------------------------------------------------------
// docs/formats/design-tags.md
// ---------------------------------------------------------------------------

/// The fenced code blocks of `doc`.
fn code_blocks(doc: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            match current.take() {
                Some(lines) => blocks.push(lines.join("\n")),
                None => current = Some(Vec::new()),
            }
        } else if let Some(lines) = current.as_mut() {
            lines.push(line);
        }
    }
    blocks
}

#[test]
fn the_format_doc_documents_components_and_instances() {
    let path = repo_root().join("docs/formats/design-tags.md");
    let doc = fs::read_to_string(&path).expect("docs/formats/design-tags.md exists");
    let mut missing = Vec::new();
    for needle in [
        "<Component",
        "`props`",
        "`overrides`",
        "`bind`",
        "`variant`",
        "`boolean`",
        "`text`",
        "`slot`",
        "`content`",
        "`fills`",
        "`visible`",
        "<instanceId>/<componentNodeId>",
    ] {
        if !doc.contains(needle) {
            missing.push(needle);
        }
    }
    assert!(
        missing.is_empty(),
        "docs/formats/design-tags.md must document components and instances, but it lacks {missing:?}"
    );

    let blocks = code_blocks(&doc);
    let component_example = blocks
        .iter()
        .find(|block| block.trim_start().starts_with("<Component"))
        .expect("the doc has an example of a `<Component>` in a code block of its own");
    let parsed = tags::parse(component_example).unwrap_or_else(|error| {
        panic!("the `<Component>` example parses: {error}\n{component_example}")
    });
    let json = to_json(&parsed.assign_ids({
        let mut n = 0u32;
        move || {
            n += 1;
            id(&format!("dcx{n:05}"))
        }
    }));
    assert_eq!(json["type"], "component", "{json:#}");
    let kinds: BTreeSet<&str> = json["props"]
        .as_object()
        .expect("the example defines properties")
        .values()
        .filter_map(|prop| prop["type"].as_str())
        .collect();
    assert_eq!(
        kinds,
        KINDS.into_iter().collect(),
        "the example shows a property of each kind"
    );

    let instance_example = blocks
        .iter()
        .find(|block| {
            block.contains("<Instance") && block.contains("props=") && block.contains("overrides=")
        })
        .expect("the doc has an example of an `<Instance>` with props and overrides");
    tags::parse(instance_example).unwrap_or_else(|error| {
        panic!("the `<Instance>` example parses: {error}\n{instance_example}")
    });
}
