//! US-021: the operation engine. One engine edits every doc type; these tests
//! run it on HTML docs, which hold any HTML since US-109 (US-020's fixed block
//! set is gone). They read no files, so the same tests run natively and in
//! WebAssembly (`wasm-pack test --node`).
//!
//! The tests pin this API and JSON form (docs/formats/operations.md documents
//! the JSON):
//!
//! - `EditDoc::from_html` starts a doc whose every block-level element has a
//!   `data-omni-id` (US-109); every node starts at revision 1. `to_html`, `node_html`, `contains`,
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
//!   level. Content is the HTML of one element, which may hold any HTML.
//! - US-109: operations address any element with an ID at any depth: inside
//!   table cells, `<details>`, `<div>`s and list items. `set` takes `html`,
//!   the inner HTML of the element (any markup); an unknown property is
//!   refused. An operation whose result would not read back as the same tree
//!   (a `<div>` inside a `<p>`, a `<tr>` outside a table) is refused, so the
//!   doc printed with `to_html_with_revisions` always reads back with
//!   `from_html` as the same doc.
//! - A revision mismatch is `ApplyError::Conflict` with the node's current
//!   revision and HTML; any other refusal is `ApplyError::Invalid { index,
//!   message }`, and the message names the ID or the tag that is wrong.

use omni_core::design::NodeId;
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
const FIGURE: &str = "b000000e";
const SVG: &str = "b000000f";
const UNKNOWN: &str = "b0000fff";

const BASE: &str = r##"<section data-omni-id="b0000001">
<h1 data-omni-id="b0000002">Orders</h1>
<p data-omni-id="b0000003">First paragraph</p>
<p data-omni-id="b0000004">Second paragraph</p>
</section>
<section data-omni-id="b0000005">
<h2 data-omni-id="b0000006">Rules</h2>
<ul data-omni-id="b0000007"><li data-omni-id="b0000008">One</li><li data-omni-id="b0000009">Two</li></ul>
<aside data-omni-id="b000000a" data-callout="note">A note</aside>
<table data-omni-id="b000000b"><tbody><tr data-omni-id="b000000c"><th data-omni-id="b000000d">Field</th></tr></tbody></table>
<figure data-omni-id="b000000e"><svg data-omni-id="b000000f" viewBox="0 0 10 10"><rect x="1" y="1" width="5" height="5"/></svg></figure>
</section>
"##;

/// US-109: a doc whose elements nest deeply: a table in a `<details>` in a
/// card `<div>`, and a `<div>` in a list item.
const DS: &str = "d0000001";
const CARD: &str = "d0000002";
const MORE: &str = "d0000003";
const INNER: &str = "d0000004";
const DT: &str = "d0000005";
const DR: &str = "d0000006";
const DC1: &str = "d0000007";
const DP1: &str = "d0000008";
const DC2: &str = "d0000009";
const DUL: &str = "d000000a";
const DLI: &str = "d000000b";
const DDIV: &str = "d000000c";
const DP2: &str = "d000000d";

const DEEP: &str = r##"<section data-omni-id="d0000001">
<div data-omni-id="d0000002" class="card"><details data-omni-id="d0000003" open><div data-omni-id="d0000004"><table data-omni-id="d0000005"><tbody><tr data-omni-id="d0000006"><td data-omni-id="d0000007"><p data-omni-id="d0000008">Deep <span class="x">text</span></p></td><td data-omni-id="d0000009">Second cell</td></tr></tbody></table></div></details></div>
<ul data-omni-id="d000000a"><li data-omni-id="d000000b"><div data-omni-id="d000000c"><p data-omni-id="d000000d">In an item</p></div></li></ul>
</section>
"##;

// --- Helpers ---

fn id(text: &str) -> NodeId {
    NodeId::from_str(text).unwrap_or_else(|error| panic!("{text} must be a node ID: {error}"))
}

fn doc() -> EditDoc {
    EditDoc::from_html(BASE).expect("the base doc is valid")
}

fn deep() -> EditDoc {
    EditDoc::from_html(DEEP).expect("the deep doc is valid")
}

/// Checks that the doc printed with revisions reads back as the same doc.
fn assert_reads_back(doc: &EditDoc) {
    let head = doc.to_html_with_revisions();
    let read = EditDoc::from_html(&head)
        .unwrap_or_else(|error| panic!("the doc no longer reads back: {error}\n{head}"));
    assert_eq!(
        read.to_html_with_revisions(),
        head,
        "the doc prints stable HTML"
    );
    let ids = printed_ids(&head);
    let distinct: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(distinct.len(), ids.len(), "every ID is used once: {head}");
    for node in &ids {
        assert_eq!(
            read.revision(id(node)),
            doc.revision(id(node)),
            "{node} reads back at its revision"
        );
        assert_eq!(read.parent(id(node)), doc.parent(id(node)), "{node}");
    }
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

/// The `data-omni-id` values of printed HTML, in order.
fn printed_ids(html: &str) -> Vec<String> {
    const ATTRIBUTE: &str = "data-omni-id=\"";
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
fn a_revision_ahead_of_the_node_is_a_conflict_too() {
    let mut doc = doc();
    let found = conflict(&mut doc, json!([set(P1, 5, json!({"html": "Never"}))]));
    assert_eq!(found.expected, 5);
    assert_eq!(found.current_revision, 1);
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
            set(UNKNOWN, 1, json!({"html": "x"})),
            UNKNOWN,
            "set of an unknown node",
        ),
        (
            replace(UNKNOWN, 1, "<p>x</p>"),
            UNKNOWN,
            "replace of an unknown node",
        ),
        (delete(UNKNOWN, 1), UNKNOWN, "delete of an unknown node"),
        (
            move_to(UNKNOWN, 1, Some(S1), None),
            UNKNOWN,
            "move of an unknown node",
        ),
        (
            move_to(P1, 1, Some(UNKNOWN), None),
            UNKNOWN,
            "move into an unknown parent",
        ),
        (
            move_to(P1, 1, Some(S2), Some(UNKNOWN)),
            UNKNOWN,
            "move after an unknown node",
        ),
        (
            insert_into(UNKNOWN, None, "<p>x</p>"),
            UNKNOWN,
            "insert into an unknown parent",
        ),
        (
            insert_into(S1, Some(UNKNOWN), "<p>x</p>"),
            UNKNOWN,
            "insert after an unknown node",
        ),
        (
            insert_into(S2, Some(P1), "<p>x</p>"),
            P1,
            "insert after a node that is not a child of the parent",
        ),
        (
            move_to(P1, 1, Some(S2), Some(P2)),
            P2,
            "move after a node that is not a child of the new parent",
        ),
    ];
    for (op, named, what) in cases {
        let (index, message) = invalid(&mut doc, json!([op]));
        assert_eq!(index, 0, "{what}");
        assert!(
            message.contains(named),
            "{what}: the error must name {named}, but it is `{message}`"
        );
    }
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
            &format!("<h3 data-omni-id=\"{P2}\">Now a heading</h3>"),
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
        "<section><h2 data-omni-id=\"{H2}\">Rules, rewritten</h2><p>A brand new paragraph</p><ul data-omni-id=\"{UL}\"><li data-omni-id=\"{LI2}\">Two, kept</li><li>A new item</li></ul></section>"
    );
    let result = apply_one(&mut doc, replace(S2, 1, &content));
    assert_eq!(result.node, id(S2), "the section keeps its ID");
    assert_eq!(result.revision, 2);

    // Kept: H2, UL and LI2 keep their IDs and place in the content.
    assert!(doc.contains(id(H2)) && doc.contains(id(UL)) && doc.contains(id(LI2)));
    // Removed: what the content leaves out.
    for gone in [LI1, NOTE, TABLE, ROW, CELL, FIGURE, SVG] {
        assert!(
            !doc.contains(id(gone)),
            "{gone} was left out, so it is removed"
        );
    }
    let mut removed: Vec<String> = result.removed.iter().map(|node| node.to_string()).collect();
    removed.sort();
    assert_eq!(
        removed,
        [LI1, NOTE, TABLE, ROW, CELL, FIGURE, SVG].map(String::from)
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
fn replace_never_takes_an_id_another_node_has() {
    // US-109: an ID in the content that another node of the doc has is not
    // kept (US-021 refused the operation): the replaced node keeps its own
    // ID, the other node keeps its ID and place, and the element gets a new
    // ID.
    let mut doc = doc();
    let result = apply_one(
        &mut doc,
        replace(P1, 1, &format!("<p data-omni-id=\"{P2}\">Other</p>")),
    );
    assert_eq!(result.node, id(P1), "the replaced node keeps its ID");
    assert!(doc.node_html(id(P1)).unwrap().contains("Other"));
    assert!(doc.node_html(id(P2)).unwrap().contains("Second paragraph"));
    assert_eq!(rev(&doc, P2), 1, "P2 is not touched");
    assert_eq!(children(&doc, Some(S1)), [H1, P1, P2]);

    let result = apply_one(
        &mut doc,
        replace(
            S1,
            1,
            &format!("<section><p data-omni-id=\"{LI1}\">Takes no ID</p></section>"),
        ),
    );
    assert_eq!(result.created.len(), 1, "the paragraph is new: {result:?}");
    assert_ne!(result.created[0], id(LI1));
    assert_eq!(
        children(&doc, Some(UL)),
        [LI1, LI2],
        "LI1 stays in its list"
    );
    assert!(doc.node_html(id(LI1)).unwrap().contains("One"));
    assert_eq!(rev(&doc, LI1), 1);
    assert_reads_back(&doc);
}

// --- Set ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn set_changes_the_inner_html_of_any_element() {
    let mut doc = doc();
    let result = apply_one(
        &mut doc,
        set(
            P1,
            1,
            json!({"html": "Some <strong>bold</strong>, <span class=\"tag\">a span</span> and <a href=\"https://example.com\">a link</a>"}),
        ),
    );
    assert_eq!(result.revision, 2);
    assert!(
        doc.node_html(id(P1))
            .unwrap()
            .contains("Some <strong>bold</strong>, <span class=\"tag\">a span</span> and <a href=\"https://example.com\">a link</a>"),
        "{}",
        doc.node_html(id(P1)).unwrap()
    );
    apply_one(
        &mut doc,
        set(NOTE, 1, json!({"html": "A <b>warning</b> now"})),
    );
    assert!(
        doc.node_html(id(NOTE))
            .unwrap()
            .contains("A <b>warning</b> now")
    );
    apply_one(&mut doc, set(CELL, 1, json!({"html": "<em>Name</em>"})));
    assert!(doc.node_html(id(CELL)).unwrap().contains("<em>Name</em>"));
    assert_eq!(rev(&doc, ROW), 1, "the row is not changed");
    assert_reads_back(&doc);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn set_refuses_unknown_properties_and_names_them() {
    let mut doc = doc();
    let refusals = [
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
            set(P1, 1, json!({"style": "color:red"})),
            "style",
            "a style property",
        ),
        (
            set(P1, 1, json!({"html": 5})),
            "html",
            "html that is no text",
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
fn a_move_that_would_not_read_back_is_refused() {
    let mut doc = doc();
    for (op, what) in [
        (
            move_to(ROW, 1, Some(S1), None),
            "a table row into a section",
        ),
        (
            move_to(P1, 1, Some(P2), None),
            "a paragraph into a paragraph",
        ),
        (move_to(S2, 1, Some(P1), None), "a section into a paragraph"),
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
    apply_one(&mut doc, move_to(P1, 1, Some(LI2), None));
    assert_eq!(
        children(&doc, Some(LI2)),
        [P1],
        "a paragraph may go into an item"
    );
    assert_reads_back(&doc);
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
fn insert_accepts_any_html() {
    let mut doc = doc();
    let content = "<div class=\"card custom\" data-theme=\"dark\" onclick=\"toggle()\"><style>.card { color: red; }</style><p>A <span class=\"tag\">span</span>, <b>b</b>, <i>i</i></p><details><summary>More</summary><p>Hidden</p></details><svg viewBox=\"0 0 10 10\"><rect width=\"5\" height=\"5\"/></svg><script>if (a < b && b > c) { go(); }</script></div>";
    let result = apply_one(&mut doc, insert_into(S1, Some(P2), content));
    let html = doc.node_html(result.node).expect("the div prints");
    for kept in [
        "class=\"card custom\"",
        "onclick=\"toggle()\"",
        "<style>.card { color: red; }</style>",
        "<span class=\"tag\">span</span>",
        ">More</summary>",
        "viewBox=\"0 0 10 10\"",
        "<script>if (a < b && b > c) { go(); }</script>",
    ] {
        assert!(html.contains(kept), "the insert keeps `{kept}`: {html}");
    }
    assert!(
        result.created.len() >= 5,
        "the div, two paragraphs, the details and the svg are new nodes: {result:?}"
    );
    assert_eq!(children(&doc, Some(S1))[3], result.node.to_string());
    assert_reads_back(&doc);
    let (index, _) = invalid(&mut doc, json!([insert_into(S1, None, "")]));
    assert_eq!(index, 0, "empty content inserts nothing, so it is refused");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn replace_accepts_any_html() {
    let mut doc = doc();
    let content = "<figure><svg viewBox=\"0 0 10 10\" onload=\"draw()\"><circle cx=\"5\" cy=\"5\" r=\"3\" onclick=\"pick()\"/></svg><figcaption>Kept</figcaption></figure>";
    let result = apply_one(&mut doc, replace(FIGURE, 1, content));
    assert_eq!(result.node, id(FIGURE));
    let html = doc.node_html(id(FIGURE)).unwrap();
    assert!(
        html.contains("<circle")
            && html.contains("onclick=\"pick()\"")
            && html.contains("onload=\"draw()\""),
        "{html}"
    );
    assert_eq!(result.removed, [id(SVG)], "the old svg is replaced");
    assert_reads_back(&doc);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_operation_whose_result_would_not_read_back_is_refused() {
    let mut doc = doc();
    for (op, what) in [
        (
            insert_into(S1, None, "<tr><td>x</td></tr>"),
            "a table row into a section",
        ),
        (
            insert_into(S1, None, "<td>x</td>"),
            "a table cell into a section",
        ),
        (
            insert_into(P1, None, "<div>Inside</div>"),
            "a div into a paragraph",
        ),
        (
            insert_into(P1, None, "<p>Inside</p>"),
            "a paragraph into a paragraph",
        ),
        (
            replace(P1, 1, "<tr><td>x</td></tr>"),
            "replacing a paragraph with a row",
        ),
    ] {
        let (index, _message) = invalid(&mut doc, json!([op]));
        assert_eq!(index, 0, "{what}");
    }
    apply_one(&mut doc, insert_into(UL, Some(LI1), "<li>Fine</li>"));
    apply_one(&mut doc, insert_into(ROW, Some(CELL), "<td>Fine</td>"));
    apply_one(
        &mut doc,
        insert_into(S1, None, "<section><h2>Fine</h2></section>"),
    );
    apply_one(
        &mut doc,
        insert_into(LI1, None, "<div><p>Fine in an item</p></div>"),
    );
    apply_one(&mut doc, replace(LI1, 1, "<li>Still an item</li>"));
    assert_reads_back(&doc);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_doc_reads_back_after_every_batch() {
    let mut doc = doc();
    let batches = [
        json!([insert_into(
            S1,
            Some(P1),
            "<ul><li>a</li><li>b <em>c</em></li></ul>"
        )]),
        json!([set(H1, 1, json!({"html": "Title <code>x</code>"}))]),
        json!([move_to(P2, 1, Some(S2), None), delete(LI1, 1)]),
        json!([replace(
            NOTE,
            1,
            "<aside class=\"decision\"><p>Decided</p></aside>"
        )]),
        json!([insert_into(
            S2,
            None,
            "<div class=\"grid\"><style>.grid { display: grid; }</style><details><summary>x</summary><table><tbody><tr><td><p>cell</p></td></tr></tbody></table></details></div>"
        )]),
    ];
    for ops in batches {
        apply(&mut doc, ops);
        assert_reads_back(&doc);
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
        set(H1, 1, json!({"colour": "red"}))
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

// --- US-109: operations at any depth ---

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn elements_at_any_depth_are_nodes_with_their_nearest_ided_ancestor_as_parent() {
    let doc = deep();
    for node in [
        DS, CARD, MORE, INNER, DT, DR, DC1, DP1, DC2, DUL, DLI, DDIV, DP2,
    ] {
        assert_eq!(rev(&doc, node), 1, "{node} is a node at revision 1");
    }
    assert_eq!(doc.parent(id(DP1)), Some(Some(id(DC1))));
    assert_eq!(doc.parent(id(DC1)), Some(Some(id(DR))));
    assert_eq!(doc.parent(id(INNER)), Some(Some(id(MORE))));
    assert_eq!(doc.parent(id(DP2)), Some(Some(id(DDIV))));
    assert_eq!(children(&doc, Some(DR)), [DC1, DC2]);
    assert_eq!(children(&doc, Some(CARD)), [MORE]);
    assert!(
        doc.node_html(id(DP1))
            .unwrap()
            .contains("Deep <span class=\"x\">text</span>"),
        "{}",
        doc.node_html(id(DP1)).unwrap()
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn insert_replace_set_move_and_delete_work_at_any_depth() {
    let mut doc = deep();

    let inserted = apply_one(&mut doc, insert_into(DC1, Some(DP1), "<p>Deeper still</p>"));
    assert_eq!(
        children(&doc, Some(DC1)),
        [DP1, &inserted.node.to_string()],
        "an insert into a table cell lands after DP1"
    );
    let nested = apply_one(
        &mut doc,
        insert_into(DDIV, None, "<div class=\"inner\"><p>x</p></div>"),
    );
    assert_eq!(nested.created.len(), 2, "{nested:?}");
    assert_eq!(children(&doc, Some(DDIV))[0], nested.node.to_string());

    let replaced = apply_one(
        &mut doc,
        replace(DP2, 1, "<p>Rewritten <span class=\"y\">here</span></p>"),
    );
    assert_eq!(replaced.node, id(DP2));
    assert_eq!(rev(&doc, DP2), 2);
    assert!(
        doc.node_html(id(DP2))
            .unwrap()
            .contains("<span class=\"y\">here</span>")
    );

    let set_cell = apply_one(&mut doc, set(DC2, 1, json!({"html": "<b>Bold</b> cell"})));
    assert_eq!(set_cell.revision, 2);
    assert!(doc.node_html(id(DC2)).unwrap().contains("<b>Bold</b> cell"));

    let moved = apply_one(&mut doc, move_to(DP1, 1, Some(DDIV), Some(DP2)));
    assert_eq!(moved.revision, 2, "the moved node's revision rises");
    assert_eq!(children(&doc, Some(DDIV))[1..], [DP2, DP1]);
    assert_eq!(children(&doc, Some(DC1)), [inserted.node.to_string()]);
    for untouched in [DC1, DDIV, DS, CARD, MORE, DT, DR] {
        assert_eq!(rev(&doc, untouched), 1, "{untouched} keeps revision 1");
    }

    let deleted = apply_one(&mut doc, delete(DT, 1));
    assert_eq!(
        deleted.removed,
        [id(DT), id(DR), id(DC1), inserted.node, id(DC2)],
        "the table and everything in it, in document order"
    );
    assert_eq!(children(&doc, Some(INNER)), Vec::<String>::new());
    assert!(
        doc.contains(id(DP1)),
        "the moved paragraph is not in the table any more"
    );
    assert_reads_back(&doc);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn revisions_are_checked_at_any_depth() {
    let mut doc = deep();
    apply_one(&mut doc, set(DC2, 1, json!({"html": "Fresh"})));
    for (name, op) in [
        ("set", set(DC2, 1, json!({"html": "Stale"}))),
        ("replace", replace(DC2, 1, "<td>Stale</td>")),
        ("move", move_to(DC2, 1, Some(DR), None)),
        ("delete", delete(DC2, 1)),
    ] {
        let found = conflict(&mut doc, json!([op]));
        assert_eq!(found.node, id(DC2), "a stale {name}");
        assert_eq!(found.expected, 1);
        assert_eq!(found.current_revision, 2);
        assert!(
            found
                .current_html
                .contains(&format!("data-omni-id=\"{DC2}\""))
                && found.current_html.contains("Fresh"),
            "a stale {name} shows the cell as it is now: {}",
            found.current_html
        );
    }
    let found = conflict(
        &mut doc,
        json!([delete(DP1, 1), set(DP2, 3, json!({"html": "x"}))]),
    );
    assert_eq!(found.index, 1);
    assert!(doc.contains(id(DP1)), "the batch changed nothing");
}
