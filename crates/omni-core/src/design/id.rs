//! Node IDs.

use super::json::parse_str;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// The alphabet of node IDs: Crockford's base32 in lowercase, which leaves out
/// `i`, `l`, `o` and `u`.
const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// A node ID: 8 lowercase base32 characters, such as `k3f9x2qa`, unique within
/// a project.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId([u8; 8]);

impl NodeId {
    /// The ID as written.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("node IDs are ASCII")
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NodeId").field(&self.as_str()).finish()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a string is not a node ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidNodeId {
    id: String,
}

impl fmt::Display for InvalidNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a node ID: node IDs are 8 lowercase base32 characters, 0-9 and a-z without i, l, o and u",
            self.id
        )
    }
}

impl std::error::Error for InvalidNodeId {}

impl FromStr for NodeId {
    type Err = InvalidNodeId;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        match <[u8; 8]>::try_from(id.as_bytes()) {
            Ok(bytes) if bytes.iter().all(|byte| ALPHABET.contains(byte)) => Ok(NodeId(bytes)),
            _ => Err(InvalidNodeId { id: id.to_string() }),
        }
    }
}

impl Serialize for NodeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        parse_str(deserializer, "a node ID such as `k3f9x2qa`")
    }
}
