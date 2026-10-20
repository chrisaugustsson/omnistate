//! Layout properties, which follow CSS flexbox, grid and absolute positioning.

use super::json::{Number, Object, first_set, read_number, some, some_number, some_object};
use super::value::{Length, non_negative, some_non_negative};
use serde::de::value::MapAccessDeserializer;
use serde::de::{self, Deserializer, MapAccess, Unexpected, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;

keywords! {
    /// How a board or frame places its children.
    pub enum LayoutMode {
        /// At their own `x` and `y`.
        None = "none",
        /// In a CSS flexbox.
        Flex = "flex",
        /// On a CSS grid.
        Grid = "grid",
    }
}

keywords! {
    /// The main axis of a flex layout, as CSS `flex-direction`.
    pub enum FlexDirection {
        Row = "row",
        Column = "column",
        RowReverse = "row-reverse",
        ColumnReverse = "column-reverse",
    }
}

keywords! {
    /// Whether a flex layout wraps its children onto more lines, as CSS
    /// `flex-wrap`.
    pub enum FlexWrap {
        NoWrap = "nowrap",
        Wrap = "wrap",
        WrapReverse = "wrap-reverse",
    }
}

keywords! {
    /// How a flex layout aligns its children on the cross axis, as CSS
    /// `align-items`.
    pub enum AlignItems {
        Start = "start",
        Center = "center",
        End = "end",
        Stretch = "stretch",
        Baseline = "baseline",
    }
}

keywords! {
    /// How a flex layout spreads its children along the main axis, as CSS
    /// `justify-content`.
    pub enum JustifyContent {
        Start = "start",
        Center = "center",
        End = "end",
        SpaceBetween = "space-between",
        SpaceAround = "space-around",
        SpaceEvenly = "space-evenly",
    }
}

/// How a board or frame lays out its children. A property left out keeps its
/// CSS initial value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", try_from = "Object<RawLayout>")]
pub enum Layout {
    /// No layout: children are placed at their own `x` and `y`.
    None,
    /// CSS `display: flex`.
    Flex(FlexLayout),
    /// CSS `display: grid`.
    Grid(GridLayout),
}

impl Layout {
    pub fn mode(&self) -> LayoutMode {
        match self {
            Layout::None => LayoutMode::None,
            Layout::Flex(_) => LayoutMode::Flex,
            Layout::Grid(_) => LayoutMode::Grid,
        }
    }
}

/// The properties of a flex layout.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlexLayout {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<FlexDirection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap: Option<FlexWrap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<Gap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Padding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align_items: Option<AlignItems>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justify_content: Option<JustifyContent>,
}

/// The properties of a grid layout: its column and row tracks, as CSS
/// `grid-template-columns` and `grid-template-rows`, and its spacing.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GridLayout {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<Track>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Track>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<Gap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Padding>,
}

/// Every layout property, before the mode says which ones apply.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawLayout {
    mode: LayoutMode,
    #[serde(default, deserialize_with = "some")]
    direction: Option<FlexDirection>,
    #[serde(default, deserialize_with = "some")]
    wrap: Option<FlexWrap>,
    #[serde(default, deserialize_with = "some_object")]
    gap: Option<Gap>,
    #[serde(default, deserialize_with = "some_object")]
    padding: Option<Padding>,
    #[serde(default, deserialize_with = "some")]
    align_items: Option<AlignItems>,
    #[serde(default, deserialize_with = "some")]
    justify_content: Option<JustifyContent>,
    #[serde(default, deserialize_with = "some")]
    columns: Option<Vec<Track>>,
    #[serde(default, deserialize_with = "some")]
    rows: Option<Vec<Track>>,
}

impl TryFrom<Object<RawLayout>> for Layout {
    type Error = String;

    fn try_from(Object(raw): Object<RawLayout>) -> Result<Self, String> {
        let flex_only = [
            ("direction", raw.direction.is_some()),
            ("wrap", raw.wrap.is_some()),
            ("alignItems", raw.align_items.is_some()),
            ("justifyContent", raw.justify_content.is_some()),
        ];
        let grid_only = [
            ("columns", raw.columns.is_some()),
            ("rows", raw.rows.is_some()),
        ];
        let spacing = [
            ("gap", raw.gap.is_some()),
            ("padding", raw.padding.is_some()),
        ];
        let unused = match raw.mode {
            LayoutMode::None => first_set(flex_only)
                .or(first_set(grid_only))
                .or(first_set(spacing)),
            LayoutMode::Flex => first_set(grid_only),
            LayoutMode::Grid => first_set(flex_only),
        };
        if let Some(key) = unused {
            return Err(format!("{} layouts have no `{key}`", raw.mode));
        }
        Ok(match raw.mode {
            LayoutMode::None => Layout::None,
            LayoutMode::Flex => Layout::Flex(FlexLayout {
                direction: raw.direction,
                wrap: raw.wrap,
                gap: raw.gap,
                padding: raw.padding,
                align_items: raw.align_items,
                justify_content: raw.justify_content,
            }),
            LayoutMode::Grid => Layout::Grid(GridLayout {
                columns: raw.columns,
                rows: raw.rows,
                gap: raw.gap,
                padding: raw.padding,
            }),
        })
    }
}

/// The space between rows and between columns, as CSS `row-gap` and
/// `column-gap`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<Length>,
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<Length>,
}

/// The space inside each edge of a box, as CSS `padding`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Padding {
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<Length>,
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Length>,
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<Length>,
    #[serde(default, deserialize_with = "some_non_negative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Length>,
}

/// The space outside each edge of a node, as CSS `margin`. Unlike padding, a
/// margin may be negative.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Margin {
    #[serde(default, deserialize_with = "some")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<Length>,
    #[serde(default, deserialize_with = "some")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Length>,
    #[serde(default, deserialize_with = "some")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<Length>,
    #[serde(default, deserialize_with = "some")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Length>,
}

keywords! {
    /// How a child of a grid aligns itself in its grid area, as CSS
    /// `justify-self` (across) and `align-self` (down).
    pub enum SelfAlignment {
        Start = "start",
        Center = "center",
        End = "end",
        Stretch = "stretch",
    }
}

/// How a node sizes one axis.
#[derive(Clone, Debug, PartialEq)]
pub enum Sizing {
    /// A fixed length: `{"fixed": 320}`.
    Fixed(Length),
    /// As small as the content allows: `"hug"`.
    Hug,
    /// As large as the parent allows: `"fill"`.
    Fill,
}

impl Serialize for Sizing {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Sizing::Fixed(length) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("fixed", length)?;
                map.end()
            }
            Sizing::Hug => serializer.serialize_str("hug"),
            Sizing::Fill => serializer.serialize_str("fill"),
        }
    }
}

impl<'de> Deserialize<'de> for Sizing {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SizingVisitor;

        impl<'de> Visitor<'de> for SizingVisitor {
            type Value = Sizing;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("`\"hug\"`, `\"fill\"` or a fixed length such as `{\"fixed\": 320}`")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Sizing, E> {
                match text {
                    "hug" => Ok(Sizing::Hug),
                    "fill" => Ok(Sizing::Fill),
                    _ => Err(E::invalid_value(Unexpected::Str(text), &self)),
                }
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Sizing, A::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Fixed {
                    #[serde(deserialize_with = "non_negative")]
                    fixed: Length,
                }

                let Fixed { fixed } = Fixed::deserialize(MapAccessDeserializer::new(map))?;
                Ok(Sizing::Fixed(fixed))
            }
        }

        deserializer.deserialize_any(SizingVisitor)
    }
}

/// A column or row of a grid layout.
#[derive(Clone, Debug, PartialEq)]
pub enum Track {
    /// A fixed length: `{"fixed": 240}`.
    Fixed(Length),
    /// A share of the free space, as the CSS `fr` unit: `{"fr": 1}`.
    Fr(f64),
    /// Sized by its content: `"auto"`.
    Auto,
}

impl Serialize for Track {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Track::Fixed(length) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("fixed", length)?;
                map.end()
            }
            Track::Fr(fr) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("fr", &Number(*fr))?;
                map.end()
            }
            Track::Auto => serializer.serialize_str("auto"),
        }
    }
}

impl<'de> Deserialize<'de> for Track {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TrackVisitor;

        impl<'de> Visitor<'de> for TrackVisitor {
            type Value = Track;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a grid track: `{\"fixed\": <length>}`, `{\"fr\": <number>}` or `\"auto\"`",
                )
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Track, E> {
                match text {
                    "auto" => Ok(Track::Auto),
                    _ => Err(E::invalid_value(Unexpected::Str(text), &self)),
                }
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Track, A::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct RawTrack {
                    #[serde(default, deserialize_with = "some_non_negative")]
                    fixed: Option<Length>,
                    #[serde(default, deserialize_with = "some_number")]
                    fr: Option<f64>,
                }

                match RawTrack::deserialize(MapAccessDeserializer::new(map))? {
                    RawTrack {
                        fixed: Some(fixed),
                        fr: None,
                    } => Ok(Track::Fixed(fixed)),
                    RawTrack {
                        fixed: None,
                        fr: Some(fr),
                    } if fr >= 0.0 => Ok(Track::Fr(fr)),
                    RawTrack {
                        fixed: None,
                        fr: Some(fr),
                    } => Err(de::Error::custom(format_args!(
                        "a grid track takes a share of the free space, so `fr` is 0 or more, not {fr}"
                    ))),
                    _ => Err(de::Error::invalid_value(Unexpected::Map, &self)),
                }
            }
        }

        deserializer.deserialize_any(TrackVisitor)
    }
}

/// The grid lines a child of a grid layout is placed on, as CSS
/// `grid-column` and `grid-row`: `{"start": 2, "span": 3}`. A child without a
/// `start` is placed automatically, and `span` is 1 when it is left out.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridPlacement {
    /// The line the child starts at. Lines count from 1 at the start of the
    /// grid, or from -1 at its end.
    #[serde(default, deserialize_with = "grid_line")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<i16>,
    /// How many tracks the child spans.
    #[serde(default, deserialize_with = "grid_span")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<u16>,
}

fn grid_line<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i16>, D::Error> {
    let line = integer(deserializer)?;
    match i16::try_from(line) {
        Ok(line) if line != 0 => Ok(Some(line)),
        _ => Err(de::Error::custom(format_args!(
            "grid line {line} does not exist: lines count from 1 at the start of the grid, or from -1 at its end"
        ))),
    }
}

fn grid_span<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u16>, D::Error> {
    let span = integer(deserializer)?;
    match u16::try_from(span) {
        Ok(span) if span != 0 => Ok(Some(span)),
        _ => Err(de::Error::custom(format_args!(
            "a grid child spans 1 track or more, not {span}"
        ))),
    }
}

/// Reads a whole number, such as `2` or `2.0`.
fn integer<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    let value = read_number(deserializer)?;
    if value.fract() != 0.0 {
        return Err(de::Error::invalid_value(
            Unexpected::Float(value),
            &"a whole number",
        ));
    }
    // The cast saturates, and callers reject numbers out of their range.
    Ok(value as i64)
}
