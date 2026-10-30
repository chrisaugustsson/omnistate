//! `omni doc`: create a doc, list the project's docs, read its head, show its
//! metadata, edit it with a batch of operations, rename or delete it, and list
//! what was written to it. The project is found as for `omni project`.

use crate::args::Args;
use crate::commands::{Agent, agent, read_body, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::projects::{creation_feature, is_uuid, project_features, project_id, resolve_feature};
use std::io::IsTerminal;

const KINDS: &str = "html, design or design-system";
const PILLARS: &str = "spec, ui, architecture, db or research";

/// The text of an API response, for `--json`.
fn print_json(text: &str) {
    say(text.trim_end());
}

fn parsed(text: &str, what: &str) -> Result<Value> {
    Value::parse(text).map_err(|_| {
        CliError::new(format!(
            "the server's answer about {what} is not readable; is it an OmniState API?"
        ))
    })
}

/// The pillar as the API takes it, or an error that lists the five.
fn known_pillar(pillar: &str) -> Result<&str> {
    match pillar {
        "spec" | "ui" | "architecture" | "db" | "research" => Ok(pillar),
        _ => bail!("--pillar {pillar:?} is not a pillar; use {PILLARS}"),
    }
}

fn doc_id(id: Option<&str>, usage: &str) -> Result<String> {
    match id {
        None => bail!("this command needs the doc's ID: {usage}"),
        Some(id) if is_uuid(id) => Ok(id.to_ascii_lowercase()),
        Some(id) => bail!("{id:?} is not a doc ID (a UUID); `omni doc show <doc>` takes the ID"),
    }
}

fn project_path(agent: &Agent, project: &str) -> String {
    format!("/api/v1/w/{}/projects/{project}", agent.workspace_id)
}

/// `omni doc create [--feature <name or id>] --kind <kind> --pillar <pillar> --title <text>`
pub fn create(args: &Args) -> Result<()> {
    const USAGE: &str = "omni doc create [--feature <name|id>] --kind html|design|design-system --pillar spec|ui|architecture|db|research --title <text>";
    let kind = match args.kind.as_deref() {
        None => bail!("a doc needs --kind, one of {KINDS}: {USAGE}"),
        Some(kind @ ("html" | "design" | "design-system")) => kind,
        Some(kind) => bail!("--kind {kind:?} is not a kind of doc; use {KINDS}"),
    };
    let pillar = match args.pillar.as_deref() {
        None => bail!("a doc needs --pillar, one of {PILLARS}: {USAGE}"),
        Some(pillar) => known_pillar(pillar)?,
    };
    if kind != "html" && pillar != "ui" {
        bail!(
            "a {kind} doc does not go in the {pillar} pillar: design and design-system docs are ui, html docs go in any pillar"
        );
    }
    let title = args.title.as_deref().map(str::trim).unwrap_or_default();
    if title.is_empty() {
        bail!("a doc needs --title <text>: {USAGE}");
    }

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let body = format!(
        "{{\"kind\":{},\"pillar\":{},\"title\":{}}}",
        quote(kind),
        quote(pillar),
        quote(title)
    );
    let path = match creation_feature(args, &agent, &project)? {
        Some(feature) => format!("{}/features/{feature}/docs", project_path(&agent, &project)),
        None => format!("{}/docs", project_path(&agent, &project)),
    };
    let response = agent.request("POST", &path, Some(body.as_bytes()))?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the new doc")?;
    let doc = answer.get("doc").unwrap_or(&answer);
    let field = |value: &Value, name: &str| value.str_at(name).unwrap_or("?").to_string();
    say(&format!(
        "created doc {}  {}  {}",
        field(doc, "id"),
        field(doc, "kind"),
        field(doc, "title")
    ));
    Ok(())
}

/// `omni doc list [--feature <name or id> | --global] [--pillar <pillar>]`
pub fn list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let mut query = Vec::new();
    match (&args.feature, args.global) {
        (Some(_), true) => bail!("give --feature or --global, not both"),
        (Some(given), false) => {
            query.push(format!(
                "feature={}",
                resolve_feature(&agent, &project, given)?
            ));
        }
        (None, true) => query.push("feature=global".to_string()),
        (None, false) => {}
    }
    if let Some(pillar) = args.pillar.as_deref() {
        query.push(format!("pillar={}", known_pillar(pillar)?));
    }
    let mut path = format!("{}/docs", project_path(&agent, &project));
    if !query.is_empty() {
        path.push('?');
        path.push_str(&query.join("&"));
    }
    let response = agent.request("GET", &path, None)?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the docs")?;
    let docs = body
        .get("docs")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if docs.is_empty() {
        say("no docs");
    }
    // The names of the features are only looked up when a doc is in one.
    let features = if docs.iter().any(|doc| doc.str_at("feature_id").is_some()) {
        project_features(&agent, &project).unwrap_or_default()
    } else {
        Vec::new()
    };
    for doc in docs {
        let scope = match doc.str_at("feature_id") {
            None => "project",
            Some(id) => features
                .iter()
                .find(|feature| feature.str_at("id") == Some(id))
                .and_then(|feature| feature.str_at("name"))
                .unwrap_or(id),
        };
        let field = |name: &str| doc.str_at(name).unwrap_or("?");
        say(&format!(
            "{}  {scope}  {}  {}  {}",
            field("id"),
            field("kind"),
            field("pillar"),
            field("title")
        ));
    }
    Ok(())
}

/// `omni doc read <doc>`
pub fn read(args: &Args, doc: Option<&str>) -> Result<()> {
    let doc = doc_id(doc, "omni doc read <doc>")?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let path = format!("{}/docs/{doc}/html", project_path(&agent, &project));
    let response = agent.request("GET", &path, None)?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the doc")?;
    // The head of a design is its tags; every other doc is its HTML, as the API
    // gave it, and nothing else.
    if let Some(tags) = answer.str_at("tags") {
        say(tags);
        return Ok(());
    }
    let Some(html) = answer.str_at("html") else {
        bail!("the server's answer has no HTML for the doc; is it an OmniState API?");
    };
    say(html);
    Ok(())
}

/// `omni doc show <doc>`
pub fn show(args: &Args, doc: Option<&str>) -> Result<()> {
    let doc = doc_id(doc, "omni doc show <doc>")?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "GET",
        &format!("{}/docs/{doc}", project_path(&agent, &project)),
        None,
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the doc")?;
    let doc = answer.get("doc").unwrap_or(&answer);
    let field = |value: &Value, name: &str| value.str_at(name).unwrap_or("?").to_string();
    say(&format!(
        "doc {}  {}",
        field(doc, "id"),
        field(doc, "title")
    ));
    say(&format!(
        "kind: {}  pillar: {}",
        field(doc, "kind"),
        field(doc, "pillar")
    ));
    Ok(())
}

/// `omni doc edit <doc> [--body-file <path|->]`: the batch applies to the doc
/// at once.
pub fn edit(args: &Args, doc: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni doc edit <doc> [--body-file <path|->]";
    let doc = doc_id(doc, USAGE)?;
    let batch = read_batch(args)?;

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let body = format!("{{\"ops\":{}}}", batch.trim());
    let path = format!("{}/docs/{doc}/ops", project_path(&agent, &project));
    let response = agent.request("POST", &path, Some(body.as_bytes()))?;
    print_edit(args, &response.text())
}

/// The batch of operations of an edit command: a JSON array from
/// `--body-file`, or from standard input without it.
pub(crate) fn read_batch(args: &Args) -> Result<String> {
    let source = match args.body_file.as_deref() {
        Some(source) => source,
        None if std::io::stdin().is_terminal() => bail!(
            "no operations to send: pass --body-file <path> or pipe the batch (a JSON array) to standard input"
        ),
        None => "-",
    };
    let bytes = read_body(source)?;
    let batch = String::from_utf8(bytes)
        .map_err(|_| CliError::new("the operations are not valid UTF-8 text"))?;
    match Value::parse(&batch) {
        Ok(Value::Array(_)) => {}
        Ok(_) => bail!(
            "the operations must be a JSON array of operations, such as [{{\"op\":\"delete\",\"node\":\"k3f9x2qa\",\"rev\":1}}]"
        ),
        Err(why) => bail!("the operations are not valid JSON ({why}); send a JSON array"),
    }
    Ok(batch)
}

/// Prints what an edit did: each node and its new revision, the IDs the
/// server assigned and the nodes that went. With `--json`, the API's answer.
pub(crate) fn print_edit(args: &Args, text: &str) -> Result<()> {
    if args.json {
        print_json(text);
        return Ok(());
    }
    let answer = parsed(text, "the edit")?;
    for result in answer
        .get("results")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        let ids = |name: &str| -> Vec<&str> {
            result
                .get(name)
                .and_then(Value::as_array)
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .collect()
        };
        let revision = match result.get("revision") {
            Some(Value::Number(revision)) => revision.as_str(),
            _ => "?",
        };
        let mut line = format!(
            "{}  revision {revision}",
            result.str_at("node").unwrap_or("?")
        );
        for (label, ids) in [("created", ids("created")), ("removed", ids("removed"))] {
            if !ids.is_empty() {
                line.push_str(&format!("  {label} {}", ids.join(" ")));
            }
        }
        say(&line);
    }
    Ok(())
}

/// `omni doc rename <doc> <title>`
pub fn rename(args: &Args, doc: Option<&str>, title: Option<&str>) -> Result<()> {
    let usage = "omni doc rename <doc> <title>";
    let doc = doc_id(doc, usage)?;
    let title = title.map(str::trim).unwrap_or_default();
    if title.is_empty() {
        bail!("a doc rename needs a new title: {usage}");
    }
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "PATCH",
        &format!("{}/docs/{doc}", project_path(&agent, &project)),
        Some(format!("{{\"title\":{}}}", quote(title)).as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the renamed doc")?;
    let renamed = answer.get("doc").unwrap_or(&answer);
    say(&format!(
        "{}  {}",
        renamed.str_at("id").unwrap_or(&doc),
        renamed.str_at("title").unwrap_or(title)
    ));
    Ok(())
}

/// `omni doc delete <doc>`
pub fn delete(args: &Args, doc: Option<&str>) -> Result<()> {
    let doc = doc_id(doc, "omni doc delete <doc>")?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    agent.request(
        "DELETE",
        &format!("{}/docs/{doc}", project_path(&agent, &project)),
        None,
    )?;
    if args.json {
        say(&format!("{{\"deleted\":{}}}", quote(&doc)));
    } else {
        say(&format!("deleted doc {doc}"));
    }
    Ok(())
}

/// A whole number from `--limit` or `--before`, for the query of `activity`.
fn count_option(value: &str, name: &str) -> Result<u32> {
    match value.parse::<u32>() {
        Ok(count) if count >= 1 => Ok(count),
        _ => bail!("{name} {value:?} is not a number of 1 or more"),
    }
}

/// `omni doc activity <doc> [--limit <n>] [--before <seq>]`
pub fn activity(args: &Args, doc: Option<&str>) -> Result<()> {
    const USAGE: &str = "omni doc activity <doc> [--limit <n>] [--before <seq>]";
    let doc = doc_id(doc, USAGE)?;
    let mut query = Vec::new();
    if let Some(limit) = args.limit.as_deref() {
        query.push(format!("limit={}", count_option(limit, "--limit")?));
    }
    if let Some(before) = args.before.as_deref() {
        query.push(format!("before={}", count_option(before, "--before")?));
    }
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let mut path = format!("{}/docs/{doc}/activity", project_path(&agent, &project));
    if !query.is_empty() {
        path.push('?');
        path.push_str(&query.join("&"));
    }
    let response = agent.request("GET", &path, None)?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the activity")?;
    let entries = answer
        .get("entries")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if entries.is_empty() {
        say("no activity");
    }
    for entry in entries {
        let seq = match entry.get("seq") {
            Some(Value::Number(seq)) => seq.as_str(),
            _ => "?",
        };
        let role = entry
            .get("agent")
            .and_then(|agent| agent.str_at("role"))
            .unwrap_or("-");
        say(&format!(
            "{seq}  {}  {role}  {}",
            entry.str_at("created_at").unwrap_or("?"),
            entry.str_at("summary").unwrap_or("?")
        ));
    }
    if let Some(Value::Number(next)) = answer.get("next_before") {
        say(&format!(
            "older entries: omni doc activity {doc} --before {next}"
        ));
    }
    Ok(())
}
