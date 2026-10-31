//! Style properties: fills, strokes, corner radii, shadows and text styles.

use super::json::{
    Object, first_set, read_number, some, some_number, some_object, some_objects, write_number,
};
use super::value::{AttachmentRef, Color, Length, TokenOr, non_negative, some_non_negative};
use serde::{Deserialize, Serialize};

keywords! {
    enum FillKind {
        Solid = "solid",
        LinearGradient = "linear-gradient",
        Image = "image",
    }
}

/// A paint that fills a box or a vector. Fills are painted in list order, so
/// the last one ends up on top.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", try_from = "Object<RawFill>")]
pub enum Fill {
    /// One color.
    Solid { color: TokenOr<Color> },
    /// A CSS linear gradient. `angle` is in degrees, clockwise from pointing
    /// up, and the stops give the color at positions from 0 to 1.
    LinearGradient {
        #[serde(serialize_with = "write_number")]
        angle: f64,
        stops: Vec<GradientStop>,
    },
    /// An image that covers its box or fits inside it.
    Image { image: AttachmentRef, fit: ImageFit },
}

/// Every fill property, before the fill's type says which ones apply.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFill {
    #[serde(rename = "type")]
    kind: FillKind,
    #[serde(default, deserialize_with = "some")]
    color: Option<TokenOr<Color>>,
    #[serde(default, deserialize_with = "some_number")]
    angle: Option<f64>,
    #[serde(default, deserialize_with = "some_objects")]
    stops: Option<Vec<GradientStop>>,
    #[serde(default, deserialize_with = "some")]
    image: Option<AttachmentRef>,
    #[serde(default, deserialize_with = "some")]
    fit: Option<ImageFit>,
}

impl TryFrom<Object<RawFill>> for Fill {
    type Error = String;

    fn try_from(Object(mut raw): Object<RawFill>) -> Result<Self, String> {
        fn required<T>(kind: FillKind, key: &str, value: Option<T>) -> Result<T, String> {
            value.ok_or_else(|| format!("{kind} fills need `{key}`"))
        }

        let kind = raw.kind;
        let fill = match kind {
            FillKind::Solid => Fill::Solid {
                color: required(kind, "color", raw.color.take())?,
            },
            FillKind::LinearGradient => {
                let angle = required(kind, "angle", raw.angle.take())?;
                let stops = required(kind, "stops", raw.stops.take())?;
                if stops.len() < 2 {
                    return Err(format!("{kind} fills need at least two stops"));
                }
                Fill::LinearGradient { angle, stops }
            }
            FillKind::Image => Fill::Image {
                image: required(kind, "image", raw.image.take())?,
                fit: required(kind, "fit", raw.fit.take())?,
            },
        };
        let unused = first_set([
            ("color", raw.color.is_some()),
            ("angle", raw.angle.is_some()),
            ("stops", raw.stops.is_some()),
            ("image", raw.image.is_some()),
            ("fit", raw.fit.is_some()),
        ]);
        match unused {
            Some(key) => Err(format!("{kind} fills have no `{key}`")),
            None => Ok(fill),
        }
    }
}

/// The color at one position along a gradient, from 0 at its start to 1 at
/// its end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradientStop {
    #[serde(serialize_with = "write_number")]
    #[serde(deserialize_with = "read_number")]
    pub position: f64,
    pub color: TokenOr<Color>,
}

keywords! {
    /// How an image fits its box, as CSS `object-fit`.
    pub enum ImageFit {
        Cover = "cover",
        Contain = "contain",
    }
}

/// A line along the edge of a box or along a vector's path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    pub color: TokenOr<Color>,
    #[serde(deserialize_with = "non_negative")]
    pub width: Length,
    /// Where the stroke sits on the edge of a frame or a board, or on the path
    /// of a vector. Left out, it lies inside. The stroke of a path with an open
    /// subpath is always centered.
    #[serde(default, deserialize_with = "some")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<StrokeAlign>,
    /// Draws the stroke as dashes. Left out, the stroke is solid.
    #[serde(default, deserialize_with = "some_object")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dash: Option<Dash>,
    /// How the stroke of a vector ends, and each of its dashes. Left out, it
    /// is `butt`. A frame or a board draws its stroke the same whatever it is.
    #[serde(default, deserialize_with = "some")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<StrokeCap>,
}

keywords! {
    /// How the ends of a stroke are shaped.
    pub enum StrokeCap {
        /// Square off at the end of the path.
        Butt = "butt",
        /// A half disc past the end, of the stroke's width.
        Round = "round",
        /// A half square past the end, of half the stroke's width.
        Square = "square",
    }
}

keywords! {
    /// Where the stroke of a frame or a board sits on the edge of its box.
    pub enum StrokeAlign {
        /// Inside the edge, like a CSS `inset` border.
        Inside = "inside",
        /// Half inside the edge and half outside it.
        Center = "center",
        /// Outside the edge, like a CSS `outline`.
        Outside = "outside",
    }
}

/// The dashes of a stroke, as SVG `stroke-dasharray: <length> <gap>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dash {
    /// The length of each dash.
    #[serde(deserialize_with = "non_negative")]
    pub length: Length,
    /// The space between dashes.
    #[serde(deserialize_with = "non_negative")]
    pub gap: Length,
}

/// The radius of each corner of a box. A corner left out is square.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Radius {
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_left: Option<Length>,
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_right: Option<Length>,
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom_right: Option<Length>,
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom_left: Option<Length>,
}

keywords! {
    /// Whether a shadow falls outside a box or inside it.
    pub enum ShadowKind {
        Drop = "drop",
        Inner = "inner",
    }
}

/// A shadow, as one CSS `box-shadow`. `blur` and `spread` are 0 when they are
/// left out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Shadow {
    #[serde(rename = "type")]
    pub kind: ShadowKind,
    pub color: TokenOr<Color>,
    pub offset_x: Length,
    pub offset_y: Length,
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blur: Option<Length>,
    #[serde(default, deserialize_with = "some")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spread: Option<Length>,
}

keywords! {
    /// Upright or italic text, as CSS `font-style`.
    pub enum FontStyle {
        Normal = "normal",
        Italic = "italic",
    }
}

keywords! {
    /// How text lines align in their box, as CSS `text-align`.
    pub enum TextAlign {
        Left = "left",
        Center = "center",
        Right = "right",
        Justify = "justify",
    }
}

keywords! {
    /// A line drawn with text, as CSS `text-decoration-line`.
    pub enum TextDecoration {
        None = "none",
        Underline = "underline",
        LineThrough = "line-through",
    }
}
