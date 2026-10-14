//! US-009: six layout rules that the US-003 checker found unprotected, each
//! pinned by the boxes that Chrome gives (`make chrome-boxes`), the boxes of
//! `design::cases`. Each case matched Chrome when it was written, so these
//! tests pass until a change of the layout code breaks the rule they protect.
//! `chrome_boxes.rs` checks that `make chrome-boxes` still prints the same
//! boxes for these designs, which keeps the expected values Chrome's.

mod design;

use design::{assert_cases, cases};

#[test]
fn a_wrapping_column_with_fill_height_in_a_grid_row_wraps_at_the_row_height() {
    assert_cases(vec![cases::wrapping_column_in_a_grid_row()]);
}

#[test]
fn a_positioned_wrapping_column_with_fill_height_wraps_at_the_boards_height() {
    assert_cases(vec![cases::positioned_wrapping_column()]);
}

#[test]
fn a_row_of_fixed_height_stretches_a_hug_row_and_the_wrapping_column_in_it() {
    assert_cases(vec![cases::column_stretched_by_a_fixed_height_row()]);
}

#[test]
fn a_fill_height_row_stretches_the_wrapping_column_in_it() {
    assert_cases(cases::column_stretched_by_a_fill_height_row());
}

#[test]
fn a_wrapping_column_with_height_max_height_and_padding_wraps_inside_its_padding() {
    assert_cases(vec![
        cases::wrapping_column_with_height_max_height_and_padding(),
    ]);
}

#[test]
fn a_wrapping_column_in_a_row_that_a_grid_row_does_not_fix_keeps_its_width() {
    assert_cases(vec![cases::row_in_a_grid_row_that_is_not_its_height()]);
}
