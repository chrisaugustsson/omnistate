//! US-026: the three-way merge behind a rebase, `EditDoc::rebase(base,
//! mainline, changes)`. `base` is where the change set started, `changes` its
//! latest version and `mainline` the mainline version now; the result holds
//! the doc with the change set's changes replayed onto the mainline and the
//! `conflicts`, the nodes that both sides changed.
//!
//! These tests pin what the API tests (crates/omni-api/tests/rebase.rs) check
//! end to end, for nested structure and concurrent reorders: only nodes
//! changed on both sides conflict, and neither side's move is silently lost.

use omni_core::design::NodeId;
use omni_core::ops::EditDoc;

fn doc(html: &str) -> EditDoc {
    EditDoc::from_html(html).expect("the doc parses")
}

fn id(text: &str) -> NodeId {
    text.parse().expect("a node ID")
}

fn p(id: &str, text: &str) -> String {
    format!("<p data-node-id=\"{id}\">{text}</p>")
}

fn section(id: &str, children: &str) -> String {
    format!("<section data-node-id=\"{id}\">{children}</section>")
}

/// The node IDs of a doc in document order.
fn order(doc: &EditDoc) -> Vec<String> {
    doc.index().iter().map(|n| n.id.to_string()).collect()
}

/// The base of the nested cases: an intro, then a section of two paragraphs.
fn nested_base() -> EditDoc {
    doc(&format!(
        "{}{}",
        p("a0000000", "Intro"),
        section(
            "s0000000",
            &format!("{}{}", p("b0000000", "One"), p("c0000000", "Two"))
        )
    ))
}

/// The mainline edits only the intro.
fn nested_mainline() -> EditDoc {
    doc(&format!(
        "{}{}",
        p("a0000000", "Intro by mainline"),
        section(
            "s0000000",
            &format!("{}{}", p("b0000000", "One"), p("c0000000", "Two"))
        )
    ))
}

#[test]
fn a_section_removed_after_its_children_moved_out_is_no_conflict() {
    // The change set moves both paragraphs to the top level, then removes the
    // empty section.
    let changes = doc(&format!(
        "{}{}{}",
        p("a0000000", "Intro"),
        p("b0000000", "One"),
        p("c0000000", "Two")
    ));
    let rebased = EditDoc::rebase(&nested_base(), &nested_mainline(), &changes);
    assert_eq!(
        rebased.conflicts,
        Vec::<NodeId>::new(),
        "nothing changed on both sides"
    );
    assert_eq!(order(&rebased.doc), ["a0000000", "b0000000", "c0000000"]);
    for node in ["a0000000", "b0000000", "c0000000"] {
        assert_eq!(rebased.doc.parent(id(node)), Some(None), "{node}");
    }
    assert!(!rebased.doc.contains(id("s0000000")), "the section is gone");
    assert!(rebased.doc.to_html().contains("Intro by mainline"));
}

#[test]
fn a_section_removed_after_its_children_were_wrapped_in_a_new_one_is_no_conflict() {
    // The change set puts both paragraphs in a new section and removes the
    // old one.
    let changes = doc(&format!(
        "{}{}",
        p("a0000000", "Intro"),
        section(
            "t0000000",
            &format!("{}{}", p("b0000000", "One"), p("c0000000", "Two"))
        )
    ));
    let rebased = EditDoc::rebase(&nested_base(), &nested_mainline(), &changes);
    assert_eq!(
        rebased.conflicts,
        Vec::<NodeId>::new(),
        "nothing changed on both sides"
    );
    assert_eq!(
        order(&rebased.doc),
        ["a0000000", "t0000000", "b0000000", "c0000000"]
    );
    for node in ["b0000000", "c0000000"] {
        assert_eq!(
            rebased.doc.parent(id(node)),
            Some(Some(id("t0000000"))),
            "{node}"
        );
    }
    assert!(
        !rebased.doc.contains(id("s0000000")),
        "the old section is gone"
    );
    assert!(rebased.doc.to_html().contains("Intro by mainline"));
}

#[test]
fn a_reorder_on_each_side_keeps_both_or_conflicts() {
    let paragraphs = |ids: &[&str]| {
        doc(&ids
            .iter()
            .map(|id| p(id, &id[..1].to_uppercase()))
            .collect::<String>())
    };
    let base = paragraphs(&["a0000000", "b0000000", "c0000000", "d0000000"]);
    // The mainline moves D to the front; the change set moves A to the end.
    let mainline = paragraphs(&["d0000000", "a0000000", "b0000000", "c0000000"]);
    let changes = paragraphs(&["b0000000", "c0000000", "d0000000", "a0000000"]);
    let rebased = EditDoc::rebase(&base, &mainline, &changes);
    if rebased.conflicts.is_empty() {
        assert_eq!(
            order(&rebased.doc),
            ["d0000000", "b0000000", "c0000000", "a0000000"],
            "a clean rebase keeps the mainline's move and the change set's move"
        );
    } else {
        assert!(
            rebased
                .conflicts
                .iter()
                .all(|node| [id("a0000000"), id("d0000000")].contains(node)),
            "only the moved nodes conflict: {:?}",
            rebased.conflicts
        );
        assert_eq!(
            order(&rebased.doc).first().map(String::as_str),
            Some("d0000000"),
            "the mainline's move stays"
        );
    }
}
