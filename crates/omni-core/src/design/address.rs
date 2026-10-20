//! Node addresses: `<id>` for a node, or `<instanceId>/<componentNodeId>` for
//! a node inside a component instance. Overrides, comments and the renderer
//! address nodes this way (US-029).

use super::NodeId;
use std::fmt;
use std::str::FromStr;

/// Where a node is: a node ID, or an instance and a node of its component.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeAddress {
    instance: Option<NodeId>,
    node: NodeId,
}

impl NodeAddress {
    pub fn new(instance: Option<NodeId>, node: NodeId) -> Self {
        Self { instance, node }
    }

    /// The instance that holds the node, for an address inside one.
    pub fn instance(&self) -> Option<&NodeId> {
        self.instance.as_ref()
    }

    /// The addressed node: the component's node for an address inside an
    /// instance.
    pub fn node(&self) -> &NodeId {
        &self.node
    }
}

/// Why a string is not a node address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidNodeAddress;

impl fmt::Display for InvalidNodeAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "not a node address: write a node ID, or an instance ID and a component node ID with a `/` between",
        )
    }
}

impl std::error::Error for InvalidNodeAddress {}

impl fmt::Display for NodeAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.instance {
            Some(instance) => write!(f, "{instance}/{}", self.node),
            None => write!(f, "{}", self.node),
        }
    }
}

impl FromStr for NodeAddress {
    type Err = InvalidNodeAddress;

    fn from_str(address: &str) -> Result<Self, Self::Err> {
        let id = |text: &str| text.parse::<NodeId>().map_err(|_| InvalidNodeAddress);
        match address.split_once('/') {
            Some((instance, node)) => Ok(Self::new(Some(id(instance)?), id(node)?)),
            None => Ok(Self::new(None, id(address)?)),
        }
    }
}
