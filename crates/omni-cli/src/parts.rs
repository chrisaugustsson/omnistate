//! `omni part`: create, list, show, rename and delete the parts of a project,
//! its apps and services. A delete is soft; the name can be used again.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::projects::{is_uuid, project_id, projects_path};

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

fn parts_path(agent: &Agent, project_id: &str) -> String {
    format!("{}/{project_id}/parts", projects_path(agent))
}

/// One line: name and kind, with the ID in front.
fn summary(part: &Value) -> String {
    let field = |name: &str| part.str_at(name).unwrap_or("?");
    format!("{}  {}  {}", field("id"), field("name"), field("kind"))
}

/// The ID of the part `given` names: an ID as it is, a name among the parts
/// that are not deleted, ignoring case.
pub(crate) fn resolve_part(agent: &Agent, project_id: &str, given: &str) -> Result<String> {
    if is_uuid(given) {
        return Ok(given.to_ascii_lowercase());
    }
    find_part(agent, project_id, given)?
        .str_at("id")
        .map(str::to_string)
        .ok_or_else(|| CliError::new("the server's answer about the parts is not readable"))
}

fn find_part(agent: &Agent, project_id: &str, given: &str) -> Result<Value> {
    let response = agent.request("GET", &parts_path(agent, project_id), None)?;
    let listed = parsed(&response.text(), "the parts")?;
    listed
        .get("parts")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .find(|part| {
            part.str_at("name")
                .is_some_and(|name| name.eq_ignore_ascii_case(given.trim()))
        })
        .cloned()
        .ok_or_else(|| {
            CliError::new(format!(
                "no part named {given:?}; `omni part list` shows the parts"
            ))
        })
}

/// `omni part create <name> --kind app|service [--description <text>]`
pub fn create(args: &Args, name: Option<&str>) -> Result<()> {
    let usage = "omni part create <name> --kind app|service [--description <text>]";
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        bail!("a part needs a name: {usage}");
    }
    let kind = match args.kind.as_deref() {
        Some(kind @ ("app" | "service")) => kind,
        Some(_) => bail!("kind must be app or service"),
        None => bail!("a part needs a kind: {usage}"),
    };
    let mut body = format!("{{\"name\":{},\"kind\":{}", quote(name), quote(kind));
    if let Some(description) = &args.description {
        body.push_str(&format!(",\"description\":{}", quote(description)));
    }
    body.push('}');
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request("POST", &parts_path(&agent, &project), Some(body.as_bytes()))?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the new part")?;
    let part = body.get("part").unwrap_or(&body);
    say(&format!(
        "{}  {}  {}",
        part.str_at("id").unwrap_or("?"),
        part.str_at("name").unwrap_or(name),
        part.str_at("kind").unwrap_or(kind)
    ));
    Ok(())
}

pub fn list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request("GET", &parts_path(&agent, &project), None)?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the parts")?;
    let parts = body
        .get("parts")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if parts.is_empty() {
        say("no parts");
    }
    for part in parts {
        let field = |name: &str| part.str_at(name).unwrap_or("?");
        say(&format!("{}  {}", field("name"), field("kind")));
    }
    Ok(())
}

pub fn show(args: &Args, part: Option<&str>) -> Result<()> {
    let Some(given) = part else {
        bail!("`omni part show` needs a part name or ID; `omni part list` shows them");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let text = if is_uuid(given) {
        let response = agent.request(
            "GET",
            &format!(
                "{}/{}",
                parts_path(&agent, &project),
                given.to_ascii_lowercase()
            ),
            None,
        )?;
        response.text()
    } else {
        format!("{{\"part\":{}}}", find_part(&agent, &project, given)?)
    };
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the part")?;
    let part = body.get("part").unwrap_or(&body);
    say(&summary(part));
    say(&format!(
        "project: {}",
        part.str_at("project_id").unwrap_or("?")
    ));
    if let Some(description) = part.str_at("description").filter(|d| !d.is_empty()) {
        say("description:");
        for line in description.lines() {
            say(&format!("  {line}"));
        }
    }
    Ok(())
}

/// `omni part rename <name|id> <new name>`
pub fn rename(args: &Args, part: Option<&str>, name: Option<&str>) -> Result<()> {
    let usage = "omni part rename <name|id> <new name>";
    let Some(given) = part else {
        bail!("a part rename needs the part and its new name: {usage}");
    };
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        bail!("a part rename needs a new name: {usage}");
    }
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let id = resolve_part(&agent, &project, given)?;
    let response = agent.request(
        "PATCH",
        &format!("{}/{id}", parts_path(&agent, &project)),
        Some(format!("{{\"name\":{}}}", quote(name)).as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the renamed part")?;
    let part = body.get("part").unwrap_or(&body);
    say(&format!(
        "{}  {}  {}",
        part.str_at("id").unwrap_or(&id),
        part.str_at("name").unwrap_or(name),
        part.str_at("kind").unwrap_or("?")
    ));
    Ok(())
}

/// `omni part delete <name|id>`
pub fn delete(args: &Args, part: Option<&str>) -> Result<()> {
    let Some(given) = part else {
        bail!("`omni part delete` needs a part name or ID: omni part delete <name|id>");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let id = resolve_part(&agent, &project, given)?;
    agent.request(
        "DELETE",
        &format!("{}/{id}", parts_path(&agent, &project)),
        None,
    )?;
    if args.json {
        say(&format!("{{\"deleted\":{}}}", quote(&id)));
    } else {
        say(&format!("deleted part {id}"));
    }
    Ok(())
}
