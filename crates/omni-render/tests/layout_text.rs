//! US-003: text nodes are measured with Skia's paragraph layout, through
//! Taffy's measure callback: their glyphs are shaped, their lines wrap at the
//! width layout gives them where Chrome breaks lines, and their size sets the
//! size of what hugs them. The expected boxes are the ones Chrome gives, as
//! `make chrome-boxes` prints them (`tests/design/cases.rs`).

mod common;
mod design;

use design::{
    SIX_WORDS, absolute, assert_box, assert_cases, at, board, cases, fixed, flex, lay_out, node,
    none, rect, text, with,
};
use omni_render::layout;
use omni_render::text::Fonts;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// A board wide enough that no text in it needs to wrap, which places its
/// children in a row from its top-left corner without stretching them.
fn wide_row(children: Vec<Value>) -> Value {
    board(
        "brd00001",
        fixed(2000.0),
        fixed(500.0),
        flex(json!({"alignItems": "start"})),
        children,
    )
}

/// A column `width` px wide that aligns its children to the start of its
/// cross axis, so CSS gives a text that hugs its width there its fit-content
/// width: as wide as its content on one line, but no wider than the column
/// unless its widest unbreakable piece is wider still.
fn start_aligned_column(width: f64, children: Vec<Value>) -> Value {
    board(
        "brd00001",
        fixed(width),
        json!("hug"),
        flex(json!({"direction": "column", "alignItems": "start"})),
        children,
    )
}

#[test]
fn text_is_shaped_so_kerning_pairs_move_closer() {
    // Inter kerns the pair AV: Chrome measures "AV" at 20.98 px in 16 px
    // Inter, where "A" and "V" are 11.04 px each, and "AVAVAVAV" at 80.66 px.
    assert_cases(cases::kerning_pairs());
}

#[test]
fn every_line_height_is_rounded_down_to_a_multiple_of_a_64th_of_a_pixel() {
    // Chrome lays out in 1/64 of a device pixel, so a line height that is a
    // multiple of 1/64 CSS px is the same at every integer pixel ratio. Layout
    // rounds every line height down to such a multiple, the normal one and
    // lineHeight times fontSize alike, and so does the CSS that `make
    // chrome-boxes` writes: 13 px Inter's normal line height of 15.7295 px is
    // 15.71875 px, where rounding it to the nearest 64th, or up, gives
    // 15.734375 px.
    assert_cases(cases::snapped_line_heights());
}

#[test]
fn a_line_breaks_when_it_is_a_tenth_of_a_pixel_too_narrow_for_its_text() {
    // "Hello world" is 85.41 px wide in 16 px Inter, in Chrome as in the
    // renderer. Chrome lets a line overflow by no more than its rounding
    // error, so 0.1 px too little breaks the text onto two lines, and 0.1 px
    // to spare keeps it on one. That error is one of its layout units, 1/128
    // px at a device pixel ratio of 2, after it rounds the width down to a
    // unit: "Hello Hello", 81.625 px, stays on one line in a text one unit
    // narrower, and breaks in a text a unit and a half narrower.
    assert_cases(cases::lines_that_are_a_tenth_too_narrow());
}

#[test]
fn labels_that_hug_their_text_inside_padded_frames_stay_on_one_line() {
    // A frame that hugs a text is as wide as the text plus its padding, and
    // the text gets its width back from that sum, which can come back a
    // rounding error short of the text's own width. Chrome keeps such labels
    // on one line: a button centered in a toolbar, and two chips in a row,
    // all with letter spacing.
    assert_cases(cases::labels_in_padded_frames());
}

#[test]
fn a_fill_text_wraps_at_the_width_its_parent_gives_it() {
    let filling_text = with(
        &text("txt00001", SIX_WORDS, 16.0, 1.5),
        "width",
        json!("fill"),
    );

    let case = "a text that fills the 100 px inside a padded 120 px column, which hugs its height";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(120.0),
        json!("hug"),
        flex(json!({
            "direction": "column",
            "padding": {"top": 10, "right": 10, "bottom": 10, "left": 10}
        })),
        vec![filling_text.clone()],
    ));
    assert_box(&boxes, "txt00001", [10.0, 10.0, 100.0, 72.0], case);
    assert_box(&boxes, "brd00001", [0.0, 0.0, 120.0, 92.0], case);

    let case = "a text that fills the 100 px a 50 px box leaves in a 150 px row";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(150.0),
        fixed(200.0),
        flex(json!({"alignItems": "start"})),
        vec![rect("a0000001", 50.0, 10.0), filling_text],
    ));
    assert_box(&boxes, "txt00001", [50.0, 0.0, 100.0, 72.0], case);
}

#[test]
fn a_hug_text_wraps_where_css_gives_it_a_fit_content_width() {
    let case = "a text that hugs its content in a 100 px column that aligns it to the start";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(100.0),
        json!("hug"),
        flex(json!({"direction": "column", "alignItems": "start"})),
        vec![text("txt00001", SIX_WORDS, 16.0, 1.5)],
    ));
    assert_box(&boxes, "txt00001", [0.0, 0.0, 100.0, 72.0], case);
    assert_box(&boxes, "brd00001", [0.0, 0.0, 100.0, 72.0], case);

    let case = "a text that hugs its content at x 100 in a 200 px board with no layout";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(200.0),
        fixed(200.0),
        none(),
        vec![at(&text("txt00001", SIX_WORDS, 16.0, 1.5), 100.0, 0.0)],
    ));
    assert_box(&boxes, "txt00001", [100.0, 0.0, 100.0, 72.0], case);
}

#[test]
fn a_word_wider_than_its_line_overflows_it_instead_of_breaking() {
    // CSS breaks no word (`overflow-wrap: normal`): a word wider than its line
    // stays whole on one line and overflows it. A text that hugs its width is
    // never narrower than its widest word, its min-content width: Chrome
    // measures "Supercalifragilistic" at 139.70 px. A word that a line break
    // puts at the start of a line overflows that line, and no empty line comes
    // before it: "Hi\nSupercalifragilistic" takes two lines at 60 px.
    assert_cases(cases::words_wider_than_their_line());
}

#[test]
fn words_after_a_word_wider_than_its_line_still_wrap_at_the_line_width() {
    // The overflowing word takes a line of its own and leaves the lines after
    // it as wide as the text. Chrome breaks this text into
    // "Supercalifragilistic", "a b c d" and "e f g h" at 60 px, and into
    // "Supercalifragilistic", "a b c", "d e f" and "g h" at 40 px. Hugging its
    // width in a 60 px column, the text is as wide as its widest word, where
    // the letters all fit on the second line.
    assert_cases(cases::words_after_a_word_wider_than_their_line());
}

#[test]
fn lines_also_break_after_hyphens_and_around_em_dashes_and_after_ellipses() {
    // Besides spaces, Chrome breaks lines where Unicode's line breaking rules
    // allow it inside a word: after a hyphen, before and after an em dash, and
    // after an ellipsis. It breaks "state-of-the-art" into "state-of-" and
    // "the-art" at 80 px, "state-", "of-the-" and "art" at 60 px, and "state-"
    // (overflowing), "of-", "the-" and "art" at 40 px; "one—two—three" into
    // "one—", "two—" and "three" at 60 px and into "one", "—", "two", "—" and
    // "three" at 40 px; and "hello…world" into "hello…" and "world" at 60 px.
    // So where CSS makes the width of a text that hugs it fit-content, the
    // text is no wider than the space, unless its widest piece, its
    // min-content width, is wider still: "state-" (44.51 px), "three" (39.11
    // px) or "hello…" (49.96 px).
    assert_cases(cases::breaks_after_hyphens_dashes_and_ellipses());
}

#[test]
fn a_line_breaks_after_a_hyphen_before_a_digit_only_when_a_letter_or_digit_comes_first() {
    // Chrome breaks between a hyphen and a digit when a letter or a digit
    // comes before the hyphen: "pre-" / "2020" and "COVID-" / "19". Anywhere
    // else the hyphen may be a minus sign, which stays with its number, so
    // "-2020", "(-2020)" and the "-12" of "x -12" stay whole.
    assert_cases(cases::hyphens_before_digits());
}

#[test]
fn a_hyphen_or_dash_that_starts_a_word_stays_with_the_word() {
    // Unicode's line breaking rules keep a hyphen, or an en dash, that starts
    // a word with the letter after it (rule LB20a of Unicode 15.1, which
    // Chrome follows), so these break only at their space: "word " / "–word".
    // A text that hugs its width in a narrow column is then as wide as
    // "–word", 45.95 px. A character of ambiguous width (UAX #14 class AI),
    // such as "§", "×" or "½", counts as a letter there, so "a -§x" takes two
    // lines at 10 px, "a " and "-§x", not three.
    assert_cases(cases::hyphens_and_dashes_that_start_a_word());
}

#[test]
fn a_no_break_space_joins_its_neighbors_except_after_a_hyphen() {
    // A no-break space (U+00A0) keeps the characters on both sides of it
    // together: "10 km" and "Mr. Smith" written with one never break. After
    // a hyphen, though, Unicode's line breaking rules allow a break before
    // it, and Chrome breaks there: "well-" / " known". So a text that hugs its
    // width in a narrow column is as wide as " known", 54.59 px.
    assert_cases(cases::no_break_spaces());
}

#[test]
fn a_tab_moves_its_line_on_to_the_next_tab_stop() {
    // CSS `tab-size: 8`: a tab moves its line on to the next stop, every
    // eight spaces from the start of the line (36 px in 16 px Inter). Letter
    // spacing widens the spaces that stops are counted in, so with 1 px of it
    // they are 44 px apart. A tab at the end of a line still counts toward
    // the text's width, and so do the spaces after a tab, from the stop it
    // moves to: "a\t b" is 4.5 px wider than "a\tb". A line also breaks before
    // the piece that the tabs before it, and the spaces after them, have moved
    // past the line's end, as "b" in "a\t    b" at 50 and 60 px.
    assert_cases(cases::tab_stops());
}

#[test]
fn a_max_width_makes_a_hug_text_wrap() {
    let json = with(
        &text("txt00001", SIX_WORDS, 16.0, 1.5),
        "maxWidth",
        json!(100),
    );
    let boxes = lay_out(&wide_row(vec![json]));
    assert_box(
        &boxes,
        "txt00001",
        [0.0, 0.0, 100.0, 72.0],
        "six words in a text that hugs its content with maxWidth 100",
    );
}

#[test]
fn a_max_width_narrower_than_the_widest_word_still_bounds_a_hug_text() {
    // A text that hugs its width is never wider than its maxWidth, even when
    // its widest word is wider, and its lines break at that width: with
    // maxWidth 50, "Supercalifragilistic word word" is 50 px wide, and each
    // word takes a line of its own, the long one overflowing it. Chrome lays
    // the text out so wherever CSS makes its width fit-content.
    let bounded = || {
        with(
            &text("txt00001", "Supercalifragilistic word word", 16.0, 1.5),
            "maxWidth",
            json!(50),
        )
    };
    for (case, x, y, design) in [
        (
            "in a 400 px column that aligns it to the start",
            0.0,
            0.0,
            start_aligned_column(400.0, vec![bounded()]),
        ),
        (
            "at x 20, y 10 in a board with no layout",
            20.0,
            10.0,
            board(
                "brd00001",
                fixed(400.0),
                fixed(300.0),
                none(),
                vec![at(&bounded(), 20.0, 10.0)],
            ),
        ),
        (
            "absolute at x 20, y 10 in a row",
            20.0,
            10.0,
            board(
                "brd00001",
                fixed(400.0),
                fixed(300.0),
                flex(json!({})),
                vec![absolute(&bounded(), 20.0, 10.0)],
            ),
        ),
    ] {
        let boxes = lay_out(&design);
        assert_box(
            &boxes,
            "txt00001",
            [x, y, 50.0, 72.0],
            &format!("a text of 16 px Inter with maxWidth 50 {case}, as three lines of 24 px"),
        );
    }
}

#[test]
fn a_line_break_starts_a_new_line_but_not_at_the_very_end_and_an_empty_text_has_no_line() {
    // A line break ends its line, but in CSS it starts a new line only when
    // something follows it, so a final line break adds no empty line and
    // nothing to the width: "Hello\n" is one line, as wide as "Hello". One
    // with nothing before it still ends an empty line, and an empty text has
    // no line at all, so its box is empty.
    assert_cases(cases::line_breaks_in_the_content());
}

#[test]
fn letter_spacing_adds_space_after_every_character() {
    // CSS `letter-spacing` adds its spacing after each of the 5 characters of
    // "Hello", the last one included: 2 px of it make the text 10 px wider.
    assert_cases(cases::letter_spacing_after_every_character());
}

#[test]
fn letter_spacing_turns_off_contextual_alternates() {
    // Inter's contextual alternates (OpenType `calt`) change glyphs by their
    // neighbors: "->" becomes one arrow, and a colon between capitals takes
    // the form made for capitals. Chrome turns them off in text with letter
    // spacing, as CSS allows for optional ligatures, so the hyphen and the
    // greater-than sign keep their own glyphs, each followed by the spacing:
    // "C:VBQ" in 16 px Inter with 2 px of letter spacing is 59.04 px wide.
    // Any spacing turns them off, however small, and without letter spacing
    // they stay on.
    assert_cases(cases::letter_spacing_without_contextual_alternates());
}

#[test]
fn font_families_that_are_missing_measure_as_the_bundled_inter() {
    // The bundled Inter stands in for every missing family, so text measures
    // the same on every machine.
    assert_cases(cases::missing_families());
}

#[test]
fn a_text_in_a_grid_column_wraps_at_the_column_width() {
    // A text that fills a 100 px column wraps there, and an auto column is as
    // wide as the text that hugs its content in it.
    assert_cases(cases::texts_in_grid_columns());
}

#[test]
fn a_frame_that_hugs_a_text_is_the_size_of_the_text_and_its_padding() {
    assert_cases(cases::frames_that_hug_a_text());
}

#[test]
fn the_content_of_a_text_is_text_and_never_markup() {
    // "a&b <i>c</i> d" is fourteen characters, all of them drawn, and so is
    // every character of "x &lt; y &amp;&amp; z > 0".
    assert_cases(cases::markup_characters_in_text());
}

#[test]
fn omni_render_measures_text_with_skias_paragraph_layout() {
    let output = common::cargo(&["metadata", "--no-deps", "--format-version", "1"]);
    assert!(
        output.status.success(),
        "`cargo metadata` failed at the repository root:\n{}",
        common::describe(&output)
    );
    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata prints JSON");
    let omni_render = metadata["packages"]
        .as_array()
        .expect("cargo metadata lists packages")
        .iter()
        .find(|package| package["name"] == "omni-render")
        .expect("omni-render is a workspace member");
    let skia = omni_render["dependencies"]
        .as_array()
        .expect("every package lists its dependencies")
        .iter()
        .find(|dependency| dependency["name"] == "skia-safe" && dependency["kind"].is_null())
        .expect("omni-render depends on skia-safe");
    let features: Vec<&str> = skia["features"]
        .as_array()
        .expect("cargo metadata lists a dependency's features")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        features.contains(&"textlayout"),
        "omni-render shapes and wraps text with Skia's paragraph layout, which skia-safe's `textlayout` feature provides; it enables {features:?}"
    );
}

#[test]
fn lines_break_where_chrome_breaks_urls_and_hyphenated_words() {
    // "state-of-the-art" in a text 60 px wide takes three lines, and so does
    // "See https://example.com/a/very/long/path/name for more" in a text 150
    // px wide: "See", the URL, which never breaks, and "for more".
    assert_cases(cases::line_breaks());
}

/// For each printable ASCII character that a line may break after, the
/// printable ASCII characters that it may break before, as Chrome breaks
/// between two ASCII characters (Blink's table, measured in Chrome 155):
///
/// - After a hyphen a line breaks before most characters, but not before a
///   digit, so that a minus sign stays with its number, nor before closing
///   punctuation or a slash.
/// - After a question mark it breaks before most characters, a digit included,
///   but not before closing punctuation, a quote or a slash.
/// - After closing punctuation and most symbols it breaks only before an
///   opening bracket.
/// - After any other character it never breaks, so that paths, URLs and
///   abbreviations stay whole.
const BREAKS_BEFORE: [(&str, &str); 21] = [
    ("!", "(<[{"),
    ("\"", "(<[{"),
    ("#", "(<[{"),
    ("%", "(<[{"),
    ("&", "(<[{"),
    (")", "(<[{"),
    ("*", "(<[{"),
    ("+", "(<[{"),
    (",", "(<[{"),
    (
        "-",
        "\"#%&'(*+-<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\^_`abcdefghijklmnopqrstuvwxyz{|~",
    ),
    (".", "(<[{"),
    (":", "(<[{"),
    (";", "(<[{"),
    ("=", "(<[{"),
    (">", "(<[{"),
    (
        "?",
        "#$%&(*+-0123456789<=>@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\^_`abcdefghijklmnopqrstuvwxyz{|~",
    ),
    ("\\", "(<[{"),
    ("]", "(<[{"),
    ("|", "(<[{"),
    ("}", "(<[{"),
    ("~", "(<[{"),
];

#[test]
fn a_line_breaks_between_two_ascii_characters_exactly_where_chrome_breaks_it() {
    // Every pair of printable ASCII characters, written without a space, in a
    // text that is 1 px wide: it takes two lines where a line may break
    // between the two characters and one line where it may not.
    let fonts = Fonts::bundled();
    let mut wrong = BTreeSet::new();
    for first in '!'..='~' {
        let breaks_before = BREAKS_BEFORE
            .iter()
            .find(|(character, _)| character.starts_with(first))
            .map_or("", |(_, seconds)| *seconds);
        let pairs: Vec<(char, Value)> = ('!'..='~')
            .enumerate()
            .map(|(index, second)| {
                let content = format!("{first}{second}");
                let id = format!("t{index:07}");
                let pair = with(&text(&id, &content, 16.0, 1.5), "width", fixed(1.0));
                (second, pair)
            })
            .collect();
        let design = board(
            "brd00001",
            fixed(100.0),
            json!("hug"),
            flex(json!({"direction": "column", "alignItems": "start"})),
            pairs.iter().map(|(_, pair)| pair.clone()).collect(),
        );
        let boxes = layout::compute(&node(&design), &fonts);
        for (index, (second, _)) in pairs.iter().enumerate() {
            let id = format!("t{index:07}").parse().expect("a valid node ID");
            let lines = (boxes[&id].height / 24.0).round();
            let expected = if breaks_before.contains(*second) {
                2.0
            } else {
                1.0
            };
            if lines != expected {
                wrong.insert(format!("{first}{second} ({lines} lines, not {expected})"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "Chrome breaks a line between two ASCII characters only where its table says, but layout breaks these pairs differently: {wrong:?}"
    );
}

#[test]
fn a_tab_moves_its_line_on_to_the_stop_after_the_next_when_less_than_half_a_space_is_left() {
    // "Date" is 34.94 px wide, which leaves 1.06 px, less than half of the
    // 4.5 px of a space, to the first tab stop at 36 px, so a tab after it goes
    // on to the stop at 72 px; "Item" is 32.95 px and its tab stops at 36 px.
    assert_cases(cases::tabs());
    // "mZ9" is 34 px wide and leaves 2 px, which is less than half a space, but
    // "xsGI" is 33.41 px wide and leaves 2.59 px. A letter spacing that cancels
    // the width of a space leaves no tab stops at all. Half a space is half of
    // the space alone, though letter spacing widens the spaces that stops are
    // counted in: "Time" with 1 px of letter spacing is 41.55 px wide and
    // leaves 2.45 px to the stop at 44 px, more than half of the 4.5 px space
    // but less than half of the space and its letter spacing, and its tab
    // stops there.
    assert_cases(cases::tabs_near_a_stop());
}

#[test]
fn text_style_leaves_boxes_alone() {
    assert_cases(cases::text_style_leaves_boxes_alone());
}

#[test]
fn the_spaces_at_the_end_of_a_text_count_in_its_width_but_not_in_its_narrowest_piece() {
    assert_cases(cases::spaces_in_text());
}

#[test]
fn every_weight_style_and_missing_family_measures_as_inter_regular() {
    assert_cases(cases::weights_styles_and_families_measure_as_inter_regular());
}

#[test]
fn a_min_width_above_a_max_width_is_the_width_a_text_breaks_its_lines_at() {
    // A min beats a max, as in CSS, also before a fit-content width breaks the
    // lines of a text: with a minWidth of 80 px and a maxWidth of 60 px,
    // "alpha beta gamma delta" is 80 px wide and takes the three lines it
    // breaks into at 80 px, not the four it would take at 60 px, in a column
    // that aligns it to its start, in a grid area and where it is positioned.
    assert_cases(cases::texts_whose_min_width_beats_their_max_width());
}
