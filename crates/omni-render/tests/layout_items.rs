//! US-053: the layout features that Penpot designs use (decision Q48). Every
//! node may have a margin on each side, which is CSS `margin`; a child of a
//! grid may align itself in its area with `justifySelf` and `alignSelf`, which
//! are CSS `justify-self` and `align-self`; and strokes, dashed or not and
//! wherever they sit on the edge, take no space. Every expected box comes from
//! `make chrome-boxes` (`design::cases`), and `chrome_boxes.rs` checks that the
//! tool still prints them.

mod design;

use design::{assert_cases, cases};

#[test]
fn margins_take_space_around_nodes_and_move_positioned_nodes_as_css_margins_do() {
    assert_cases(cases::margins());
}

#[test]
fn grid_items_align_themselves_in_their_areas_as_css_justify_self_and_align_self_do() {
    assert_cases(cases::grid_self_alignment());
}
