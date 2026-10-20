//! Node addresses: `<id>` for a node, or `<instanceId>/<componentNodeId>` for
//! a node inside a component instance. Overrides, comments and the renderer
//! address nodes this way (US-029).

use super::NodeId;
use std::fmt;
use std::str::FromStr;

/// Where a node is: a node ID, or an instance and a node of its component.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeAddress {
    _todo: (),
}

impl NodeAddress {
    pub fn new(_instance: Option<NodeId>, _node: NodeId) -> Self {
        todo!("US-029: node addresses")
    }

    /// The instance that holds the node, for an address inside one.
    pub fn instance(&self) -> Option<&NodeId> {
        todo!("US-029: node addresses")
    }

    /// The addressed node: the component's node for an address inside an
    /// instance.
    pub fn node(&self) -> &NodeId {
        todo!("US-029: node addresses")
    }
}

/// Why a string is not a node address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidNodeAddress;

impl fmt::Display for InvalidNodeAddress {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("US-029: node addresses")
    }
}

impl std::error::Error for InvalidNodeAddress {}

impl fmt::Display for NodeAddress {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("US-029: node addresses")
    }
}

impl FromStr for NodeAddress {
    type Err = InvalidNodeAddress;

    fn from_str(_address: &str) -> Result<Self, Self::Err> {
        todo!("US-029: node addresses")
    }
}
