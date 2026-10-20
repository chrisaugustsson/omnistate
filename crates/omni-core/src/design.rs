//! Design files: pages, boards, frames, text, images, vectors and component
//! instances, as typed nodes with explicit design properties, and the JSON
//! form that the renderer, the API and the CLI share.
//!
//! A design node is a JSON object with an `id` (8 lowercase Crockford base32
//! characters, such as `k3f9x2qa`), a `type`, a `name` and `visible`, which is
//! `true` when left out. Pages, boards and frames hold ordered `children`: a
//! page holds boards, and boards sit only on pages.
//!
//! - Placement: `x` and `y` place the boards of a page, the children of a
//!   `none` layout, and children taken out of a flex or grid flow with
//!   `"absolute": true`. Layout computes every other position when rendering,
//!   so no computed position, box or transform is ever stored. Grid children
//!   may set `gridColumn` and `gridRow` to `{"start": <line>, "span": <tracks>}`,
//!   and align themselves in their grid areas with `justifySelf` and
//!   `alignSelf` (`start`, `center`, `end` or `stretch`).
//! - Margin: every node below a page may set a `margin`, a length per side
//!   (`top`, `right`, `bottom`, `left`) as CSS `margin`, which may be negative.
//! - Size: every node below a page sizes each axis with `width` and `height`,
//!   as `{"fixed": <length>}`, `"hug"` or `"fill"`, with optional `minWidth`,
//!   `maxWidth`, `minHeight` and `maxHeight`. An instance may leave its size
//!   to its component.
//! - Layout: boards and frames state a `layout`: `{"mode": "none"}`, a flex
//!   layout (`direction`, `wrap`, `gap`, `padding`, `alignItems` and
//!   `justifyContent`) or a grid layout (`columns`, `rows`, `gap` and
//!   `padding`). Keywords are the CSS ones, and a property left out keeps its
//!   CSS initial value.
//! - Style: `fills` (solid, linear gradient or image), `strokes` (which may be
//!   dashed with a `dash` of a `length` and a `gap`, and on frames and boards
//!   set `align` to `inside`, `center` or `outside`), `radius` per
//!   corner, drop and inner `shadows`, `opacity` from 0 to 1, and `clip`.
//! - Text: plain-text `content` in one style: `fontFamily`, `fontSize`,
//!   `fontWeight`, `fontStyle`, `lineHeight` (a multiple of the font size),
//!   `letterSpacing`, `textAlign`, `textDecoration` and `color`.
//! - Images show an `attachment:<id>`, vectors hold SVG `path` data (never SVG
//!   markup), and instances name their `component`.
//!
//! Lengths are CSS pixels and colors are `#rrggbb` or `#rrggbbaa`. Every
//! color, every length except `x` and `y`, every opacity and font value, and a
//! whole shadow may instead be a token reference such as `$color.surface`.
//! Literals stay in the ranges CSS allows: sizes, min and max sizes, gaps,
//! padding, grid tracks, stroke widths, dash lengths and gaps, corner radii, shadow blurs, font sizes
//! and line heights are 0 or more, opacity goes from 0 to 1 and font weights
//! from 1 to 1000, while `x`, `y`, margins, shadow offsets and spread, letter spacing
//! and gradient angles may be negative. The format is closed: unknown
//! properties, `null` values and properties that a node's type has no use for
//! are rejected. Serializing writes back every property a node sets, and
//! every number as the same double, so a round trip is lossless.

/// Defines an enum of keywords, written in JSON as strings.
macro_rules! keywords {
    (
        $(#[$attr:meta])*
        $vis:vis enum $name:ident {
            $($(#[$variant_attr:meta])* $variant:ident = $keyword:literal,)+
        }
    ) => {
        $(#[$attr])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        $vis enum $name {
            $($(#[$variant_attr])* $variant,)+
        }

        impl $name {
            /// Every keyword, in order.
            $vis const KEYWORDS: &[&str] = &[$($keyword),+];

            /// The keyword as written in JSON.
            $vis fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $keyword,)+
                }
            }

            fn from_keyword(keyword: &str) -> Option<Self> {
                match keyword {
                    $($keyword => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct KeywordVisitor;

                impl ::serde::de::Visitor<'_> for KeywordVisitor {
                    type Value = $name;

                    fn expecting(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                        write!(f, "one of {}", $crate::design::json::quoted($name::KEYWORDS))
                    }

                    fn visit_str<E: ::serde::de::Error>(self, value: &str) -> Result<$name, E> {
                        $name::from_keyword(value)
                            .ok_or_else(|| E::invalid_value(::serde::de::Unexpected::Str(value), &self))
                    }
                }

                deserializer.deserialize_str(KeywordVisitor)
            }
        }
    };
}

mod id;
mod json;
mod layout;
mod node;
mod path;
mod style;
pub mod tags;
pub mod tokens;
mod value;

pub use id::{InvalidNodeId, NodeId};
pub use layout::{
    AlignItems, FlexDirection, FlexLayout, FlexWrap, Gap, GridLayout, GridPlacement,
    JustifyContent, Layout, LayoutMode, Margin, Padding, SelfAlignment, Sizing, Track,
};
pub use node::{
    Frame, Image, Instance, Node, NodeData, NodeKind, Page, Placement, Size, Text, Vector,
};
pub use path::PathData;
pub use style::{
    Dash, Fill, FontStyle, GradientStop, ImageFit, Radius, Shadow, ShadowKind, Stroke, StrokeAlign,
    TextAlign, TextDecoration,
};
pub use value::{AttachmentRef, Color, FontFamily, InvalidValue, Length, TokenOr, TokenRef};
