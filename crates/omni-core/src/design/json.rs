//! serde helpers for the closed JSON form of designs.

use serde::de::value::MapAccessDeserializer;
use serde::de::{self, Deserialize, Deserializer, MapAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

/// Reads a property that is present. `null` is not a value of the format: a
/// property is written with a value or left out.
pub(crate) fn some<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Like [`some`], for a value written as a JSON object.
pub(crate) fn some_object<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Object::deserialize(deserializer).map(|Object(value)| Some(value))
}

/// Like [`some`], for a list of values written as JSON objects.
pub(crate) fn some_objects<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let objects = Vec::<Object<T>>::deserialize(deserializer)?;
    Ok(Some(
        objects.into_iter().map(|Object(value)| value).collect(),
    ))
}

/// A `T` read from a JSON object only. serde's derived structs also read an
/// array of their field values, which the format does not allow.
pub(crate) struct Object<T>(pub(crate) T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
            type Value = T;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map))
            }
        }

        deserializer
            .deserialize_map(ObjectVisitor(PhantomData))
            .map(Object)
    }
}

/// Reads a JSON string through `T`'s `FromStr`.
pub(crate) fn parse_str<'de, D, T>(deserializer: D, expecting: &'static str) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    struct ParseVisitor<T>(&'static str, PhantomData<T>);

    impl<T: FromStr> Visitor<'_> for ParseVisitor<T>
    where
        T::Err: fmt::Display,
    {
        type Value = T;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }

        fn visit_str<E: de::Error>(self, text: &str) -> Result<T, E> {
            text.parse().map_err(E::custom)
        }
    }

    deserializer.deserialize_str(ParseVisitor(expecting, PhantomData))
}

/// Reads a number that no token reference stands in for, such as `x`.
pub(crate) fn read_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    struct NumberVisitor;

    impl Visitor<'_> for NumberVisitor {
        type Value = f64;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a number")
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<f64, E> {
            Ok(value as f64)
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<f64, E> {
            Ok(value as f64)
        }

        fn visit_f64<E: de::Error>(self, value: f64) -> Result<f64, E> {
            Ok(value)
        }
    }

    deserializer.deserialize_f64(NumberVisitor)
}

/// Like [`read_number`], for a property that may be left out.
pub(crate) fn some_number<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    read_number(deserializer).map(Some)
}

/// Writes a number, as an integer when it is one: `320` rather than `320.0`.
pub(crate) fn write_number<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    // Integers below 2^53 convert to i64 and back without loss.
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

/// A number that serializes with [`write_number`].
pub(crate) struct Number(pub(crate) f64);

impl Serialize for Number {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        write_number(&self.0, serializer)
    }
}

/// Writes `key` when the property is set.
pub(crate) fn entry<M, T>(map: &mut M, key: &'static str, value: Option<&T>) -> Result<(), M::Error>
where
    M: SerializeMap,
    T: Serialize + ?Sized,
{
    match value {
        Some(value) => map.serialize_entry(key, value),
        None => Ok(()),
    }
}

/// The first of `properties` that is set, to name in an error.
pub(crate) fn first_set<const N: usize>(
    properties: [(&'static str, bool); N],
) -> Option<&'static str> {
    properties
        .into_iter()
        .find_map(|(key, set)| set.then_some(key))
}

/// Keywords quoted for a message: `` `row`, `column` ``.
pub(crate) fn quoted(keywords: &[&str]) -> String {
    keywords
        .iter()
        .map(|keyword| format!("`{keyword}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
