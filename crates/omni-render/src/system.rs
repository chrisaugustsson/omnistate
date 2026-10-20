//! The design system that a design's instances and token references resolve
//! against (US-032): the components of a design system version and its token
//! tree, which has a value for each of its modes.

use omni_core::design::{NodeAddress, NodeId, TokenRef};
use serde_json::Value;

/// A design system version: its token tree and its components.
pub struct DesignSystem {}

impl DesignSystem {
    /// Reads a design system version in the form the API serves it: a DTCG
    /// token tree (`docs/formats/tokens.md`) with the design system's
    /// components, `component` nodes, in a list under `$components`.
    pub fn from_json(_json: &Value) -> Result<Self, String> {
        todo!("US-032")
    }

    /// The design system's modes, in the order its token tree lists them.
    pub fn modes(&self) -> &[String] {
        todo!("US-032")
    }
}

/// What the renderer could not find while drawing a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Missing {
    /// The component that an instance places is not in the design system.
    Component(NodeId),
    /// The token that a value refers to is not in the design system.
    Token(TokenRef),
}

/// An error that drawing a design reports to the host page: the node that was
/// drawn as an error placeholder, and what it was missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError {
    /// The node, `<instanceId>/<componentNodeId>` for a node inside an
    /// instance.
    pub node: NodeAddress,
    pub missing: Missing,
}
