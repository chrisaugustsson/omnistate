//! `omni tokens import` and `omni tokens export`: replace the design tokens of
//! the design system's head with a DTCG file, and write the tokens of the head
//! as DTCG JSON, whole or one file per collection and mode
//! (docs/formats/tokens.md); `omni tokens set` and `remove` change one token
//! by reading the tree and putting it back. The
//! project is found as for `omni project`.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::Value;
use crate::projects::project_id;
use omni_core::design::tokens::TokenTree;
use serde_json::{Map, Value as Json, json};
use std::path::Path;

fn parsed(text: &str, what: &str) -> Result<Value> {
    Value::parse(text).map_err(|_| {
        CliError::new(format!(
            "the server's answer about {what} is not readable; is it an OmniState API?"
        ))
    })
}

fn modes_of(answer: &Value) -> Vec<&str> {
    answer
        .get("modes")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

/// `omni tokens import <file>`
pub fn import(args: &Args, file: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni tokens import <file>";
    let Some(file) = file else {
        bail!("this command needs the token file to import: {USAGE}");
    };
    let text = std::fs::read_to_string(file)
        .map_err(|e| CliError::new(format!("cannot read the token file {file}: {e}")))?;
    if let Err(reason) = Value::parse(&text) {
        bail!(
            "the token file {file} is not JSON ({reason}); write the DTCG format of docs/formats/tokens.md"
        );
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let body = format!("{{\"tokens\":{}}}", text.trim());
    let path = format!("{}/tokens", project_path(&agent, &project));
    let response = agent.request("PUT", &path, Some(body.as_bytes()))?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = parsed(&text, "the tokens")?;
    say("imported the token tree into the head of the design system");
    say(&format!("modes: {}", modes_of(&answer).join(", ")));
    Ok(())
}

fn project_path(agent: &Agent, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", agent.workspace_id)
}

/// The collections of the answer with the modes of each; a collection with
/// no modes has no file.
fn collections_of(answer: &Value) -> Vec<(&str, Vec<&str>)> {
    answer
        .get("collections")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|collection| {
            let modes = collection
                .get("modes")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            Some((collection.str_at("name")?, modes))
        })
        .collect()
}

/// A collection and a mode become part of a file name, and the server named
/// them.
fn file_name_of(collection: &str, mode: &str) -> Result<String> {
    for (what, name) in [("collection", collection), ("mode", mode)] {
        let safe = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
        if !safe {
            bail!(
                "the server named a {what} {name:?}, which is not a {what} name; is it an OmniState API?"
            );
        }
    }
    Ok(format!("tokens.{collection}.{mode}.json"))
}

fn write(path: &Path, tokens: &Value) -> Result<()> {
    std::fs::write(path, format!("{}\n", tokens.pretty()))
        .map_err(|e| CliError::new(format!("cannot write {}: {e}", path.display())))
}

/// `omni tokens export [--out <file>] [--per-mode --out <dir>]`
pub fn export(args: &Args) -> Result<()> {
    const USAGE: &str = "omni tokens export [--out <file>] [--per-mode --out <dir>]";
    if args.per_mode && args.out.is_none() {
        bail!(
            "--per-mode writes one file per collection and mode, so it needs --out <directory>: {USAGE}"
        );
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let path = format!("{}/tokens", project_path(&agent, &project));
    let mut query: Vec<String> = Vec::new();
    let read = |query: &[String]| -> Result<crate::http::Response> {
        let full = match query {
            [] => path.clone(),
            query => format!("{path}?{}", query.join("&")),
        };
        agent.request("GET", &full, None)
    };

    let response = read(&query)?;
    let text = response.text();
    if args.json && !args.per_mode && args.out.is_none() {
        say(text.trim_end());
        return Ok(());
    }
    let answer = parsed(&text, "the tokens")?;
    let Some(tokens) = answer
        .get("tokens")
        .filter(|tokens| **tokens != Value::Null)
    else {
        bail!(
            "the design system has no tokens in its head yet, so there are none to export; `omni tokens import <file>` adds them"
        )
    };

    if !args.per_mode {
        match args.out.as_deref() {
            None => say(&tokens.pretty()),
            Some(out) => {
                write(Path::new(out), tokens)?;
                say(&format!("wrote {out}"));
            }
        }
        return Ok(());
    }

    let dir = Path::new(args.out.as_deref().unwrap_or_default());
    // Every read comes before the first write, so a failure leaves no half.
    let mut files = vec![("tokens.json".to_string(), tokens.clone())];
    for (collection, modes) in collections_of(&answer) {
        for mode in modes {
            let name = file_name_of(collection, mode)?;
            query.push(format!("mode={collection}:{mode}"));
            let response = read(&query)?;
            query.pop();
            let one = parsed(&response.text(), "the tokens of a mode")?;
            let Some(group) = one.get("tokens").and_then(|tree| tree.get(collection)) else {
                bail!(
                    "the server's answer has no tokens for {collection} in the mode {mode}; is it an OmniState API?"
                );
            };
            files.push((
                name,
                Value::Object(vec![(collection.to_string(), group.clone())]),
            ));
        }
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| CliError::new(format!("cannot make the directory {}: {e}", dir.display())))?;
    for (name, tree) in &files {
        let path = dir.join(name);
        write(&path, tree)?;
        say(&format!("wrote {}", path.display()));
    }
    Ok(())
}

/// The types `--type` may create.
const NEW_TYPES: [&str; 5] = ["color", "dimension", "number", "string", "boolean"];

/// The type that a top-level group gives its tokens when it states none.
fn implicit_type(top: &str) -> Option<&'static str> {
    match top {
        "color" => Some("color"),
        "spacing" | "radius" => Some("dimension"),
        "typography" => Some("typography"),
        "shadow" => Some("shadow"),
        _ => None,
    }
}

fn refuse<T>(message: String) -> Result<T> {
    Err(CliError::with_code(2, message))
}

fn path_parts(path: &str) -> Result<Vec<&str>> {
    let parts: Vec<&str> = path.split('.').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || part.starts_with('$'))
    {
        return refuse(format!(
            "`{path}` is not a token path: dotted names such as state.count, none empty or starting with $"
        ));
    }
    Ok(parts)
}

fn is_token(node: &Json) -> bool {
    node.get("$value").is_some()
}

/// Reads the head's token tree; `None` while the design system has none.
fn read_tree(agent: &Agent, path: &str) -> Result<Option<Json>> {
    let response = agent.request("GET", path, None)?;
    let answer: Json = serde_json::from_str(&response.text()).map_err(|_| {
        CliError::new(
            "the server's answer about the tokens is not readable; is it an OmniState API?",
        )
    })?;
    Ok(answer
        .get("tokens")
        .filter(|tokens| !tokens.is_null())
        .cloned())
}

/// Puts the tree back after the reader of the tree has accepted it.
fn put_tree(args: &Args, agent: &Agent, path: &str, tree: &Json, done: &str) -> Result<()> {
    if let Err(error) = TokenTree::from_json(tree) {
        return refuse(error.to_string());
    }
    let body = json!({ "tokens": tree }).to_string();
    let response = agent.request("PUT", path, Some(body.as_bytes()))?;
    if args.json {
        say(response.text().trim_end());
    } else {
        say(done);
    }
    Ok(())
}

/// The modes of the top-level group `top`: its own list, or the root's.
fn modes_in(tree: &Json, top: &str) -> Vec<String> {
    let listed = |node: Option<&Json>| -> Option<Vec<String>> {
        let modes = node?.get("$extensions")?.get("omnistate")?.get("modes")?;
        Some(
            modes
                .as_array()?
                .iter()
                .filter_map(|mode| mode.as_str().map(str::to_string))
                .collect(),
        )
    };
    listed(tree.get(top))
        .or_else(|| listed(Some(tree)))
        .unwrap_or_default()
}

/// The type of the token at `parts`: its own, the nearest group's, or the
/// implicit one of its top-level group.
fn type_at(tree: &Json, parts: &[&str]) -> Option<String> {
    let mut found = implicit_type(parts[0]).map(str::to_string);
    let mut node = tree;
    for part in parts {
        node = node.get(*part)?;
        if let Some(ty) = node.get("$type").and_then(Json::as_str) {
            found = Some(ty.to_string());
        }
    }
    found
}

/// `omni tokens set <path> <value> [--mode <m>] [--type <t>]`
pub fn set(args: &Args, path: &str, value: &str) -> Result<()> {
    let parts = path_parts(path)?;
    if let Some(ty) = args.token_type.as_deref() {
        if !NEW_TYPES.contains(&ty) {
            return refuse(format!(
                "`{ty}` is not a type to create a token with: use {}",
                NEW_TYPES.join(", ")
            ));
        }
        if parts.len() < 2 {
            return refuse(format!(
                "`{path}` is a collection, so a token needs a path inside it, such as {path}.x"
            ));
        }
    }
    let value: Json = serde_json::from_str(value).unwrap_or_else(|_| Json::String(value.into()));

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let tokens = format!("{}/tokens", project_path(&agent, &project));
    // A design system without tokens starts from the default modes, which
    // the tree needs at its root.
    let mut tree = read_tree(&agent, &tokens)?.unwrap_or_else(
        || json!({ "$extensions": { "omnistate": { "modes": ["light", "dark"] } } }),
    );

    // Find how much of the path is there.
    let mut node = &tree;
    let mut at = 0;
    while at < parts.len() {
        match node.get(parts[at]) {
            Some(child) => node = child,
            None => break,
        }
        at += 1;
        if is_token(node) && at < parts.len() {
            return refuse(format!(
                "`{}` is a token, so `{path}` cannot be inside it",
                parts[..at].join(".")
            ));
        }
    }
    let exists = at == parts.len() && is_token(node);
    if at == parts.len() && !exists {
        return match args.token_type {
            Some(_) => refuse(format!("`{path}` is a group, not a token")),
            None => refuse(format!("`{path}` is not a token; pass --type to create it")),
        };
    }
    if !exists && args.token_type.is_none() {
        return refuse(format!("`{path}` is not a token; pass --type to create it"));
    }
    if exists
        && let Some(ty) = args.token_type.as_deref()
        && let Some(has) = type_at(&tree, &parts)
        && has != ty
    {
        return refuse(format!("`{path}` is a {has} token"));
    }

    let modes = modes_in(&tree, parts[0]);
    if let Some(mode) = args.mode.as_deref()
        && !modes.iter().any(|listed| listed == mode)
    {
        return refuse(format!("`{mode}` is not a mode of `{}`", parts[0]));
    }

    // Walk down again, making the groups and the token that are missing.
    let mut node = &mut tree;
    for part in &parts[..parts.len() - 1] {
        node = node
            .as_object_mut()
            .expect("a group is an object")
            .entry(*part)
            .or_insert_with(|| Json::Object(Map::new()));
    }
    let token = node
        .as_object_mut()
        .expect("a group is an object")
        .entry(parts[parts.len() - 1])
        .or_insert_with(|| {
            let ty = args.token_type.as_deref().unwrap_or_default();
            json!({ "$type": ty, "$value": value })
        })
        .as_object_mut()
        .expect("a token is an object");
    match args.mode.as_deref() {
        None => {
            token.insert("$value".into(), value.clone());
        }
        Some(mode) => {
            // A token with mode values has one for every mode: start them
            // from the value the token has.
            let current = token.get("$value").cloned().unwrap_or(Json::Null);
            let omnistate = token
                .entry("$extensions")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .and_then(|extensions| {
                    extensions
                        .entry("omnistate")
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                });
            let Some(omnistate) = omnistate else {
                return refuse(format!("`{path}` has $extensions that are not objects"));
            };
            let values = omnistate
                .entry("modes")
                .or_insert_with(|| {
                    Json::Object(
                        modes
                            .iter()
                            .map(|listed| (listed.clone(), current.clone()))
                            .collect(),
                    )
                })
                .as_object_mut();
            let Some(values) = values else {
                return refuse(format!("`{path}` has mode values that are not an object"));
            };
            values.insert(mode.to_string(), value.clone());
        }
    }

    let shown = match args.mode.as_deref() {
        None => format!("{path} = {value}"),
        Some(mode) => format!("{path} ({mode}) = {value}"),
    };
    put_tree(args, &agent, &tokens, &tree, &shown)
}

/// The paths of the tokens under `node`, in the tree's order.
fn token_paths(node: &Json, prefix: &str, found: &mut Vec<String>) {
    let Some(object) = node.as_object() else {
        return;
    };
    for (name, child) in object {
        if name.starts_with('$') {
            continue;
        }
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if is_token(child) {
            found.push(path);
        } else {
            token_paths(child, &path, found);
        }
    }
}

fn mentions(value: &Json, reference: &str) -> bool {
    match value {
        Json::String(text) => text.contains(reference),
        Json::Array(items) => items.iter().any(|item| mentions(item, reference)),
        Json::Object(fields) => fields.values().any(|item| mentions(item, reference)),
        _ => false,
    }
}

/// `omni tokens remove <path>`
pub fn remove(args: &Args, path: &str) -> Result<()> {
    let parts = path_parts(path)?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let tokens = format!("{}/tokens", project_path(&agent, &project));
    let Some(mut tree) = read_tree(&agent, &tokens)? else {
        return refuse(format!("`{path}` is not a token"));
    };
    let mut node = &tree;
    for part in &parts {
        match node.get(*part) {
            Some(child) => node = child,
            None => return refuse(format!("`{path}` is not a token")),
        }
    }
    if !is_token(node) {
        return refuse(format!("`{path}` is not a token"));
    }

    let reference = format!("{{{path}}}");
    let mut all = Vec::new();
    token_paths(&tree, "", &mut all);
    for other in all.iter().filter(|other| *other != path) {
        let token = other
            .split('.')
            .try_fold(&tree, |node, part| node.get(part));
        let modes = token
            .and_then(|token| token.get("$extensions"))
            .and_then(|extensions| extensions.get("omnistate"))
            .and_then(|omnistate| omnistate.get("modes"));
        if token.is_some_and(|token| mentions(&token["$value"], &reference))
            || modes.is_some_and(|modes| mentions(modes, &reference))
        {
            return refuse(format!("`{other}` refers to `{path}`: change it first"));
        }
    }

    // Take the token out, then every group that has no token left.
    for depth in (1..=parts.len()).rev() {
        let mut parent = &mut tree;
        for part in &parts[..depth - 1] {
            parent = &mut parent[*part];
        }
        let object = parent.as_object_mut().expect("a group is an object");
        object.remove(parts[depth - 1]);
        if object.keys().any(|name| !name.starts_with('$')) || depth == 1 {
            break;
        }
    }
    put_tree(args, &agent, &tokens, &tree, &format!("removed {path}"))
}
