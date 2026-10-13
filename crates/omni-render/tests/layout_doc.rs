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
        "line-height:normal",
        "white-space:pre-wrap",
    ] {
        assert!(
            declarations.contains(declaration),
            "docs/formats/layout.md must spell out the CSS declaration `{}` that the layout tests rely on",
            declaration.replace(':', ": ")
        );
    }
}
