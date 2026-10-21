//! The design tokens of an export as one DTCG tree (docs/formats/tokens.md).
//!
//! Penpot keeps tokens in sets (`Bas`, `Mörkt`, `Ljust`) and turns sets on in
//! themes. Each theme becomes a mode of the tree: a token has the value that
//! the last enabled set in the theme's order gives it, and a token that has the
//! same value in every mode states it once. Colors, spacings and border radii
//! become `color`, `spacing` and `radius` tokens with their Penpot path;
//! references to other tokens are followed and written out as values. The
//! library colors and typographies of the file become `color.library` and
//! `typography` tokens. What the tree has no group for is noted.

use std::collections::{BTreeMap, HashMap, HashSet};

use omni_core::design::tokens::TokenTree;
use serde_json::{Map, Value, json};

/// Something of the design tokens that is not imported.
pub(crate) struct Note {
    /// The token, color or typography, by name.
    pub(crate) name: String,
    /// The property that was left out, or None if the whole thing was.
    pub(crate) property: Option<String>,
    pub(crate) reason: String,
}

/// The design system's tokens.
pub(crate) struct Tokens {
    /// The token tree, with the modes listed at its root.
    pub(crate) tree: Value,
    /// The mode of the theme that is active in Penpot.
    pub(crate) active: Option<String>,
    /// The tree path, written as a reference (`$color.surface`), of each color
    /// token, by its Penpot name.
    pub(crate) links: HashMap<String, String>,
    pub(crate) notes: Vec<Note>,
}

/// A token of a Penpot token set.
#[derive(Clone)]
struct Raw {
    kind: String,
    value: Value,
    description: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Group {
    Color,
    Spacing,
    Radius,
}

impl Group {
    fn of(kind: &str) -> Option<Group> {
        match kind {
            "color" => Some(Group::Color),
            "spacing" => Some(Group::Spacing),
            "borderRadius" => Some(Group::Radius),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Group::Color => "color",
            Group::Spacing => "spacing",
            Group::Radius => "radius",
        }
    }
}

/// A name a design can write after `$`: letters, digits, `-` and `_`.
fn segment(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        let c = if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            c
        } else {
            '-'
        };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "_".to_string()
    } else {
        out.to_string()
    }
}

/// The name of a mode: letters of any script, digits, `-` and `_`.
fn mode_name(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        let c = if c.is_alphanumeric() || c == '-' || c == '_' {
            c
        } else {
            '-'
        };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "mode".to_string()
    } else {
        out.to_string()
    }
}

/// A color as `#rrggbb`, or `#rrggbbaa` when it is not opaque.
fn parse_color(text: &str) -> Option<String> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let hex = hex.to_lowercase();
        let hex: String = match hex.len() {
            3 | 4 => hex.chars().flat_map(|digit| [digit, digit]).collect(),
            6 | 8 => hex,
            _ => return None,
        };
        return Some(format!("#{hex}"));
    }
    let args = text
        .strip_prefix("rgba(")
        .or_else(|| text.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = args
        .split([',', ' ', '/'])
        .filter(|part| !part.is_empty())
        .collect();
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let number = |part: &str, whole: f64| -> Option<f64> {
        match part.strip_suffix('%') {
            Some(percent) => Some(percent.parse::<f64>().ok()? / 100.0 * whole),
            None => part.parse().ok(),
        }
    };
    let mut channels = [0u8; 3];
    for (slot, part) in channels.iter_mut().zip(&parts) {
        *slot = number(part, 255.0)?.clamp(0.0, 255.0).round() as u8;
    }
    let alpha = match parts.get(3) {
        Some(part) => number(part, 1.0)?.clamp(0.0, 1.0),
        None => 1.0,
    };
    let [r, g, b] = channels;
    if alpha >= 1.0 {
        Some(format!("#{r:02x}{g:02x}{b:02x}"))
    } else {
        Some(format!("#{r:02x}{g:02x}{b:02x}{:02x}", alpha_byte(alpha)))
    }
}

fn alpha_byte(alpha: f64) -> u8 {
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A length in pixels, written `16px`, from `16`, `1.5` or `16px`.
fn parse_dimension(text: &str) -> Option<String> {
    let text = text.trim();
    let number = text.strip_suffix("px").unwrap_or(text).trim();
    let digits = number.strip_prefix('-').unwrap_or(number);
    let plain = match digits.split_once('.') {
        Some((whole, fraction)) => [whole, fraction]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())),
        None => !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
    };
    if plain {
        return Some(format!("{number}px"));
    }
    let value: f64 = number
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite())?;
    Some(format!("{value}px"))
}

/// The path that a value refers to when it is written `{color.surface}`.
fn reference(text: &str) -> Option<&str> {
    let path = text.trim().strip_prefix('{')?.strip_suffix('}')?;
    (!path.is_empty() && !path.contains(['{', '}'])).then_some(path)
}

/// The text of a token's value: a string, or a number written out.
fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// Every token under `node`, with its path.
fn walk(node: &Value, path: &str, inherited: &str, found: &mut Vec<(String, Raw)>) {
    let Some(map) = node.as_object() else { return };
    if let Some(value) = map.get("$value") {
        let kind = map
            .get("$type")
            .and_then(Value::as_str)
            .unwrap_or(inherited)
            .to_string();
        found.push((
            path.to_string(),
            Raw {
                kind,
                value: value.clone(),
                description: map
                    .get("$description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            },
        ));
        return;
    }
    let inherited = map
        .get("$type")
        .and_then(Value::as_str)
        .unwrap_or(inherited);
    for (name, child) in map {
        if name.starts_with('$') {
            continue;
        }
        let path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}.{name}")
        };
        walk(child, &path, inherited, found);
    }
}

/// Puts `token` at `path` in `root`, which is a tree of groups. Fails when a
/// token or a group is in the way.
fn insert(root: &mut Map<String, Value>, path: &[String], token: Value) -> Result<(), String> {
    let (last, groups) = path.split_last().ok_or("the token has no name")?;
    let mut at = root;
    for name in groups {
        let entry = at.entry(name.clone()).or_insert_with(|| json!({}));
        at = match entry.as_object_mut() {
            Some(group) if !group.contains_key("$value") => group,
            _ => {
                return Err(format!(
                    "`{name}` is a token, but other tokens are below it"
                ));
            }
        };
    }
    if at.contains_key(last) {
        return Err(format!(
            "another token or group already has the path {}",
            path.join(".")
        ));
    }
    at.insert(last.clone(), token);
    Ok(())
}

/// The token's object: its value and, if the modes differ, a value for each.
fn token_object(modes: &[String], values: &[Value], description: &str) -> Value {
    let mut object = Map::new();
    object.insert("$value".into(), values[0].clone());
    if !description.is_empty() {
        object.insert("$description".into(), json!(description));
    }
    if values.iter().any(|value| value != &values[0]) {
        let per_mode: Map<String, Value> =
            modes.iter().cloned().zip(values.iter().cloned()).collect();
        object.insert(
            "$extensions".into(),
            json!({ "omnistate": { "modes": per_mode } }),
        );
    }
    Value::Object(object)
}

/// The value of the token `path` in one mode, with the references it holds
/// followed.
fn resolve(
    mode: &HashMap<String, Raw>,
    path: &str,
    group: Group,
    depth: usize,
) -> Result<String, String> {
    if depth > 32 {
        return Err(format!("{path} refers to itself"));
    }
    let token = mode
        .get(path)
        .ok_or_else(|| format!("it refers to {path}, which is no token"))?;
    if Group::of(&token.kind) != Some(group) {
        return Err(format!(
            "it refers to {path}, a {} token, where a {} is needed",
            token.kind,
            group.name()
        ));
    }
    let text = text_of(&token.value).ok_or_else(|| format!("{path} has no usable value"))?;
    if let Some(target) = reference(&text) {
        return resolve(mode, target, group, depth + 1);
    }
    match group {
        Group::Color => parse_color(&text),
        Group::Spacing | Group::Radius => parse_dimension(&text),
    }
    .ok_or_else(|| format!("the value `{text}` of {path} is not a {}", group.name()))
}

/// Reads the token sets, themes, colors and typographies of an export.
pub(crate) fn build(tokens: Option<&Value>, colors: &[Value], typographies: &[Value]) -> Tokens {
    let mut notes: Vec<Note> = Vec::new();
    let empty = Map::new();
    let root = tokens.and_then(Value::as_object).unwrap_or(&empty);

    // The sets, in the order Penpot applies them.
    let mut sets: BTreeMap<String, Vec<(String, Raw)>> = BTreeMap::new();
    for (name, set) in root {
        if !name.starts_with('$') && set.is_object() {
            let mut found = Vec::new();
            walk(set, "", "", &mut found);
            sets.insert(name.clone(), found);
        }
    }
    let metadata = root.get("$metadata");
    let strings = |value: Option<&Value>| -> Vec<String> {
        value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect()
    };
    let mut order: Vec<String> = strings(metadata.and_then(|m| m.get("tokenSetOrder")))
        .into_iter()
        .filter(|name| sets.contains_key(name))
        .collect();
    for name in sets.keys() {
        if !order.contains(name) {
            order.push(name.clone());
        }
    }

    // The themes become the modes.
    struct Theme {
        id: String,
        mode: String,
        enabled: Vec<String>,
    }
    let mut themes: Vec<Theme> = Vec::new();
    for theme in root
        .get("$themes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = theme
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let group = theme
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut mode = mode_name(name);
        let mut number = 1;
        let base = mode.clone();
        while themes.iter().any(|known| known.mode == mode) {
            number += 1;
            mode = format!("{base}-{number}");
        }
        let selected = theme.get("selectedTokenSets").and_then(Value::as_object);
        themes.push(Theme {
            id: if group.is_empty() {
                name.to_string()
            } else {
                format!("{group}/{name}")
            },
            mode,
            enabled: order
                .iter()
                .filter(|set| {
                    selected
                        .and_then(|selected| selected.get(*set))
                        .and_then(Value::as_str)
                        == Some("enabled")
                })
                .cloned()
                .collect(),
        });
    }
    if themes.is_empty() {
        let active = strings(metadata.and_then(|m| m.get("activeSets")));
        let enabled: Vec<String> = if active.is_empty() {
            order.clone()
        } else {
            order
                .iter()
                .filter(|set| active.contains(set))
                .cloned()
                .collect()
        };
        themes.push(Theme {
            id: "default".to_string(),
            mode: "default".to_string(),
            enabled,
        });
    }
    let modes: Vec<String> = themes.iter().map(|theme| theme.mode.clone()).collect();
    let active_theme = strings(metadata.and_then(|m| m.get("activeThemes")))
        .into_iter()
        .next();
    let active = active_theme
        .and_then(|active| {
            themes
                .iter()
                .find(|theme| theme.id == active || theme.id.ends_with(&format!("/{active}")))
        })
        .or(themes.first())
        .map(|theme| theme.mode.clone());

    // The tokens of each mode: later sets win.
    let mut per_mode: Vec<HashMap<String, Raw>> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for theme in &themes {
        let mut tokens = HashMap::new();
        for set in &theme.enabled {
            for (path, raw) in &sets[set] {
                if seen.insert(path.clone()) {
                    paths.push(path.clone());
                }
                tokens.insert(path.clone(), raw.clone());
            }
        }
        per_mode.push(tokens);
    }

    let mut groups: BTreeMap<&'static str, Map<String, Value>> = BTreeMap::new();
    let mut links = HashMap::new();
    for path in &paths {
        let Some(first) = per_mode.iter().find_map(|tokens| tokens.get(path)) else {
            continue;
        };
        let Some(group) = Group::of(&first.kind) else {
            notes.push(Note {
                name: path.clone(),
                property: None,
                reason: format!(
                    "Penpot tokens of type `{}` have no group in OmniState's token tree, so the token is not imported",
                    first.kind
                ),
            });
            continue;
        };
        let description = first.description.clone();
        let mut values: Vec<Option<String>> = Vec::new();
        let mut failed = None;
        for tokens in &per_mode {
            if !tokens.contains_key(path) {
                values.push(None);
                continue;
            }
            match resolve(tokens, path, group, 0) {
                Ok(value) => values.push(Some(value)),
                Err(reason) => {
                    failed = Some(reason);
                    break;
                }
            }
        }
        if let Some(reason) = failed {
            notes.push(Note {
                name: path.clone(),
                property: None,
                reason: format!("the token is not imported: {reason}"),
            });
            continue;
        }
        let present = values.iter().flatten().next().cloned().unwrap_or_default();
        if values.iter().any(Option::is_none) {
            let missing: Vec<&str> = modes
                .iter()
                .zip(&values)
                .filter(|(_, value)| value.is_none())
                .map(|(mode, _)| mode.as_str())
                .collect();
            notes.push(Note {
                name: path.clone(),
                property: Some("modes".to_string()),
                reason: format!(
                    "the token is in no enabled set of the {} theme(s); it takes the value of the other themes there",
                    missing.join(", ")
                ),
            });
        }
        let values: Vec<Value> = values
            .into_iter()
            .map(|value| json!(value.unwrap_or_else(|| present.clone())))
            .collect();
        let mut tree_path: Vec<String> = path.split('.').map(segment).collect();
        if tree_path.first().map(String::as_str) != Some(group.name()) || group != Group::Color {
            tree_path.insert(0, group.name().to_string());
        }
        let tokens = groups.entry(group.name()).or_default();
        // The group is the first part of the path; it lives in `tokens`.
        let token = token_object(&modes, &values, &description);
        match insert(tokens, &tree_path[1..], token) {
            Ok(()) => {
                if group == Group::Color {
                    links.insert(path.clone(), format!("${}", tree_path.join(".")));
                }
            }
            Err(reason) => notes.push(Note {
                name: path.clone(),
                property: None,
                reason: format!("the token is not imported: {reason}"),
            }),
        }
    }

    library_colors(colors, &modes, &mut groups, &mut notes);
    typography_tokens(typographies, &modes, &mut groups, &mut notes);

    let mut tree = Map::new();
    tree.insert(
        "$extensions".into(),
        json!({ "omnistate": { "modes": modes } }),
    );
    for (name, group) in &groups {
        if !group.is_empty() {
            tree.insert((*name).to_string(), Value::Object(group.clone()));
        }
    }
    let mut tree = Value::Object(tree);
    if let Err(error) = TokenTree::from_json(&tree) {
        notes.push(Note {
            name: String::new(),
            property: None,
            reason: format!("the design tokens are refused ({error}), so none are imported"),
        });
        tree = json!({ "$extensions": { "omnistate": { "modes": modes } } });
        links.clear();
    }
    Tokens {
        tree,
        active,
        links,
        notes,
    }
}

/// The `path` of a library asset and its `name` as the segments of a token
/// path.
fn asset_path(asset: &Value) -> (String, Vec<String>) {
    let name = asset
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut path: Vec<String> = asset
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split('/')
        .filter(|part| !part.trim().is_empty())
        .map(segment)
        .collect();
    path.push(segment(&name));
    (name, path)
}

/// Puts `token` below `prefix` in the group, with a number after the name when
/// the path is taken.
fn put_asset(
    group: &mut Map<String, Value>,
    prefix: &[&str],
    path: &[String],
    token: &Value,
) -> Result<(), String> {
    let mut full: Vec<String> = prefix.iter().map(|part| (*part).to_string()).collect();
    full.extend(path.iter().cloned());
    let mut result = insert(group, &full, token.clone());
    let mut number = 1;
    while result.is_err() && number < 100 {
        number += 1;
        let last = full.len() - 1;
        let base = path.last().cloned().unwrap_or_default();
        full[last] = format!("{base}-{number}");
        result = insert(group, &full, token.clone());
    }
    result
}

/// The library colors, as `color.library` tokens with one value in every mode.
fn library_colors(
    colors: &[Value],
    modes: &[String],
    groups: &mut BTreeMap<&'static str, Map<String, Value>>,
    notes: &mut Vec<Note>,
) {
    for color in colors {
        let (name, path) = asset_path(color);
        let Some(hex) = color
            .get("color")
            .and_then(Value::as_str)
            .and_then(parse_color)
        else {
            notes.push(Note {
                name,
                property: None,
                reason: "the library color has no plain color (a gradient or an image); it is not imported"
                    .to_string(),
            });
            continue;
        };
        let opacity = color
            .get("opacity")
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let hex = if opacity < 1.0 {
            let own = if hex.len() == 9 {
                u8::from_str_radix(&hex[7..9], 16).unwrap_or(255)
            } else {
                255
            };
            let alpha = alpha_byte(opacity * f64::from(own) / 255.0);
            format!("{}{alpha:02x}", &hex[..7])
        } else {
            hex
        };
        let values = vec![json!(hex); modes.len()];
        let token = token_object(modes, &values, "");
        let group = groups.entry("color").or_default();
        if let Err(reason) = put_asset(group, &["library"], &path, &token) {
            notes.push(Note {
                name,
                property: None,
                reason: format!("the library color is not imported: {reason}"),
            });
        }
    }
}

/// The library typographies, as `typography` tokens.
fn typography_tokens(
    typographies: &[Value],
    modes: &[String],
    groups: &mut BTreeMap<&'static str, Map<String, Value>>,
    notes: &mut Vec<Note>,
) {
    for typography in typographies {
        let (name, path) = asset_path(typography);
        let text = |key: &str| -> String {
            typography
                .get(key)
                .map(|value| text_of(value).unwrap_or_default())
                .unwrap_or_default()
        };
        let family = text("fontFamily");
        if family.trim().is_empty() {
            notes.push(Note {
                name,
                property: None,
                reason: "the typography has no font family; it is not imported".to_string(),
            });
            continue;
        }
        let Some(size) = parse_dimension(&text("fontSize")) else {
            notes.push(Note {
                name,
                property: None,
                reason: "the typography has no usable font size; it is not imported".to_string(),
            });
            continue;
        };
        let mut value = Map::new();
        value.insert("fontFamily".into(), json!(family.trim()));
        value.insert("fontSize".into(), json!(size));
        if let Ok(weight) = text("fontWeight").trim().parse::<f64>() {
            if weight.fract() == 0.0 {
                value.insert("fontWeight".into(), json!(weight as u64));
            } else {
                value.insert("fontWeight".into(), json!(weight));
            }
        }
        match text("lineHeight").trim().parse::<f64>() {
            Ok(height) if height >= 0.0 => {
                value.insert("lineHeight".into(), json!(height));
            }
            _ => {
                value.insert("lineHeight".into(), json!(1.2));
                notes.push(Note {
                    name: name.clone(),
                    property: Some("lineHeight".to_string()),
                    reason: "the typography has no usable line height; 1.2 is used".to_string(),
                });
            }
        }
        if let Some(spacing) = parse_dimension(&text("letterSpacing"))
            .filter(|_| !text("letterSpacing").trim().is_empty())
        {
            value.insert("letterSpacing".into(), json!(spacing));
        }
        for (key, plain) in [
            ("fontStyle", "normal"),
            ("textTransform", "none"),
            ("textDecoration", "none"),
        ] {
            let found = text(key);
            if !found.is_empty() && found != plain {
                notes.push(Note {
                    name: name.clone(),
                    property: Some(key.to_string()),
                    reason: format!("a typography token holds no {key}; `{found}` is left out"),
                });
            }
        }
        let values = vec![Value::Object(value); modes.len()];
        let token = token_object(modes, &values, "");
        let group = groups.entry("typography").or_default();
        if let Err(reason) = put_asset(group, &[], &path, &token) {
            notes.push(Note {
                name,
                property: None,
                reason: format!("the typography is not imported: {reason}"),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_written_as_hex() {
        assert_eq!(parse_color("#FFF").as_deref(), Some("#ffffff"));
        assert_eq!(parse_color("#1C1C21").as_deref(), Some("#1c1c21"));
        assert_eq!(
            parse_color("rgba(229,72,77,0.40)").as_deref(),
            Some("#e5484d66")
        );
        assert_eq!(
            parse_color("rgba(255,255,255,0.045)").as_deref(),
            Some("#ffffff0b")
        );
        assert_eq!(parse_color("rgb(0, 128, 255)").as_deref(), Some("#0080ff"));
        assert_eq!(parse_color("hotpink"), None);
        assert_eq!(parse_color("#12"), None);
    }

    #[test]
    fn dimensions_are_lengths_in_pixels() {
        assert_eq!(parse_dimension("16").as_deref(), Some("16px"));
        assert_eq!(parse_dimension("1.50").as_deref(), Some("1.50px"));
        assert_eq!(parse_dimension("-2px").as_deref(), Some("-2px"));
        assert_eq!(parse_dimension("1e1").as_deref(), Some("10px"));
        assert_eq!(parse_dimension("wide"), None);
    }

    #[test]
    fn names_are_what_a_design_can_write() {
        assert_eq!(segment("Page title"), "Page-title");
        assert_eq!(segment("1-5"), "1-5");
        assert_eq!(segment("  "), "_");
        assert_eq!(mode_name("Mörkt"), "Mörkt");
        assert_eq!(mode_name("Tema / Ljust"), "Tema-Ljust");
    }

    #[test]
    fn a_token_that_one_set_changes_has_a_value_per_mode() {
        let tokens = json!({
            "Bas": { "space": { "a": { "$value": "{space.b}", "$type": "spacing" },
                                "b": { "$value": "4", "$type": "spacing" } } },
            "Mörkt": { "color": { "ink": { "$value": "#000", "$type": "color" } } },
            "Ljust": { "color": { "ink": { "$value": "#fff", "$type": "color" } } },
            "$themes": [
                { "name": "Mörkt", "group": "Tema", "selectedTokenSets": { "Bas": "enabled", "Mörkt": "enabled" } },
                { "name": "Ljust", "group": "Tema", "selectedTokenSets": { "Bas": "enabled", "Ljust": "enabled" } }
            ],
            "$metadata": { "tokenSetOrder": ["Bas", "Mörkt", "Ljust"], "activeThemes": ["Tema/Ljust"] }
        });
        let built = build(Some(&tokens), &[], &[]);
        assert_eq!(built.active.as_deref(), Some("Ljust"));
        assert_eq!(built.links["color.ink"], "$color.ink");
        assert_eq!(built.tree["spacing"]["space"]["a"]["$value"], "4px");
        assert_eq!(
            built.tree["color"]["ink"]["$extensions"]["omnistate"]["modes"],
            json!({ "Mörkt": "#000000", "Ljust": "#ffffff" })
        );
        assert!(built.notes.is_empty(), "{:?}", built.notes.len());
    }
}
