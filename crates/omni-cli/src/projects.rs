//! `omni project`: create and list projects. The helpers that every other
//! command uses to find its project and feature live here too.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::config;
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::links::{Links, Resolution, resolve};

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

pub(crate) fn projects_path(agent: &Agent) -> String {
    format!("/api/v1/w/{}/projects", agent.workspace_id)
}

/// The ID of the project a command works in: `--project`, else the project
/// the folder is linked to. `--project` is a project ID, used as it is, or a
/// project name, which the API's list of projects turns into its ID.
pub(crate) fn project_id(args: &Args, agent: &Agent) -> Result<String> {
    let given = match args.project.clone() {
        Some(project) => project,
        None => return linked_project(agent),
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

/// The project the current folder or its nearest linked parent is linked to.
fn linked_project(agent: &Agent) -> Result<String> {
    let folder = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|e| CliError::new(format!("cannot find the current folder: {e}")))?;
    match resolve(&Links::load()?, &agent.workspace_id, &folder) {
        Resolution::Linked { project_id, .. } => Ok(project_id),
        Resolution::OtherWorkspace {
            matched,
            workspace_id,
        } => bail!(
            "{} is linked to a project of another workspace ({workspace_id}); run omni link here, or omni login",
            matched.display()
        ),
        Resolution::None => bail!(
            "this folder is not linked to a project: run omni link <project name|id> here, or pass --project"
        ),
    }
}

/// The features of a project, as the API lists them.
pub(crate) fn project_features(agent: &Agent, project_id: &str) -> Result<Vec<Value>> {
    let path = format!("{}/{project_id}/features", projects_path(agent));
    let response = agent.request("GET", &path, None)?;
    let body = parsed(&response.text(), "the features")?;
    Ok(body
        .get("features")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .to_vec())
}

/// The ID of the feature a name or ID given by the user stands for: an ID is
/// used as it is, a name is looked up (ignoring case) in the project's features.
pub(crate) fn resolve_feature(agent: &Agent, project_id: &str, given: &str) -> Result<String> {
    if is_uuid(given) {
        return Ok(given.to_ascii_lowercase());
    }
    project_features(agent, project_id)?
        .iter()
        .find(|feature| {
            feature
                .str_at("name")
                .is_some_and(|name| name.eq_ignore_ascii_case(given.trim()))
        })
        .and_then(|feature| feature.str_at("id"))
        .map(str::to_string)
        .ok_or_else(|| {
            CliError::new(format!(
                "no feature named {given:?}; `omni feature list` shows the features"
            ))
        })
}

/// The ID of the feature `omni doc create` works in: `--feature`, else
/// the feature of `.omni.toml`, else none (project level). `--global` means
/// none whatever `.omni.toml` says.
pub(crate) fn creation_feature(
    args: &Args,
    agent: &Agent,
    project_id: &str,
) -> Result<Option<String>> {
    if args.global {
        if args.feature.is_some() {
            bail!("give --feature or --global, not both");
        }
        return Ok(None);
    }
    let given = match args.feature.clone() {
        Some(feature) => Some(feature),
        None => config::feature()?,
    };
    given
        .map(|given| resolve_feature(agent, project_id, &given))
        .transpose()
}
