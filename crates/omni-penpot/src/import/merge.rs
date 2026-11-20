//! An import onto a design system that the project already has: its tokens and
//! components are added to the ones of the design system's head, which all
//! stay, because `PUT .../tokens` and `PUT .../components` replace the whole
//! head. A component of the import that the head already has from Penpot (the
//! same Penpot file and component in its `origin`) is not added again: the
//! import's instances place the head's.

use omni_core::design::tokens::TokenTree;

use super::*;

/// Whether a value of a token tree is a token rather than a group.
fn is_token(value: &Value) -> bool {
    value.get("$value").is_some()
}

/// The modes listed on the root of a token tree.
fn modes_of(tree: &Value) -> Vec<String> {
    tree["$extensions"]["omnistate"]["modes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|mode| mode.as_str().map(str::to_string))
        .collect()
}

fn token_entry(name: String, reason: String) -> ImportEntry {
    ImportEntry {
        page: String::new(),
        board: String::new(),
        board_id: String::new(),
        shape_id: String::new(),
        name,
        property: None,
        reason,
    }
}

/// Puts `incoming` at `name` of the group `at`. A token replaces the token of
/// the same path; a group adds to the group. A token where the design system
/// has a group, or the other way round, is not imported.
fn add(
    at: &mut Map<String, Value>,
    name: &str,
    incoming: &Value,
    path: &str,
    report: &mut Vec<ImportEntry>,
) {
    let Some(existing) = at.get_mut(name) else {
        at.insert(name.to_string(), incoming.clone());
        return;
    };
    match (is_token(existing), is_token(incoming)) {
        (true, true) => *existing = incoming.clone(),
        (false, false) => {
            let (Some(group), Some(fields)) = (existing.as_object_mut(), incoming.as_object())
            else {
                return;
            };
            for (key, value) in fields {
                if key.starts_with('$') {
                    group.entry(key.clone()).or_insert_with(|| value.clone());
                } else {
                    add(group, key, value, &format!("{path}.{key}"), report);
                }
            }
        }
        (kept_token, _) => {
            let (kept, new) = if kept_token {
                ("a token", "group")
            } else {
                ("a group", "token")
            };
            report.push(token_entry(
                path.to_string(),
                format!(
                    "the {new} is not imported: the project's design system already has {kept} with this path"
                ),
            ));
        }
    }
}

/// Gives every token with a value per mode a value in each of `modes`: its
/// `$value` where it has none. `imported` holds the paths of the tokens that
/// the import brought.
fn fill_modes(
    value: &mut Value,
    path: &str,
    modes: &[String],
    imported: &HashSet<String>,
    report: &mut Vec<ImportEntry>,
) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object.contains_key("$value") {
        let fallback = object["$value"].clone();
        let Some(per_mode) = object
            .get_mut("$extensions")
            .and_then(|extensions| extensions.get_mut("omnistate"))
            .and_then(|omnistate| omnistate.get_mut("modes"))
            .and_then(Value::as_object_mut)
        else {
            return;
        };
        for mode in modes {
            if per_mode.contains_key(mode) {
                continue;
            }
            per_mode.insert(mode.clone(), fallback.clone());
            let whose = if imported.contains(path) {
                "the export's token"
            } else {
                "the design system's token"
            };
            report.push(token_entry(
                path.to_string(),
                format!("{whose} has no value in the mode {mode}, so it takes its $value there"),
            ));
        }
        return;
    }
    for (key, child) in object.iter_mut() {
        if !key.starts_with('$') {
            let path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            fill_modes(child, &path, modes, imported, report);
        }
    }
}

/// The paths of the tokens of a tree.
fn token_paths(value: &Value, path: &str, out: &mut HashSet<String>) {
    if is_token(value) {
        out.insert(path.to_string());
        return;
    }
    for (key, child) in value.as_object().into_iter().flatten() {
        if !key.starts_with('$') {
            let path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            token_paths(child, &path, out);
        }
    }
}

/// The token tree of the design system with the tokens of `incoming` added.
/// The modes are the design system's, then the ones of the export's themes
/// that it lacks. A
/// tree that is refused once the two are put together leaves the design
/// system's tokens as they are, and the report says why.
pub(super) fn merge_tokens(kept: &Value, incoming: &Value, report: &mut Vec<ImportEntry>) -> Value {
    if !kept.is_object() {
        return incoming.clone();
    }
    // An export with one mode, or none (Penpot without themes), says nothing
    // about modes: its tokens have one value in every mode.
    let mut modes = modes_of(kept);
    let incoming_modes = modes_of(incoming);
    if incoming_modes.len() > 1 {
        for mode in incoming_modes {
            if !modes.contains(&mode) {
                modes.push(mode);
            }
        }
    }
    let mut notes = Vec::new();
    let mut tree = kept.clone();
    let root = tree.as_object_mut().expect("the tree is an object");
    for (key, value) in incoming.as_object().into_iter().flatten() {
        if key == "$extensions" {
            continue;
        }
        if key.starts_with('$') {
            root.entry(key.clone()).or_insert_with(|| value.clone());
        } else {
            add(root, key, value, key, &mut notes);
        }
    }
    root.entry("$extensions").or_insert_with(|| json!({}))["omnistate"]["modes"] = json!(modes);
    let mut imported = HashSet::new();
    token_paths(incoming, "", &mut imported);
    fill_modes(&mut tree, "", &modes, &imported, &mut notes);
    match TokenTree::from_json(&tree) {
        Ok(_) => {
            report.extend(notes);
            tree
        }
        Err(error) => {
            report.push(token_entry(
                String::new(),
                format!(
                    "the design tokens of the export are refused beside the ones of the project's design system ({error}), so none are imported"
                ),
            ));
            kept.clone()
        }
    }
}

/// Every node ID of `value` and below it.
pub(super) fn node_ids(value: &Value, out: &mut Vec<String>) {
    if let Some(id) = value["id"].as_str() {
        out.push(id.to_string());
    }
    for child in value["children"].as_array().into_iter().flatten() {
        node_ids(child, out);
    }
}

fn answer_of(text: &str, what: &str) -> Result<Value, ReadError> {
    serde_json::from_str(text).map_err(|error| {
        ReadError(format!(
            "the server's answer about {what} is not readable: {error}"
        ))
    })
}

/// The token tree with the components under `$components` that a design
/// system's head holds, from the server's answer to `GET .../docs/{id}/html`
/// as JSON text.
pub(super) fn head_tree(head: &str) -> Result<Value, ReadError> {
    let head = answer_of(head, "the design system's head")?;
    let text = head["html"].as_str().unwrap_or_default();
    if text.trim().is_empty() {
        Ok(json!({}))
    } else {
        answer_of(text, "the design system's head")
    }
}

/// The Penpot file and component a component came from, if it did.
fn penpot_origin(component: &Value) -> Option<(&str, &str)> {
    let origin = &component["origin"];
    (origin["tool"] == "penpot").then_some(())?;
    Some((origin["file"].as_str()?, origin["component"].as_str()?))
}

/// Says that the nodes of `component` are those of `same`, the head's
/// component it stands for: the component itself, and each layer by the Penpot
/// layer it came from.
fn reuse(component: &Value, same: &Value, ids: &mut Ids) {
    if let (Some(old), Some(new)) = (component["id"].as_str(), same["id"].as_str()) {
        ids.nodes.insert(old.to_string(), new.to_string());
    }
    let theirs = same["origin"]["layers"].as_object();
    for (layer, old) in component["origin"]["layers"]
        .as_object()
        .into_iter()
        .flatten()
    {
        if let (Some(old), Some(new)) = (
            old.as_str(),
            theirs.and_then(|layers| layers.get(layer)?.as_str()),
        ) {
            ids.nodes.insert(old.to_string(), new.to_string());
        }
    }
}

/// A component as content alone (see [`normalized`]), written as OmniState
/// writes nodes, so that the server's `12` and the import's `12.0` agree.
fn content_of(component: &Value, ids: &Ids, incoming: bool) -> Value {
    let written = serde_json::from_value::<Node>(component.clone())
        .ok()
        .and_then(|node| serde_json::to_value(node).ok())
        .unwrap_or_else(|| component.clone());
    normalized(&written, ids, incoming)
}

/// A component as content alone, to compare an import's with the head's:
/// without node IDs or origin, with the components its instances place and
/// the nodes their overrides change named as the head names them (`ids`, for
/// the import's), and the import's images as the server named them.
fn normalized(node: &Value, ids: &Ids, incoming: bool) -> Value {
    let rename = |id: &str| -> String {
        if incoming {
            ids.nodes.get(id).cloned().unwrap_or_else(|| id.to_string())
        } else {
            id.to_string()
        }
    };
    match node {
        Value::Object(object) => {
            let mut out = Map::new();
            for (key, value) in object {
                match key.as_str() {
                    "id" | "origin" => {}
                    "component" => {
                        let value = value
                            .as_str()
                            .map_or_else(|| value.clone(), |id| json!(rename(id)));
                        out.insert(key.clone(), value);
                    }
                    "overrides" => {
                        let changes = value
                            .as_object()
                            .into_iter()
                            .flatten()
                            .map(|(node, change)| (rename(node), normalized(change, ids, incoming)))
                            .collect();
                        out.insert(key.clone(), Value::Object(changes));
                    }
                    _ => {
                        out.insert(key.clone(), normalized(value, ids, incoming));
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| normalized(item, ids, incoming))
                .collect(),
        ),
        Value::String(text) if incoming => match text
            .strip_prefix("attachment:")
            .and_then(|old| ids.attachments.get(old))
        {
            Some(new) => json!(format!("attachment:{new}")),
            None => node.clone(),
        },
        _ => node.clone(),
    }
}

impl Import {
    /// The import onto the design system's head, which holds the tokens and
    /// the components in one text (`GET .../docs/{id}/html`, whose `html` is
    /// the token tree with the components under `$components`). `tokens` is the
    /// answer about the tokens (`GET .../tokens`) and `head` the head read, both
    /// as JSON text. Otherwise as [`Import::onto_design_system`].
    pub fn onto_design_system_head(
        &self,
        tokens: &str,
        head: &str,
        ids: &mut Ids,
    ) -> Result<Import, ReadError> {
        let tree = head_tree(head)?;
        let components = tree["$components"].as_array().cloned().unwrap_or_default();
        let version = json!({ "version": { "components": components } });
        self.onto_design_system(tokens, &version.to_string(), ids)
    }

    /// The import onto the design system that the project already has, so
    /// that sending it keeps what that design system holds. `tokens` is the
    /// server's answer about the head's tokens (`GET .../tokens`) and
    /// `version` holds the head's components under `version.components`, both
    /// as JSON text.
    ///
    /// The tokens of the import are added to the design system's, a token of
    /// the same path taking the import's value, and its components to the
    /// design system's, which are sent first and keep their IDs: `ids` learns
    /// them.
    pub fn onto_design_system(
        &self,
        tokens: &str,
        version: &str,
        ids: &mut Ids,
    ) -> Result<Import, ReadError> {
        let tokens = answer_of(tokens, "the tokens")?;
        let version = answer_of(version, "the design system version")?;
        let kept = version["version"]["components"].as_array().ok_or_else(|| {
            ReadError(
                "the server's answer about the design system version has no components; is it an OmniState API?"
                    .to_string(),
            )
        })?;
        let mut report = self.report.clone();
        let mut tree = merge_tokens(&tokens["tokens"], &self.token_tree(), &mut report);
        let mut components = kept.clone();
        for component in kept {
            let mut found = Vec::new();
            node_ids(component, &mut found);
            for id in found {
                ids.nodes.insert(id.clone(), id);
            }
        }
        // A component the head already has from Penpot stays the head's: the
        // import's instances and overrides name its nodes instead. That is a
        // component of the same Penpot file and component, or one of another
        // file with the same Penpot component id and the same content (a file
        // duplicated from the library keeps its components' ids).
        let mut pending: Vec<&Value> = self.design_system["$components"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        loop {
            let before = pending.len();
            pending.retain(|component| {
                let Some((file, id)) = penpot_origin(component) else {
                    return true;
                };
                let same = kept
                    .iter()
                    .find(|other| penpot_origin(other) == Some((file, id)))
                    .or_else(|| {
                        let theirs = content_of(component, ids, true);
                        kept.iter().find(|other| {
                            penpot_origin(other).is_some_and(|(_, other_id)| other_id == id)
                                && content_of(other, ids, false) == theirs
                        })
                    });
                let Some(same) = same else {
                    return true;
                };
                reuse(component, same, ids);
                false
            });
            if pending.len() == before {
                break;
            }
        }
        components.extend(pending.into_iter().cloned());
        if let Some(root) = tree.as_object_mut()
            && !components.is_empty()
        {
            root.insert("$components".into(), Value::Array(components));
        }
        Ok(Import {
            pages: self.pages.clone(),
            design_system: tree,
            attachments: self.attachments.clone(),
            report,
            mode: self.mode.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(dark: &str, light: &str) -> Value {
        json!({ "$value": dark, "$extensions": { "omnistate": { "modes": { "Mörkt": dark, "Ljust": light } } } })
    }

    #[test]
    fn tokens_are_added_and_the_kept_ones_stay() {
        let kept = json!({
            "$extensions": { "omnistate": { "modes": ["Mörkt", "Ljust"] } },
            "color": { "text": color("#f4f4f5", "#18181b"), "surface": color("#1c1c21", "#ffffff") }
        });
        let incoming = json!({
            "$extensions": { "omnistate": { "modes": ["Mörkt", "Ljust"] } },
            "color": { "border": color("#3f3f46", "#e4e4e7"), "surface": color("#000000", "#ffffff") }
        });
        let mut report = Vec::new();
        let tree = merge_tokens(&kept, &incoming, &mut report);
        assert!(report.is_empty(), "{report:?}");
        assert_eq!(tree["color"]["text"], color("#f4f4f5", "#18181b"));
        assert_eq!(tree["color"]["border"], color("#3f3f46", "#e4e4e7"));
        assert_eq!(tree["color"]["surface"], color("#000000", "#ffffff"));
        assert_eq!(modes_of(&tree), ["Mörkt", "Ljust"]);
    }

    #[test]
    fn a_new_mode_is_added_and_tokens_without_it_take_their_value() {
        let kept = json!({
            "$extensions": { "omnistate": { "modes": ["Mörkt", "Ljust"] } },
            "color": { "text": color("#f4f4f5", "#18181b") }
        });
        let incoming = json!({
            "$extensions": { "omnistate": { "modes": ["Mörkt", "Hög"] } },
            "color": { "line": { "$value": "#ff0000" } }
        });
        let mut report = Vec::new();
        let tree = merge_tokens(&kept, &incoming, &mut report);
        assert_eq!(modes_of(&tree), ["Mörkt", "Ljust", "Hög"]);
        assert_eq!(
            tree["color"]["text"]["$extensions"]["omnistate"]["modes"]["Hög"],
            "#f4f4f5"
        );
        assert_eq!(tree["color"]["line"], json!({ "$value": "#ff0000" }));
        assert_eq!(report.len(), 1, "{report:?}");
        assert_eq!(report[0].name, "color.text");
    }

    #[test]
    fn an_export_without_themes_adds_no_mode() {
        let kept = json!({
            "$extensions": { "omnistate": { "modes": ["Mörkt", "Ljust"] } },
            "color": { "text": color("#f4f4f5", "#18181b") }
        });
        let incoming = json!({
            "$extensions": { "omnistate": { "modes": ["default"] } },
            "color": { "line": { "$value": "#ff0000" } }
        });
        let mut report = Vec::new();
        let tree = merge_tokens(&kept, &incoming, &mut report);
        assert_eq!(modes_of(&tree), ["Mörkt", "Ljust"]);
        assert_eq!(tree["color"]["line"], json!({ "$value": "#ff0000" }));
        assert!(report.is_empty(), "{report:?}");
    }

    #[test]
    fn a_token_where_the_design_system_has_a_group_is_reported() {
        let kept = json!({
            "$extensions": { "omnistate": { "modes": ["light"] } },
            "color": { "brand": { "500": { "$value": "#0000ff" } } }
        });
        let incoming = json!({
            "$extensions": { "omnistate": { "modes": ["light"] } },
            "color": { "brand": { "$value": "#ff0000" } }
        });
        let mut report = Vec::new();
        let tree = merge_tokens(&kept, &incoming, &mut report);
        assert_eq!(tree, kept);
        assert_eq!(report.len(), 1, "{report:?}");
        assert_eq!(report[0].name, "color.brand");
    }
}
