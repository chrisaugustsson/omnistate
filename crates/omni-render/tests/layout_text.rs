//! US-003: text nodes are measured with Skia's paragraph layout, through
//! Taffy's measure callback: their glyphs are shaped, their lines wrap at the
//! width layout gives them where Chrome breaks lines, and their size sets the
//! size of what hugs them.

mod common;
mod design;

use design::{
    SIX_WORDS, absolute, assert_box, assert_box_within, assert_cases, at, board, box_of, cases,
    fixed, flex, frame, grid, lay_out, node, none, rect, sized, text, with, without,
};
use omni_render::layout;
use omni_render::text::Fonts;
use serde_json::{Value, json};
use skia_safe::{Data, Font, FontMgr};
use std::collections::BTreeSet;

/// The bundled Inter, which the tests measure their expectations with.
const INTER: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");

/// How far a measured text width may be from the width of its glyphs.
const TEXT_TOLERANCE: f32 = 0.05;

/// The bundled Inter at `size` px.
fn inter(size: f32) -> Font {
    let typeface = FontMgr::custom_empty()
        .unwrap_or_default()
        .new_from_data(Data::new_copy(INTER), None)
        .expect("Skia loads the bundled Inter");
    let mut font = Font::from_typeface(typeface, size);
    font.set_subpixel(true);
    font.set_linear_metrics(true);
    font
}

/// The width of `content` in the bundled Inter at `size` px: the sum of its
/// glyphs' advances, unshaped, so without kerning.
fn advance_width(content: &str, size: f32) -> f32 {
    inter(size).measure_str(content, None).0
}

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

/// The box of the one text of a wide row.
fn hug_text_box(text_json: Value) -> omni_render::layout::LayoutBox {
    let id = text_json["id"]
        .as_str()
        .expect("text fixtures have an id")
        .to_string();
    box_of(&lay_out(&wide_row(vec![text_json])), &id)
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

/// Lays out a text of `content` in 16 px Inter with lineHeight 1.5 (24 px
/// lines), `width` px wide, in a wide row, and asserts that it breaks into
/// `lines` lines.
#[track_caller]
fn assert_lines_at_fixed_width(content: &str, width: f64, lines: f32) {
    let json = with(&text("txt00001", content, 16.0, 1.5), "width", fixed(width));
    let boxes = lay_out(&wide_row(vec![json]));
    assert_box(
        &boxes,
        "txt00001",
        [0.0, 0.0, width as f32, 24.0 * lines],
        &format!("{content:?} in a {width} px wide text of 16 px Inter, as {lines} lines of 24 px"),
    );
}

/// Lays out a text of `content` in 16 px Inter with lineHeight 1.5 that hugs
/// its width in a start-aligned column `column` px wide, and asserts that it
/// is `width` px wide (give or take [`TEXT_TOLERANCE`]) and breaks into
/// `lines` lines of 24 px.
#[track_caller]
fn assert_hug_text_in_column(content: &str, column: f64, width: f32, lines: f32) {
    let boxes = lay_out(&start_aligned_column(
        column,
        vec![text("txt00001", content, 16.0, 1.5)],
    ));
    assert_box_within(
        &boxes,
        "txt00001",
        [0.0, 0.0, width, 24.0 * lines],
        TEXT_TOLERANCE,
        &format!(
            "{content:?} in 16 px Inter hugging its width in a {column} px column that aligns it to the start, as {lines} lines of 24 px"
        ),
    );
}

#[test]
fn a_hug_text_is_as_wide_as_its_glyphs_and_as_tall_as_its_line() {
    // A line is lineHeight x fontSize tall, fractions included, as in CSS:
    // 28.8 px for 24 px text with lineHeight 1.2, not a rounded 29.
    for (content, size, line_height) in [
        ("Hello", 16.0, 1.5),
        ("Hello", 32.0, 1.25),
        ("minimum", 16.0, 1.0),
        ("Hi", 13.0, 2.0),
        ("Hello", 24.0, 1.2),
        ("Hi", 14.0, 1.4),
    ] {
        // These words have no kerning pairs in Inter, so shaping leaves their
        // advances as they are.
        let expected_width = advance_width(content, size as f32);
        let expected_height = (size * line_height) as f32;
        let boxes = lay_out(&wide_row(vec![text(
            "txt00001",
            content,
            size,
            line_height,
        )]));
        assert_box_within(
            &boxes,
            "txt00001",
            [0.0, 0.0, expected_width, expected_height],
            TEXT_TOLERANCE,
            &format!("{content:?} in {size} px Inter with lineHeight {line_height}"),
        );
    }
}

#[test]
fn text_is_shaped_so_kerning_pairs_move_closer() {
    let unshaped = advance_width("AVAVAVAV", 16.0);
    let shaped = hug_text_box(text("txt00001", "AVAVAVAV", 16.0, 1.5)).width;
    assert!(
        shaped < unshaped - 4.0 && shaped > unshaped * 0.8,
        "Inter kerns the pair AV, so shaped \"AVAVAVAV\" at 16 px is several pixels narrower than its {unshaped} px of unshaped advances; layout measured {shaped} px"
    );
}

#[test]
fn a_text_without_a_line_height_gets_its_fonts_normal_line_height_at_full_precision() {
    // Decision Q44: a text without lineHeight is as tall per line as its
    // font's normal line height, ascent + descent + line gap, never rounded to
    // whole pixels. Inter's ascent is 15.5 px at 16 px and its descent 3.859375
    // px, and it has no line gap, so a line is 19.359375 px tall. That is not
    // what Chrome's own `line-height: normal` gives (19.5 px at a device pixel
    // ratio of 2, 20 px at 1), which is why the export writes the line height
    // in px. The font sizes are those where rounding down and rounding to the
    // nearest 1/64 px agree.
    assert_cases(cases::normal_line_heights());
}

/// The normal line height of Inter at `size` px at full precision: its ascent
/// of 1984, descent of 494 and line gap of 0, in units of 1/2048 em.
fn normal_line_height(size: f32) -> f32 {
    size * (1984.0 + 494.0) / 2048.0
}

#[test]
fn every_line_height_is_snapped_to_a_multiple_of_a_64th_of_a_pixel() {
    // Chrome lays out in 1/64 of a device pixel, so a line height that is a
    // multiple of 1/64 CSS px is the same at every integer pixel ratio. Layout
    // snaps every line height, the normal one and lineHeight times fontSize,
    // to such a multiple: within 1/64 px of the exact height, and nothing
    // coarser than 1/64 px or finer. These sizes have exact heights between
    // two multiples.
    let snapped = |height: f32, exact: f32, case: &str| {
        let units = height * 64.0;
        assert!(
            (units - units.round()).abs() <= 0.001 && (height - exact).abs() < 1.0 / 64.0,
            "{case} is {exact} px tall exactly, so a line height snapped to a multiple of 1/64 px is within 1/64 px of it; layout gave {height} px"
        );
    };
    for size in [11.0, 13.0, 15.0, 18.0, 20.0, 24.0] {
        let height =
            hug_text_box(without(&text("txt00001", "Hello", size, 1.5), "lineHeight")).height;
        snapped(
            height,
            normal_line_height(size as f32),
            &format!("a line of {size} px Inter without a lineHeight"),
        );
    }
    for (size, multiple) in [(11.0, 1.33), (13.0, 1.4), (17.0, 1.35), (15.0, 1.1)] {
        let height = hug_text_box(text("txt00001", "Hello", size, multiple)).height;
        snapped(
            height,
            (size * multiple) as f32,
            &format!("a line of {size} px Inter with lineHeight {multiple}"),
        );
    }
}

#[test]
fn a_fixed_width_text_wraps_its_lines_at_that_width() {
    for (width, line_height, lines) in [
        (70.0, 1.5, 6.0),
        (100.0, 1.5, 3.0),
        (130.0, 1.5, 2.0),
        (300.0, 1.5, 1.0),
        (100.0, 1.3, 3.0),
    ] {
        let json = with(
            &text("txt00001", SIX_WORDS, 16.0, line_height),
            "width",
            fixed(width),
        );
        let boxes = lay_out(&wide_row(vec![json]));
        let line = (16.0 * line_height) as f32;
        assert_box_within(
            &boxes,
            "txt00001",
            [0.0, 0.0, width as f32, line * lines],
            TEXT_TOLERANCE,
            &format!(
                "six words of 16 px Inter in a {width} px wide text, as {lines} lines of {line} px"
            ),
        );
    }
}

#[test]
fn a_line_breaks_when_it_is_a_tenth_of_a_pixel_too_narrow_for_its_text() {
    // "Hello world" is 85.41 px wide in 16 px Inter, in Chrome as in the
    // renderer. Chrome lets a line overflow by no more than its rounding
    // error, so 0.1 px too little breaks the text onto two lines, and 0.1 px
    // to spare keeps it on one.
    assert_lines_at_fixed_width("Hello world", 85.3, 2.0);
    assert_lines_at_fixed_width("Hello world", 85.5, 1.0);
}

#[test]
fn labels_that_hug_their_text_inside_padded_frames_stay_on_one_line() {
    // A frame that hugs a text is as wide as the text plus its padding, and
    // the text gets its width back from that sum, which can come back a
    // rounding error short of the text's own width. Chrome keeps such labels
    // on one line: a button centered in a toolbar, and two chips in a row,
    // all with letter spacing. These are Chrome's boxes.
    let case = "a button whose 18 px label has letterSpacing 0.2, centered in an 800 px row";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(800.0),
        json!("hug"),
        flex(json!({"alignItems": "start", "justifyContent": "center"})),
        vec![frame(
            "frm00001",
            json!("hug"),
            json!("hug"),
            flex(json!({"padding": {"left": 12, "right": 12}})),
            vec![with(
                &text("txt00001", "Save changes", 18.0, 1.5),
                "letterSpacing",
                json!(0.2),
            )],
        )],
    ));
    assert_box_within(
        &boxes,
        "frm00001",
        [327.086, 0.0, 145.82, 27.0],
        TEXT_TOLERANCE,
        case,
    );
    assert_box_within(
        &boxes,
        "txt00001",
        [339.086, 0.0, 121.82, 27.0],
        TEXT_TOLERANCE,
        case,
    );

    let case =
        "two chips whose 14 px labels have letterSpacing 0.1, 4 px apart in a row that hugs them";
    let chip = |id: &str, label_id: &str, label: &str| {
        frame(
            id,
            json!("hug"),
            json!("hug"),
            flex(json!({"padding": {"top": 4, "right": 12, "bottom": 4, "left": 12}})),
            vec![with(
                &text(label_id, label, 14.0, 1.2),
                "letterSpacing",
                json!(0.1),
            )],
        )
    };
    let boxes = lay_out(&board(
        "brd00001",
        json!("hug"),
        json!("hug"),
        flex(json!({"gap": {"column": 4}})),
        vec![
            chip("c0000001", "t0000001", "Sign in"),
            chip("c0000002", "t0000002", "Overview"),
        ],
    ));
    for (id, expected) in [
        ("brd00001", [0.0, 0.0, 160.914, 24.8]),
        ("c0000001", [0.0, 0.0, 69.531, 24.8]),
        ("t0000001", [12.0, 4.0, 45.531, 16.8]),
        ("c0000002", [73.531, 0.0, 87.383, 24.8]),
        ("t0000002", [85.531, 4.0, 63.383, 16.8]),
    ] {
        assert_box_within(&boxes, id, expected, TEXT_TOLERANCE, case);
    }
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
    // stays whole on one line and overflows it.
    for width in [20.0, 60.0, 100.0] {
        assert_lines_at_fixed_width("Supercalifragilistic", width, 1.0);
    }
    // A text that hugs its width is never narrower than its widest word, its
    // min-content width: Chrome measures "Supercalifragilistic" at 139.70 px.
    assert_hug_text_in_column("Supercalifragilistic", 60.0, 139.703, 1.0);
}

#[test]
fn words_after_a_word_wider_than_its_line_still_wrap_at_the_line_width() {
    // The overflowing word takes a line of its own and leaves the lines after
    // it as wide as the text. Chrome breaks this text into
    // "Supercalifragilistic", "a b c d" and "e f g h" at 60 px, and into
    // "Supercalifragilistic", "a b c", "d e f" and "g h" at 40 px.
    let content = "Supercalifragilistic a b c d e f g h";
    assert_lines_at_fixed_width(content, 60.0, 3.0);
    assert_lines_at_fixed_width(content, 40.0, 4.0);
    // Hugging its width in a 60 px column, the text is as wide as its widest
    // word, where the letters all fit on the second line.
    assert_hug_text_in_column(content, 60.0, 139.703, 2.0);
}

#[test]
fn lines_also_break_after_hyphens_and_around_em_dashes_and_after_ellipses() {
    // Besides spaces, Chrome breaks lines where Unicode's line breaking rules
    // allow it inside a word: after a hyphen, before and after an em dash, and
    // after an ellipsis. The comments give the lines Chrome breaks each text
    // into.
    for (content, width, lines) in [
        // "state-of-" / "the-art"
        ("state-of-the-art", 80.0, 2.0),
        // "state-" / "of-the-" / "art"
        ("state-of-the-art", 60.0, 3.0),
        // "state-" (overflowing) / "of-" / "the-" / "art"
        ("state-of-the-art", 40.0, 4.0),
        // "one—" / "two—" / "three"
        ("one—two—three", 60.0, 3.0),
        // "one" / "—" / "two" / "—" / "three"
        ("one—two—three", 40.0, 5.0),
        // "hello…" / "world"
        ("hello…world", 60.0, 2.0),
    ] {
        assert_lines_at_fixed_width(content, width, lines);
    }

    // So where CSS makes the width of a text that hugs it fit-content, the
    // text is no wider than the space, unless its widest piece, its
    // min-content width, is wider still.
    for (content, column, width, lines) in [
        ("state-of-the-art", 60.0, 60.0, 3.0),
        // As wide as "state-", 44.52 px in Chrome.
        ("state-of-the-art", 20.0, 44.516, 4.0),
        // As wide as "three", 39.11 px in Chrome.
        ("one—two—three", 20.0, 39.109, 5.0),
        // As wide as "hello…", 49.97 px in Chrome.
        ("hello…world", 20.0, 49.969, 2.0),
    ] {
        assert_hug_text_in_column(content, column, width, lines);
    }
}

#[test]
fn lines_never_break_after_a_slash_between_letters() {
    // Unicode allows a break after a slash, but Chrome keeps letters joined by
    // a slash together, so paths and URLs stay whole and overflow their line
    // instead.
    assert_lines_at_fixed_width("input/output", 40.0, 1.0);
    // "word" / "input/output" / "word"
    assert_lines_at_fixed_width("word input/output word", 60.0, 3.0);
    assert_lines_at_fixed_width("word input/output word", 100.0, 3.0);
    // "See" / "example.com/a/very/long/path" / "today"
    assert_lines_at_fixed_width("See example.com/a/very/long/path today", 180.0, 3.0);
    // Hugging its width in a 60 px column, the text is as wide as
    // "input/output", 92.38 px in Chrome.
    assert_hug_text_in_column("word input/output word", 60.0, 92.375, 3.0);
}

#[test]
fn a_line_breaks_after_a_hyphen_before_a_digit_only_when_a_letter_or_digit_comes_first() {
    // Chrome breaks between a hyphen and a digit when a letter or a digit
    // comes before the hyphen: "pre-" / "2020" and "COVID-" / "19". Anywhere
    // else the hyphen may be a minus sign, which stays with its number, so
    // "-2020", "(-2020)" and the "-12" of "x -12" stay whole.
    for (content, lines) in [
        ("pre-2020", 2.0),
        ("COVID-19", 2.0),
        ("-2020", 1.0),
        ("(-2020)", 1.0),
        ("x -12", 2.0),
    ] {
        assert_lines_at_fixed_width(content, 10.0, lines);
    }
}

#[test]
fn a_line_breaks_after_a_question_mark_and_before_a_bracket_that_follows_punctuation() {
    // Between two ASCII characters Chrome breaks lines by its own table:
    // besides after hyphens, after a question mark ("what?" / "why"), and
    // before an opening bracket that follows closing punctuation or a symbol
    // ("matrix[0]" / "[1]", "f(x)=" / "(y)"), but not before one that follows
    // a letter.
    for content in ["what?why", "matrix[0][1]", "f(x)=(y)"] {
        assert_lines_at_fixed_width(content, 10.0, 2.0);
    }
}

#[test]
fn a_hyphen_or_dash_that_starts_a_word_stays_with_the_word() {
    // Unicode's line breaking rules keep a hyphen, or an en dash, that starts
    // a word with the letter after it (rule LB20a of Unicode 15.1, which
    // Chrome follows), so these break only at their space: "word " / "–word".
    for content in ["word \u{2013}word", "word \u{2010}word", "a -\u{e9}lan"] {
        assert_lines_at_fixed_width(content, 10.0, 2.0);
    }
    // A text that hugs its width in a narrow column is then as wide as
    // "–word", 45.95 px in Chrome.
    assert_hug_text_in_column("word \u{2013}word", 10.0, 45.953, 2.0);
}

#[test]
fn a_no_break_space_joins_its_neighbors_except_after_a_hyphen() {
    // A no-break space (U+00A0) keeps the characters on both sides of it
    // together: "10 km" and "Mr. Smith" written with one never break. After
    // a hyphen, though, Unicode's line breaking rules allow a break before
    // it, and Chrome breaks there: "well-" / " known".
    assert_lines_at_fixed_width("10\u{a0}km", 10.0, 1.0);
    assert_lines_at_fixed_width("Mr.\u{a0}Smith", 10.0, 1.0);
    assert_lines_at_fixed_width("well-\u{a0}known", 10.0, 2.0);
    // So a text that hugs its width in a narrow column is as wide as
    // " known", 54.59 px in Chrome.
    assert_hug_text_in_column("well-\u{a0}known", 10.0, 54.586, 2.0);
}

#[test]
fn a_tab_moves_its_line_on_to_the_next_tab_stop() {
    // CSS `tab-size: 8`: a tab moves its line on to the next stop, every
    // eight spaces from the start of the line (36 px in 16 px Inter). Letter
    // spacing widens the spaces that stops are counted in, so with 1 px of it
    // they are 44 px apart. A tab at the end of a line still counts toward
    // the text's width. The widths are Chrome's.
    for (content, spacing, width) in [
        ("a\tb", 0.0, 45.797),
        ("a\t\tb", 0.0, 81.797),
        ("Name\tValue", 0.0, 113.875),
        ("x\t", 0.0, 36.0),
        ("a\tb", 2.0, 63.797),
        ("Name\tValue", 1.0, 134.875),
    ] {
        let json = with(
            &text("txt00001", content, 16.0, 1.5),
            "letterSpacing",
            json!(spacing),
        );
        let boxes = lay_out(&wide_row(vec![json]));
        assert_box_within(
            &boxes,
            "txt00001",
            [0.0, 0.0, width, 24.0],
            TEXT_TOLERANCE,
            &format!(
                "{content:?} in 16 px Inter with letterSpacing {spacing}, hugging its width on one line"
            ),
        );
    }
    // A line also breaks before the piece that the tabs before it have moved
    // past the line's end.
    for (content, width, lines) in [
        ("Item\tPrice\tQty", 120.0, 2.0),
        ("Item\tPrice\tQty", 140.0, 1.0),
        ("a\tb\tc", 40.0, 3.0),
        ("a\tb\tc", 60.0, 2.0),
    ] {
        assert_lines_at_fixed_width(content, width, lines);
    }
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
fn a_line_break_in_the_content_starts_a_new_line() {
    let measured = hug_text_box(text("txt00001", "Hello\nminimum", 16.0, 1.5));
    let expected_width = advance_width("minimum", 16.0);
    assert!(
        (measured.width - expected_width).abs() <= TEXT_TOLERANCE
            && (measured.height - 48.0).abs() <= 0.01,
        "\"Hello\\nminimum\" is two lines of 24 px, as wide as \"minimum\" ({expected_width} px); layout gave {measured:?}"
    );
}

#[test]
fn a_line_break_at_the_end_of_the_content_starts_no_line() {
    // A line break ends its line, but in CSS it starts a new line only when
    // something follows it, so a final line break adds no empty line. One
    // with nothing before it still ends an empty line, and an empty text has
    // no line at all.
    for (content, lines) in [
        ("Hello\n", 1.0),
        ("Hello\nworld\n", 2.0),
        ("Hello\n\n", 2.0),
        ("\n", 1.0),
        ("\n\n", 2.0),
        ("", 0.0),
    ] {
        let height = hug_text_box(text("txt00001", content, 16.0, 1.5)).height;
        let expected = 24.0 * lines;
        assert!(
            (height - expected).abs() <= 0.01,
            "{content:?} in 16 px Inter is {lines} lines of 24 px, {expected} px in all; layout gave {height} px"
        );
    }

    let hello = advance_width("Hello", 16.0);
    let measured = hug_text_box(text("txt00001", "Hello\n", 16.0, 1.5));
    assert!(
        (measured.width - hello).abs() <= TEXT_TOLERANCE,
        "a final line break adds nothing to the width of \"Hello\\n\", which is as wide as \"Hello\" ({hello} px); layout gave {measured:?}"
    );
    let empty = hug_text_box(text("txt00001", "", 16.0, 1.5));
    assert!(
        empty.width.abs() <= 0.01 && empty.height.abs() <= 0.01,
        "an empty text that hugs its content has no line, so its box is empty; layout gave {empty:?}"
    );
    assert_lines_at_fixed_width("Hello\n", 50.0, 1.0);
}

#[test]
fn letter_spacing_adds_space_after_every_character() {
    let plain = advance_width("Hello", 16.0);
    for spacing in [2.0, -0.5] {
        let json = with(
            &text("txt00001", "Hello", 16.0, 1.5),
            "letterSpacing",
            json!(spacing),
        );
        let width = hug_text_box(json).width;
        let expected = plain + 5.0 * spacing as f32;
        assert!(
            (width - expected).abs() <= TEXT_TOLERANCE,
            "CSS `letter-spacing: {spacing}px` adds {spacing} px after each of the 5 characters of \"Hello\", so it is {expected} px wide; layout measured {width} px"
        );
    }
}

#[test]
fn letter_spacing_turns_off_contextual_alternates() {
    // Inter's contextual alternates (OpenType `calt`) change glyphs by their
    // neighbors: "->" becomes one arrow, and a colon between capitals takes
    // the form made for capitals. Chrome turns them off in text with letter
    // spacing, as CSS allows for optional ligatures, so the hyphen and the
    // greater-than sign keep their own glyphs, each followed by the spacing.
    // Without letter spacing they stay on. The widths are Chrome's.
    for (content, spacing, width) in [
        ("a->b", 0.0, 34.047),
        ("a->b", 2.0, 44.727),
        ("Learn more ->", 2.0, 134.883),
        ("C:VBQ", 0.0, 50.039),
        ("C:VBQ", 2.0, 59.039),
        ("C:VBQ", -0.5, 46.539),
        // Any spacing turns them off, however small.
        ("C:VBQ", 0.2, 50.039),
        ("a->b", 0.1, 37.133),
        ("a->b", -0.1, 36.328),
    ] {
        let json = with(
            &text("txt00001", content, 16.0, 1.5),
            "letterSpacing",
            json!(spacing),
        );
        let measured = hug_text_box(json).width;
        assert!(
            (measured - width).abs() <= TEXT_TOLERANCE,
            "{content:?} in 16 px Inter with letterSpacing {spacing} is {width} px wide in Chrome, which shapes letter-spaced text without contextual alternates; layout measured {measured} px"
        );
    }
}

#[test]
fn font_families_that_are_missing_measure_as_the_bundled_inter() {
    let inter = hug_text_box(text("txt00001", "Hello world", 16.0, 1.5)).width;
    for family in ["Helvetica", "Times New Roman", "Missing Sans"] {
        let json = with(
            &text("txt00001", "Hello world", 16.0, 1.5),
            "fontFamily",
            json!(family),
        );
        let width = hug_text_box(json).width;
        assert!(
            (width - inter).abs() <= 0.01,
            "the bundled Inter stands in for the missing family {family:?}, so text measures the same on every machine: {inter} px, but layout measured {width} px"
        );
    }
}

#[test]
fn a_text_in_a_grid_column_wraps_at_the_column_width() {
    let case = "a text that fills a 100 px column";
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        json!("hug"),
        grid(json!({"columns": [{"fixed": 100}, {"fr": 1}]})),
        vec![with(
            &text("txt00001", SIX_WORDS, 16.0, 1.5),
            "width",
            json!("fill"),
        )],
    ));
    assert_box(&boxes, "txt00001", [0.0, 0.0, 100.0, 72.0], case);
    assert_box(&boxes, "brd00001", [0.0, 0.0, 400.0, 72.0], case);

    let case = "an auto column as wide as the text that hugs its content in it";
    let hello = advance_width("Hello", 16.0);
    let boxes = lay_out(&board(
        "brd00001",
        fixed(400.0),
        json!("hug"),
        grid(json!({"columns": ["auto", {"fr": 1}]})),
        vec![
            text("txt00001", "Hello", 16.0, 1.5),
            sized("a0000001", json!("fill"), json!("fill")),
        ],
    ));
    assert_box_within(
        &boxes,
        "txt00001",
        [0.0, 0.0, hello, 24.0],
        TEXT_TOLERANCE,
        case,
    );
    assert_box_within(
        &boxes,
        "a0000001",
        [hello, 0.0, 400.0 - hello, 24.0],
        TEXT_TOLERANCE,
        case,
    );
}

#[test]
fn a_frame_that_hugs_a_text_is_the_size_of_the_text_and_its_padding() {
    let hello = advance_width("Hello", 16.0);
    let button = frame(
        "frm00001",
        json!("hug"),
        json!("hug"),
        flex(json!({"padding": {"top": 8, "right": 8, "bottom": 8, "left": 8}})),
        vec![text("txt00001", "Hello", 16.0, 1.5)],
    );
    let boxes = lay_out(&wide_row(vec![button]));
    let case = "a padded frame that hugs one line of 16 px text";
    assert_box_within(
        &boxes,
        "frm00001",
        [0.0, 0.0, hello + 16.0, 40.0],
        TEXT_TOLERANCE,
        case,
    );
    assert_box_within(
        &boxes,
        "txt00001",
        [8.0, 8.0, hello, 24.0],
        TEXT_TOLERANCE,
        case,
    );
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
    // the width of a space leaves no tab stops at all.
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
