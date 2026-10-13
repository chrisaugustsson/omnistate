//! Text: the fonts that text is shaped and measured with, and the lines that
//! Skia's paragraph layout breaks the text of a text node into.

use omni_core::design::{self, Text, TokenOr};
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle, TypefaceFontProvider,
};
use skia_safe::{Data, Font, FontHinting, FontMgr, FontStyle, Typeface};

/// Inter Regular, under the SIL Open Font License (`fonts/LICENSE.txt`).
/// Skia has no system fonts in the browser, so the font is compiled in.
const INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");

/// The family name of the bundled Inter.
const INTER: &str = "Inter";

/// The font size of a text whose `fontSize` is a token that is not resolved:
/// CSS's initial `font-size`, `medium`.
const MEDIUM_FONT_SIZE: f32 = 16.0;

/// How far a width may fall short of the text's widest line and still hold
/// it. Widths that layout computes from the text's own width can come back a
/// rounding error short, which must not break a line.
const FIT_TOLERANCE: f32 = 0.01;

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

/// The content of a text node, shaped in its font by Skia's paragraph layout,
/// whose lines break at any width the way a browser breaks them with CSS
/// `white-space: pre-wrap`: at spaces and at line breaks, never inside a word.
pub struct ShapedText {
    paragraph: Paragraph,
    /// The width of the widest word, which is as narrow as the text gets.
    min_content_width: f32,
    /// The width of the widest line when lines break only at line breaks.
    max_content_width: f32,
    /// The height of every line.
    line_height: f32,
    /// How far the baseline of the first line is below the top of the text,
    /// or `None` when the text is empty and so has no line.
    first_baseline: Option<f32>,
    /// Whether the content ends with a line break, which starts no line of its
    /// own in CSS.
    ends_with_line_break: bool,
    /// The width the paragraph's lines were last broken at.
    broken_at: f32,
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

        let mut style = TextStyle::new();
        style
            .set_font_families(&[family])
            .set_font_size(font_size)
            .set_font_style(font_style)
            .set_letter_spacing(
                text.letter_spacing
                    .as_ref()
                    .and_then(literal)
                    .unwrap_or(0.0),
            );
        if let Some(multiple) = line_height_multiple {
            // CSS splits the leading evenly above and below the glyphs.
            style
                .set_height(multiple)
                .set_height_override(true)
                .set_half_leading(true);
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
        paragraph.layout(f32::INFINITY);

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

        ShapedText {
            min_content_width: paragraph.min_intrinsic_width(),
            max_content_width: paragraph.max_intrinsic_width(),
            paragraph,
            line_height,
            first_baseline: (!text.content.is_empty()).then_some(first_baseline),
            ends_with_line_break: text.content.ends_with('\n'),
            broken_at: f32::INFINITY,
        }
    }

    /// The CSS min-content width: the width of the widest word.
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
    pub fn height(&mut self, width: f32) -> f32 {
        self.line_count(width) as f32 * self.line_height
    }

    /// How many lines the text breaks into at `width`.
    fn line_count(&mut self, width: f32) -> usize {
        let width = if width + FIT_TOLERANCE >= self.max_content_width {
            f32::INFINITY
        } else {
            // Skia breaks a word that is wider than the line into pieces,
            // where CSS lets it overflow the line. A line at least as wide as
            // the widest word keeps every word whole.
            width.max(self.min_content_width)
        };
        if width != self.broken_at {
            self.paragraph.layout(width);
            self.broken_at = width;
        }
        // Skia starts an empty line after a final line break; CSS does not.
        self.paragraph
            .line_number()
            .saturating_sub(usize::from(self.ends_with_line_break))
    }
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
