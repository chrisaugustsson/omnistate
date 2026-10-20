//! Revisions in printed tags (US-030): `print_with` writes `rev={n}` after the
//! `id` of every tag, `print` writes none, and a tag that sets `rev` is not
//! part of the format.

use omni_core::design::tags::{self, Revision};
use omni_core::design::{Node, NodeId};

const TAGS: &str = r#"<Board id="brd00001" name="Cart" x={0} y={0} width={360} height={640} dir="col">
  <Text id="txt00001" name="Total" content="Summa 12 kr" fontFamily="Inter" fontSize={16}/>
</Board>"#;

fn board() -> Node {
    let root = tags::parse(TAGS).expect("the tags parse");
    root.assign_ids(|| unreachable!("every node has an ID"))
}

#[test]
fn print_writes_no_rev() {
    assert_eq!(tags::print(&board()), TAGS);
}

#[test]
fn print_with_writes_the_rev_after_the_id_of_every_tag() {
    let revision = |id: &NodeId| match id.to_string().as_str() {
        "brd00001" => Some(3),
        "txt00001" => Some(2),
        _ => None,
    };
    let revision: Revision = &revision;
    let printed = tags::print_with(&board(), Some(revision));
    assert_eq!(
        printed,
        TAGS.replace(r#"id="brd00001""#, r#"id="brd00001" rev={3}"#)
            .replace(r#"id="txt00001""#, r#"id="txt00001" rev={2}"#)
    );
}

#[test]
fn a_node_without_a_revision_is_printed_without_one() {
    let revision = |id: &NodeId| (id.to_string() == "brd00001").then_some(1);
    let revision: Revision = &revision;
    let printed = tags::print_with(&board(), Some(revision));
    assert!(
        printed.contains(r#"<Board id="brd00001" rev={1} "#),
        "{printed}"
    );
    assert!(
        printed.contains(r#"<Text id="txt00001" name="Total""#),
        "{printed}"
    );
}

#[test]
fn a_tag_that_sets_rev_is_refused() {
    let tags_with_rev = TAGS.replace(r#"id="brd00001""#, r#"id="brd00001" rev={3}"#);
    assert!(tags::parse(&tags_with_rev).is_err());
}
