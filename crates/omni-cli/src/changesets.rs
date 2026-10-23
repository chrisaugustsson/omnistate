//! `omni project` and `omni changeset`: create and list projects, create, list
//! and show change sets, move them along their lifecycle (submit, reopen,
//! abandon) and rebase them, list their conflicts and resolve those. Rejecting
//! and approving are the human's, in the web app, so they have no command.

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

pub(crate) fn projects_path(agent: &Agent) -> String {
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

/// The ID of the feature `omni changeset create` and `omni doc create` work in: `--feature`, else
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

// Change sets -------------------------------------------------------------------

pub(crate) fn changesets_path(agent: &Agent, project_id: &str) -> String {
    format!(
        "/api/v1/w/{}/projects/{project_id}/changesets",
        agent.workspace_id
    )
}

pub(crate) fn changeset_id(id: &str) -> Result<String> {
    if is_uuid(id) {
        Ok(id.to_ascii_lowercase())
    } else {
        bail!("{id:?} is not a change set ID; `omni changeset list` shows them")
    }
}

/// One line: ID, status and title.
pub(crate) fn summary(changeset: &Value) -> String {
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
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let feature = creation_feature(args, &agent, &project)?;
    let mut fields = vec![("title", title.to_string())];
    fields.extend(description.map(|text| ("description", text)));
    fields.extend(args.run.clone().map(|tag| ("run_tag", tag)));
    fields.extend(feature.map(|id| ("feature_id", id)));
    let body = format!(
        "{{{}}}",
        fields
            .iter()
            .map(|(name, value)| format!("\"{name}\":{}", quote(value)))
            .collect::<Vec<_>>()
            .join(",")
    );
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
    let mut path = changesets_path(&agent, &project);
    match (&args.feature, args.global) {
        (Some(_), true) => bail!("give --feature or --global, not both"),
        (Some(given), false) => {
            path.push_str(&format!(
                "?feature={}",
                resolve_feature(&agent, &project, given)?
            ));
        }
        (None, true) => path.push_str("?feature=global"),
        (None, false) => {}
    }
    let response = agent.request("GET", &path, None)?;
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
    // The names of the features are only looked up when a change set is in one.
    let features = if changesets
        .iter()
        .any(|changeset| changeset.str_at("feature_id").is_some())
    {
        project_features(&agent, &project).unwrap_or_default()
    } else {
        Vec::new()
    };
    for changeset in changesets {
        let scope = match changeset.str_at("feature_id") {
            None => "project",
            Some(id) => features
                .iter()
                .find(|feature| feature.str_at("id") == Some(id))
                .and_then(|feature| feature.str_at("name"))
                .unwrap_or(id),
        };
        let field = |name: &str| changeset.str_at(name).unwrap_or("?");
        say(&format!(
            "{}  {}  {scope}  {}",
            field("id"),
            field("status"),
            field("title")
        ));
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
    let decisions = changeset
        .get("decisions")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if !decisions.is_empty() {
        say("decisions:");
        for decision in decisions {
            say(&format!(
                "  {}  {}",
                decision.str_at("id").unwrap_or("?"),
                decision.str_at("status").unwrap_or("?")
            ));
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

// Rebase and conflicts ----------------------------------------------------------

pub(crate) fn conflict_id(id: &str) -> Result<String> {
    if is_uuid(id) {
        Ok(id.to_ascii_lowercase())
    } else {
        bail!("{id:?} is not a conflict ID; `omni changeset conflicts <id>` lists them")
    }
}

/// One line of text from an HTML fragment: its lines joined by single spaces.
fn one_line(html: &str) -> String {
    html.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One conflict: ID, node, doc and whether it is resolved, then its two sides
/// and, once resolved, the note.
fn conflict_lines(conflict: &Value) -> Vec<String> {
    let field = |name: &str| conflict.str_at(name).unwrap_or("?");
    let resolution = conflict
        .get("resolution")
        .filter(|r| !matches!(r, Value::Null));
    let state = if resolution.is_some() {
        "resolved"
    } else {
        "unresolved"
    };
    let side = |name: &str| match conflict.get(name).and_then(|side| side.str_at("html")) {
        Some(html) => one_line(html),
        None => "removed".to_string(),
    };
    let mut lines = vec![
        format!(
            "{}  node {}  doc {}  {state}",
            field("id"),
            field("node"),
            field("doc_id")
        ),
        format!("  mainline: {}", side("mainline")),
        format!("  changeset: {}", side("changeset")),
    ];
    if let Some(note) = resolution.and_then(|r| r.str_at("note")) {
        lines.push(format!("  resolution: {note}"));
    }
    lines
}

/// `omni changeset rebase <id>`: replays the change set onto the current
/// mainline and prints where it ended up and the conflicts it recorded.
pub fn rebase(args: &Args, id: &str) -> Result<()> {
    let id = changeset_id(id)?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "POST",
        &format!("{}/{id}/rebase", changesets_path(&agent, &project)),
        Some(b"{}"),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the rebase")?;
    say(&summary(body.get("changeset").unwrap_or(&body)));
    let conflicts = body
        .get("conflicts")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if conflicts.is_empty() {
        say("no conflicts");
        return Ok(());
    }
    say(&format!(
        "{} conflicts: fix the draft, then `omni changeset resolve <conflict-id> --note <text>` for each",
        conflicts.len()
    ));
    for conflict in conflicts {
        say(&format!(
            "  {}  node {}  doc {}",
            conflict.str_at("id").unwrap_or("?"),
            conflict.str_at("node").unwrap_or("?"),
            conflict.str_at("doc_id").unwrap_or("?")
        ));
    }
    Ok(())
}

/// `omni changeset conflicts <id>`: lists the change set's conflicts with both
/// sides of each.
pub fn conflicts(args: &Args, id: &str) -> Result<()> {
    let id = changeset_id(id)?;
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "GET",
        &format!("{}/{id}/conflicts", changesets_path(&agent, &project)),
        None,
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the conflicts")?;
    let conflicts = body
        .get("conflicts")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if conflicts.is_empty() {
        say("no conflicts");
    }
    for conflict in conflicts {
        for line in conflict_lines(conflict) {
            say(&line);
        }
    }
    Ok(())
}

/// `omni changeset resolve <conflict-id> --note <text>`: resolves a conflict
/// after the agent fixed the draft with normal operations.
pub fn resolve(args: &Args, id: &str) -> Result<()> {
    let id = conflict_id(id)?;
    let note = args.note.as_deref().map(str::trim).unwrap_or_default();
    if note.is_empty() {
        bail!(
            "resolving a conflict needs a note about how it was fixed: omni changeset resolve <conflict-id> --note <text>"
        );
    }
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let body = format!("{{\"note\":{}}}", quote(note));
    let response = agent.request(
        "POST",
        &format!(
            "/api/v1/w/{}/projects/{project}/conflicts/{id}/resolve",
            agent.workspace_id
        ),
        Some(body.as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let answer = parsed(&text, "the conflict")?;
    let conflict = answer.get("conflict").unwrap_or(&answer);
    say(&format!(
        "resolved conflict {}  node {}",
        conflict.str_at("id").unwrap_or(&id),
        conflict.str_at("node").unwrap_or("?")
    ));
    Ok(())
}
