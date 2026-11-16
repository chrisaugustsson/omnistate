//! `omni feature`: create, list, show, rename and delete the features of a
//! project's parts, what each part can do. A delete is soft; the name can be
//! used again. `resolve_feature` is the one lookup every `--feature` flag uses.

use crate::args::Args;
use crate::commands::{Agent, agent, say};
use crate::error::{CliError, Result, bail};
use crate::json::{Value, quote};
use crate::parts::resolve_part;
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

fn project_path(agent: &Agent, project_id: &str) -> String {
    format!("{}/{project_id}/parts", projects_path(agent))
}

fn features_path(agent: &Agent, project_id: &str, part_id: &str) -> String {
    format!("{}/{part_id}/features", project_path(agent, project_id))
}

/// A feature found in the project, with the name of its part.
pub struct Found {
    pub part_id: String,
    pub part_name: String,
    pub feature: Value,
}

impl Found {
    pub fn id(&self) -> &str {
        self.feature.str_at("id").unwrap_or_default()
    }
}

/// The undeleted features of the project's undeleted parts, or of the one
/// part `part` names.
pub fn project_features(agent: &Agent, project_id: &str, part: Option<&str>) -> Result<Vec<Found>> {
    let listed = parsed(
        &agent
            .request("GET", &project_path(agent, project_id), None)?
            .text(),
        "the parts",
    )?;
    let only = match part {
        Some(given) => Some(resolve_part(agent, project_id, given)?),
        None => None,
    };
    let mut found = Vec::new();
    for part in listed
        .get("parts")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        let Some(part_id) = part.str_at("id") else {
            continue;
        };
        if only.as_deref().is_some_and(|only| only != part_id) {
            continue;
        }
        let response = agent.request("GET", &features_path(agent, project_id, part_id), None)?;
        let features = parsed(&response.text(), "the features")?;
        for feature in features
            .get("features")
            .and_then(Value::as_array)
            .unwrap_or_default()
        {
            found.push(Found {
                part_id: part_id.to_string(),
                part_name: part.str_at("name").unwrap_or("?").to_string(),
                feature: feature.clone(),
            });
        }
    }
    Ok(found)
}

/// The feature `given` names, an ID or a name among the project's undeleted
/// features, ignoring case; `part` narrows the search to one part. Several
/// matches end with `ambiguous_feature` naming the parts.
pub fn resolve_feature(
    agent: &Agent,
    project_id: &str,
    given: &str,
    part: Option<&str>,
) -> Result<Found> {
    let given = given.trim();
    let by_id = is_uuid(given);
    let mut matches: Vec<Found> = project_features(agent, project_id, part)?
        .into_iter()
        .filter(|found| {
            if by_id {
                found.id().eq_ignore_ascii_case(given)
            } else {
                found
                    .feature
                    .str_at("name")
                    .is_some_and(|name| name.eq_ignore_ascii_case(given))
            }
        })
        .collect();
    match matches.len() {
        0 => bail!("unknown_feature: no feature {given:?}; `omni feature list` shows the features"),
        1 => Ok(matches.remove(0)),
        _ => {
            let parts: Vec<&str> = matches.iter().map(|m| m.part_name.as_str()).collect();
            bail!(
                "ambiguous_feature: {given:?} is a feature of {}; add --part <part> to say which",
                parts.join(", ")
            )
        }
    }
}

/// One line: ID, name and the part it is on.
fn summary(found: &Found) -> String {
    format!(
        "{}  {}  {}",
        found.id(),
        found.feature.str_at("name").unwrap_or("?"),
        found.part_name
    )
}

/// `omni feature create <name> --part <part> [--description <text>]`
pub fn create(args: &Args, name: Option<&str>) -> Result<()> {
    let usage = "omni feature create <name> --part <name|id> [--description <text>]";
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        bail!("a feature needs a name: {usage}");
    }
    let Some(part) = args.part.as_deref() else {
        bail!("part_required: a feature sits on a part: {usage}");
    };
    let mut body = format!("{{\"name\":{}", quote(name));
    if let Some(description) = &args.description {
        body.push_str(&format!(",\"description\":{}", quote(description)));
    }
    body.push('}');
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let part_id = resolve_part(&agent, &project, part)?;
    let response = agent.request(
        "POST",
        &features_path(&agent, &project, &part_id),
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

/// `omni feature list [--part <name|id>]`
pub fn list(args: &Args) -> Result<()> {
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let features = project_features(&agent, &project, args.part.as_deref())?;
    if args.json {
        let items: Vec<String> = features.iter().map(|f| f.feature.to_string()).collect();
        say(&format!("{{\"features\":[{}]}}", items.join(",")));
        return Ok(());
    }
    if features.is_empty() {
        say("no features");
    }
    for found in &features {
        say(&summary(found));
    }
    Ok(())
}

/// `omni feature show <name|id> [--part <name|id>]`
pub fn show(args: &Args, feature: Option<&str>) -> Result<()> {
    let Some(given) = feature else {
        bail!("`omni feature show` needs a feature name or ID; `omni feature list` shows them");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let found = resolve_feature(&agent, &project, given, args.part.as_deref())?;
    if args.json {
        say(&format!("{{\"feature\":{}}}", found.feature));
        return Ok(());
    }
    say(&summary(&found));
    say(&format!("part: {}", found.part_name));
    say(&format!(
        "name: {}",
        found.feature.str_at("name").unwrap_or("?")
    ));
    if let Some(description) = found
        .feature
        .str_at("description")
        .filter(|d| !d.is_empty())
    {
        say("description:");
        for line in description.lines() {
            say(&format!("  {line}"));
        }
    }
    Ok(())
}

/// `omni feature rename <name|id> <new name> [--part <name|id>]`
pub fn rename(args: &Args, feature: Option<&str>, name: Option<&str>) -> Result<()> {
    let usage = "omni feature rename <name|id> <new name>";
    let Some(given) = feature else {
        bail!("a feature rename needs the feature and its new name: {usage}");
    };
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        bail!("a feature rename needs a new name: {usage}");
    }
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let found = resolve_feature(&agent, &project, given, args.part.as_deref())?;
    let response = agent.request(
        "PATCH",
        &format!(
            "{}/{}",
            features_path(&agent, &project, &found.part_id),
            found.id()
        ),
        Some(format!("{{\"name\":{}}}", quote(name)).as_bytes()),
    )?;
    let text = response.text();
    if args.json {
        print_json(&text);
        return Ok(());
    }
    let body = parsed(&text, "the renamed feature")?;
    let renamed = body.get("feature").unwrap_or(&body);
    say(&format!(
        "{}  {}  {}",
        renamed.str_at("id").unwrap_or(found.id()),
        renamed.str_at("name").unwrap_or(name),
        found.part_name
    ));
    Ok(())
}

/// `omni feature delete <name|id> [--part <name|id>]`
pub fn delete(args: &Args, feature: Option<&str>) -> Result<()> {
    let Some(given) = feature else {
        bail!("`omni feature delete` needs a feature name or ID: omni feature delete <name|id>");
    };
    let agent = agent(args)?;
    let project = project_id(args, &agent)?;
    let found = resolve_feature(&agent, &project, given, args.part.as_deref())?;
    agent.request(
        "DELETE",
        &format!(
            "{}/{}",
            features_path(&agent, &project, &found.part_id),
            found.id()
        ),
        None,
    )?;
    if args.json {
        say(&format!("{{\"deleted\":{}}}", quote(found.id())));
    } else {
        say(&format!("deleted feature {}", found.id()));
    }
    Ok(())
}
