//! US-003: text nodes are measured with Skia's paragraph layout, through
//! Taffy's measure callback: their glyphs are shaped, their lines wrap at the
//! width layout gives them where Chrome breaks lines, and their size sets the
//! size of what hugs them.

mod common;
mod design;

use design::{
    SIX_WORDS, assert_box, assert_box_within, at, board, box_of, fixed, flex, frame, grid, lay_out,
    none, rect, sized, text, with,
};
use serde_json::{Value, json};
use skia_safe::{Data, Font, FontMgr};

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

/// The height a browser gives one line of the bundled Inter at `size` px with
/// `line-height: normal`: the font's ascent, descent and line gap, each
/// rounded to whole pixels, as Chrome rounds them.
fn normal_line_height(size: f32) -> f32 {
    let (_, metrics) = inter(size).metrics();
    (-metrics.ascent).round() + metrics.descent.round() + metrics.leading.round()
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
fn a_text_without_a_line_height_gets_the_normal_line_height_a_browser_gives_its_font() {
    // Chrome makes each line of `line-height: normal` as tall as the font's
    // ascent plus its descent (Inter has no line gap), each rounded to whole
    // pixels: 16 + 4 = 20 px at 16 px, where the unrounded sum is 19.36 px.
    for (content, size, lines) in [
        ("Hello", 13.0, 1.0),
        ("Hello", 16.0, 1.0),
        ("Hello", 18.0, 1.0),
        ("Hello", 32.0, 1.0),
        ("Hello\nworld", 16.0, 2.0),
    ] {
        let mut json = text("txt00001", content, size, 1.5);
        json.as_object_mut()
            .expect("text fixtures are objects")
            .remove("lineHeight");
        let height = hug_text_box(json).height;
        let expected = lines * normal_line_height(size as f32);
        assert!(
            (height - expected).abs() <= 0.01,
            "{content:?} in {size} px Inter with lineHeight left out is {lines} lines of CSS `line-height: normal`, {expected} px in all; layout gave {height} px"
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
