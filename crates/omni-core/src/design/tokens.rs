//! Design tokens: the token tree of a project's design system doc, in the W3C
//! Design Tokens (DTCG) format, with one value per mode
//! (docs/formats/tokens.md).

use super::value::{AttachmentRef, TokenRef};
use serde_json::Value;
use std::fmt;

/// A checked token tree: every reference resolves, no reference loops, and
/// every token has a value for each of the design system's modes.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenTree(());

/// Why a token tree was refused; the message names the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenError {
    message: String,
}

impl TokenTree {
    /// Reads and checks a DTCG token tree.
    pub fn from_json(_json: &Value) -> Result<TokenTree, TokenError> {
        todo!()
    }

    /// The tree as DTCG JSON, as it was read.
    pub fn to_json(&self) -> Value {
        todo!()
    }

    /// The design system's modes, in the order the tree lists them.
    pub fn modes(&self) -> &[String] {
        todo!()
    }

    /// The value of the token that a design refers to, such as
    /// `$color.surface`, in `mode`, with every reference followed.
    pub fn resolve(&self, _token: &TokenRef, _mode: &str) -> Option<Value> {
        todo!()
    }

    /// The tree for one mode: each token's `$value` is its value in `mode`.
    pub fn for_mode(&self, _mode: &str) -> Option<Value> {
        todo!()
    }

    /// Every font file that the typography tokens use.
    pub fn font_files(&self) -> Vec<AttachmentRef> {
        todo!()
    }
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TokenError {}
