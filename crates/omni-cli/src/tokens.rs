//! `omni tokens import` and `omni tokens export`: replace the design tokens of
//! the design system's head with a DTCG file, and write the tokens of the head
//! as DTCG JSON, whole or one file per collection and mode
//! (docs/formats/tokens.md). The
//! project is found as for `omni project`.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::Value;
use crate::projects::project_id;
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
