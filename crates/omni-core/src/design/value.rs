//! Style and layout values: literals or token references, colors, font
//! families and attachments.

use super::json::{parse_str, write_number};
use super::style::Shadow;
use serde::de::value::MapAccessDeserializer;
use serde::de::{self, Deserialize, Deserializer, Expected, MapAccess, Unexpected, Visitor};
use serde::{Serialize, Serializer};
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

/// A style or layout value: a literal, or a reference to the design token
/// that stands in for it, such as `$color.surface`.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenOr<T> {
    Literal(T),
    Token(TokenRef),
}

/// A length in CSS pixels, or a token reference.
pub type Length = TokenOr<f64>;

mod sealed {
    use serde::Serializer;
    use serde::de::{self, Expected, MapAccess, Unexpected};

    /// A literal that a token reference can stand in for, and its JSON form.
    pub trait Literal: Sized {
        /// What the value looks like, for error messages.
        const EXPECTING: &str;

        fn write<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error>;

        fn from_number<E: de::Error>(
            unexpected: Unexpected<'_>,
            _value: f64,
            expected: &dyn Expected,
        ) -> Result<Self, E> {
            Err(E::invalid_type(unexpected, expected))
        }

        fn from_text<E: de::Error>(text: &str, expected: &dyn Expected) -> Result<Self, E> {
            Err(E::invalid_type(Unexpected::Str(text), expected))
        }

        fn from_object<'de, A: MapAccess<'de>>(
            _map: A,
            expected: &dyn Expected,
        ) -> Result<Self, A::Error> {
            Err(de::Error::invalid_type(Unexpected::Map, expected))
        }
    }
}

use sealed::Literal;

impl Literal for f64 {
    const EXPECTING: &str = "a number or a token reference such as `$space.md`";

    fn write<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        write_number(self, serializer)
    }

    fn from_number<E: de::Error>(
        _unexpected: Unexpected<'_>,
        value: f64,
        _expected: &dyn Expected,
    ) -> Result<Self, E> {
        Ok(value)
    }
}

impl Literal for Color {
    const EXPECTING: &str =
        "a color such as `#2563eb` or a token reference such as `$color.surface`";

    fn write<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.serialize(serializer)
    }

    fn from_text<E: de::Error>(text: &str, _expected: &dyn Expected) -> Result<Self, E> {
        text.parse().map_err(E::custom)
    }
}

impl Literal for FontFamily {
    const EXPECTING: &str =
        "a font family such as `Inter` or a token reference such as `$font.family.sans`";

    fn write<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.serialize(serializer)
    }

    fn from_text<E: de::Error>(text: &str, _expected: &dyn Expected) -> Result<Self, E> {
        text.parse().map_err(E::custom)
    }
}

impl Literal for Shadow {
    const EXPECTING: &str = "a shadow or a token reference such as `$shadow.card`";

    fn write<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.serialize(serializer)
    }

    fn from_object<'de, A: MapAccess<'de>>(
        map: A,
        _expected: &dyn Expected,
    ) -> Result<Self, A::Error> {
        Shadow::deserialize(MapAccessDeserializer::new(map))
    }
}

impl<T: Literal> Serialize for TokenOr<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            TokenOr::Literal(literal) => literal.write(serializer),
            TokenOr::Token(token) => token.serialize(serializer),
        }
    }
}

impl<'de, T: Literal> Deserialize<'de> for TokenOr<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(TokenOrVisitor(PhantomData))
    }
}

struct TokenOrVisitor<T>(PhantomData<T>);

impl<'de, T: Literal> Visitor<'de> for TokenOrVisitor<T> {
    type Value = TokenOr<T>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(T::EXPECTING)
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        T::from_number(Unexpected::Unsigned(value), value as f64, &self).map(TokenOr::Literal)
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        T::from_number(Unexpected::Signed(value), value as f64, &self).map(TokenOr::Literal)
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        T::from_number(Unexpected::Float(value), value, &self).map(TokenOr::Literal)
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Self::Value, E> {
        if text.starts_with('$') {
            text.parse().map(TokenOr::Token).map_err(E::custom)
        } else if text.len() > 1 && text.starts_with('{') && text.ends_with('}') {
            Err(E::custom(format_args!(
                "`{text}` is token file syntax: designs refer to tokens as `${}`",
                &text[1..text.len() - 1]
            )))
        } else {
            T::from_text(text, &self).map(TokenOr::Literal)
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        T::from_object(map, &self).map(TokenOr::Literal)
    }
}

/// Why a string is not a value of the design format, such as a color or a
/// token reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidValue(pub(crate) String);

impl fmt::Display for InvalidValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidValue {}

/// A reference to a design token: `$` and the token's dot-separated path, such
/// as `$color.surface`. Token files write the same reference as
/// `{color.surface}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct TokenRef(String);

impl TokenRef {
    /// The reference as written, such as `$color.surface`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The token's path, such as `color.surface`.
    pub fn path(&self) -> &str {
        &self.0[1..]
    }
}

impl FromStr for TokenRef {
    type Err = InvalidValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let is_name = |name: &str| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        };
        match text.strip_prefix('$') {
            Some(path) if path.split('.').all(is_name) => Ok(TokenRef(text.to_string())),
            _ => Err(InvalidValue(format!(
                "`{text}` is not a token reference: write `$` and the token's dot-separated path, such as `$color.surface`"
            ))),
        }
    }
}

impl<'de> Deserialize<'de> for TokenRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        parse_str(deserializer, "a token reference such as `$color.surface`")
    }
}

/// A color written `#rrggbb` or `#rrggbbaa` in hexadecimal digits, kept as
/// written.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Color(String);

impl Color {
    /// The color as written, such as `#2563eb`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Color {
    type Err = InvalidValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.strip_prefix('#') {
            Some(hex)
                if matches!(hex.len(), 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit()) =>
            {
                Ok(Color(text.to_string()))
            }
            _ => Err(InvalidValue(format!(
                "`{text}` is not a color: write `#rrggbb` or `#rrggbbaa`, such as `#2563eb`"
            ))),
        }
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        parse_str(deserializer, "a color such as `#2563eb`")
    }
}

/// The name of a font family, such as `Inter`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct FontFamily(String);

impl FontFamily {
    /// The family name, such as `IBM Plex Sans`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for FontFamily {
    type Err = InvalidValue;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        if name.is_empty() || name.trim() != name || name.chars().any(char::is_control) {
            return Err(InvalidValue(format!(
                "`{name}` is not a font family: write the family's name, such as `Inter`, without surrounding spaces"
            )));
        }
        Ok(FontFamily(name.to_string()))
    }
}

impl<'de> Deserialize<'de> for FontFamily {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        parse_str(deserializer, "a font family such as `Inter`")
    }
}

/// A file uploaded to the workspace, written `attachment:<id>`. Designs show
/// images from attachments only, never from URLs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct AttachmentRef(String);

impl AttachmentRef {
    const PREFIX: &str = "attachment:";

    /// The reference as written, such as `attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The attachment's ID.
    pub fn id(&self) -> &str {
        &self.0[Self::PREFIX.len()..]
    }
}

impl FromStr for AttachmentRef {
    type Err = InvalidValue;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.strip_prefix(Self::PREFIX) {
            Some(id) if is_uuid(id) => Ok(AttachmentRef(text.to_string())),
            _ => Err(InvalidValue(format!(
                "`{text}` is not an attachment: images are files uploaded to the workspace, written `attachment:<id>`"
            ))),
        }
    }
}

impl<'de> Deserialize<'de> for AttachmentRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        parse_str(deserializer, "an attachment such as `attachment:<id>`")
    }
}

/// Whether `id` is a UUID in its hyphenated form, as attachment IDs are.
fn is_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}
