//! US-003: grid layouts place and size their children by CSS grid: fixed,
//! fractional and auto tracks, gap and padding, placement by start line and
//! span, automatic placement, and `fixed`, `hug` and `fill` sizing inside a
//! grid area.

mod design;

use design::{assert_cases, cases};

#[test]
fn an_absolute_child_ignores_its_grid_lines_and_a_placement_without_a_start_or_span_is_automatic() {
    // An absolute child is placed by its `x` and `y` only, so it is not placed
    // in the grid area its `gridColumn` and `gridRow` name; and an empty
    // placement, `{}`, is `auto / auto` like one that is left out. A frame in
    // the second column of a grid places a child at x 10 from its own left
    // edge, at x 110 on the page.
    assert_cases(cases::absolute_children_and_empty_grid_lines());
}

#[test]
fn an_empty_grid_keeps_the_tracks_it_declares() {
    // A grid without children still has its `grid-template-columns` and
    // `grid-template-rows`: one that hugs its content is as large as its fixed
    // tracks, its gaps and its padding. Columns of 50 and 30 px make it 80 px
    // wide.
    assert_cases(cases::empty_grids());
}

#[test]
fn a_texts_max_height_bounds_the_grid_row_that_holds_it() {
    // A grid row is as tall as its items, and a text's maxHeight bounds the
    // text there as everywhere else: "alpha beta gamma" needs three lines, 72
    // px, at 50 px wide, but with a maxHeight of 30 px the text, its row and a
    // grid that hugs them are 30 px tall, whether the text is 50 px wide, hugs
    // its width or fills a column of 50 px. A taller item of the row, a 20 x 40
    // box, makes it 40 px tall, and a box in the next row starts at y 30.
    assert_cases(cases::max_heights_of_texts_in_grid_rows());
}
