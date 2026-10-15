//! US-021: the operation engine. One engine edits every doc type; these tests
//! run it on HTML docs (US-020). They read no files, so the same tests run
//! natively and in WebAssembly (`wasm-pack test --node`).
//!
//! The tests pin this API and JSON form (docs/formats/operations.md documents
//! the JSON):
//!
//! - `EditDoc::from_html` starts a doc whose every block has a `data-node-id`;
//!   every node starts at revision 1. `to_html`, `node_html`, `contains`,
//!   `revision`, `children` (`None` is the top level) and `sort_key` read it.
//! - `EditDoc::apply(&ops, &mut new_ids)` applies a batch all or nothing and
//!   returns one `OpResult` per operation: the `node`, its `revision` after
//!   the operation, the IDs `created` for new nodes (in document order) and
//!   the nodes `removed` (in document order). `new_ids` hands out the IDs.
//! - An `Operation` is read from JSON, tagged by `op`:
//!   `{"op":"insert","parent":<id>|null,"position":{"after":<id>|null},"content":<html>}`,
//!   `{"op":"replace","node":<id>,"rev":<n>,"content":<html>}`,
//!   `{"op":"set","node":<id>,"rev":<n>,"properties":{...}}`,
//!   `{"op":"move","node":<id>,"rev":<n>,"parent":<id>|null,"position":{"after":<id>|null}}`
//!   and `{"op":"delete","node":<id>,"rev":<n>}`. A `null` parent is the top
//!   level. Content is the HTML of one block, with the US-020 rules.
//! - `set` properties for HTML docs: `level` (heading, 1 to 6), `kind`
//!   (callout: note, warning or decision), `ordered` (list), `header` (table
//!   cell), `alt` (image), `caption` (figure) and `html` (the inner HTML of a
//!   paragraph, heading, quote, list item, table cell, callout or requirement).
//! - A revision mismatch is `ApplyError::Conflict` with the node's current
//!   revision and HTML; any other refusal is `ApplyError::Invalid { index,
//!   message }`, and the message names the ID or the tag that is wrong.

use omni_core::design::NodeId;
use omni_core::html_doc::HtmlDoc;
use omni_core::ops::{Applied, ApplyError, Conflict, EditDoc, Operation};
use serde_json::{Value, json};
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};

const S1: &str = "b0000001";
const H1: &str = "b0000002";
const P1: &str = "b0000003";
const P2: &str = "b0000004";
const S2: &str = "b0000005";
const H2: &str = "b0000006";
const UL: &str = "b0000007";
const LI1: &str = "b0000008";
const LI2: &str = "b0000009";
const NOTE: &str = "b000000a";
const TABLE: &str = "b000000b";
const ROW: &str = "b000000c";
const CELL: &str = "b000000d";
const IMG: &str = "b000000e";
const FIGURE: &str = "b000000f";
const UNKNOWN: &str = "b0000fff";

const BASE: &str = r##"<section data-node-id="b0000001">
<h1 data-node-id="b0000002">Orders</h1>
<p data-node-id="b0000003">First paragraph</p>
<p data-node-id="b0000004">Second paragraph</p>
</section>
<section data-node-id="b0000005">
<h2 data-node-id="b0000006">Rules</h2>
<ul data-node-id="b0000007"><li data-node-id="b0000008">One</li><li data-node-id="b0000009">Two</li></ul>
<aside data-node-id="b000000a" data-callout="note">A note</aside>
<table data-node-id="b000000b"><tr data-node-id="b000000c"><th data-node-id="b000000d">Field</th></tr></table>
<img data-node-id="b000000e" src="attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd" alt="A screenshot">
<figure data-node-id="b000000f"><pre class="mermaid">graph TD;
  A--&gt;B</pre><figcaption>The flow</figcaption></figure>
</section>
"##;

// --- Helpers ---

fn id(text: &str) -> NodeId {
    NodeId::from_str(text).unwrap_or_else(|error| panic!("{text} must be a node ID: {error}"))
}

fn doc() -> EditDoc {
    EditDoc::from_html(BASE).expect("the base doc is valid")
}

/// The next ID for a new node, never used before by any test or doc.
fn new_id() -> NodeId {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    id(&format!("c{:07}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

fn batch(ops: Value) -> Vec<Operation> {
    serde_json::from_value(ops.clone())
        .unwrap_or_else(|error| panic!("{ops} must be a batch of operations: {error}"))
}

fn apply(doc: &mut EditDoc, ops: Value) -> Applied {
    let ops = batch(ops);
    doc.apply(&ops, &mut new_id)
        .unwrap_or_else(|error| panic!("the batch must apply, but: {error}"))
}

/// The one result of a batch of one operation.
fn apply_one(doc: &mut EditDoc, op: Value) -> omni_core::ops::OpResult {
    let mut applied = apply(doc, json!([op]));
    assert_eq!(applied.ops.len(), 1, "one operation, one result");
    applied.ops.remove(0)
}

/// A doc's HTML and, per node, its ID, revision and sort key.
type Snapshot = (String, Vec<(String, Option<u64>, Option<String>)>);

fn snapshot(doc: &EditDoc) -> Snapshot {
    let html = doc.to_html();
    let nodes = printed_ids(&html)
        .into_iter()
        .map(|text| {
            let node = id(&text);
            (text, doc.revision(node), doc.sort_key(node))
        })
        .collect();
    (html, nodes)
}

/// Applies `ops`, which must fail, and checks that the doc did not change.
fn refused(doc: &mut EditDoc, ops: Value) -> ApplyError {
    let before = snapshot(doc);
    let parsed = batch(ops.clone());
    let error = match doc.apply(&parsed, &mut new_id) {
        Ok(applied) => panic!("{ops} must be refused, but it applied: {applied:?}"),
        Err(error) => error,
    };
    assert_eq!(
        snapshot(doc),
        before,
        "a refused batch changes nothing ({error})"
    );
    error
}

fn invalid(doc: &mut EditDoc, ops: Value) -> (usize, String) {
    match refused(doc, ops.clone()) {
        ApplyError::Invalid { index, message } => (index, message),
        ApplyError::Conflict(conflict) => {
            panic!("{ops} must be invalid, not a conflict: {conflict:?}")
        }
    }
}

fn conflict(doc: &mut EditDoc, ops: Value) -> Conflict {
    match refused(doc, ops.clone()) {
        ApplyError::Conflict(conflict) => conflict,
        other => panic!("{ops} must be a conflict, but it is: {other}"),
    }
}

/// The `data-node-id` values of printed HTML, in order.
fn printed_ids(html: &str) -> Vec<String> {
    const ATTRIBUTE: &str = "data-node-id=\"";
    let mut ids = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find(ATTRIBUTE) {
        rest = &rest[start + ATTRIBUTE.len()..];
        let end = rest.find('"').expect("an attribute value is closed");
        ids.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    ids
}

fn children(doc: &EditDoc, parent: Option<&str>) -> Vec<String> {
    doc.children(parent.map(id))
        .unwrap_or_else(|| panic!("{parent:?} must be in the doc"))
        .iter()
        .map(|node| node.to_string())
        .collect()
}

fn rev(doc: &EditDoc, node: &str) -> u64 {
    doc.revision(id(node))
        .unwrap_or_else(|| panic!("{node} must be in the doc"))
}

/// Whether `message` names the tag, attribute or word `problem`, or any of its
/// alternatives when it lists several as `a|b`. A name of one or two letters
/// must be written as `<b>` or in quotes.
fn names_problem(message: &str, problem: &str) -> bool {
    let message = message.to_lowercase();
    problem.split('|').any(|name| {
        if name.len() > 2 {
            message.contains(name)
        } else {
            [
                format!("<{name}"),
                format!("`{name}`"),
                format!("'{name}'"),
                format!("\"{name}\""),
            ]
            .iter()
            .any(|form| message.contains(form.as_str()))
        }
    })
}

fn insert_into(parent: &str, after: Option<&str>, content: &str) -> Value {
    json!({"op": "insert", "parent": parent, "position": {"after": after}, "content": content})
}

fn set(node: &str, rev: u64, properties: Value) -> Value {
    json!({"op": "set", "node": node, "rev": rev, "properties": properties})
}

fn delete(node: &str, rev: u64) -> Value {
    json!({"op": "delete", "node": node, "rev": rev})
}

fn replace(node: &str, rev: u64, content: &str) -> Value {
    json!({"op": "replace", "node": node, "rev": rev, "content": content})
}

fn move_to(node: &str, rev: u64, parent: Option<&str>, after: Option<&str>) -> Value {
    json!({"op": "move", "node": node, "rev": rev, "parent": parent, "position": {"after": after}})
}

// --- The JSON form of operations ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn every_kind_of_operation_is_read_from_its_json_form() {
    let ops = batch(json!([
        {"op": "insert", "parent": S1, "position": {"after": P1}, "content": "<p>New</p>"},
        {"op": "insert", "parent": null, "position": {"after": null}, "content": "<hr>"},
        {"op": "replace", "node": P1, "rev": 1, "content": "<p>Replaced</p>"},
        {"op": "set", "node": H1, "rev": 1, "properties": {"level": 2}},
        {"op": "move", "node": P2, "rev": 1, "parent": S2, "position": {"after": null}},
        {"op": "move", "node": S2, "rev": 1, "parent": null, "position": {"after": S1}},
        {"op": "delete", "node": LI1, "rev": 1}
    ]));
    assert_eq!(ops.len(), 7, "each operation in the batch is read");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_operations_are_rejected_when_read() {
    let malformed = [
        (
            json!({"op": "explode", "node": P1, "rev": 1}),
            "an unknown op",
        ),
        (
            json!({"op": "insert", "parent": S1, "content": "<p>x</p>"}),
            "an insert without a position",
        ),
        (
            json!({"op": "insert", "position": {"after": null}, "content": "<p>x</p>"}),
            "an insert without a parent (the top level is a null parent)",
        ),
        (
            json!({"op": "insert", "parent": S1, "position": {"after": null}}),
            "an insert without content",
        ),
        (
            json!({"op": "insert", "parent": S1, "position": {}, "content": "<p>x</p>"}),
            "a position without `after` (first is a null `after`)",
        ),
        (
            json!({"op": "insert", "parent": S1, "position": {"after": 5}, "content": "<p>x</p>"}),
            "a position after a number",
        ),
        (
            json!({"op": "insert", "parent": S1, "position": {"after": "NOPE"}, "content": "<p>x</p>"}),
            "a position after something that is not a node ID",
        ),
        (
            json!({"op": "set", "node": P1, "properties": {"level": 2}}),
            "a set without the node's revision",
        ),
        (
            json!({"op": "set", "node": P1, "rev": 1}),
            "a set without properties",
        ),
        (
            json!({"op": "replace", "node": P1, "content": "<p>x</p>"}),
            "a replace without the node's revision",
        ),
        (
            json!({"op": "move", "node": P1, "parent": S2, "position": {"after": null}}),
            "a move without the node's revision",
        ),
        (
            json!({"op": "delete", "node": P1}),
            "a delete without the node's revision",
        ),
        (
            json!({"op": "delete", "node": "not-an-id", "rev": 1}),
            "a delete of something that is not a node ID",
        ),
        (
            json!({"op": "delete", "node": P1, "rev": -1}),
            "a negative revision",
        ),
        (
            json!({"op": "delete", "node": P1, "rev": 1, "colour": "red"}),
            "an operation with an unknown field",
        ),
    ];
    for (op, what) in malformed {
        if let Ok(parsed) = serde_json::from_value::<Operation>(op.clone()) {
            panic!("{what} must be rejected, but {op} was read as {parsed:?}");
        }
    }
}

// --- Revisions and conflicts ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn every_node_starts_at_revision_one() {
    let doc = doc();
    let ids = printed_ids(&doc.to_html());
    assert_eq!(ids.len(), 15, "the base doc has 15 blocks");
    for node in &ids {
        assert_eq!(rev(&doc, node), 1, "{node} starts at revision 1");
    }
    assert_eq!(doc.revision(id(UNKNOWN)), None);
    assert!(doc.contains(id(P1)));
    assert!(!doc.contains(id(UNKNOWN)));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_operation_raises_only_the_revision_of_the_node_it_changes() {
    let mut doc = doc();
    let result = apply_one(
        &mut doc,
        set(P1, 1, json!({"html": "Changed <strong>text</strong>"})),
    );
    assert_eq!(result.node, id(P1));
    assert_eq!(result.revision, 2, "the result tells the new revision");
    assert_eq!(rev(&doc, P1), 2);
    for untouched in [S1, H1, P2, S2, H2, UL, LI1] {
        assert_eq!(
            rev(&doc, untouched),
            1,
            "{untouched} was not changed, so keeps revision 1"
        );
    }
    assert!(
        doc.to_html().contains("Changed <strong>text</strong>"),
        "the set text is in the doc:\n{}",
        doc.to_html()
    );
    let result = apply_one(&mut doc, set(P1, 2, json!({"html": "Again"})));
    assert_eq!(result.revision, 3);
    assert_eq!(rev(&doc, P1), 3);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_stale_revision_is_a_conflict_with_the_current_state() {
    let mut doc = doc();
    apply_one(
        &mut doc,
        set(P1, 1, json!({"html": "Updated by someone else"})),
    );
    let found = conflict(&mut doc, json!([set(P1, 1, json!({"html": "My update"}))]));
    assert_eq!(found.index, 0);
    assert_eq!(found.node, id(P1));
    assert_eq!(found.expected, 1);
    assert_eq!(found.current_revision, 2);
    assert!(
        found.current_html.contains("Updated by someone else"),
        "the conflict shows the node as it is now: {}",
        found.current_html
    );
    assert!(
        found
            .current_html
            .contains(&format!("data-node-id=\"{P1}\"")),
        "the conflict's HTML carries the node's ID: {}",
        found.current_html
    );
    assert!(
        !found.current_html.contains("My update"),
        "nothing of the refused operation is applied"
    );
    assert!(
        ApplyError::Conflict(found).to_string().contains(P1),
        "the error text names the node"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revision_ahead_of_the_node_is_a_conflict_too() {
    let mut doc = doc();
    let found = conflict(&mut doc, json!([set(P1, 5, json!({"html": "Never"}))]));
    assert_eq!(found.expected, 5);
    assert_eq!(found.current_revision, 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn every_operation_that_changes_a_node_checks_its_revision() {
    let stale = [
        ("set", set(P2, 1, json!({"html": "Stale"}))),
        ("replace", replace(P2, 1, "<p>Stale</p>")),
        ("move", move_to(P2, 1, Some(S2), None)),
        ("delete", delete(P2, 1)),
    ];
    for (name, op) in stale {
        let mut doc = doc();
        apply_one(&mut doc, set(P2, 1, json!({"html": "Fresh"})));
        let found = conflict(&mut doc, json!([op]));
        assert_eq!(found.node, id(P2), "a stale {name} conflicts on its node");
        assert_eq!(found.current_revision, 2, "a stale {name}");
        assert!(
            doc.to_html().contains("Fresh") && doc.contains(id(P2)),
            "a stale {name} changes nothing"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_conflict_in_a_later_operation_names_its_place_in_the_batch() {
    let mut doc = doc();
    apply_one(&mut doc, set(P2, 1, json!({"html": "Fresh"})));
    let found = conflict(
        &mut doc,
        json!([
            set(P1, 1, json!({"html": "Fine"})),
            insert_into(S1, None, "<p>Fine too</p>"),
            set(P2, 1, json!({"html": "Stale"}))
        ]),
    );
    assert_eq!(found.index, 2);
    assert_eq!(found.node, id(P2));
    assert_eq!(rev(&doc, P1), 1, "the operations before it are undone too");
}

// --- Insert and fractional order ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn insert_adds_a_block_with_a_new_id_at_the_position() {
    let mut doc = doc();
    let result = apply_one(&mut doc, insert_into(S1, Some(P1), "<p>Inserted</p>"));
    let inserted = result.node;
    assert!(
        ![S1, H1, P1, P2, S2].contains(&inserted.to_string().as_str()),
        "a new node gets a new ID"
    );
    assert_eq!(result.created, [inserted], "the new ID is reported");
    assert_eq!(result.revision, 1, "a new node starts at revision 1");
    assert!(doc.contains(inserted));
    assert_eq!(rev(&doc, &inserted.to_string()), 1);
    assert_eq!(
        children(&doc, Some(S1)),
        [H1, P1, &inserted.to_string(), P2],
        "inserted after P1"
    );
    assert!(
        doc.node_html(inserted)
            .expect("the new node prints")
            .contains("Inserted")
    );

    let first = apply_one(&mut doc, insert_into(S1, None, "<p>First</p>")).node;
    assert_eq!(
        children(&doc, Some(S1))[0],
        first.to_string(),
        "a null `after` inserts first"
    );

    let top = apply_one(
        &mut doc,
        json!({"op": "insert", "parent": null, "position": {"after": S1}, "content": "<hr>"}),
    )
    .node;
    assert_eq!(
        children(&doc, None),
        [S1, &top.to_string(), S2],
        "a null parent inserts at the top level"
    );
    let top_first = apply_one(
        &mut doc,
        json!({"op": "insert", "parent": null, "position": {"after": null}, "content": "<hr>"}),
    )
    .node;
    assert_eq!(children(&doc, None)[0], top_first.to_string());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn insert_gives_every_new_block_of_the_content_an_id_in_document_order() {
    let mut doc = doc();
    let result = apply_one(
        &mut doc,
        insert_into(S2, None, "<ul><li>a</li><li>b</li></ul>"),
    );
    assert_eq!(result.created.len(), 3, "a list and two items: {result:?}");
    assert_eq!(result.created[0], result.node, "the block comes first");
    let distinct: std::collections::HashSet<_> = result.created.iter().collect();
    assert_eq!(distinct.len(), 3, "every new node has its own ID");
    let list = result.node.to_string();
    assert_eq!(
        children(&doc, Some(&list)),
        [result.created[1].to_string(), result.created[2].to_string()],
        "the items are the list's children, in order"
    );
    for node in &result.created {
        assert_eq!(rev(&doc, &node.to_string()), 1);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn inserts_neither_check_nor_change_the_parents_revision() {
    let mut doc = doc();
    apply_one(&mut doc, set(UL, 1, json!({"ordered": true})));
    assert_eq!(rev(&doc, UL), 2);
    apply_one(&mut doc, insert_into(UL, Some(LI2), "<li>Three</li>"));
    apply_one(&mut doc, insert_into(UL, None, "<li>Zero</li>"));
    assert_eq!(
        rev(&doc, UL),
        2,
        "inserting children leaves the parent at 2"
    );
    apply_one(&mut doc, insert_into(S1, Some(P2), "<p>Last</p>"));
    assert_eq!(rev(&doc, S1), 1);
    assert_eq!(children(&doc, Some(UL)).len(), 4);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn concurrent_inserts_after_the_same_block_both_apply() {
    let mut doc = doc();
    // Two agents compose their batches against the same version.
    let agent_a = batch(json!([insert_into(S1, Some(P1), "<p>From agent A</p>")]));
    let agent_b = batch(json!([insert_into(S1, Some(P1), "<p>From agent B</p>")]));
    let a = doc.apply(&agent_a, &mut new_id).expect("A applies").ops[0].node;
    let b = doc.apply(&agent_b, &mut new_id).expect("B applies").ops[0].node;
    assert_eq!(
        children(&doc, Some(S1)),
        [H1, P1, &b.to_string(), &a.to_string(), P2],
        "each insert lands right after P1"
    );
    assert_eq!(rev(&doc, S1), 1, "the parent was not touched");
    let html = doc.to_html();
    assert!(html.contains("From agent A") && html.contains("From agent B"));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn child_order_uses_fractional_indexes_that_never_renumber_siblings() {
    let mut doc = doc();
    let mut previous: Vec<(String, String)> = Vec::new();
    let collect = |doc: &EditDoc| -> Vec<(String, String)> {
        children(doc, Some(S1))
            .into_iter()
            .map(|node| {
                let key = doc
                    .sort_key(id(&node))
                    .unwrap_or_else(|| panic!("{node} has a sort key"));
                (node, key)
            })
            .collect()
    };
    let check = |now: &[(String, String)], before: &[(String, String)]| {
        for pair in now.windows(2) {
            assert!(
                pair[0].1 < pair[1].1,
                "sort keys increase along the children: {now:?}"
            );
        }
        for (node, key) in before {
            let found = now.iter().find(|(other, _)| other == node);
            assert_eq!(
                found.map(|(_, key)| key),
                Some(key),
                "an insert never changes the sort key of other siblings"
            );
        }
    };
    previous.extend(collect(&doc));
    for round in 0..60 {
        let after = if round % 3 == 0 {
            None
        } else if round % 3 == 1 {
            Some(P1)
        } else {
            Some(P2)
        };
        apply_one(&mut doc, insert_into(S1, after, "<p>More</p>"));
        let now = collect(&doc);
        check(&now, &previous);
        previous = now;
    }
    assert_eq!(previous.len(), 3 + 60, "every insert added a child");
}

// --- Unknown references ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn unknown_ids_are_errors_that_name_them() {
    let mut doc = doc();
    let cases = [
        (
            set(UNKNOWN, 1, json!({"level": 2})),
            "set of an unknown node",
        ),
        (
            replace(UNKNOWN, 1, "<p>x</p>"),
            "replace of an unknown node",
        ),
        (delete(UNKNOWN, 1), "delete of an unknown node"),
        (
            move_to(UNKNOWN, 1, Some(S1), None),
            "move of an unknown node",
        ),
        (
            move_to(P1, 1, Some(UNKNOWN), None),
            "move into an unknown parent",
        ),
        (
            move_to(P1, 1, Some(S2), Some(UNKNOWN)),
            "move after an unknown node",
        ),
        (
            insert_into(UNKNOWN, None, "<p>x</p>"),
            "insert into an unknown parent",
        ),
        (
            insert_into(S1, Some(UNKNOWN), "<p>x</p>"),
            "insert after an unknown node",
        ),
        (
            insert_into(S2, Some(P1), "<p>x</p>"),
            "insert after a node that is not a child of the parent",
        ),
        (
            move_to(P1, 1, Some(S2), Some(P2)),
            "move after a node that is not a child of the new parent",
        ),
    ];
    for (op, what) in cases {
        let (index, message) = invalid(&mut doc, json!([op.clone()]));
        assert_eq!(index, 0, "{what}");
        let named = [UNKNOWN, P1, P2].iter().any(|node| message.contains(node));
        assert!(
            named,
            "{what}: the error must name the ID, but it is `{message}`"
        );
        if what.contains("unknown") {
            assert!(
                message.contains(UNKNOWN),
                "{what}: the error must name {UNKNOWN}, but it is `{message}`"
            );
        }
    }
    let error = refused(&mut doc, json!([delete(UNKNOWN, 1)]));
    assert!(error.to_string().contains(UNKNOWN), "{error}");
}

// --- Replace ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn replace_keeps_the_nodes_id_and_place() {
    let mut doc = doc();
    let result = apply_one(&mut doc, replace(P1, 1, "<p>Rewritten <em>fully</em></p>"));
    assert_eq!(result.node, id(P1), "replace keeps the replaced node's ID");
    assert_eq!(result.revision, 2);
    assert!(result.created.is_empty(), "no new node: {result:?}");
    assert!(result.removed.is_empty(), "no node left: {result:?}");
    assert_eq!(rev(&doc, P1), 2);
    assert_eq!(children(&doc, Some(S1)), [H1, P1, P2], "same place");
    let html = doc.node_html(id(P1)).expect("P1 prints");
    assert!(html.contains("Rewritten <em>fully</em>"), "{html}");
    assert!(!html.contains("First paragraph"), "{html}");

    // The content may carry the same ID; it may also be another kind of block.
    let result = apply_one(
        &mut doc,
        replace(
            P2,
            1,
            &format!("<h3 data-node-id=\"{P2}\">Now a heading</h3>"),
        ),
    );
    assert_eq!(result.node, id(P2));
    let html = doc.node_html(id(P2)).expect("P2 prints");
    assert!(html.starts_with("<h3"), "P2 is a heading now: {html}");
    assert_eq!(children(&doc, Some(S1)), [H1, P1, P2]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn replace_keeps_descendants_that_the_content_includes_removes_the_rest_and_marks_new_ones() {
    let mut doc = doc();
    let content = format!(
        "<section><h2 data-node-id=\"{H2}\">Rules, rewritten</h2><p>A brand new paragraph</p><ul data-node-id=\"{UL}\"><li data-node-id=\"{LI2}\">Two, kept</li><li>A new item</li></ul></section>"
    );
    let result = apply_one(&mut doc, replace(S2, 1, &content));
    assert_eq!(result.node, id(S2), "the section keeps its ID");
    assert_eq!(result.revision, 2);

    // Kept: H2, UL and LI2 keep their IDs and place in the content.
    assert!(doc.contains(id(H2)) && doc.contains(id(UL)) && doc.contains(id(LI2)));
    // Removed: what the content leaves out.
    for gone in [LI1, NOTE, TABLE, ROW, CELL, IMG, FIGURE] {
        assert!(
            !doc.contains(id(gone)),
            "{gone} was left out, so it is removed"
        );
    }
    let mut removed: Vec<String> = result.removed.iter().map(|node| node.to_string()).collect();
    removed.sort();
    assert_eq!(
        removed,
        [LI1, NOTE, TABLE, ROW, CELL, IMG, FIGURE].map(String::from)
    );
    // New: the new paragraph and the new item, each with a new ID at revision 1.
    assert_eq!(result.created.len(), 2, "two new nodes: {result:?}");
    for node in &result.created {
        assert!(![S1, H1, P1, P2, S2, H2, UL, LI1, LI2].contains(&node.to_string().as_str()));
        assert_eq!(rev(&doc, &node.to_string()), 1);
    }
    let kids = children(&doc, Some(S2));
    assert_eq!(kids.len(), 3);
    assert_eq!(kids[0], H2);
    assert_eq!(
        kids[1],
        result.created[0].to_string(),
        "the new paragraph is second"
    );
    assert_eq!(kids[2], UL);
    assert_eq!(
        children(&doc, Some(UL)),
        [LI2.to_string(), result.created[1].to_string()],
        "the list keeps LI2 and gets the new item after it"
    );
    assert!(
        doc.node_html(id(H2))
            .expect("H2 prints")
            .contains("Rules, rewritten")
    );
    // Other sections are not touched.
    assert_eq!(rev(&doc, S1), 1);
    assert_eq!(children(&doc, None), [S1, S2]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn replace_refuses_content_with_a_different_root_id_or_an_id_used_elsewhere() {
    let mut doc = doc();
    let (index, message) = invalid(
        &mut doc,
        json!([replace(
            P1,
            1,
            &format!("<p data-node-id=\"{P2}\">Other</p>")
        )]),
    );
    assert_eq!(index, 0);
    assert!(
        message.contains(P2),
        "the error must name the clashing ID: {message}"
    );
    let (_, message) = invalid(
        &mut doc,
        json!([replace(
            S1,
            1,
            &format!("<section><p data-node-id=\"{LI1}\">Steals an ID</p></section>")
        )]),
    );
    assert!(message.contains(LI1), "{message}");
}

// --- Set ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn set_changes_the_properties_of_a_block() {
    let mut doc = doc();
    apply_one(&mut doc, set(H1, 1, json!({"level": 3})));
    assert!(doc.node_html(id(H1)).unwrap().starts_with("<h3"));
    apply_one(&mut doc, set(NOTE, 1, json!({"kind": "warning"})));
    assert!(
        doc.node_html(id(NOTE))
            .unwrap()
            .contains("data-callout=\"warning\""),
        "{}",
        doc.node_html(id(NOTE)).unwrap()
    );
    apply_one(&mut doc, set(UL, 1, json!({"ordered": true})));
    assert!(doc.node_html(id(UL)).unwrap().starts_with("<ol"));
    assert_eq!(
        doc.children(Some(id(UL))).unwrap().len(),
        2,
        "items are kept"
    );
    apply_one(&mut doc, set(CELL, 1, json!({"header": false})));
    assert!(doc.node_html(id(CELL)).unwrap().starts_with("<td"));
    apply_one(&mut doc, set(IMG, 1, json!({"alt": "A new description"})));
    assert!(
        doc.node_html(id(IMG))
            .unwrap()
            .contains("A new description")
    );
    apply_one(
        &mut doc,
        set(FIGURE, 1, json!({"caption": "A new caption"})),
    );
    assert!(doc.node_html(id(FIGURE)).unwrap().contains("A new caption"));
    apply_one(
        &mut doc,
        set(
            P1,
            1,
            json!({"html": "Some <strong>bold</strong> and <a href=\"https://example.com\">a link</a>"}),
        ),
    );
    assert!(
        doc.node_html(id(P1))
            .unwrap()
            .contains("Some <strong>bold</strong> and <a href=\"https://example.com\">a link</a>")
    );
    // Several properties in one set, one revision.
    let result = apply_one(&mut doc, set(H2, 1, json!({"level": 4, "html": "Both"})));
    assert_eq!(result.revision, 2);
    let html = doc.node_html(id(H2)).unwrap();
    assert!(html.starts_with("<h4") && html.contains("Both"), "{html}");
    HtmlDoc::parse(&doc.to_html()).expect("the doc is still a valid HTML doc");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn set_runs_the_validation_of_the_doc_type() {
    let mut doc = doc();
    let refusals = [
        (
            set(H1, 1, json!({"level": 7})),
            "level",
            "a heading level of 7",
        ),
        (
            set(H1, 1, json!({"level": 0})),
            "level",
            "a heading level of 0",
        ),
        (set(H1, 1, json!({"level": "big"})), "level", "a text level"),
        (
            set(P1, 1, json!({"level": 2})),
            "level",
            "a level on a paragraph",
        ),
        (
            set(NOTE, 1, json!({"kind": "danger"})),
            "kind|danger",
            "an unknown callout kind",
        ),
        (
            set(P1, 1, json!({"kind": "note"})),
            "kind",
            "a callout kind on a paragraph",
        ),
        (
            set(P1, 1, json!({"ordered": true})),
            "ordered",
            "ordered on a paragraph",
        ),
        (
            set(P1, 1, json!({"colour": "red"})),
            "colour",
            "an unknown property",
        ),
        (
            set(P1, 1, json!({"id": "b0000fff"})),
            "id|propert",
            "a new id",
        ),
        (
            set(P1, 1, json!({"alt": "text"})),
            "alt",
            "alt on a paragraph",
        ),
        (
            set(P1, 1, json!({"html": "<span>x</span>"})),
            "span",
            "a tag outside the format",
        ),
        (
            set(P1, 1, json!({"html": "<p>nested block</p>"})),
            "p|block",
            "a block in inline content",
        ),
        (
            set(
                P1,
                1,
                json!({"html": "<a href=\"javascript:alert(1)\">x</a>"}),
            ),
            "javascript|href|link",
            "a script link",
        ),
        (
            set(S1, 1, json!({"html": "text"})),
            "html|section|inline|text",
            "inline content on a section",
        ),
        (
            set(P1, 1, json!({"style": "color:red"})),
            "style",
            "a style",
        ),
    ];
    for (op, problem, what) in refusals {
        let (index, message) = invalid(&mut doc, json!([op]));
        assert_eq!(index, 0, "{what}");
        assert!(
            names_problem(&message, problem),
            "{what}: the error must name `{problem}`, but it is `{message}`"
        );
    }
}

// --- Move ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn move_reparents_a_block_with_its_subtree_and_keeps_ids() {
    let mut doc = doc();
    let result = apply_one(&mut doc, move_to(P1, 1, Some(S2), Some(H2)));
    assert_eq!(result.node, id(P1));
    assert_eq!(result.revision, 2, "the moved node's revision rises");
    assert!(
        result.created.is_empty() && result.removed.is_empty(),
        "{result:?}"
    );
    assert_eq!(children(&doc, Some(S1)), [H1, P2]);
    assert_eq!(children(&doc, Some(S2))[..3], [H2, P1, UL]);
    assert_eq!(rev(&doc, S1), 1, "the old parent is not changed");
    assert_eq!(rev(&doc, S2), 1, "the new parent is not changed");

    // A list moves with its items, which keep their IDs and revisions.
    apply_one(&mut doc, move_to(UL, 1, None, Some(S1)));
    assert_eq!(children(&doc, None), [S1, UL, S2]);
    assert_eq!(children(&doc, Some(UL)), [LI1, LI2]);
    assert_eq!(rev(&doc, LI1), 1);
    assert_eq!(rev(&doc, UL), 2);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn move_within_a_parent_reorders_its_children() {
    let mut doc = doc();
    apply_one(&mut doc, move_to(P2, 1, Some(S1), None));
    assert_eq!(children(&doc, Some(S1)), [P2, H1, P1]);
    apply_one(&mut doc, move_to(P2, 2, Some(S1), Some(P1)));
    assert_eq!(children(&doc, Some(S1)), [H1, P1, P2]);
    assert_eq!(rev(&doc, P2), 3);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_block_cannot_move_into_itself_or_its_descendants() {
    let mut doc = doc();
    for (op, what) in [
        (move_to(S2, 1, Some(S2), None), "a section into itself"),
        (
            move_to(S2, 1, Some(UL), None),
            "a section into its own list",
        ),
        (
            move_to(S2, 1, Some(LI1), None),
            "a section into its own item",
        ),
    ] {
        let (index, _message) = invalid(&mut doc, json!([op]));
        assert_eq!(index, 0, "{what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_move_obeys_the_rules_of_the_new_parent() {
    let mut doc = doc();
    for (op, what) in [
        (move_to(P1, 1, Some(UL), None), "a paragraph into a list"),
        (
            move_to(LI1, 1, Some(S1), None),
            "a list item into a section",
        ),
        (move_to(LI1, 1, None, None), "a list item to the top level"),
        (
            move_to(ROW, 1, Some(S1), None),
            "a table row into a section",
        ),
    ] {
        let (index, _message) = invalid(&mut doc, json!([op]));
        assert_eq!(index, 0, "{what}");
    }
    apply_one(&mut doc, move_to(LI1, 1, Some(UL), Some(LI2)));
    assert_eq!(
        children(&doc, Some(UL)),
        [LI2, LI1],
        "an item may move within its list"
    );
}

// --- Delete ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn delete_removes_the_block_and_all_its_descendants() {
    let mut doc = doc();
    let result = apply_one(&mut doc, delete(UL, 1));
    assert_eq!(result.node, id(UL));
    let removed: Vec<String> = result.removed.iter().map(|node| node.to_string()).collect();
    assert_eq!(
        removed,
        [UL, LI1, LI2],
        "the block and its items, in document order"
    );
    for gone in [UL, LI1, LI2] {
        assert!(!doc.contains(id(gone)));
        assert_eq!(doc.revision(id(gone)), None);
    }
    assert_eq!(children(&doc, Some(S2))[..2], [H2, NOTE]);
    assert_eq!(rev(&doc, S2), 1, "the parent's revision is not changed");
    assert!(!doc.to_html().contains(LI1));
    let (_, message) = invalid(&mut doc, json!([set(LI1, 1, json!({"html": "Too late"}))]));
    assert!(
        message.contains(LI1),
        "a deleted node is unknown: {message}"
    );
}

// --- Validation and cleaning on every operation ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn insert_runs_the_html_doc_rules() {
    let mut doc = doc();
    let bad = [
        ("<span>x</span>", "span"),
        ("<p>x <b>bold</b></p>", "b"),
        ("<p>x</p><p>y</p>", ""),
        ("<p>x<script>alert(1)</script></p>", "script"),
        (
            "<p><a href=\"javascript:alert(1)\">x</a></p>",
            "javascript|href|link",
        ),
        ("<p><a href=\"/relative\">x</a></p>", "href|link|relative"),
        (
            "<img src=\"https://example.com/x.png\" alt=\"\">",
            "src|attachment|img",
        ),
        ("<p style=\"color:red\">x</p>", "style"),
        ("<aside data-callout=\"danger\">x</aside>", "danger|callout"),
        ("<table><tbody><tr><td>x</td></tr></tbody></table>", "tbody"),
        ("<p>unclosed", ""),
        ("plain text", ""),
        ("", ""),
    ];
    for (content, problem) in bad {
        let (index, message) = invalid(&mut doc, json!([insert_into(S1, None, content)]));
        assert_eq!(index, 0, "{content}");
        assert!(
            problem.is_empty() || names_problem(&message, problem),
            "inserting `{content}` must be refused naming `{problem}`, but the error is `{message}`"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn insert_cleans_what_it_keeps() {
    let mut doc = doc();
    let content = "<figure><svg viewBox=\"0 0 10 10\" onload=\"steal()\"><script>steal()</script><foreignObject><p>x</p></foreignObject><rect x=\"1\" y=\"1\" width=\"5\" height=\"5\" onclick=\"steal()\" fill=\"#336699\"/><image href=\"https://evil.example/x.png\"/></svg><figcaption>Clean</figcaption></figure>";
    let figure = apply_one(&mut doc, insert_into(S1, None, content)).node;
    let html = doc.node_html(figure).unwrap().to_lowercase();
    assert!(html.contains("<rect") && html.contains("clean"), "{html}");
    for dirty in [
        "script",
        "onclick",
        "onload",
        "foreignobject",
        "evil.example",
        "<image",
    ] {
        assert!(
            !html.contains(dirty),
            "the SVG is cleaned of `{dirty}`: {html}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn replace_runs_the_html_doc_rules_and_cleaning() {
    let mut doc = doc();
    let (_, message) = invalid(&mut doc, json!([replace(P1, 1, "<p><span>x</span></p>")]));
    assert!(names_problem(&message, "span"), "{message}");
    let (_, message) = invalid(
        &mut doc,
        json!([replace(P1, 1, "<p><a href=\"data:text/html,x\">x</a></p>")]),
    );
    assert!(names_problem(&message, "data|href|link"), "{message}");

    let content = "<figure><svg viewBox=\"0 0 10 10\"><script>steal()</script><circle cx=\"5\" cy=\"5\" r=\"3\" onclick=\"steal()\"/></svg><figcaption>Cleaned</figcaption></figure>";
    apply_one(&mut doc, replace(FIGURE, 1, content));
    let html = doc.node_html(id(FIGURE)).unwrap().to_lowercase();
    assert!(
        html.contains("<circle") && !html.contains("script") && !html.contains("onclick"),
        "{html}"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_operation_obeys_the_rules_of_the_parent_it_puts_a_block_in() {
    let mut doc = doc();
    for (op, what) in [
        (
            insert_into(UL, None, "<p>Not an item</p>"),
            "a paragraph into a list",
        ),
        (
            insert_into(S1, None, "<li>Loose item</li>"),
            "a list item into a section",
        ),
        (
            insert_into(S1, None, "<tr><td>x</td></tr>"),
            "a table row into a section",
        ),
        (
            insert_into(S1, None, "<td>x</td>"),
            "a table cell into a section",
        ),
        (
            insert_into(TABLE, None, "<p>Not a row</p>"),
            "a paragraph into a table",
        ),
        (
            insert_into(ROW, None, "<p>Not a cell</p>"),
            "a paragraph into a row",
        ),
        (
            insert_into(P1, None, "<p>Inside</p>"),
            "a block into a paragraph",
        ),
        (
            json!({"op": "insert", "parent": null, "position": {"after": null}, "content": "<li>x</li>"}),
            "a list item at the top level",
        ),
        (
            replace(LI1, 1, "<p>Not an item</p>"),
            "replacing an item with a paragraph",
        ),
        (
            replace(CELL, 1, "<li>x</li>"),
            "replacing a cell with an item",
        ),
    ] {
        let (index, _message) = invalid(&mut doc, json!([op]));
        assert_eq!(index, 0, "{what}");
    }
    apply_one(&mut doc, insert_into(UL, Some(LI1), "<li>Fine</li>"));
    apply_one(&mut doc, insert_into(ROW, Some(CELL), "<td>Fine</td>"));
    apply_one(&mut doc, insert_into(TABLE, None, "<tr><td>Fine</td></tr>"));
    apply_one(
        &mut doc,
        insert_into(S1, None, "<section><h2>Fine</h2></section>"),
    );
    apply_one(
        &mut doc,
        json!({"op": "insert", "parent": null, "position": {"after": null}, "content": "<section><p>Fine</p></section>"}),
    );
    apply_one(&mut doc, replace(LI1, 1, "<li>Still an item</li>"));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_doc_is_a_valid_html_doc_after_every_batch() {
    let mut doc = doc();
    let batches = [
        json!([insert_into(
            S1,
            Some(P1),
            "<ul><li>a</li><li>b <em>c</em></li></ul>"
        )]),
        json!([set(
            H1,
            1,
            json!({"level": 2, "html": "Title <code>x</code>"})
        )]),
        json!([move_to(P2, 1, Some(S2), None), delete(LI1, 1)]),
        json!([replace(
            NOTE,
            1,
            "<aside data-callout=\"decision\">Decided</aside>"
        )]),
        json!([insert_into(
            S2,
            None,
            "<figure><pre class=\"mermaid\">graph TD;\n  A--&gt;B</pre><figcaption>x</figcaption></figure>"
        )]),
    ];
    for ops in batches {
        apply(&mut doc, ops);
        let html = doc.to_html();
        let parsed = HtmlDoc::parse(&html)
            .unwrap_or_else(|error| panic!("the doc is no longer valid: {error}\n{html}"));
        assert_eq!(parsed.to_html(), html, "the doc prints stable HTML");
        let ids = printed_ids(&html);
        let distinct: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(distinct.len(), ids.len(), "every ID is used once");
        for node in &ids {
            assert!(doc.revision(id(node)).is_some(), "{node} has a revision");
        }
    }
}

// --- Batches ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_batch_applies_its_operations_in_order() {
    let mut doc = doc();
    let applied = apply(
        &mut doc,
        json!([
            set(P1, 1, json!({"html": "Changed"})),
            insert_into(S1, Some(P1), "<p>New</p>"),
            move_to(P2, 1, Some(S2), None),
            delete(LI1, 1)
        ]),
    );
    assert_eq!(applied.ops.len(), 4, "one result per operation");
    assert_eq!(applied.ops[0].node, id(P1));
    assert_eq!(applied.ops[0].revision, 2);
    assert_eq!(applied.ops[1].created.len(), 1);
    assert_eq!(applied.ops[2].node, id(P2));
    assert_eq!(applied.ops[3].removed, [id(LI1)]);
    assert_eq!(
        children(&doc, Some(S1)),
        [H1, P1, &applied.ops[1].node.to_string()]
    );
    assert_eq!(children(&doc, Some(S2))[0], P2);
    assert!(!doc.contains(id(LI1)));

    // Later operations see the effect of earlier ones.
    let (index, message) = invalid(
        &mut doc,
        json!([
            delete(UL, 1),
            insert_into(UL, None, "<li>Into a deleted list</li>")
        ]),
    );
    assert_eq!(index, 1, "the second operation refers to a deleted node");
    assert!(message.contains(UL), "{message}");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_batch_that_fails_changes_nothing() {
    let mut doc = doc();
    let before = snapshot(&doc);
    let ops = json!([
        set(P1, 1, json!({"html": "Changed"})),
        insert_into(S1, None, "<p>New</p>"),
        move_to(P2, 1, Some(S2), None),
        delete(UL, 1),
        replace(S2, 1, "<section><h2>Replaced</h2></section>"),
        set(H1, 1, json!({"level": 9}))
    ]);
    let (index, message) = invalid(&mut doc, ops);
    assert_eq!(
        index, 5,
        "the sixth operation is the invalid one: {message}"
    );
    assert_eq!(snapshot(&doc), before, "the first five were undone");
    for node in [P1, H1, P2, S2, UL, LI1] {
        assert_eq!(rev(&doc, node), 1, "{node} is back at revision 1");
    }

    // Nothing was half applied: the valid part of the batch still applies, in full.
    let applied = apply(
        &mut doc,
        json!([
            set(P1, 1, json!({"html": "Changed"})),
            insert_into(S1, None, "<p>New</p>"),
            move_to(P2, 1, Some(S2), None),
            delete(UL, 1)
        ]),
    );
    assert_eq!(applied.ops.len(), 4);
    assert_eq!(rev(&doc, P1), 2);
    assert!(!doc.contains(id(UL)));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_empty_batch_applies_and_changes_nothing() {
    let mut doc = doc();
    let before = snapshot(&doc);
    let applied = apply(&mut doc, json!([]));
    assert!(applied.ops.is_empty());
    assert_eq!(snapshot(&doc), before);
}
