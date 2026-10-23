//! `omni feature`: create, list and show the features of a project (Q51). A
//! feature is a part of a project with docs, change sets and threads of its
//! own. Agents create features; marking one done is the human's.

use crate::args::Args;
use crate::changesets::{is_uuid, project_id, projects_path};
use crate::commands::{Agent, agent, say};
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

fn features_path(agent: &Agent, project_id: &str) -> String {
    format!("{}/{project_id}/features", projects_path(agent))
}

/// One line: ID, status and name.
fn summary(feature: &Value) -> String {
    let field = |name: &str| feature.str_at(name).unwrap_or("?");
    format!("{}  {}  {}", field("id"), field("status"), field("name"))
}

pub fn create(args: &Args, name: Option<&str>) -> Result<()> {
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        bail!("a feature needs a name: omni feature create <name> [--description <text>]");
    }
    let mut body = format!("{{\"name\":{}", quote(name));
    if let Some(description) = &args.description {
        body.push_str(&format!(",\"description\":{}", quote(description)));
    }
    body.push('}');
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request(
        "POST",
        &features_path(&agent, &project),
        Some(body.as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the new feature")?;
    let feature = body.get("feature").unwrap_or(&body);
    say(&format!(
        "{}  {}",
        feature.str_at("id").unwrap_or("?"),
        feature.str_at("name").unwrap_or(name)
    ));
    Ok(())
}

pub fn list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let response = agent.request("GET", &features_path(&agent, &project), None)?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the features")?;
    let features = body
        .get("features")
        .and_then(Value::as_array)
        .unwrap_or_default();
    if features.is_empty() {
        say("no features");
    }
    for feature in features {
        say(&summary(feature));
    }
    Ok(())
}

pub fn show(args: &Args, feature: Option<&str>) -> Result<()> {
    let Some(given) = feature else {
        bail!("`omni feature show` needs a feature name or ID; `omni feature list` shows them");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let path = features_path(&agent, &project);
    let text = if is_uuid(given) {
        let response = agent.request(
            "GET",
            &format!("{path}/{}", given.to_ascii_lowercase()),
            None,
        )?;
        response.text()
    } else {
        let response = agent.request("GET", &path, None)?;
        let listed = parsed(&response.text(), "the features")?;
        let found = listed
            .get("features")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .find(|feature| {
                feature
                    .str_at("name")
                    .is_some_and(|name| name.eq_ignore_ascii_case(given.trim()))
            })
            .ok_or_else(|| {
                CliError::new(format!(
                    "no feature named {given:?}; `omni feature list` shows the features"
                ))
            })?;
        format!("{{\"feature\":{found}}}")
    };
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the feature")?;
    let feature = body.get("feature").unwrap_or(&body);
    say(&summary(feature));
    say(&format!(
        "project: {}",
        feature.str_at("project_id").unwrap_or("?")
    ));
    if let Some(description) = feature.str_at("description").filter(|d| !d.is_empty()) {
        say("description:");
        for line in description.lines() {
            say(&format!("  {line}"));
        }
    }
    Ok(())
}
