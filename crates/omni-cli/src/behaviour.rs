//! `omni behaviour add` and `read`: the behaviour tree of a feature. A node is
//! named by its uuid, by its path (the slugs from the story down, joined by
//! `/`) or by `.` for the root. The project and the feature are found as for
//! `omni story add`.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::projects::{creation_feature, is_uuid, project_id};

const ADD_USAGE: &str =
    "omni behaviour add --title <t> [--slug <s>] [--summary <text>] [--parent <node>]";
const READ_USAGE: &str = "omni behaviour read <node>";

fn unreadable() -> CliError {
    CliError::new(
        "the server's answer about the behaviour tree is not readable; is it an OmniState API?",
    )
}

/// The behaviour path of the feature the command works in.
fn behaviour_path(args: &Args, agent: &Agent) -> Result<String> {
    let project = project_id(args, agent)?;
    let Some(feature) = creation_feature(args, agent, &project)? else {
        bail!("no feature: pass --feature <name or id>, or put feature = \"name\" in .omni.toml");
    };
    Ok(format!(
        "/api/v1/w/{}/projects/{project}/features/{feature}/behaviour",
        agent.workspace_id
    ))
}

/// The nodes of the feature, in tree order.
fn list_nodes(agent: &Agent, base: &str) -> Result<Vec<Value>> {
    let response = agent.request("GET", base, None)?;
    let body = Value::parse(&response.text()).map_err(|_| unreadable())?;
    Ok(body
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(unreadable)?
        .to_vec())
}

fn version_of(node: &Value) -> Result<i64> {
    match node.get("version") {
        Some(Value::Number(n)) => n.parse().map_err(|_| unreadable()),
        _ => Err(unreadable()),
    }
}

/// The id and the version of the node that `given` names: a uuid, a path, or
/// `.` for the root.
pub(crate) fn node_ref(agent: &Agent, base: &str, given: &str) -> Result<(String, i64)> {
    let nodes = list_nodes(agent, base)?;
    let path = if given == "." {
        ""
    } else {
        given.trim_matches('/')
    };
    let found = nodes.iter().find(|node| {
        if is_uuid(given) {
            node.str_at("id") == Some(given.to_ascii_lowercase().as_str())
        } else {
            node.str_at("path") == Some(path)
        }
    });
    let Some(found) = found else {
        bail!("not_found: no node {given} in this feature");
    };
    let id = found.str_at("id").ok_or_else(unreadable)?;
    Ok((id.to_string(), version_of(found)?))
}

/// `omni behaviour add --title <t> [--slug <s>] [--summary <text>] [--parent <node>]`
pub fn add(args: &Args) -> Result<()> {
    let Some(title) = &args.title else {
        bail!("this command needs a title: {ADD_USAGE}");
    };
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let mut fields = vec![format!("\"title\":{}", quote(title))];
    if let Some(slug) = &args.slug {
        fields.push(format!("\"slug\":{}", quote(slug)));
    }
    if let Some(summary) = &args.summary {
        fields.push(format!("\"summary\":{}", quote(summary)));
    }
    if let Some(parent) = &args.parent {
        let (id, _) = node_ref(&agent, &base, parent)?;
        fields.push(format!("\"parent_id\":{}", quote(&id)));
    }
    let body = format!("{{{}}}", fields.join(","));
    let response = agent.request("POST", &format!("{base}/nodes"), Some(body.as_bytes()))?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable())?;
    match answer.get("node").unwrap_or(&answer).str_at("path") {
        Some(path) => say(path),
        None => bail!("the server's answer has no node path; try again"),
    }
    Ok(())
}

/// `omni behaviour read <node>`
pub fn read(args: &Args, node: Option<&str>) -> Result<()> {
    let Some(given) = node else {
        bail!("this command needs a node: {READ_USAGE}");
    };
    let agent = agent(args)?;
    let base = behaviour_path(args, &agent)?;
    let (id, _) = node_ref(&agent, &base, given)?;
    let response = agent.request("GET", &format!("{base}/nodes/{id}"), None)?;
    let text = response.text();
    if args.json {
        say(text.trim_end());
        return Ok(());
    }
    let answer = Value::parse(&text).map_err(|_| unreadable())?;
    let node = answer.get("node").ok_or_else(unreadable)?;
    let path = match node.str_at("path") {
        Some("") => ".",
        Some(path) => path,
        None => "?",
    };
    let level = match node.get("level") {
        Some(Value::Number(n)) => n.as_str(),
        _ => "?",
    };
    say(&format!(
        "{path}  level {level}  {}",
        node.str_at("status").unwrap_or("?")
    ));
    say(node.str_at("title").unwrap_or_default());
    if let Some(summary) = node.str_at("summary").filter(|s| !s.is_empty()) {
        say(summary);
    }
    Ok(())
}
