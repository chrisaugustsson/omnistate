//! `omni project` and `omni changeset`: create and list projects, create, list
//! and show change sets, and move them along their lifecycle (submit, reopen,
//! abandon). Rejecting is the human's, in the web app, so it has no command.

use crate::args::Args;
use crate::commands::{Agent, agent, read_body, say};
use crate::config;
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};

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

/// A UUID in its 36-character form, which is all that goes into a path.
pub(crate) fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

// Projects ----------------------------------------------------------------------

pub fn project_create(args: &Args, name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("the project name is empty: omni project create <name>");
    }
    let agent = agent(args)?;
    let body = format!("{{\"name\":{}}}", quote(name.trim()));
    let response = agent.request("POST", &projects_path(&agent), Some(body.as_bytes()))?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let project = parsed(&text, "the new project")?;
    let project = project.get("project").unwrap_or(&project);
    say(&format!(
        "created project {} ({})",
        project.str_at("name").unwrap_or(name),
        project.str_at("id").unwrap_or("?")
    ));
    Ok(())
}

pub fn project_list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let response = agent.request("GET", &projects_path(&agent), None)?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the projects")?;
    let projects = body
        .get("projects")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if projects.is_empty() {
        say("no projects");
    }
    for project in projects {
        say(&format!(
            "{}  {}",
            project.str_at("id").unwrap_or("?"),
            project.str_at("name").unwrap_or("?")
        ));
    }
    Ok(())
}

fn projects_path(agent: &Agent) -> String {
    format!("/api/v1/w/{}/projects", agent.workspace_id)
}

/// The ID of the project a command works in: `--project`, else the project of
/// `.omni.toml`. Either is a project ID, used as it is, or a project name,
/// which the API's list of projects turns into its ID.
pub(crate) fn project_id(args: &Args, agent: &Agent) -> Result<String> {
    let given = match args.project.clone() {
        Some(project) => project,
        None => config::project()?.ok_or_else(|| {
            CliError::new(
                "no project: pass --project <name or id>, or put project = \"name\" in .omni.toml",
            )
        })?,
    };
    if is_uuid(&given) {
        return Ok(given.to_ascii_lowercase());
    }
    let response = agent.request("GET", &projects_path(agent), None)?;
    let body = parsed(&response.text(), "the projects")?;
    body.get("projects")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .find(|project| project.str_at("name") == Some(given.as_str()))
        .and_then(|project| project.str_at("id"))
        .map(str::to_string)
        .ok_or_else(|| {
            CliError::new(format!(
                "no project named {given:?}; `omni project list` shows the projects"
            ))
        })
}

// Change sets -------------------------------------------------------------------

fn changesets_path(agent: &Agent, project_id: &str) -> String {
    format!(
        "/api/v1/w/{}/projects/{project_id}/changesets",
        agent.workspace_id
    )
}

fn changeset_id(id: &str) -> Result<String> {
    if is_uuid(id) {
        Ok(id.to_ascii_lowercase())
    } else {
        bail!("{id:?} is not a change set ID; `omni changeset list` shows them")
    }
}

/// One line: ID, status and title.
fn summary(changeset: &Value) -> String {
    let field = |name: &str| changeset.str_at(name).unwrap_or("?");
    format!("{}  {}  {}", field("id"), field("status"), field("title"))
}

pub fn create(args: &Args) -> Result<()> {
    let title = args.title.as_deref().map(str::trim).unwrap_or_default();
    if title.is_empty() {
        bail!("a change set needs a title: omni changeset create --title <title>");
    }
    let description = match (&args.description, &args.body_file) {
        (Some(_), Some(_)) => {
            bail!("give the description with --description or --body-file, not both")
        }
        (Some(text), None) => Some(text.clone()),
        (None, Some(source)) => Some(String::from_utf8(read_body(source)?).map_err(|_| {
            CliError::new(format!(
                "the description in {source} is not valid UTF-8 text"
            ))
        })?),
        (None, None) => None,
    };
    let mut fields = vec![("title", title.to_string())];
    fields.extend(description.map(|text| ("description", text)));
    fields.extend(args.run.clone().map(|tag| ("run_tag", tag)));
    let body = format!(
        "{{{}}}",
        fields
            .iter()
            .map(|(name, value)| format!("\"{name}\":{}", quote(value)))
            .collect::<Vec<_>>()
            .join(",")
    );

    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "POST",
        &changesets_path(&agent, &project),
        Some(body.as_bytes()),
    )?;
    show_changeset_line(args, &response.text(), "created ")
}

pub fn list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request("GET", &changesets_path(&agent, &project), None)?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the change sets")?;
    let changesets = body
        .get("changesets")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if changesets.is_empty() {
        say("no change sets");
    }
    for changeset in changesets {
        say(&summary(changeset));
    }
    Ok(())
}

pub fn show(args: &Args, id: &str) -> Result<()> {
    let id = changeset_id(id)?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "GET",
        &format!("{}/{id}", changesets_path(&agent, &project)),
        None,
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the change set")?;
    let changeset = body.get("changeset").unwrap_or(&body);
    say(&summary(changeset));
    say(&format!(
        "project: {}",
        changeset.str_at("project_id").unwrap_or("?")
    ));
    say(&format!(
        "run: {}",
        changeset.str_at("run_tag").unwrap_or("-")
    ));
    if let Some(description) = changeset.str_at("description").filter(|d| !d.is_empty()) {
        say("description:");
        for line in description.lines() {
            say(&format!("  {line}"));
        }
    }
    say("history:");
    for entry in changeset
        .get("history")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        say(&history_line(entry));
    }
    Ok(())
}

/// One status change: when, from where to where, who did it and, for a
/// rejection, why.
fn history_line(entry: &Value) -> String {
    let field = |name: &str| entry.str_at(name).unwrap_or("?");
    let change = match entry.str_at("from_status") {
        Some(from) => format!("{from} -> {}", field("to_status")),
        None => format!("created as {}", field("to_status")),
    };
    let mut line = format!("  {}  {change}  by {}", field("at"), field("actor"));
    if let Some(reason) = entry.str_at("reason") {
        line.push_str(&format!(": {reason}"));
    }
    line
}

/// `submit`, `reopen` or `abandon`: asks the API to move the change set and
/// prints where it ended up.
pub fn act(args: &Args, id: &str, action: &str) -> Result<()> {
    let id = changeset_id(id)?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "POST",
        &format!("{}/{id}/{action}", changesets_path(&agent, &project)),
        Some(b"{}"),
    )?;
    show_changeset_line(args, &response.text(), "")
}

/// Prints the change set of a create or lifecycle answer: the API's JSON with
/// `--json`, else one line led by `prefix`.
fn show_changeset_line(args: &Args, text: &str, prefix: &str) -> Result<()> {
    if args.json {
        print_json(text);
        return Ok(());
    }
    let body = parsed(text, "the change set")?;
    say(&format!(
        "{prefix}{}",
        summary(body.get("changeset").unwrap_or(&body))
    ));
    Ok(())
}
