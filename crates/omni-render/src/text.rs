//! Text: the fonts that text is shaped and measured with, and the lines that
//! the text of a text node breaks into.

mod breaks;

use omni_core::design::{self, Text, TextAlign, TextDecoration, TokenOr};
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, RectHeightStyle, RectWidthStyle,
    TextStyle, TypefaceFontProvider,
};
use skia_safe::{
    Canvas, Color, Data, Font, FontHinting, FontMgr, FontStyle, Paint, Rect, Typeface,
};
use std::ops::Range;

/// Inter Regular, under the SIL Open Font License (`fonts/LICENSE.txt`).
/// Skia has no system fonts in the browser, so the font is compiled in.
const INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");

/// The family name of the bundled Inter.
const INTER: &str = "Inter";

/// The font size of a text whose `fontSize` is a token that is not resolved:
/// CSS's initial `font-size`, `medium`.
const MEDIUM_FONT_SIZE: f32 = 16.0;

/// How many layout units Chrome has to a CSS pixel at a device pixel ratio of 2:
/// it lays out in 1/64 of a device pixel.
const LAYOUT_UNITS_PER_PX: f32 = 64.0 * DEVICE_PIXEL_RATIO;

/// How many spaces apart tab stops are: CSS's initial `tab-size`.
const TAB_SIZE: f32 = 8.0;

/// The device pixel ratio that text is laid out for, as the renderer gate
/// (US-008, US-009) compares it with Chrome: two device pixels to a CSS
/// pixel. Chrome rounds the font's ascent and descent and the leading above a
/// line's glyphs to device pixels, so baselines depend on it.
const DEVICE_PIXEL_RATIO: f32 = 2.0;

/// How many parts of a CSS pixel line heights are snapped to: 1/64 px, which
/// Chrome lays out in at every integer device pixel ratio.
const LINE_HEIGHT_UNITS_PER_PX: f64 = 64.0;

/// The OpenType features that Chrome turns off in text with letter spacing:
/// common ligatures and contextual alternates, which would join or reshape
/// characters that the spacing sets apart.
const FEATURES_OFF_WITH_LETTER_SPACING: [&str; 3] = ["liga", "clig", "calt"];

/// The fonts that text is shaped and measured with. A clone shares the
/// registered typefaces with the original.
#[derive(Clone)]
pub struct Fonts {
    /// The registry of the typefaces that were supplied, which `collection`
    /// looks families up in.
    provider: TypefaceFontProvider,
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
        collection.set_default_font_manager(Some(provider.clone().into()), INTER);
        Fonts {
            provider,
            collection,
        }
    }

    /// Supplies a font file (TrueType or OpenType) for text in `family`: text
    /// in that family is measured and drawn with it from now on. Fails if the
    /// bytes are not a font.
    pub fn add_font(&mut self, family: &str, data: &[u8]) -> Result<(), String> {
        let typeface = FontMgr::custom_empty()
            .unwrap_or_default()
            .new_from_data(Data::new_copy(data), None)
            .ok_or_else(|| format!("the bytes for font family {family:?} are not a font"))?;
        self.provider.register_typeface(typeface, family);
        self.collection.clear_caches();
        Ok(())
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
    /// The text, kept to draw from.
    content: String,
    /// How the text is drawn: its style and paragraph style, from which the
    /// paragraphs of the runs of a line are built, and the lines that
    /// decorate it.
    style: TextStyle,
    paragraph_style: ParagraphStyle,
    decoration: Decoration,
    /// The lines that the content's line breaks make, each cut into the
    /// pieces that it may break between. An empty line has no pieces.
    lines: Vec<Vec<Piece>>,
    /// Where tabs move a line on to.
    tab_stops: TabStops,
    /// The width of the widest piece, which is as narrow as the text gets.
    min_content_width: f32,
    /// The width of the widest line when lines break only at line breaks.
    max_content_width: f32,
    /// The height of every line, a multiple of 1/64 px.
    line_height: f32,
    /// How far the baseline of the first line is below the top of the text,
    /// or `None` when the text is empty and so has no line.
    first_baseline: Option<f32>,
    /// How the lines of the text align in its box.
    align: TextAlign,
}

/// The line that decorates text, if it has one, as the font sets it.
struct Decoration {
    /// How far below the baseline the top of the line is, up being negative,
    /// and how thick it is.
    rule: Option<(f32, f32)>,
}

impl Decoration {
    fn of(text: &Text, metrics: &skia_safe::FontMetrics, font_size: f32) -> Self {
        // Thinner than a pixel, a line fades instead of showing.
        let thickness = |thickness: Option<f32>| thickness.unwrap_or(font_size / 16.0).max(1.0);
        let rule = match text.text_decoration {
            Some(TextDecoration::Underline) => Some((
                // The font gives where the top of an underline is.
                metrics.underline_position().unwrap_or(font_size / 10.0),
                thickness(metrics.underline_thickness()),
            )),
            Some(TextDecoration::LineThrough) => {
                let thickness = thickness(metrics.strikeout_thickness());
                // The font gives where the bottom of the line is.
                let bottom = metrics.strikeout_position().unwrap_or(-font_size * 0.3);
                Some((bottom - thickness, thickness))
            }
            Some(TextDecoration::None) | None => None,
        };
        Decoration { rule }
    }
}

/// A piece of a line, from one place where the line may break to the next.
struct Piece {
    /// Where the piece starts in the content, in bytes, and where its word
    /// ends and the spaces and tabs that end it start. It ends where the next
    /// piece of the line starts.
    start: usize,
    word_end: usize,
    end: usize,
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
        let font_size_in_px = font_size_in_px(text);
        let font_size = font_size_in_px as f32;
        let font_style = font_style(text);
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
        if letter_spacing != 0.0 {
            for feature in FEATURES_OFF_WITH_LETTER_SPACING {
                style.add_font_feature(feature, 0);
            }
        }
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

        let mut font = Font::from_typeface(fonts.typeface(family, font_style), font_size);
        font.set_hinting(FontHinting::None)
            .set_linear_metrics(true)
            .set_subpixel(true);
        let (_, metrics) = font.metrics();
        // Every line height is explicit (decision Q44): a multiple of the font
        // size, or else the font's normal line height, its ascent, descent
        // and line gap, at full precision. Both are snapped to 1/64 px, so
        // that Chrome, given the height in px, lays out the same at any pixel
        // ratio and never rounds the font's metrics, as it does for
        // `line-height: normal`.
        let line_height = snap_line_height(match text.line_height.as_ref() {
            Some(TokenOr::Literal(multiple)) => font_size_in_px * multiple,
            Some(TokenOr::Token(_)) | None => {
                f64::from(-metrics.ascent + metrics.descent + metrics.leading)
            }
        });
        // Chrome places baselines in device pixels: it rounds the font's ascent
        // and descent to whole device pixels, and half the leading above the
        // glyphs down to a whole device pixel.
        let ascent = (-metrics.ascent * DEVICE_PIXEL_RATIO).round();
        let descent = (metrics.descent * DEVICE_PIXEL_RATIO).round();
        let line_height_in_device_pixels = line_height * DEVICE_PIXEL_RATIO;
        let first_baseline =
            ascent + ((line_height_in_device_pixels - ascent - descent) / 2.0).floor();
        let space = font.measure_str(" ", None).0;
        let decoration = Decoration::of(text, &metrics, font_size);
        let tab_stops = TabStops {
            every: TAB_SIZE * (space + letter_spacing),
            space,
        };

        ShapedText {
            content: text.content.clone(),
            style,
            paragraph_style,
            decoration,
            align: text.text_align.unwrap_or(TextAlign::Left),
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
            first_baseline: (!text.content.is_empty())
                .then_some(first_baseline / DEVICE_PIXEL_RATIO),
        }
    }

    /// The CSS min-content width: the width of the widest piece, without the
    /// spaces that end it, rounded up to a layout unit as Chrome rounds an
    /// intrinsic width, so that a text sized to it keeps the lines that Chrome
    /// keeps.
    pub fn min_content_width(&self) -> f32 {
        (self.min_content_width * LAYOUT_UNITS_PER_PX).ceil() / LAYOUT_UNITS_PER_PX
    }

    /// The CSS max-content width: the width of the widest line when lines
    /// break only at line breaks, rounded up to a layout unit like the
    /// min-content width.
    pub fn max_content_width(&self) -> f32 {
        (self.max_content_width * LAYOUT_UNITS_PER_PX).ceil() / LAYOUT_UNITS_PER_PX
    }

    /// How far the baseline of the first line is below the top of the text,
    /// or `None` when the text is empty and so has no line.
    pub fn first_baseline(&self) -> Option<f32> {
        self.first_baseline
    }

    /// The height of the text when its lines break at `width`: every line is
    /// `lineHeight` × `fontSize` tall, or as tall as the font's normal line
    /// height when `lineHeight` is left out, snapped to 1/64 px.
    pub fn height(&self, width: f32) -> f32 {
        self.line_count(width) as f32 * self.line_height
    }

    /// How many lines the text breaks into at `width`.
    fn line_count(&self, width: f32) -> usize {
        let mut lines = 0;
        self.each_line(width, |_, _| lines += 1);
        lines
    }

    /// Calls `visit` with the pieces of each line that the text breaks into at
    /// `width`, top to bottom, and whether a line break or the end of the text
    /// ends the line. As in a browser, each line takes as many pieces as fit,
    /// and a piece wider than `width` overflows a line of its own. An empty
    /// line, which a line break ends, has no pieces.
    fn each_line(&self, width: f32, mut visit: impl FnMut(&[Piece], bool)) {
        for pieces in &self.lines {
            let Some(first) = pieces.first() else {
                visit(&[], true);
                continue;
            };
            let mut start = 0;
            let mut line = first.end(0.0, self.tab_stops);
            for (index, piece) in pieces.iter().enumerate().skip(1) {
                if !fits(line + piece.width, width) {
                    visit(&pieces[start..index], false);
                    start = index;
                    line = 0.0;
                }
                line = piece.end(line, self.tab_stops);
            }
            visit(&pieces[start..], true);
        }
    }

    /// Draws the text in `color` with its top-left corner at `origin`, in a
    /// box `width` px wide: each line is where layout put it, breaking at
    /// `width`, a line height below the one above, aligned in the box.
    pub fn draw(
        &self,
        canvas: &Canvas,
        fonts: &Fonts,
        color: Color,
        origin: (f32, f32),
        width: f32,
    ) {
        let Some(first_baseline) = self.first_baseline else {
            return;
        };
        let mut style = self.style.clone();
        style.set_color(color);
        let mut paragraph_style = self.paragraph_style.clone();
        paragraph_style.set_text_style(&style);
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(color);

        let mut baseline = origin.1 + first_baseline;
        self.each_line(width, |pieces, ends_paragraph| {
            let line = self.arrange(pieces, width, ends_paragraph);
            for run in &line.runs {
                let mut builder = ParagraphBuilder::new(&paragraph_style, &fonts.collection);
                builder.add_text(&self.content[run.range.clone()]);
                let mut paragraph = builder.build();
                paragraph.layout(f32::INFINITY);
                paragraph.paint(
                    canvas,
                    (
                        origin.0 + line.x + run.x,
                        baseline - paragraph.alphabetic_baseline(),
                    ),
                );
            }
            // A line of text is decorated across its whole width, between
            // words too.
            if let Some((offset, thickness)) = self.decoration.rule.filter(|_| line.width > 0.0) {
                let rule =
                    Rect::from_xywh(origin.0 + line.x, baseline + offset, line.width, thickness);
                canvas.draw_rect(rule, &paint);
            }
            baseline += self.line_height;
        });
    }

    /// Places the `pieces` of a line in a box `width` px wide, as `text-align`
    /// does. Text is a set of runs that each shape as one: a line is one run,
    /// except where a tab or a justified space moves the next word on.
    fn arrange(&self, pieces: &[Piece], width: f32, ends_paragraph: bool) -> Line {
        let Some(last) = pieces.last() else {
            return Line {
                x: 0.0,
                width: 0.0,
                runs: Vec::new(),
            };
        };
        // Where the last word ends, without the spaces and tabs that hang
        // after it.
        let mut x = 0.0;
        for piece in &pieces[..pieces.len() - 1] {
            x = piece.end(x, self.tab_stops);
        }
        let natural_width = x + last.width;
        // Justified lines spread the free space over their spaces, but not
        // the spaces that hang at their end, and not at the end of a line that
        // a line break or the end of the text ends.
        let spaces = pieces[..pieces.len() - 1]
            .iter()
            .map(|piece| self.content[piece.word_end..piece.end].matches(' ').count())
            .sum::<usize>();
        let justify = self.align == TextAlign::Justify && !ends_paragraph && spaces > 0;
        let extra_per_space = if justify {
            ((width - natural_width) / spaces as f32).max(0.0)
        } else {
            0.0
        };
        let line_width = natural_width + extra_per_space * spaces as f32;
        let x = match self.align {
            TextAlign::Center => ((width - line_width) / 2.0).max(0.0),
            TextAlign::Right => (width - line_width).max(0.0),
            TextAlign::Left | TextAlign::Justify => 0.0,
        };

        let mut runs: Vec<Run> = Vec::new();
        let mut open: Option<(usize, f32)> = None;
        let mut at = 0.0;
        for piece in pieces {
            let (start, run_x) = *open.get_or_insert((piece.start, at));
            let ends_run = justify || !piece.tabs.is_empty();
            if ends_run || std::ptr::eq(piece, last) {
                if start < piece.word_end {
                    runs.push(Run {
                        range: start..piece.word_end,
                        x: run_x,
                    });
                }
                open = None;
            }
            at = piece.end(at, self.tab_stops);
            if justify {
                at += extra_per_space
                    * self.content[piece.word_end..piece.end].matches(' ').count() as f32;
            }
        }
        Line {
            x,
            width: line_width,
            runs,
        }
    }
}

/// A line of text, placed in its box.
struct Line {
    /// How far the line starts from the left edge of the box.
    x: f32,
    /// The width of the line without the spaces and tabs that hang after it.
    width: f32,
    runs: Vec<Run>,
}

/// Text on a line that is shaped as one, and starts `x` px after the line.
struct Run {
    range: Range<usize>,
    x: f32,
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
                    start: piece_start,
                    word_end: spaces_start,
                    end: piece_end,
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

/// The font size of `text` in CSS pixels, as written. A token reference counts
/// as left out, which is CSS's initial `font-size`.
fn font_size_in_px(text: &Text) -> f64 {
    match &text.font_size {
        TokenOr::Literal(size) => *size,
        TokenOr::Token(_) => f64::from(MEDIUM_FONT_SIZE),
    }
}

/// Whether a line of `line` px fits in the `width` px that it breaks at. Chrome
/// works in layout units, 1/128 px: it rounds the width down and the line up,
/// and lets a line be one unit wider than its width. So a line fits when it is
/// no wider than the width rounded down, plus a unit. That also takes in the
/// rounding error of a width that layout computed from the line's own width.
fn fits(line: f32, width: f32) -> bool {
    line * LAYOUT_UNITS_PER_PX <= (width * LAYOUT_UNITS_PER_PX).floor() + 1.0
}

/// A line height in CSS pixels, snapped down to a multiple of 1/64 px.
fn snap_line_height(height: f64) -> f32 {
    ((height * LINE_HEIGHT_UNITS_PER_PX).floor() / LINE_HEIGHT_UNITS_PER_PX) as f32
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
