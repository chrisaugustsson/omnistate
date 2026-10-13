//! Text: the fonts that text is shaped and measured with, and the lines that
//! the text of a text node breaks into.

mod breaks;

use omni_core::design::{self, Text, TokenOr};
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, RectHeightStyle, RectWidthStyle,
    TextStyle, TypefaceFontProvider,
};
use skia_safe::{Data, Font, FontHinting, FontMgr, FontStyle, Typeface};
use std::ops::Range;

/// Inter Regular, under the SIL Open Font License (`fonts/LICENSE.txt`).
/// Skia has no system fonts in the browser, so the font is compiled in.
const INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");

/// The family name of the bundled Inter.
const INTER: &str = "Inter";

/// The font size of a text whose `fontSize` is a token that is not resolved:
/// CSS's initial `font-size`, `medium`.
const MEDIUM_FONT_SIZE: f32 = 16.0;

/// How far a line may be wider than the width it breaks at and still fit.
/// Widths that layout computes from the text's own width can come back a
/// rounding error short, which must not break a line.
const FIT_TOLERANCE: f32 = 0.01;

/// How many spaces apart tab stops are: CSS's initial `tab-size`.
const TAB_SIZE: f32 = 8.0;

/// The fonts that text is shaped and measured with.
pub struct Fonts {
    collection: FontCollection,
}

impl Fonts {
    /// The fonts bundled with the renderer: Inter Regular
    /// (`fonts/Inter-Regular.ttf`), which also stands in for every font family
    /// that is missing, so text measures the same on every machine.
    pub fn bundled() -> Self {
        let mut provider = TypefaceFontProvider::new();
        provider.register_typeface(inter(), INTER);
        let mut collection = FontCollection::new();
        // No system fonts: a family that is not registered falls back to the
        // default family, Inter.
        collection.set_default_font_manager(Some(provider.into()), INTER);
        Fonts { collection }
    }

    /// The typeface that text in `family` and `style` is set in: the closest
    /// face of the family, or the bundled Inter when the family is missing.
    fn typeface(&self, family: &str, style: FontStyle) -> Typeface {
        self.collection
            .clone()
            .find_typefaces(&[family], style)
            .into_iter()
            .next()
            .unwrap_or_else(inter)
    }
}

/// The bundled Inter typeface. Where Skia is built with FreeType (WebAssembly)
/// it loads the font through an empty FreeType font manager; elsewhere the
/// platform font manager loads it (CoreText on macOS).
pub(crate) fn inter() -> Typeface {
    FontMgr::custom_empty()
        .unwrap_or_default()
        .new_from_data(Data::new_copy(INTER_REGULAR), None)
        .expect("Skia loads the bundled Inter font")
}

/// The content of a text node, shaped in its font by Skia's paragraph layout
/// and cut into the pieces that its lines may break between, so its lines
/// break at any width the way Chrome breaks them with CSS
/// `white-space: pre-wrap`: at line breaks, after spaces and wherever else
/// Chrome allows a line to break (see `text/breaks.rs`), never inside a word.
pub struct ShapedText {
    /// The lines that the content's line breaks make, each cut into the
    /// pieces that it may break between. An empty line has no pieces.
    lines: Vec<Vec<Piece>>,
    /// Where tabs move a line on to.
    tab_stops: TabStops,
    /// The width of the widest piece, which is as narrow as the text gets.
    min_content_width: f32,
    /// The width of the widest line when lines break only at line breaks.
    max_content_width: f32,
    /// The height of every line.
    line_height: f32,
    /// How far the baseline of the first line is below the top of the text,
    /// or `None` when the text is empty and so has no line.
    first_baseline: Option<f32>,
}

/// A piece of a line, from one place where the line may break to the next.
struct Piece {
    /// The width of the piece without the spaces and tabs that end it, which
    /// hang at the end of a line: the line holds them even where they do not
    /// fit.
    width: f32,
    /// The width of the spaces that follow the piece, up to its first tab.
    spaces: f32,
    /// The width of the spaces that follow each tab of the piece.
    tabs: Vec<f32>,
}

impl Piece {
    /// Where a line goes on after the piece and the spaces and tabs that end
    /// it, when the piece starts at `x`.
    fn end(&self, x: f32, tab_stops: TabStops) -> f32 {
        self.tabs
            .iter()
            .fold(x + self.width + self.spaces, |x, spaces| {
                tab_stops.after(x) + spaces
            })
    }
}

/// Where tabs move a line on to, as in Chrome: to the next stop, a stop every
/// [`TAB_SIZE`] spaces from the start of the line, but on by half a space at
/// least.
#[derive(Clone, Copy)]
struct TabStops {
    /// How far apart the stops are: [`TAB_SIZE`] spaces, letter spacing
    /// included.
    every: f32,
    /// The width of a space.
    space: f32,
}

impl TabStops {
    /// The stop that a tab at `x` moves the line on to.
    fn after(self, x: f32) -> f32 {
        if self.every <= 0.0 {
            return x;
        }
        let mut stop = x + self.every - x % self.every;
        if stop - x < self.space / 2.0 {
            stop += self.every;
        }
        stop
    }
}

impl ShapedText {
    /// Shapes the content of `text` in its font, from `fonts`.
    pub fn new(text: &Text, fonts: &Fonts) -> Self {
        let family = match &text.font_family {
            TokenOr::Literal(family) => family.as_str(),
            TokenOr::Token(_) => INTER,
        };
        let font_size = literal(&text.font_size).unwrap_or(MEDIUM_FONT_SIZE);
        let font_style = font_style(text);
        // A unitless CSS `line-height`: a multiple of the font size.
        let line_height_multiple = text.line_height.as_ref().and_then(literal);
        let letter_spacing = text
            .letter_spacing
            .as_ref()
            .and_then(literal)
            .unwrap_or(0.0);

        let mut style = TextStyle::new();
        style
            .set_font_families(&[family])
            .set_font_size(font_size)
            .set_font_style(font_style)
            .set_letter_spacing(letter_spacing);
        let mut paragraph_style = ParagraphStyle::new();
        paragraph_style
            .set_text_style(&style)
            // Keep the exact advances, as browsers do: no widths rounded to
            // hundredths, and no hinting, which FreeType applies to advances.
            .set_apply_rounding_hack(false)
            .turn_hinting_off();
        let mut builder = ParagraphBuilder::new(&paragraph_style, &fonts.collection);
        builder.add_text(&text.content);
        let mut paragraph = builder.build();
        // Lines break only at line breaks, so each piece is measured where
        // the shaping of its whole line puts it, kerning included.
        paragraph.layout(f32::INFINITY);
        let lines = pieces(&text.content, &paragraph);

        // Browsers round the font's ascent and descent to whole pixels before
        // they size lines and place baselines.
        let mut font = Font::from_typeface(fonts.typeface(family, font_style), font_size);
        font.set_hinting(FontHinting::None)
            .set_linear_metrics(true)
            .set_subpixel(true);
        let (_, metrics) = font.metrics();
        let ascent = (-metrics.ascent).round();
        let descent = metrics.descent.round();
        let line_height = match line_height_multiple {
            Some(multiple) => multiple * font_size,
            // CSS `line-height: normal`.
            None => ascent + descent + metrics.leading.round(),
        };
        // Half the leading goes above the glyphs, rounded down to a whole pixel.
        let first_baseline = ascent + ((line_height - ascent - descent) / 2.0).floor();
        let space = font.measure_str(" ", None).0;
        let tab_stops = TabStops {
            every: TAB_SIZE * (space + letter_spacing),
            space,
        };

        ShapedText {
            // The spaces and tabs that end a piece hang, so they never widen
            // the text when every piece takes a line of its own.
            min_content_width: lines
                .iter()
                .flatten()
                .map(|piece| piece.width)
                .fold(0.0, f32::max),
            // Spaces and tabs at the end of a line still count here, as in
            // Chrome.
            max_content_width: lines
                .iter()
                .map(|line| line.iter().fold(0.0, |x, piece| piece.end(x, tab_stops)))
                .fold(0.0, f32::max),
            lines,
            tab_stops,
            line_height,
            first_baseline: (!text.content.is_empty()).then_some(first_baseline),
        }
    }

    /// The CSS min-content width: the width of the widest piece, without the
    /// spaces that end it.
    pub fn min_content_width(&self) -> f32 {
        self.min_content_width
    }

    /// The CSS max-content width: the width of the widest line when lines
    /// break only at line breaks.
    pub fn max_content_width(&self) -> f32 {
        self.max_content_width
    }

    /// How far the baseline of the first line is below the top of the text,
    /// or `None` when the text is empty and so has no line.
    pub fn first_baseline(&self) -> Option<f32> {
        self.first_baseline
    }

    /// The height of the text when its lines break at `width`: every line is
    /// `lineHeight` × `fontSize` tall, or as tall as CSS makes
    /// `line-height: normal`.
    pub fn height(&self, width: f32) -> f32 {
        self.line_count(width) as f32 * self.line_height
    }

    /// How many lines the text breaks into at `width`. As in a browser, each
    /// line takes as many pieces as fit, and a piece wider than `width`
    /// overflows a line of its own.
    fn line_count(&self, width: f32) -> usize {
        self.lines
            .iter()
            .map(|pieces| {
                let Some((first, rest)) = pieces.split_first() else {
                    // An empty line, which a line break ends.
                    return 1;
                };
                let mut lines = 1;
                let mut line = first.end(0.0, self.tab_stops);
                for piece in rest {
                    if line + piece.width > width + FIT_TOLERANCE {
                        lines += 1;
                        line = 0.0;
                    }
                    line = piece.end(line, self.tab_stops);
                }
                lines
            })
            .sum()
    }
}

/// Cuts `content` into its lines at its line breaks, and each line into the
/// pieces that it may break between, measured as `paragraph` shaped them.
///
/// A line break at the very end of the content starts no line, and empty
/// content has no line at all, as in CSS.
fn pieces(content: &str, paragraph: &Paragraph) -> Vec<Vec<Piece>> {
    // Skia counts offsets into the text in UTF-16 code units.
    let mut utf16_offsets = vec![0; content.len() + 1];
    let mut utf16_offset = 0;
    for (offset, c) in content.char_indices() {
        utf16_offsets[offset] = utf16_offset;
        utf16_offset += c.len_utf16();
    }
    utf16_offsets[content.len()] = utf16_offset;
    // The width of the text in `range`: the widths of the boxes of its
    // glyphs, which bidirectional text may split.
    let width = |range: Range<usize>| -> f32 {
        if range.is_empty() {
            return 0.0;
        }
        paragraph
            .get_rects_for_range(
                utf16_offsets[range.start]..utf16_offsets[range.end],
                RectHeightStyle::Tight,
                RectWidthStyle::Tight,
            )
            .iter()
            .map(|text_box| text_box.rect.width())
            .sum()
    };

    let mut line_ranges = Vec::new();
    let mut line_start = 0;
    for (line_break, _) in content.match_indices('\n') {
        line_ranges.push(line_start..line_break);
        line_start = line_break + 1;
    }
    if line_start < content.len() {
        line_ranges.push(line_start..content.len());
    }

    let mut soft_breaks = breaks::soft_breaks(content).into_iter().peekable();
    line_ranges
        .into_iter()
        .map(|line| {
            let mut pieces = Vec::new();
            let mut piece_start = line.start;
            while piece_start < line.end {
                let piece_end = soft_breaks
                    .next_if(|&soft_break| soft_break < line.end)
                    .unwrap_or(line.end);
                let piece = &content[piece_start..piece_end];
                let spaces_start = piece_start + piece.trim_end_matches([' ', '\t']).len();
                // The runs of spaces before, between and after the tabs.
                let mut run_start = spaces_start;
                let mut runs = content[spaces_start..piece_end].split('\t').map(|run| {
                    let run = run_start..run_start + run.len();
                    run_start = run.end + '\t'.len_utf8();
                    width(run)
                });
                pieces.push(Piece {
                    width: width(piece_start..spaces_start),
                    spaces: runs.next().unwrap_or(0.0),
                    tabs: runs.collect(),
                });
                piece_start = piece_end;
            }
            pieces
        })
        .collect()
}

/// The value of a length or number, or `None` for a token reference, which
/// layout does not resolve: the property then keeps its CSS initial value.
pub(crate) fn literal(value: &TokenOr<f64>) -> Option<f32> {
    match value {
        TokenOr::Literal(value) => Some(*value as f32),
        TokenOr::Token(_) => None,
    }
}

/// The weight and slant of `text`, as CSS `font-weight` and `font-style`.
fn font_style(text: &Text) -> FontStyle {
    let weight = text
        .font_weight
        .as_ref()
        .and_then(literal)
        .map_or(Weight::NORMAL, |weight| Weight::from(weight.round() as i32));
    let slant = match text.font_style {
        Some(design::FontStyle::Italic) => Slant::Italic,
        Some(design::FontStyle::Normal) | None => Slant::Upright,
    };
    FontStyle::new(weight, Width::NORMAL, slant)
}
