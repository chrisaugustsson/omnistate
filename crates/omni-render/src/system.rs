//! The design system that a design's instances and token references resolve
//! against (US-032): the components of a design system version and its token
//! tree, which has a value for each of its modes.

use omni_core::design::tokens::TokenTree;
use omni_core::design::{Node, NodeAddress, NodeData, NodeId, PropDef, PropDefs, TokenRef};
use serde_json::Value;
use std::collections::HashMap;

/// A design system version: its token tree, `None` for a design system
/// without tokens, and its components.
#[derive(Clone)]
pub struct DesignSystem {
    tokens: Option<TokenTree>,
    components: Vec<ComponentDef>,
    by_id: HashMap<NodeId, usize>,
}

/// A component of the design system, with its node in the JSON form that
/// instances expand from.
#[derive(Clone)]
pub(crate) struct ComponentDef {
    pub(crate) id: NodeId,
    pub(crate) name: String,
    pub(crate) props: PropDefs,
    pub(crate) json: Value,
}

impl ComponentDef {
    /// The variant properties of the component, by name with their values.
    fn variants(&self) -> Vec<(&str, &[String])> {
        self.props
            .iter()
            .filter_map(|(name, def)| match def {
                PropDef::Variant { values, .. } => Some((name.as_str(), values.as_slice())),
                _ => None,
            })
            .collect()
    }

    /// The default of the variant property `name`.
    fn variant_default(&self, name: &str) -> Option<&str> {
        match self.props.get(name) {
            Some(PropDef::Variant { default, .. }) => Some(default),
            _ => None,
        }
    }
}

impl DesignSystem {
    /// Reads a design system version in the form the API serves it: a DTCG
    /// token tree (`docs/formats/tokens.md`) with the design system's
    /// components, `component` nodes, in a list under `$components`.
    pub fn from_json(json: &Value) -> Result<Self, String> {
        let Some(root) = json.as_object() else {
            return Err(
                "a design system version is a JSON object: a token tree with its components under `$components`"
                    .to_string(),
            );
        };
        let mut tree = root.clone();
        let listed = tree.remove("$components");
        let tokens = if tree.is_empty() {
            None
        } else {
            let tree = TokenTree::from_json(&Value::Object(tree));
            Some(tree.map_err(|error| error.to_string())?)
        };
        let listed = match listed {
            None => Vec::new(),
            Some(Value::Array(items)) => items,
            Some(_) => {
                return Err("`$components` is a list of component nodes".to_string());
            }
        };
        let mut components = Vec::with_capacity(listed.len());
        let mut by_id = HashMap::new();
        for item in listed {
            let node: Node = serde_json::from_value(item)
                .map_err(|error| format!("a component in `$components` is not valid: {error}"))?;
            let NodeData::Component(component) = node.data() else {
                return Err(format!(
                    "`$components` holds component nodes, but `{}` is a {}",
                    node.id(),
                    node.kind()
                ));
            };
            if by_id.insert(*node.id(), components.len()).is_some() {
                return Err(format!(
                    "component IDs are unique, but `{}` names more than one component",
                    node.id()
                ));
            }
            components.push(ComponentDef {
                id: *node.id(),
                name: node.name().to_string(),
                props: component.props.clone().unwrap_or_default(),
                json: serde_json::to_value(&node).map_err(|error| error.to_string())?,
            });
        }
        Ok(Self {
            tokens,
            components,
            by_id,
        })
    }

    /// The design system's modes, in the order its token tree lists them.
    pub fn modes(&self) -> &[String] {
        self.tokens.as_ref().map_or(&[], TokenTree::modes)
    }

    /// `page` with its instances expanded and its token references resolved in
    /// `mode`: an ordinary page that layout and painting handle like any
    /// other, and the nodes that were drawn as error placeholders.
    pub fn resolve(&self, page: &Node, mode: &str) -> (Node, Vec<RenderError>) {
        let resolved = crate::resolve::resolve(page, self, mode);
        (resolved.page, resolved.errors)
    }

    /// The value of `token` in `mode`, with its references followed.
    pub(crate) fn token(&self, token: &TokenRef, mode: &str) -> Option<Value> {
        self.tokens.as_ref()?.resolve(token, mode)
    }

    pub(crate) fn component(&self, id: &NodeId) -> Option<&ComponentDef> {
        self.by_id.get(id).map(|&at| &self.components[at])
    }

    /// The variant of `base`'s set that shows `wanted`, the instance's values
    /// for the variant properties. The set is the components with the name of
    /// `base` and its variant properties (names and values), and each one
    /// shows its own defaults. A value the instance leaves out is `base`'s
    /// default. `base` itself shows when it matches, and when nothing does.
    pub(crate) fn variant<'a>(
        &'a self,
        base: &'a ComponentDef,
        wanted: &HashMap<String, String>,
    ) -> &'a ComponentDef {
        let variants = base.variants();
        if variants.is_empty() {
            return base;
        }
        let wanted_value = |name: &str| {
            wanted
                .get(name)
                .map(String::as_str)
                .or_else(|| base.variant_default(name))
        };
        let shows = |candidate: &ComponentDef| {
            variants
                .iter()
                .all(|(name, _)| candidate.variant_default(name) == wanted_value(name))
        };
        if shows(base) {
            return base;
        }
        self.components
            .iter()
            .find(|candidate| {
                candidate.name == base.name && candidate.variants() == variants && shows(candidate)
            })
            .unwrap_or(base)
    }
}

/// What the renderer could not find while drawing a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Missing {
    /// The component that an instance places is not in the design system.
    Component(NodeId),
    /// The token that a value refers to is not in the design system.
    Token(TokenRef),
    /// The token that a value refers to has a value that does not fit where
    /// it is used, such as a color for a padding, in `property` of the node.
    Unfit { token: TokenRef, property: String },
    /// The node holds nodes deeper than `nesting::MAX_NESTING` levels below
    /// the page once instances are expanded, which are not drawn.
    Nesting,
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
