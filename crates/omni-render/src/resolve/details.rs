//! What resolving a page notes for the inspect panel of the design viewer
//! (US-044): the tokens each node used and what each instance stores. Both
//! are gone from the resolved page, where instances are frames and tokens
//! are values.

use crate::inspect::{
    Inspection, InstanceInspection, LAYOUT_KEYS, Property, SIZE_KEYS, STYLE_KEYS, TEXT_KEYS,
    TokenUse,
};
use crate::layout::LayoutBox;
use omni_core::design::{Node, NodeAddress, NodeId, TokenRef};
use serde_json::Value;
use std::collections::HashMap;

/// A token reference that resolving replaced by the token's value.
pub(crate) struct UsedToken {
    /// Where in the node, as a JSON pointer.
    pub(crate) pointer: String,
    pub(crate) reference: TokenRef,
    /// The token's value as the node takes it.
    pub(crate) value: Value,
}

/// The notes of a resolved page, by the IDs its nodes have there.
#[derive(Default)]
pub(crate) struct Details {
    pub(crate) tokens: HashMap<NodeId, Vec<UsedToken>>,
    /// What each expanded instance stores.
    pub(crate) instances: HashMap<NodeId, InstanceInspection>,
}

/// The facts about a node that the index has.
pub(crate) struct Facts {
    pub(crate) address: NodeAddress,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) mode: Option<String>,
    pub(crate) bounds: LayoutBox,
    pub(crate) interactions: Vec<Value>,
    pub(crate) overflow: Option<String>,
    pub(crate) scroll_position: Option<String>,
}

impl Details {
    /// The inspection of `node`, which is drawn as it is in the resolved page.
    pub(crate) fn inspect(&self, node: &Node, facts: Facts) -> Inspection {
        let mut json = serde_json::to_value(node).unwrap_or(Value::Null);
        if let Some(properties) = json.as_object_mut() {
            properties.remove("children");
        }
        let group = |keys: &[&str]| -> Vec<Property> {
            keys.iter()
                .filter_map(|key| {
                    let value = json.get(*key).filter(|value| !value.is_null())?;
                    Some(Property {
                        name: (*key).to_string(),
                        value: value.clone(),
                        tokens: self.tokens_of(node.id(), key),
                    })
                })
                .collect()
        };
        Inspection {
            layout: group(LAYOUT_KEYS),
            size: group(SIZE_KEYS),
            style: group(STYLE_KEYS),
            text: group(TEXT_KEYS),
            instance: self.instances.get(node.id()).cloned(),
            address: facts.address,
            name: facts.name,
            kind: facts.kind,
            mode: facts.mode,
            bounds: facts.bounds,
            interactions: facts.interactions,
            overflow: facts.overflow,
            scroll_position: facts.scroll_position,
        }
    }

    /// The tokens the setting `key` of node `id` refers to, each once, in the
    /// order they first appear in it.
    fn tokens_of(&self, id: &NodeId, key: &str) -> Vec<TokenUse> {
        let mut uses: Vec<TokenUse> = Vec::new();
        for used in self.tokens.get(id).into_iter().flatten() {
            let first = used.pointer.split('/').nth(1).unwrap_or_default();
            let first = first.replace("~1", "/").replace("~0", "~");
            let token = used.reference.as_str();
            if first == key && !uses.iter().any(|listed| listed.token == token) {
                uses.push(TokenUse {
                    token: token.to_string(),
                    value: used.value.clone(),
                });
            }
        }
        uses
    }
}
