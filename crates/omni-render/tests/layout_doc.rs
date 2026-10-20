//! US-003: `docs/formats/layout.md` defines the exact CSS that each layout
//! property of a design maps to. The layout engine follows it, and US-009
//! exports designs as HTML and CSS with it to compare the renderer's boxes
//! with a browser's.

mod common;

use std::fs;

fn layout_doc() -> String {
    let path = common::repo_root().join("docs/formats/layout.md");
    fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("docs/formats/layout.md must define how design layout maps to CSS, but it cannot be read ({}): {error}", path.display())
    })
}

/// `text` with every run of whitespace turned into one space and no spaces
/// around colons, so `display:  flex` reads as `display:flex`.
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" :", ":")
        .replace(": ", ":")
}

/// Whether `word` appears in `text` as a whole CSS name, not as part of a
/// longer one (`grid-row` inside `grid-row-start` does not count).
fn names(text: &str, word: &str) -> bool {
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '-';
    text.match_indices(word).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let after = text[start + word.len()..].chars().next();
        !before.is_some_and(is_name) && !after.is_some_and(is_name)
    })
}

#[test]
fn the_doc_names_every_layout_property_of_the_design_format() {
    let doc = layout_doc();
    for property in [
        "mode",
        "none",
        "flex",
        "grid",
        "direction",
        "wrap",
        "gap",
        "padding",
        "alignItems",
        "justifyContent",
        "columns",
        "rows",
        "gridColumn",
        "gridRow",
        "start",
        "span",
        "width",
        "height",
        "fixed",
        "hug",
        "fill",
        "fr",
        "auto",
        "minWidth",
        "maxWidth",
        "minHeight",
        "maxHeight",
        "x",
        "y",
        "absolute",
        "visible",
        "fontFamily",
        "fontSize",
        "lineHeight",
        "letterSpacing",
    ] {
        assert!(
            doc.contains(&format!("`{property}`")),
            "docs/formats/layout.md must say what CSS the design property or keyword `{property}` maps to, written as `{property}`"
        );
    }
}

#[test]
fn the_doc_maps_layout_to_css_properties() {
    let doc = layout_doc();
    for property in [
        "display",
        "position",
        "left",
        "top",
        "box-sizing",
        "flex-direction",
        "flex-wrap",
        "row-gap",
        "column-gap",
        "padding",
        "align-items",
        "justify-content",
        "flex-grow",
        "flex-shrink",
        "flex-basis",
        "align-self",
        "justify-self",
        "grid-template-columns",
        "grid-template-rows",
        "grid-column",
        "grid-row",
        "width",
        "height",
        "min-width",
        "max-width",
        "min-height",
        "max-height",
        "font-family",
        "font-size",
        "line-height",
        "letter-spacing",
        "white-space",
    ] {
        assert!(
            names(&doc, property),
            "docs/formats/layout.md must name the CSS property `{property}` that design layout maps to"
        );
    }
}

#[test]
fn the_doc_names_the_device_pixel_ratio_that_layout_matches_chrome_at() {
    // Chrome rounds font metrics and the leading above a line's glyphs to
    // device pixels, so the boxes it gives text depend on the device pixel
    // ratio. Layout matches the boxes Chrome gives at a ratio of 2, the ratio
    // the renderer gate (US-008, US-009) uses, and the mapping must say so.
    let doc = normalized(&layout_doc()).to_lowercase();
    let names_two = doc.match_indices("pixel ratio").any(|(start, phrase)| {
        let after: String = doc[start + phrase.len()..].chars().take(16).collect();
        after
            .split(|c: char| !c.is_ascii_digit())
            .any(|number| number == "2")
    });
    assert!(
        names_two,
        "docs/formats/layout.md must say that layout gives the boxes Chrome gives at a device pixel ratio of 2, the ratio of the renderer gate (US-008, US-009), because Chrome rounds line heights and baselines to device pixels"
    );
}

#[test]
fn the_doc_defines_fixed_hug_and_fill_per_axis_in_flex_and_in_grid() {
    let doc = layout_doc();
    let lower = doc.to_lowercase();
    for axis in ["main axis", "cross axis"] {
        assert!(
            lower.contains(axis),
            "docs/formats/layout.md must define `fixed`, `hug` and `fill` on the {axis} of a flex layout"
        );
    }
    let declarations = normalized(&doc);
    for declaration in [
        "display:flex",
        "display:grid",
        "display:none",
        "position:relative",
        "position:absolute",
        "box-sizing:border-box",
        "flex-grow:1",
        "flex-shrink:0",
        "flex-basis:0",
        "align-self:stretch",
        "justify-self:stretch",
        "align-self:start",
        "justify-self:start",
        "white-space:pre-wrap",
    ] {
        assert!(
            declarations.contains(declaration),
            "docs/formats/layout.md must spell out the CSS declaration `{}` that the layout tests rely on",
            declaration.replace(':', ": ")
        );
    }
}

/// The text of the section of `doc` whose heading is `title`, up to the next
/// heading of the same or a higher level, or `None` when it has no such
/// section. Lines inside code fences are not headings.
fn section(doc: &str, title: &str) -> Option<String> {
    let mut in_fence = false;
    let mut level_of_section = None;
    let mut text = String::new();
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        let level = if in_fence {
            0
        } else {
            line.chars().take_while(|&c| c == '#').count()
        };
        let is_heading = level > 0 && line[level..].starts_with(' ');
        match level_of_section {
            None if is_heading && line[level..].trim().eq_ignore_ascii_case(title) => {
                level_of_section = Some(level);
            }
            Some(start) if is_heading && level <= start => break,
            Some(_) => {
                text.push_str(line);
                text.push('\n');
            }
            None => {}
        }
    }
    level_of_section.map(|_| text)
}

#[test]
fn the_doc_lists_the_known_differences_from_chrome_under_a_heading_of_that_name() {
    // The differences that the checker finds on random designs and that stay
    // are listed under "Known differences", for US-009 to resolve or to
    // confirm within 2 px. The text differences the doc already names belong
    // there too.
    let doc = layout_doc();
    let known = section(&doc, "Known differences").unwrap_or_else(|| {
        panic!("docs/formats/layout.md must have a section with the heading `Known differences`, which lists where the renderer's boxes differ from Chrome's for US-009 to resolve")
    });
    let lower = known.to_lowercase();
    for difference in ["thai", "soft hyphen"] {
        assert!(
            lower.contains(difference),
            "the `Known differences` section of docs/formats/layout.md must list the difference that the doc already names for {difference:?}, the text that the renderer still breaks differently from Chrome; it says:\n{known}"
        );
    }
}

/// Whether `doc`, which `normalized` has turned into a single line with no
/// spaces around colons, spells out a CSS `line-height` in px.
fn writes_a_line_height_in_px(doc: &str) -> bool {
    doc.match_indices("line-height:").any(|(start, property)| {
        let value: String = doc[start + property.len()..]
            .chars()
            .take_while(|&c| !c.is_whitespace() && !matches!(c, '`' | ';' | ')' | ','))
            .collect();
        value.ends_with("px")
    })
}

#[test]
fn the_doc_gives_every_line_an_explicit_height_in_px_at_full_precision() {
    // Decision Q44: a text without lineHeight has its font's normal line
    // height (ascent + descent + line gap) at full precision, never rounded to
    // whole pixels: 19.359375 px for 16 px Inter. Every line height is snapped
    // to a multiple of 1/64 px, and the export writes it in px, never
    // `normal`, so Chrome gives the same result at any pixel ratio.
    let raw = layout_doc();
    let doc = normalized(&raw);
    assert!(
        doc.contains("19.359375"),
        "docs/formats/layout.md must give the normal line height of 16 px Inter at full precision, 19.359375 px (15.5 + 3.859375 + 0), as the example of a text without a lineHeight"
    );
    assert!(
        doc.contains("1/64"),
        "docs/formats/layout.md must say that every line height is snapped to a multiple of 1/64 px"
    );
    assert!(
        writes_a_line_height_in_px(&doc),
        "docs/formats/layout.md must say that the CSS of a text's `lineHeight` is a `line-height` in px, never `normal` or a multiple of the font size"
    );
}

/// The contents of the fenced code blocks of `doc` in `language`, such as
/// `html`.
fn fenced_blocks(doc: &str, language: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut open: Option<(bool, String)> = None;
    for line in doc.lines() {
        let fence = line.trim_start().strip_prefix("```");
        match (&mut open, fence) {
            (None, Some(info)) => open = Some((info.trim() == language, String::new())),
            (Some((wanted, block)), Some(_)) => {
                if *wanted {
                    blocks.push(std::mem::take(block));
                }
                open = None;
            }
            (Some((_, block)), None) => {
                block.push_str(line);
                block.push('\n');
            }
            (None, None) => {}
        }
    }
    blocks
}

#[test]
fn the_html_of_the_docs_example_writes_its_line_height_in_px() {
    // The export writes every line height in px (decision Q44), never a
    // multiple of the font size, and the HTML that the doc gives for its
    // example is the CSS of its card by the doc's own rules. The title is 20 px
    // text with lineHeight 1.2, so its line height is `line-height: 24px`.
    let doc = layout_doc();
    let html = fenced_blocks(&doc, "html").join("\n");
    let html = normalized(&html);
    let line_heights: Vec<String> = html
        .match_indices("line-height:")
        .map(|(start, property)| {
            html[start + property.len()..]
                .chars()
                .take_while(|&c| !c.is_whitespace() && !matches!(c, ';' | '"' | '\''))
                .collect()
        })
        .collect();
    assert!(
        !line_heights.is_empty(),
        "docs/formats/layout.md must give the HTML of its example, whose title has a lineHeight, in an ```html block with the title's `line-height`"
    );
    for value in line_heights {
        let px = value
            .strip_suffix("px")
            .and_then(|number| number.parse::<f64>().ok());
        assert_eq!(
            px,
            Some(24.0),
            "the HTML of the example in docs/formats/layout.md must write the title's line height as the export does, in px (decision Q44): `line-height: 24px` for 20 px text with lineHeight 1.2, but it writes `line-height: {value}`"
        );
    }
}

#[test]
fn the_doc_maps_margins_grid_self_alignment_and_stroke_dashes_and_alignment_to_css() {
    // US-053 (decision Q48): every node may have a `margin` on each side, a
    // child of a grid may set `justifySelf` and `alignSelf`, and a stroke may
    // be dashed and, on a frame, sit inside, centered on or outside the edge.
    // The doc names each new property and the CSS it maps to.
    let doc = layout_doc();
    for property in [
        "margin",
        "alignSelf",
        "justifySelf",
        "dash",
        "align",
        "inside",
        "center",
        "outside",
    ] {
        assert!(
            doc.contains(&format!("`{property}`")),
            "docs/formats/layout.md must say what CSS the design property or keyword `{property}` maps to, written as `{property}`"
        );
    }
    for property in [
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "justify-self",
        "align-self",
        "stroke-dasharray",
        "outline",
    ] {
        assert!(
            names(&doc, property),
            "docs/formats/layout.md must name the CSS property `{property}` that a US-053 property maps to"
        );
    }
}
